//! `GET /api/storm-prefetch` and `GET /api/storm-prefetch/style.json` (spec 5.9, 5.9.1).
//!
//! The plan logic is `radar_storms::build_storm_prefetch_plan`. This module supplies the request
//! parsing, the manifest snapshot, the request origin, and the style document.
//!
//! D1 fix: the style document resolves each tile path from the manifest frame's `path` (for a
//! nowcast frame, `runs/<anchor>/<valid>`). Python always used the timestamp, so a run-scoped
//! frame had no tiles. The `style_url` query still carries `timestamp`, as the contract requires.
//! D9 fix: a corrupt manifest with no last-good copy no longer 500s when no storm is selected.

use axum::extract::State;
use axum::http::{HeaderMap, StatusCode, header};
use axum::response::{IntoResponse, Response};
use radar_storms::{PrefetchRequest, build_storm_prefetch_plan};
use serde_json::{Value, json};

use crate::manifest::cached_manifest;
use crate::params::{Issues, query_param};
use crate::respond;
use crate::state::AppState;

/// Palettes accepted by both prefetch routes.
const PALETTES: [&str; 3] = ["classic", "vivid", "muted"];

/// `request.base_url` without the trailing slash: `X-Forwarded-Proto` (or `http`) plus `Host`.
#[must_use]
pub fn request_base(headers: &HeaderMap) -> String {
    let proto = headers
        .get("x-forwarded-proto")
        .and_then(|v| v.to_str().ok())
        .filter(|s| !s.is_empty())
        .unwrap_or("http");
    let host = headers
        .get(header::HOST)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("localhost");
    format!("{proto}://{host}")
}

/// Read a JSON file under the state directory: `None` on any error.
fn read_json_file(path: &std::path::Path) -> Option<Value> {
    let text = std::fs::read_to_string(path).ok()?;
    serde_json::from_str(&text).ok()
}

/// The plan body, shared by the route and tests.
///
/// # Errors
/// Returns a response for an unusable manifest (500) or an out-of-range request (422).
pub async fn plan_for(
    state: &AppState,
    lat: f64,
    lon: f64,
    zoom: i64,
    palette: &str,
    base_url: &str,
) -> Result<Value, Response> {
    let state_dir = state.config.state_dir.clone();
    let storms = tokio::task::spawn_blocking(move || {
        read_json_file(&state_dir.join("storms.json"))
            .unwrap_or_else(|| json!({ "type": "FeatureCollection", "features": [] }))
    })
    .await
    .unwrap_or_else(|_| json!({ "type": "FeatureCollection", "features": [] }));

    let has_features = storms
        .get("features")
        .and_then(Value::as_array)
        .is_some_and(|f| !f.is_empty());
    let manifest = match cached_manifest(state).await {
        Ok(snap) => snap.value.clone(),
        Err(resp) if has_features => return Err(resp),
        Err(_) => std::sync::Arc::new(json!({ "schema_version": 2, "layers": {} })),
    };
    let cfg = state.storm_cfg.clone();
    let tile_dir = state.config.tile_dir.clone();
    let palette = palette.to_string();
    let base_url = base_url.to_string();
    let plan = tokio::task::spawn_blocking(move || {
        let req = PrefetchRequest {
            lat,
            lon,
            zoom,
            palette: &palette,
            base_url: &base_url,
        };
        build_storm_prefetch_plan(&storms, &manifest, &tile_dir, &req, &cfg)
    })
    .await
    .map_err(|_| respond::internal_error())?
    .map_err(|err| {
        tracing::error!("storm prefetch plan failed: {err}");
        respond::internal_error()
    })?;
    Ok(serde_json::to_value(plan).unwrap_or(Value::Null))
}

/// `GET /api/storm-prefetch`.
pub async fn get_storm_prefetch(
    State(state): State<AppState>,
    headers: HeaderMap,
    uri: axum::http::Uri,
) -> Response {
    let raw = uri.query();
    let mut issues = Issues::new();
    let lat = issues.float("query", "lat", query_param(raw, "lat").as_deref());
    let lon = issues.float("query", "lon", query_param(raw, "lon").as_deref());
    let zoom_raw = query_param(raw, "zoom");
    let zoom = issues.int("query", "zoom", zoom_raw.as_deref(), Some(6));
    let palette = query_param(raw, "palette").unwrap_or_else(|| "classic".to_string());
    if let Err(resp) = issues.finish() {
        return resp;
    }
    let (Some(lat), Some(lon), Some(zoom)) = (lat, lon, zoom) else {
        return respond::internal_error();
    };
    if !(-90.0..=90.0).contains(&lat) || !(-180.0..=180.0).contains(&lon) {
        return respond::detail(StatusCode::UNPROCESSABLE_ENTITY, "lat/lon out of range");
    }
    if !(4..=8).contains(&zoom) {
        return respond::detail(
            StatusCode::UNPROCESSABLE_ENTITY,
            "zoom must be between 4 and 8",
        );
    }
    if !PALETTES.contains(&palette.as_str()) {
        return respond::detail(StatusCode::UNPROCESSABLE_ENTITY, "unsupported palette");
    }
    let base = request_base(&headers);
    match plan_for(&state, lat, lon, zoom, &palette, &base).await {
        Ok(plan) => {
            respond::json(StatusCode::OK, &plan, Some(&respond::public_max_age(30))).into_response()
        }
        Err(resp) => resp,
    }
}

