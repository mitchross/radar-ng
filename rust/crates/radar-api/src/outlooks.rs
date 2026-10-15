//! `GET /api/outlooks?day=1..3`: the SPC categorical convective outlook as slim GeoJSON.
//!
//! Upstream is `{SPC_OUTLOOK_BASE}/day{N}otlk_cat.lyr.geojson` (a few KB, issued a few times a day).
//! Each feature keeps `label` (TSTM, MRGL, ...), `name`, SPC's `fill`/`stroke`, and the valid window.

use axum::body::Bytes;
use axum::extract::State;
use axum::http::{StatusCode, Uri};
use axum::response::Response;
use serde_json::{Map, Value, json};

use crate::params::{Issues, query_param};
use crate::respond;
use crate::state::AppState;
use crate::upstream;

/// Upstream property name to response name.
const FIELDS: [(&str, &str); 7] = [
    ("LABEL", "label"),
    ("LABEL2", "name"),
    ("fill", "fill"),
    ("stroke", "stroke"),
    ("VALID_ISO", "valid"),
    ("EXPIRE_ISO", "expire"),
    ("ISSUE_ISO", "issued"),
];

/// Seconds clients and the CDN may keep a response, and the server-side cache TTL.
pub const OUTLOOK_TTL_S: u64 = 600;

/// Slim an SPC outlook collection, dropping features without geometry.
#[must_use]
pub fn slim(body: &Value) -> Value {
    let features: Vec<Value> = body
        .get("features")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|f| {
            let geometry = f.get("geometry").filter(|g| !g.is_null())?;
            let props = f.get("properties").and_then(Value::as_object);
            let mut out = Map::new();
            for (from, to) in FIELDS {
                let v = props
                    .and_then(|p| p.get(from))
                    .cloned()
                    .unwrap_or(Value::Null);
                out.insert(to.to_string(), v);
            }
            Some(json!({"type": "Feature", "geometry": geometry, "properties": out}))
        })
        .collect();
    json!({"type": "FeatureCollection", "features": features})
}

/// `GET /api/outlooks?day=`.
pub async fn get_outlooks(State(state): State<AppState>, uri: Uri) -> Response {
    let mut issues = Issues::new();
    let day = issues.int_in(
        "query",
        "day",
        query_param(uri.query(), "day").as_deref(),
        Some(1),
        1,
        3,
    );
    if let Err(resp) = issues.finish() {
        return resp;
    }
    let Some(day) = day else {
        return respond::internal_error();
    };
    let cache_control = respond::public_max_age(OUTLOOK_TTL_S);
    if let Some(hit) = state.outlook_cache.get(&day) {
        return respond::json_bytes(StatusCode::OK, hit, Some(&cache_control));
    }
    let url = format!(
        "{}/day{day}otlk_cat.lyr.geojson",
        state.config.spc_outlook_base
    );
    let headers = [("User-Agent", state.config.nws_user_agent.as_str())];
    let timeout = std::time::Duration::from_secs(8);
    let fetched = upstream::get(&state.http, &url, &[], &headers, timeout).await;
    let Some(body) = fetched
        .ok()
        .filter(upstream::Fetched::is_success)
        .and_then(|f| f.json())
    else {
        return respond::upstream_unavailable_no_store();
    };
    let encoded: Bytes = respond::encode(&slim(&body));
    state.outlook_cache.insert(day, encoded.clone());
    respond::json_bytes(StatusCode::OK, encoded, Some(&cache_control))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn slim_keeps_category_fields_and_drops_geometryless_features() {
        let body = json!({"features": [
            {"geometry": {"type": "Polygon", "coordinates": []},
             "properties": {"DN": 4, "LABEL": "SLGT", "LABEL2": "Slight Risk", "fill": "#FFE066",
                            "stroke": "#DDAA00", "VALID_ISO": "v", "EXPIRE_ISO": "e", "ISSUE_ISO": "i",
                            "FORECASTER": "x"}},
            {"geometry": null, "properties": {"LABEL": "TSTM"}}
        ]});
        let out = slim(&body);
        let features = out["features"].as_array().unwrap();
        assert_eq!(features.len(), 1);
        let props = &features[0]["properties"];
        assert_eq!(props["label"], "SLGT");
        assert_eq!(props["name"], "Slight Risk");
        assert_eq!(props["fill"], "#FFE066");
        assert_eq!(props["expire"], "e");
        assert!(props.get("FORECASTER").is_none());
    }
}
