//! Location endpoints that keep clients talking only to this server (spec 5.17, 5.18, 8). The
//! alert endpoints are in `alerts`.
//!
//! - `GET /api/geocode` and `GET /api/reverse-geocode`: the self-hosted geocoder when
//!   `GEOCODER_URL` is set, otherwise Photon. Neither set means 503 with no upstream call.
//!
//! Upstream failures are `502 {"error":"upstream_unavailable"}` with `no-store`, never a cached or
//! empty body.
//!
//! Defect D8 fix: a Photon `osm_id` that is not an integer drops that feature. Python raised
//! `ValueError` and answered 500.

use std::time::Duration;

use axum::extract::State;
use axum::http::{StatusCode, Uri};
use axum::response::Response;
use bytes::Bytes;
use radar_core::pycompat::{float_repr, is_falsy, py_int, py_str};
use radar_core::time::round_digits;
use serde_json::{Map, Value, json};

use crate::params::{Issues, py_float, query_param};
use crate::respond;
use crate::state::AppState;
use crate::upstream;

/// OSM `place=*` values that read as a settlement name.
const SETTLEMENTS: [&str; 10] = [
    "city",
    "town",
    "village",
    "hamlet",
    "municipality",
    "suburb",
    "borough",
    "quarter",
    "neighbourhood",
    "locality",
];

/// Cache key for a point: `"<round(lat,3)>,<round(lon,3)>"` with Python float repr.
#[must_use]
pub fn point_key(lat: f64, lon: f64) -> String {
    format!(
        "{},{}",
        float_repr(round_digits(lat, 3)),
        float_repr(round_digits(lon, 3))
    )
}

fn in_range(lat: f64, lon: f64) -> bool {
    (-90.0..=90.0).contains(&lat) && (-180.0..=180.0).contains(&lon)
}

fn ok_json(body: Bytes, max_age: u64) -> Response {
    respond::json_bytes(
        StatusCode::OK,
        body,
        Some(&respond::public_max_age(max_age)),
    )
}

fn not_configured() -> Response {
    respond::error_code(
        StatusCode::SERVICE_UNAVAILABLE,
        "geocoding_not_configured",
        true,
    )
}

fn geocoding_configured(state: &AppState) -> bool {
    !state.config.geocoder_url.is_empty() || !state.config.photon_url.is_empty()
}

fn upstream_timeout(state: &AppState, factor: f64) -> Duration {
    Duration::from_secs_f64((state.config.places_timeout_s * factor).max(0.001))
}

/// `_place_from_feature` (spec 8.1). Settlements only when `reverse` is false.
#[must_use]
pub fn place_from_feature(feature: &Value, reverse: bool) -> Option<Value> {
    let empty = Map::new();
    let props = feature
        .get("properties")
        .filter(|p| !is_falsy(p))
        .and_then(Value::as_object)
        .unwrap_or(&empty);
    let coords = feature
        .get("geometry")
        .filter(|g| !is_falsy(g))
        .and_then(|g| g.get("coordinates"))
        .filter(|c| !is_falsy(c))
        .and_then(Value::as_array)?;
    if coords.len() < 2 {
        return None;
    }
    let (lon, lat) = (coords[0].clone(), coords[1].clone());
    let is_settlement = props.get("osm_key") == Some(&json!("place"))
        && props
            .get("osm_value")
            .and_then(Value::as_str)
            .is_some_and(|v| SETTLEMENTS.contains(&v));
    let truthy = |key: &str| props.get(key).filter(|v| !is_falsy(v));
    let name = if reverse {
        truthy("city")
            .or_else(|| truthy("locality"))
            .or_else(|| truthy("district"))
            .or_else(|| if is_settlement { truthy("name") } else { None })
            .or_else(|| truthy("county"))?
    } else {
        if !is_settlement {
            return None;
        }
        truthy("name")?
    };
    let osm_id = props.get("osm_id").filter(|v| !v.is_null())?;
    // D8: a non-integer osm_id drops the feature (Python raised and answered 500).
    let id = py_int(osm_id)?;
    let country_code = match props.get("countrycode").and_then(Value::as_str) {
        Some(code) => json!(code.to_uppercase()),
        None => Value::Null,
    };
    Some(json!({
        "id": id,
        "name": name.clone(),
        "latitude": lat,
        "longitude": lon,
        "admin1": props.get("state").cloned().unwrap_or(Value::Null),
        "country": props.get("country").cloned().unwrap_or(Value::Null),
        "countryCode": country_code,
    }))
}

/// Python `float(value)` for the JSON shapes a geocoder row can carry.
fn to_f64(value: &Value) -> Option<f64> {
    match value {
        Value::Number(n) => n.as_f64(),
        Value::Bool(b) => Some(if *b { 1.0 } else { 0.0 }),
        Value::String(s) => py_float(s),
        _ => None,
    }
}

