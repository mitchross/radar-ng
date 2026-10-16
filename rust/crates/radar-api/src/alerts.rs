//! `GET /api/alerts` and `GET /api/alerts/map`: NWS active alerts (spec 5.15, 5.16).
//!
//! One national feed (`/alerts/active`) is fetched at most once per `ALERTS_TTL_S` and shared by
//! every point and by the map. A point gets the alerts that NWS's own `?point=` query returns:
//!
//! - an alert with a polygon matches when the point is inside the polygon (NWS does not widen a
//!   polygon warning to its whole county);
//! - an alert without a polygon matches when one of its `affectedZones` is the point's county,
//!   public forecast zone or fire weather zone, from `/points/{lat},{lon}` (cached for a day).
//!
//! A point that NWS does not map to zones (`/points` 404, such as offshore), or a failed feed or
//! zone lookup, falls back to the direct `?point=` query, cached per 3-dp point. Every upstream
//! call is single-flight: concurrent misses for one key make one request.
//!
//! Upstream failures are `502 {"error":"upstream_unavailable"}` with `no-store`, never a cached or
//! empty body. An empty alert list must only ever mean that NWS said there are none.

use std::sync::Arc;
use std::time::Duration;

use axum::extract::State;
use axum::http::{StatusCode, Uri};
use axum::response::{IntoResponse, Response};
use bytes::Bytes;
use radar_core::pycompat::{float_repr, is_falsy};
use radar_core::time::round_digits;
use serde_json::{Map, Value, json};

use crate::params::{Issues, query_param};
use crate::places::point_key;
use crate::respond;
use crate::state::AppState;
use crate::upstream;

/// Properties kept on `/api/alerts/map` features (spec 5.16).
const MAP_FIELDS: [&str; 9] = [
    "id", "event", "severity", "urgency", "headline", "sent", "expires", "ends", "areaDesc",
];

/// The `/points` properties that name the zones a point's zone-based alerts are issued for.
const POINT_ZONE_KEYS: [&str; 3] = ["county", "forecastZone", "fireWeatherZone"];

/// Zones change a few times a year, so a point's zones are kept for a day.
pub const ZONES_TTL: Duration = Duration::from_secs(86_400);

/// The NWS zones of one 3-dp point.
#[derive(Debug, PartialEq, Eq)]
pub enum PointZones {
    /// Zone URLs, as `affectedZones` spells them.
    Zones(Vec<String>),
    /// NWS has no zones here (`/points` answered 404).
    Uncovered,
}

impl PointZones {
    /// Cache weight: the zone URL bytes.
    #[must_use]
    pub fn weight(&self) -> usize {
        match self {
            PointZones::Zones(zones) => zones.iter().map(String::len).sum(),
            PointZones::Uncovered => 0,
        }
    }
}

/// One feature of the national feed, ready to match and copy into a point body.
#[derive(Debug)]
struct FeedAlert {
    /// The feature, compact-encoded.
    body: Bytes,
    /// The polygon, for alerts that have one.
    polygon: Option<Map<String, Value>>,
    /// `properties.affectedZones`.
    zones: Vec<String>,
}

impl FeedAlert {
    fn matches(&self, lat: f64, lon: f64, zones: &[String]) -> bool {
        match &self.polygon {
            Some(polygon) => radar_alerts::alerts::point_in_geojson(lon, lat, polygon),
            None => self.zones.iter().any(|z| zones.contains(z)),
        }
    }
}

/// The national feed, indexed for point lookups, plus the encoded `/api/alerts/map` body.
#[derive(Debug)]
pub struct AlertFeed {
    context: Option<Bytes>,
    updated: Option<Bytes>,
    alerts: Vec<FeedAlert>,
    map: Bytes,
}

impl AlertFeed {
    /// Index a parsed feed. `None` when it is not an object with a `features` list.
    #[must_use]
    pub fn from_value(body: &Value) -> Option<Self> {
        let obj = body.as_object()?;
        let features = obj.get("features")?.as_array()?;
        let alerts = features
            .iter()
            .filter(|f| f.is_object())
            .map(|f| {
                let polygon = f
                    .get("geometry")
                    .filter(|g| !is_falsy(g))
                    .and_then(Value::as_object)
                    .filter(|g| {
                        matches!(
                            g.get("type").and_then(Value::as_str),
                            Some("Polygon" | "MultiPolygon")
                        )
                    })
                    .cloned();
                let zones = f
                    .get("properties")
                    .and_then(|p| p.get("affectedZones"))
                    .and_then(Value::as_array)
                    .into_iter()
                    .flatten()
                    .filter_map(|z| z.as_str().map(str::to_string))
                    .collect();
                FeedAlert {
                    body: respond::encode(f),
                    polygon,
                    zones,
                }
            })
            .collect();
        Some(Self {
            context: obj.get("@context").map(respond::encode),
            updated: obj.get("updated").map(respond::encode),
            alerts,
            map: respond::encode(&map_features(body)),
        })
    }