/// `GET /api/storm-prefetch/style.json`.
pub async fn get_storm_prefetch_style(
    State(state): State<AppState>,
    headers: HeaderMap,
    uri: axum::http::Uri,
) -> Response {
    let raw = uri.query();
    let layer = query_param(raw, "layer");
    let palette = query_param(raw, "palette");
    let timestamp = query_param(raw, "timestamp");
    let mut issues = Issues::new();
    let layer = issues.string("query", "layer", layer.as_deref(), None, 0, usize::MAX);
    let palette = issues.string("query", "palette", palette.as_deref(), None, 0, usize::MAX);
    let timestamp = issues.string(
        "query",
        "timestamp",
        timestamp.as_deref(),
        None,
        0,
        usize::MAX,
    );
    let zoom = issues.int("query", "zoom", query_param(raw, "zoom").as_deref(), None);
    if let Err(resp) = issues.finish() {
        return resp;
    }
    let (Some(layer), Some(palette), Some(timestamp), Some(zoom)) =
        (layer, palette, timestamp, zoom)
    else {
        return respond::internal_error();
    };
    if !matches!(layer.as_str(), "radar" | "nowcast") || !PALETTES.contains(&palette.as_str()) {
        return respond::detail(
            StatusCode::UNPROCESSABLE_ENTITY,
            "unsupported layer or palette",
        );
    }
    let max_zoom = if layer == "radar" { 7 } else { 6 };
    if !(4..=max_zoom).contains(&zoom) {
        return respond::detail(
            StatusCode::UNPROCESSABLE_ENTITY,
            "zoom outside layer coverage",
        );
    }
    let safe: String = timestamp
        .chars()
        .filter(|c| c.is_alphanumeric() || ":-_+.T".contains(*c))
        .collect();
    if safe != timestamp {
        return respond::detail(StatusCode::UNPROCESSABLE_ENTITY, "invalid timestamp");
    }

    // D1: tile path from the manifest frame (run-scoped for nowcast), falling back to the timestamp.
    let path = match cached_manifest(&state).await {
        Ok(snap) => snap
            .value
            .get("layers")
            .and_then(|l| l.get(&layer))
            .and_then(|l| l.get("frames"))
            .and_then(Value::as_array)
            .and_then(|frames| {
                frames.iter().find(|f| {
                    f.get("timestamp").and_then(Value::as_str) == Some(timestamp.as_str())
                })
            })
            .and_then(|f| f.get("path").and_then(Value::as_str))
            .filter(|p| !p.is_empty())
            .map(str::to_string)
            .unwrap_or_else(|| timestamp.clone()),
        Err(_) => timestamp.clone(),
    };
    let base = request_base(&headers);
    let template = format!("{base}/tiles/{layer}/{palette}/{path}/{{z}}/{{x}}/{{y}}.png");
    let style = json!({
        "version": 8,
        "name": "radar-ng storm prefetch",
        "sources": {
            "storm-prefetch": {
                "type": "raster",
                "tiles": [template],
                "tileSize": 256,
                "minzoom": zoom,
                "maxzoom": zoom,
            }
        },
        "layers": [{
            "id": "storm-prefetch",
            "type": "raster",
            "source": "storm-prefetch",
            "paint": {"raster-fade-duration": 0},
        }],
    });
    respond::json(StatusCode::OK, &style, Some(&respond::public_max_age(30))).into_response()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn base_uses_forwarded_proto_then_http() {
        let mut h = HeaderMap::new();
        h.insert(header::HOST, "radar.example".parse().expect("hv"));
        assert_eq!(request_base(&h), "http://radar.example");
        h.insert("x-forwarded-proto", "https".parse().expect("hv"));
        assert_eq!(request_base(&h), "https://radar.example");
    }
}
