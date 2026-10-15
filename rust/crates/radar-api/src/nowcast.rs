//! `GET /api/nowcast/{lat}/{lon}` and `GET /api/nowcast/skill` (spec 5.4, 5.5).
//!
//! The point body is cached per `(run_id, lat3, lon3)` **without** `skill`. Every response then
//! merges the current headline in, so the skill can change without invalidating the body. The
//! skill summary is a single-entry snapshot re-read every 15 s.

use std::sync::Arc;
use std::sync::atomic::Ordering;
use std::time::{Duration, Instant};

use axum::extract::{Path as AxPath, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use chrono::{DateTime, Utc};
use radar_core::nowcast_skill;
use radar_core::pycompat::{float_repr, is_falsy, py_str};
use radar_core::time::{parse_iso_as_utc, round_digits};
use serde_json::{Map, Value, json};

use crate::grid;
use crate::manifest::cached_manifest;
use crate::params::Issues;
use crate::respond;
use crate::state::AppState;

/// TTL of the skill snapshot (`_MANIFEST_TTL_S`).
pub const SKILL_TTL: Duration = Duration::from_secs(15);

/// A published run is expired when none of its frames remains valid in the future.
pub(crate) fn run_expired(layer: &Value, now: DateTime<Utc>) -> bool {
    let Some(frames) = layer.get("frames").and_then(Value::as_array) else {
        return false;
    };
    !frames.is_empty()
        && !frames.iter().any(|frame| {
            frame
                .get("timestamp")
                .and_then(Value::as_str)
                .and_then(parse_iso_as_utc)
                .is_some_and(|valid| valid > now)
        })
}

/// Marshall-Palmer Z-R estimate in mm/h (spec 5.4): `0` below 5 dBZ, clamped to `[0, 300]`.
#[must_use]
pub fn dbz_to_rain_rate(dbz: f64) -> f64 {
    if dbz < 5.0 {
        return 0.0;
    }
    let z = 10f64.powf(dbz / 10.0);
    ((z / 200.0).powf(1.0 / 1.6)).clamp(0.0, 300.0)
}

/// The skill summary as JSON, from the cache or the skill log. Synchronous (reads a file).
#[must_use]
pub fn skill_summary_blocking(state: &AppState) -> Arc<Value> {
    {
        let guard = lock(&state.skill);
        if let Some((expires, value)) = guard.as_ref()
            && Instant::now() < *expires
        {
            return value.clone();
        }
    }
    let log = nowcast_skill::read_log(&state.config.state_dir, &state.skill_cfg);
    let summary = nowcast_skill::summarize(&log, Utc::now(), None, &state.skill_cfg);
    let value = Arc::new(serde_json::to_value(&summary).unwrap_or(Value::Null));
    *lock(&state.skill) = Some((Instant::now() + SKILL_TTL, value.clone()));
    value
}

/// The skill summary without blocking the runtime.
pub async fn skill_summary(state: &AppState) -> Arc<Value> {
    let st = state.clone();
    tokio::task::spawn_blocking(move || skill_summary_blocking(&st))
        .await
        .unwrap_or_else(|_| Arc::new(Value::Null))
}

/// The headline (`summary.headline`, or `null`).
#[must_use]
pub fn headline(summary: &Value) -> Value {
    summary.get("headline").cloned().unwrap_or(Value::Null)
}

/// Result of a point lookup: the JSON body and its `Cache-Control` max-age.
pub struct PointResult {
    /// The body, with `skill` already merged when the status is ok or degraded.
    pub body: Value,
    /// `max-age` for the response (30 for ok, 15 for unavailable).
    pub max_age: u64,
}

fn lock<T>(m: &std::sync::Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    m.lock().unwrap_or_else(std::sync::PoisonError::into_inner)
}

/// Read `STATE_DIR/nowcast-status.json` if it parses to an object.
fn read_status(state: &AppState) -> Option<Map<String, Value>> {
    let text = std::fs::read_to_string(state.state_file("nowcast-status.json")).ok()?;
    match serde_json::from_str::<Value>(&text).ok()? {
        Value::Object(map) => Some(map),
        _ => None,
    }
}

/// The nowcast point for a validated `(lat, lon)`. Shared by the route and the briefing.
///
/// # Errors
/// Returns the 503 response when the manifest is unreadable and has no last-good copy.
pub async fn nowcast_point(state: &AppState, lat: f64, lon: f64) -> Result<PointResult, Response> {
    state
        .metrics
        .counters
        .nowcast_point_requests_total
        .fetch_add(1, Ordering::Relaxed);
    let snap = cached_manifest(state).await?;
    let layer = snap
        .value
        .get("layers")
        .and_then(|l| l.get("nowcast"))
        .filter(|l| l.is_object());
    let frames: Vec<Value> = layer
        .and_then(|l| l.get("frames"))
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    let Some(layer) = layer.filter(|_| !frames.is_empty()) else {
        let st = state.clone();
        let status = tokio::task::spawn_blocking(move || read_status(&st))
            .await
            .ok()
            .flatten()
            .unwrap_or_default();
        let reason = status
            .get("reason")
            .cloned()
            .unwrap_or_else(|| json!("no_frames"));
        let detail = status.get("detail").cloned().unwrap_or(Value::Null);
        return Ok(PointResult {
            body: json!({ "status": "unavailable", "reason": reason, "detail": detail, "points": [] }),
            max_age: 15,
        });
    };

    // Check before the cache: a cached run can expire without a manifest update.
    if run_expired(layer, Utc::now()) {
        return Ok(PointResult {
            body: json!({"status": "unavailable", "reason": "nowcast_expired", "points": []}),
            max_age: 15,
        });
    }

    let run_id = {
        let raw = layer
            .get("run_id")
            .filter(|v| !is_falsy(v))
            .or_else(|| layer.get("latest").filter(|v| !is_falsy(v)));
        raw.map_or_else(|| "unknown".to_string(), py_str)
    };
    let glat = round_digits(lat, 3);
    let glon = round_digits(lon, 3);
    let cache_key = (run_id.clone(), float_repr(glat), float_repr(glon));

    if let Some(cached) = state.nowcast_cache.get(&cache_key) {
        state
            .metrics
            .counters
            .nowcast_point_cache_hits_total
            .fetch_add(1, Ordering::Relaxed);
        return Ok(PointResult {
            body: with_skill(&cached, state).await,
            max_age: 30,
        });
    }

    // Each frame: (timestamp, grid_key). Frames without a timestamp are skipped, as in Python.
    let mut requests: Vec<(String, Option<String>)> = Vec::new();
    for frame in &frames {
        let Some(obj) = frame.as_object() else {
            continue;
        };
        let Some(ts) = obj.get("timestamp").filter(|v| !is_falsy(v)) else {
            continue;
        };
        let grid_key = obj
            .get("grid_key")
            .and_then(Value::as_str)
            .map(str::to_string);
        requests.push((py_str(ts), grid_key));
    }
    let dir = state.config.grid_dir.clone();
    let samples: Vec<Value> = tokio::task::spawn_blocking(move || {
        requests
            .iter()
            .map(|(ts, key)| grid::sample_point(&dir, "nowcast", ts, glat, glon, key.as_deref()))
            .collect()
    })
    .await
    .unwrap_or_default();

    let mut points: Vec<Value> = Vec::new();
    let mut missing = 0usize;
    let mut out_of_bounds = 0usize;
    let mut sample_iter = samples.into_iter();
    for frame in frames.iter().filter(|f| {
        f.as_object()
            .is_some_and(|o| o.get("timestamp").is_some_and(|t| !is_falsy(t)))
    }) {
        let Some(sample) = sample_iter.next() else {
            break;
        };
        let obj = frame.as_object().expect("filtered to objects");
        if sample.get("ok") != Some(&Value::Bool(true)) {
            missing += 1;
            if sample.get("reason") == Some(&json!("out_of_bounds")) {
                out_of_bounds += 1;
            }
            continue;
        }
        let dbz = sample.get("value").and_then(Value::as_f64);
        points.push(json!({
            "timestamp": py_str(obj.get("timestamp").unwrap_or(&Value::Null)),
            "lead_minutes": obj.get("lead_minutes").cloned().unwrap_or(Value::Null),
            "dbz": dbz,
            "precipitation_mm_h": dbz.map_or(0.0, dbz_to_rain_rate),
        }));
    }

    if points.is_empty() {
        let reason = if out_of_bounds > 0 && out_of_bounds == missing {
            "out_of_bounds"
        } else {
            "grids_warming_up"
        };
        return Ok(PointResult {
            body: json!({
                "status": "unavailable",
                "reason": reason,
                "issued_at": layer.get("run_id").cloned().unwrap_or(Value::Null),
                "points": [],
            }),
            max_age: 15,
        });
    }

    let spatial = frames
        .iter()
        .filter_map(Value::as_object)
        .filter_map(|f| f.get("spatial_resolution_km").filter(|v| !v.is_null()))
        .next()
        .cloned()
        .unwrap_or(Value::Null);
    let body = json!({
        "status": if missing == 0 { "ok" } else { "degraded" },
        "source": "mrms-nowcast",
        "method": layer.get("method").cloned().unwrap_or(Value::Null),
        "issued_at": layer.get("run_id").cloned().unwrap_or(Value::Null),
        "horizon_minutes": layer.get("horizon_minutes").cloned().unwrap_or(json!(60)),
        "step_minutes": layer.get("step_minutes").cloned().unwrap_or(json!(5)),
        "spatial_resolution_km": spatial,
        "latitude": glat,
        "longitude": glon,
        "points": points,
    });
    let cached = Arc::new(body);
    state.nowcast_cache.insert(cache_key, cached.clone());
    Ok(PointResult {
        body: with_skill(&cached, state).await,
        max_age: 30,
    })
}

/// Clone a cached body and append the current skill headline as the last key.
async fn with_skill(body: &Value, state: &AppState) -> Value {
    let mut out = body.clone();
    let summary = skill_summary(state).await;
    let headline = headline(summary.as_ref());
    if let Some(map) = out.as_object_mut() {
        map.insert("skill".into(), headline);
    }
    out
}

/// `GET /api/nowcast/{lat}/{lon}`.
pub async fn get_nowcast_point(
    State(state): State<AppState>,
    AxPath((lat_raw, lon_raw)): AxPath<(String, String)>,
) -> Response {
    let mut issues = Issues::new();
    let lat = issues.float("path", "lat", Some(&lat_raw));
    let lon = issues.float("path", "lon", Some(&lon_raw));
    if let Err(resp) = issues.finish() {
        return resp;
    }
    let (Some(lat), Some(lon)) = (lat, lon) else {
        return respond::internal_error();
    };
    if !(-90.0..=90.0).contains(&lat) || !(-180.0..=180.0).contains(&lon) {
        return respond::detail(StatusCode::UNPROCESSABLE_ENTITY, "lat/lon out of range");
    }
    match nowcast_point(&state, lat, lon).await {
        Ok(result) => respond::json(
            StatusCode::OK,
            &result.body,
            Some(&respond::public_max_age(result.max_age)),
        ),
        Err(resp) => resp,
    }
}

/// `GET /api/nowcast/skill`.
pub async fn get_nowcast_skill(State(state): State<AppState>) -> Response {
    let summary = skill_summary(&state).await;
    respond::json(StatusCode::OK, &summary, Some(&respond::public_max_age(30))).into_response()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn marshall_palmer_matches_spec_formula() {
        assert_eq!(dbz_to_rain_rate(4.9), 0.0);
        let r = dbz_to_rain_rate(30.0);
        // Z = 1000, R = (1000/200)^(1/1.6)
        assert!((r - 5f64.powf(1.0 / 1.6)).abs() < 1e-12);
        assert_eq!(dbz_to_rain_rate(200.0), 300.0);
    }

    #[test]
    fn headline_is_null_when_absent() {
        assert_eq!(headline(&json!({"available": false})), Value::Null);
        assert_eq!(
            headline(&json!({"headline": {"pod": 0.8}})),
            json!({"pod": 0.8})
        );
    }
}