    /// The `?point=` body for a 3-dp point and its zones, in NWS's key and feature order.
    #[must_use]
    pub fn point_body(&self, lat: f64, lon: f64, zones: &[String]) -> Bytes {
        let mut out = Vec::with_capacity(256);
        out.push(b'{');
        if let Some(context) = &self.context {
            out.extend_from_slice(b"\"@context\":");
            out.extend_from_slice(context);
            out.push(b',');
        }
        out.extend_from_slice(b"\"type\":\"FeatureCollection\",\"features\":[");
        let mut first = true;
        for alert in self.alerts.iter().filter(|a| a.matches(lat, lon, zones)) {
            if !first {
                out.push(b',');
            }
            first = false;
            out.extend_from_slice(&alert.body);
        }
        out.extend_from_slice(b"],\"title\":");
        out.extend_from_slice(&respond::encode(&json!(point_title(lat, lon))));
        if let Some(updated) = &self.updated {
            out.extend_from_slice(b",\"updated\":");
            out.extend_from_slice(updated);
        }
        out.push(b'}');
        Bytes::from(out)
    }

    /// Approximate heap size, for logs.
    fn len_bytes(&self) -> usize {
        self.alerts.iter().map(|a| a.body.len()).sum::<usize>() + self.map.len()
    }
}

/// NWS's point title: `"... for 43 N, 85.5 W"`.
fn point_title(lat: f64, lon: f64) -> String {
    let label = |v: f64| {
        let text = float_repr(v.abs());
        text.strip_suffix(".0").map_or(text.clone(), str::to_string)
    };
    let ns = if lat < 0.0 { "S" } else { "N" };
    let ew = if lon < 0.0 { "W" } else { "E" };
    format!(
        "Current watches, warnings, and advisories for {} {ns}, {} {ew}",
        label(lat),
        label(lon)
    )
}

fn in_range(lat: f64, lon: f64) -> bool {
    (-90.0..=90.0).contains(&lat) && (-180.0..=180.0).contains(&lon)
}

fn upstream_timeout(state: &AppState, factor: f64) -> Duration {
    Duration::from_secs_f64((state.config.places_timeout_s * factor).max(0.001))
}

/// GET an NWS URL. `Ok(None)` for a 404, `Err(())` for any other failure or a non-JSON body.
async fn fetch_nws(
    state: &AppState,
    url: &str,
    query: &[(&str, String)],
    timeout: Duration,
) -> Result<Option<Value>, ()> {
    let headers = [
        ("User-Agent", state.config.nws_user_agent.as_str()),
        ("Accept", "application/geo+json"),
    ];
    let fetched = upstream::get(&state.http, url, query, &headers, timeout)
        .await
        .map_err(|_| ())?;
    if fetched.status == StatusCode::NOT_FOUND {
        return Ok(None);
    }
    if !fetched.is_success() {
        return Err(());
    }
    fetched.json().map(Some).ok_or(())
}

/// True for a JSON object with a `features` list.
fn has_features(body: &Value) -> bool {
    body.as_object()
        .and_then(|o| o.get("features"))
        .is_some_and(Value::is_array)
}

/// The national feed, from the cache or NWS.
async fn alert_feed(state: &AppState) -> Result<Arc<AlertFeed>, ()> {
    state
        .alert_feed
        .try_get_with((), async {
            let url = state.config.nws_point_alerts_url.clone();
            let body = fetch_nws(state, &url, &[], upstream_timeout(state, 2.0))
                .await?
                .ok_or(())?;
            // Indexing re-encodes every feature (a few MB), so it runs on the blocking pool.
            let feed = tokio::task::spawn_blocking(move || AlertFeed::from_value(&body))
                .await
                .map_err(|_| ())?
                .ok_or(())?;
            tracing::debug!(
                alerts = feed.alerts.len(),
                bytes = feed.len_bytes(),
                "alert_feed_loaded"
            );
            Ok::<_, ()>(Arc::new(feed))
        })
        .await
        .map_err(|_| ())
}