/// `_place_from_geocoder` (spec 8.2). `countryCode` is passed through without uppercasing.
#[must_use]
pub fn place_from_geocoder(row: &Value) -> Option<Value> {
    let obj = row.as_object()?;
    let id = py_int(obj.get("id")?)?;
    let name = py_str(obj.get("name")?);
    let lat = to_f64(obj.get("lat")?)?;
    let lon = to_f64(obj.get("lon")?)?;
    Some(json!({
        "id": id,
        "name": name,
        "latitude": lat,
        "longitude": lon,
        "admin1": obj.get("admin1").cloned().unwrap_or(Value::Null),
        "country": obj.get("country").cloned().unwrap_or(Value::Null),
        "countryCode": obj.get("country_code").cloned().unwrap_or(Value::Null),
    }))
}

/// Set a place's coordinates to the requested (rounded) point.
fn set_point(place: &mut Value, lat: f64, lon: f64) {
    if let Some(obj) = place.as_object_mut() {
        obj.insert("latitude".into(), json!(lat));
        obj.insert("longitude".into(), json!(lon));
    }
}

async fn geocoder_search(state: &AppState, query: &str, limit: i64) -> Option<Value> {
    let url = format!("{}/v1/places", state.config.geocoder_url);
    let params = vec![("q", query.to_string()), ("limit", limit.to_string())];
    let timeout = upstream_timeout(state, 1.0);
    let fetched = upstream::get(&state.http, &url, &params, &[], timeout)
        .await
        .ok()?;
    if !fetched.is_success() {
        return None;
    }
    let body = fetched.json()?;
    let rows = body
        .as_object()?
        .get("results")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    let results: Vec<Value> = rows.iter().filter_map(place_from_geocoder).collect();
    Some(json!({ "results": results }))
}

async fn photon_search(state: &AppState, query: &str, limit: i64) -> Option<Value> {
    let url = format!("{}/api", state.config.photon_url);
    // Ask for extra rows: non-settlement hits are filtered out below.
    let params = vec![
        ("q", query.to_string()),
        ("limit", (limit * 3).to_string()),
        ("osm_tag", "place".to_string()),
        ("lang", "en".to_string()),
    ];
    let timeout = upstream_timeout(state, 1.0);
    let fetched = upstream::get(&state.http, &url, &params, &[], timeout)
        .await
        .ok()?;
    if !fetched.is_success() {
        return None;
    }
    let body = fetched.json()?;
    let features = body
        .as_object()?
        .get("features")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    let mut results: Vec<Value> = Vec::new();
    let mut seen: Vec<i64> = Vec::new();
    for feature in &features {
        if let Some(place) = place_from_feature(feature, false) {
            let id = place.get("id").and_then(Value::as_i64).unwrap_or_default();
            if !seen.contains(&id) {
                seen.push(id);
                results.push(place);
            }
        }
        if results.len() as i64 >= limit {
            break;
        }
    }
    Some(json!({ "results": results }))
}

/// `GET /api/geocode?q=&limit=`.
pub async fn get_geocode(State(state): State<AppState>, uri: Uri) -> Response {
    let raw = uri.query();
    let mut issues = Issues::new();
    let q = issues.string("query", "q", query_param(raw, "q").as_deref(), None, 2, 100);
    let limit = issues.int_in(
        "query",
        "limit",
        query_param(raw, "limit").as_deref(),
        Some(8),
        1,
        20,
    );
    if let Err(resp) = issues.finish() {
        return resp;
    }
    let (Some(q), Some(limit)) = (q, limit) else {
        return respond::internal_error();
    };
    if !geocoding_configured(&state) {
        return not_configured();
    }
    let query = q.split_whitespace().collect::<Vec<_>>().join(" ");
    let key = format!("{}|{}", query.to_lowercase(), limit);
    if let Some(hit) = state.geocode_cache.get(&key) {
        return ok_json(hit, 3600);
    }
    let found = if state.config.geocoder_url.is_empty() {
        photon_search(&state, &query, limit).await
    } else {
        geocoder_search(&state, &query, limit).await
    };
    match found {
        Some(body) => {
            let body = respond::encode(&body);
            state.geocode_cache.insert(key, body.clone());
            ok_json(body, 3600)
        }
        None => respond::upstream_unavailable_no_store(),
    }
}

/// `Ok(None)` is a real "nothing here" answer (404 or no features). `Err(())` is an upstream failure.
async fn geocoder_reverse(state: &AppState, lat: f64, lon: f64) -> Result<Option<Value>, ()> {
    let url = format!("{}/v1/reverse-place", state.config.geocoder_url);
    let params = vec![("lat", float_repr(lat)), ("lon", float_repr(lon))];
    let timeout = upstream_timeout(state, 1.0);
    let fetched = upstream::get(&state.http, &url, &params, &[], timeout)
        .await
        .map_err(|_| ())?;
    if fetched.status == StatusCode::NOT_FOUND {
        return Ok(None);
    }
    if !fetched.is_success() {
        return Err(());
    }
    let body = fetched.json().ok_or(())?;
    let mut place = place_from_geocoder(&body);
    if let Some(p) = place.as_mut() {
        set_point(p, lat, lon);
    }
    Ok(place)
}