/// The zones of a 3-dp point, from the cache or `/points`.
async fn point_zones(state: &AppState, key: &str) -> Result<Arc<PointZones>, ()> {
    state
        .alert_zones
        .try_get_with_by_ref(key, async {
            let url = format!("{}/{key}", state.config.nws_points_url);
            let Some(body) = fetch_nws(state, &url, &[], upstream_timeout(state, 1.0)).await?
            else {
                return Ok(Arc::new(PointZones::Uncovered));
            };
            let props = body.get("properties");
            let zones: Vec<String> = POINT_ZONE_KEYS
                .iter()
                .filter_map(|k| props.and_then(|p| p.get(*k)).and_then(Value::as_str))
                .map(str::to_string)
                .collect();
            Ok::<_, ()>(Arc::new(if zones.is_empty() {
                PointZones::Uncovered
            } else {
                PointZones::Zones(zones)
            }))
        })
        .await
        .map_err(|_| ())
}

/// NWS's own `?point=` answer for a 3-dp key, from the cache or NWS.
async fn direct_point(state: &AppState, key: &str) -> Result<Bytes, ()> {
    state
        .alerts_cache
        .try_get_with_by_ref(key, async {
            let query = [("point", key.to_string())];
            let url = &state.config.nws_point_alerts_url;
            match fetch_nws(state, url, &query, upstream_timeout(state, 1.0)).await? {
                Some(body) if has_features(&body) => Ok::<_, ()>(respond::encode(&body)),
                _ => Err(()),
            }
        })
        .await
        .map_err(|_| ())
}

/// The encoded alert collection for a point. `Err(())` on any upstream failure.
///
/// # Errors
/// Returns `Err(())` when neither the feed path nor the direct point query produced a body.
pub async fn alerts_body(state: &AppState, lat: f64, lon: f64) -> Result<Bytes, ()> {
    let key = point_key(lat, lon);
    let (zones, feed) = tokio::join!(point_zones(state, &key), alert_feed(state));
    if let (Ok(zones), Ok(feed)) = (zones, feed)
        && let PointZones::Zones(zones) = zones.as_ref()
    {
        return Ok(feed.point_body(round_digits(lat, 3), round_digits(lon, 3), zones));
    }
    direct_point(state, &key).await
}

/// The alert collection for a point, parsed. Shared with the briefing, which treats a failure as
/// "no alerts".
///
/// # Errors
/// Returns `Err(())` for any upstream failure.
pub async fn alerts_for(state: &AppState, lat: f64, lon: f64) -> Result<Value, ()> {
    let body = alerts_body(state, lat, lon).await?;
    serde_json::from_slice(&body).map_err(|_| ())
}

fn ok_json(body: Bytes, max_age: u64) -> Response {
    respond::json_bytes(
        StatusCode::OK,
        body,
        Some(&respond::public_max_age(max_age)),
    )
}

/// `GET /api/alerts?lat=&lon=`.
pub async fn get_alerts(State(state): State<AppState>, uri: Uri) -> Response {
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
    if !in_range(lat, lon) {
        return respond::detail(StatusCode::UNPROCESSABLE_ENTITY, "lat/lon out of range");
    }
    match alerts_body(&state, lat, lon).await {
        Ok(body) => ok_json(body, state.config.alerts_ttl_s),
        Err(()) => respond::upstream_unavailable_no_store(),
    }
}

/// Slim the feed to `Actual` storm-based polygons (`geometry` truthy), keeping nine properties.
#[must_use]
pub fn map_features(body: &Value) -> Value {
    let features: Vec<Value> = body
        .get("features")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|f| {
            let obj = f.as_object()?;
            let geometry = obj.get("geometry").filter(|g| !is_falsy(g))?;
            let props = obj.get("properties").filter(|p| !is_falsy(p)).and_then(Value::as_object);
            // The feed carries every status; the map shows what `?status=actual` returned.
            if props.and_then(|p| p.get("status")).and_then(Value::as_str) != Some("Actual") {
                return None;
            }
            let mut out = Map::new();
            for key in MAP_FIELDS {
                let value = props.and_then(|p| p.get(key)).cloned().unwrap_or(Value::Null);
                out.insert(key.to_string(), value);
            }
            Some(json!({ "type": "Feature", "geometry": geometry, "properties": Value::Object(out) }))
        })
        .collect();
    json!({ "type": "FeatureCollection", "features": features })
}

/// `GET /api/alerts/map`.
pub async fn get_alerts_map(State(state): State<AppState>) -> Response {
    match alert_feed(&state).await {
        Ok(feed) => ok_json(feed.map.clone(), state.config.alerts_ttl_s),
        Err(()) => respond::upstream_unavailable_no_store(),
    }
    .into_response()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn square(lon0: f64, lat0: f64) -> Value {
        json!({"type": "Polygon", "coordinates": [[
            [lon0, lat0], [lon0 + 1.0, lat0], [lon0 + 1.0, lat0 + 1.0], [lon0, lat0 + 1.0], [lon0, lat0]
        ]]})
    }

    fn feed() -> AlertFeed {
        let body = json!({
            "@context": ["https://geojson.org/geojson-ld/geojson-context.jsonld"],
            "type": "FeatureCollection",
            "features": [
                {"id": "a", "geometry": square(-86.0, 42.0),
                 "properties": {"status": "Actual", "affectedZones": ["z/county/MIC081"]}},
                {"id": "b", "geometry": null,
                 "properties": {"status": "Actual", "affectedZones": ["z/forecast/MIZ057", "z/forecast/MIZ058"]}},
                {"id": "c", "geometry": null,
                 "properties": {"status": "Test", "affectedZones": ["z/county/ILC185"]}},
            ],
            "title": "Current watches, warnings, and advisories",
            "updated": "2026-10-15T22:00:00+00:00",
        });
        AlertFeed::from_value(&body).expect("feed")
    }

    fn ids(body: &Bytes) -> Vec<String> {
        let v: Value = serde_json::from_slice(body).expect("json");
        v["features"]
            .as_array()
            .expect("features")
            .iter()
            .map(|f| f["id"].as_str().expect("id").to_string())
            .collect()
    }

    #[test]
    fn polygon_alerts_match_by_containment_not_by_county() {
        let feed = feed();
        let county = vec!["z/county/MIC081".to_string()];
        // Inside the polygon.
        assert_eq!(ids(&feed.point_body(42.5, -85.5, &county)), vec!["a"]);
        // Same county, outside the polygon: NWS leaves the polygon warning out.
        assert!(ids(&feed.point_body(43.5, -85.5, &county)).is_empty());
    }

    #[test]
    fn zone_alerts_match_any_point_zone() {
        let feed = feed();
        let zones = vec!["z/county/X".to_string(), "z/forecast/MIZ058".to_string()];
        assert_eq!(ids(&feed.point_body(10.0, 10.0, &zones)), vec!["b"]);
        let test_zone = vec!["z/county/ILC185".to_string()];
        assert_eq!(
            ids(&feed.point_body(10.0, 10.0, &test_zone)),
            vec!["c"],
            "points get every status, like ?point="
        );
    }

    #[test]
    fn point_body_keeps_nws_key_order_and_title() {
        let body = feed().point_body(43.0, -85.5, &[]);
        let text = std::str::from_utf8(&body).expect("utf8");
        assert!(text.starts_with("{\"@context\":[\"https://geojson.org/geojson-ld/geojson-context.jsonld\"],\"type\":\"FeatureCollection\",\"features\":[]"));
        assert!(text.ends_with(",\"title\":\"Current watches, warnings, and advisories for 43 N, 85.5 W\",\"updated\":\"2026-10-15T22:00:00+00:00\"}"));
    }

    #[test]
    fn map_keeps_storm_polygons_and_nine_properties() {
        let body = json!({"features": [
            {"geometry": {"type": "Polygon", "coordinates": []},
             "properties": {"id": "urn:1", "status": "Actual", "event": "Severe Thunderstorm Warning", "description": "long"}},
            {"geometry": null, "properties": {"id": "urn:2", "status": "Actual"}}
        ]});
        let out = map_features(&body);
        let features = out["features"].as_array().expect("array");
        assert_eq!(features.len(), 1);
        let props = features[0]["properties"].as_object().expect("props");
        assert_eq!(props.len(), 9);
        assert!(!props.contains_key("description"));
        assert_eq!(props["ends"], Value::Null);
    }

    #[test]
    fn map_keeps_actual_polygons_only() {
        let feed = feed();
        let map: Value = serde_json::from_slice(&feed.map).expect("json");
        let features = map["features"].as_array().expect("features");
        assert_eq!(features.len(), 1);
        assert_eq!(
            features[0]["properties"].as_object().expect("props").len(),
            9
        );
    }
}