async fn photon_reverse(state: &AppState, lat: f64, lon: f64) -> Result<Option<Value>, ()> {
    let url = format!("{}/reverse", state.config.photon_url);
    let params = vec![
        ("lat", float_repr(lat)),
        ("lon", float_repr(lon)),
        ("limit", "1".to_string()),
        ("lang", "en".to_string()),
    ];
    let timeout = upstream_timeout(state, 1.0);
    let fetched = upstream::get(&state.http, &url, &params, &[], timeout)
        .await
        .map_err(|_| ())?;
    if !fetched.is_success() {
        return Err(());
    }
    let body = fetched.json().ok_or(())?;
    let features = body
        .as_object()
        .ok_or(())?
        .get("features")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    let mut place = features.first().and_then(|f| place_from_feature(f, true));
    if let Some(p) = place.as_mut() {
        set_point(p, lat, lon);
    }
    Ok(place)
}

/// `GET /api/reverse-geocode?lat=&lon=`. Order: parse (422), config (503), range (422).
pub async fn get_reverse_geocode(State(state): State<AppState>, uri: Uri) -> Response {
    let raw = uri.query();
    let mut issues = Issues::new();
    let lat = issues.float("query", "lat", query_param(raw, "lat").as_deref());
    let lon = issues.float("query", "lon", query_param(raw, "lon").as_deref());
    if let Err(resp) = issues.finish() {
        return resp;
    }
    let (Some(lat), Some(lon)) = (lat, lon) else {
        return respond::internal_error();
    };
    if !geocoding_configured(&state) {
        return not_configured();
    }
    if !in_range(lat, lon) {
        return respond::detail(StatusCode::UNPROCESSABLE_ENTITY, "lat/lon out of range");
    }
    let key = point_key(lat, lon);
    if let Some(hit) = state.reverse_cache.get(&key) {
        return ok_json(hit, 3600);
    }
    let (rlat, rlon) = (round_digits(lat, 3), round_digits(lon, 3));
    let found = if state.config.geocoder_url.is_empty() {
        photon_reverse(&state, rlat, rlon).await
    } else {
        geocoder_reverse(&state, rlat, rlon).await
    };
    match found {
        Ok(place) => {
            let body = respond::encode(&json!({ "place": place }));
            state.reverse_cache.insert(key, body.clone());
            ok_json(body, 3600)
        }
        Err(()) => respond::upstream_unavailable_no_store(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn point_key_uses_python_repr_after_rounding() {
        assert_eq!(point_key(42.9631, -85.6684), "42.963,-85.668");
        assert_eq!(point_key(42.0, -85.0), "42.0,-85.0");
    }

    #[test]
    fn forward_feature_needs_a_settlement() {
        let town = json!({
            "geometry": {"coordinates": [-85.6, 42.9]},
            "properties": {"osm_key": "place", "osm_value": "town", "name": "Grand Rapids",
                           "osm_id": 42, "state": "Michigan", "country": "United States", "countrycode": "us"}
        });
        let place = place_from_feature(&town, false).expect("town");
        assert_eq!(place["id"], json!(42));
        assert_eq!(place["countryCode"], json!("US"));
        let street = json!({
            "geometry": {"coordinates": [-85.6, 42.9]},
            "properties": {"osm_key": "highway", "osm_value": "primary", "name": "Main", "osm_id": 7}
        });
        assert!(place_from_feature(&street, false).is_none());
    }

    #[test]
    fn d8_non_integer_osm_id_drops_the_feature() {
        let feature = json!({
            "geometry": {"coordinates": [-85.6, 42.9]},
            "properties": {"osm_key": "place", "osm_value": "city", "name": "X", "osm_id": "abc"}
        });
        assert!(place_from_feature(&feature, false).is_none());
    }

    #[test]
    fn reverse_prefers_city_then_locality_then_settlement_name() {
        let feature = json!({
            "geometry": {"coordinates": [-85.6, 42.9]},
            "properties": {"osm_key": "highway", "locality": "Eastown", "county": "Kent", "osm_id": 9}
        });
        let place = place_from_feature(&feature, true).expect("reverse");
        assert_eq!(place["name"], json!("Eastown"));
    }

    #[test]
    fn geocoder_row_keeps_country_code_raw() {
        let row =
            json!({"id": 3, "name": "Lansing", "lat": "42.7", "lon": 84.5, "country_code": "US"});
        let place = place_from_geocoder(&row).expect("row");
        assert_eq!(place["countryCode"], json!("US"));
        assert!(place_from_geocoder(&json!({"bad": 1})).is_none());
    }
}
