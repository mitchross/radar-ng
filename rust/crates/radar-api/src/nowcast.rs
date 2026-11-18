//! `GET /api/nowcast/{lat}/{lon}` and `GET /api/nowcast/skill` (spec 5.4, 5.5).
//!
//! The point body is cached per `(run_id, lat3, lon3)` **without** `skill`. Every response then
//! merges the current headline in, so the skill can change without invalidating the body. The
//! skill summary is a single-entry snapshot. It is re-read only when the skill log file changes or
//! when its oldest counted entry leaves the window, so it always equals a fresh summary.
//!
//! Each point may carry `precip_class` (rain, snow, mix or ice): the class of the echoes near the
//! point in the observed frame the run holds still. Old clients ignore it.

use std::fs::File;
use std::os::unix::fs::FileExt;
use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::Ordering;
use std::time::SystemTime;

use axum::extract::{Path as AxPath, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use chrono::{DateTime, Utc};
use radar_core::grid_dump::NOWCAST_INPUT_LAYER;
use radar_core::grid_read::{GridMeta, nearest_cell, nearest_grid_meta, read_meta};
use radar_core::ice::{MAX_OBSERVED_TYPE_DELTA, nearest_observed_types};
use radar_core::nowcast_skill::{self, Summary};
use radar_core::pycompat::{float_repr, is_falsy, py_str};
use radar_core::time::{parse_iso_as_utc, round_digits};
use serde_json::{Map, Value, json};

use crate::grid;
use crate::manifest::cached_manifest;
use crate::params::Issues;
use crate::respond;
use crate::state::AppState;

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

/// The skill summary and its JSON form.
#[derive(Debug)]
pub struct Skill {
    /// The summary (for `/api/metrics`).
    pub summary: Summary,
    /// The summary as JSON (for the routes).
    pub value: Value,
}

/// Identity of the skill log file. The writer replaces it with tmp + rename, which changes it.
#[derive(Debug, Clone, PartialEq, Eq)]
struct LogStamp {
    len: u64,
    modified: Option<SystemTime>,
    ino: u64,
}

fn log_stamp(path: &Path) -> Option<LogStamp> {
    let meta = std::fs::metadata(path).ok()?;
    #[cfg(unix)]
    let ino = std::os::unix::fs::MetadataExt::ino(&meta);
    #[cfg(not(unix))]
    let ino = 0;
    Some(LogStamp {
        len: meta.len(),
        modified: meta.modified().ok(),
        ino,
    })
}

/// The cached summary, the log stamp it was read from, and when it stops being exact.
#[derive(Debug)]
pub struct SkillCache {
    stamp: Option<LogStamp>,
    expires: Option<DateTime<Utc>>,
    skill: Arc<Skill>,
}

impl SkillCache {
    fn is_current(&self, stamp: Option<&LogStamp>, now: DateTime<Utc>) -> bool {
        self.stamp.as_ref() == stamp && self.expires.is_none_or(|t| now < t)
    }
}

/// The skill summary, from the cache or the skill log. Synchronous (stats and may read a file).
#[must_use]
pub fn skill_summary_blocking(state: &AppState) -> Arc<Skill> {
    let path = nowcast_skill::skill_path(&state.config.state_dir);
    let now = Utc::now();
    // Stat before reading, so a rewrite during the read is picked up by the next call.
    let stamp = log_stamp(&path);
    if let Some(cached) = lock(&state.skill).as_ref()
        && cached.is_current(stamp.as_ref(), now)
    {
        return cached.skill.clone();
    }
    let log = nowcast_skill::read_log(&state.config.state_dir, &state.skill_cfg);
    let summary = nowcast_skill::summarize(&log, now, None, &state.skill_cfg);
    let expires = nowcast_skill::summary_expiry(&log, now, None, &state.skill_cfg);
    let value = serde_json::to_value(&summary).unwrap_or(Value::Null);
    let skill = Arc::new(Skill { summary, value });
    *lock(&state.skill) = Some(SkillCache {
        stamp,
        expires,
        skill: skill.clone(),
    });
    skill
}

/// The skill summary without blocking the runtime.
pub async fn skill_summary(state: &AppState) -> Arc<Skill> {
    let st = state.clone();
    tokio::task::spawn_blocking(move || skill_summary_blocking(&st))
        .await
        .unwrap_or_else(|_| {
            let log = nowcast_skill::SkillLog::empty(&state.skill_cfg);
            let summary = nowcast_skill::summarize(&log, Utc::now(), None, &state.skill_cfg);
            let value = serde_json::to_value(&summary).unwrap_or(Value::Null);
            Arc::new(Skill { summary, value })
        })
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
#[allow(
    clippy::result_large_err,
    reason = "the Err is the ready-made HTTP response; boxing it would add a deref at about 50 call sites for no gain on this cold path"
)]
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

    if let Some(cached) = state
        .nowcast_cache
        .get(&cache_key)
        .and_then(|b| serde_json::from_slice::<Value>(&b).ok())
    {
        state
            .metrics
            .counters
            .nowcast_point_cache_hits_total
            .fetch_add(1, Ordering::Relaxed);
        return Ok(PointResult {
            body: with_skill(cached, state).await,
            max_age: 30,
        });
    }

    // Each frame: (timestamp, grid_key). Frames without a timestamp are skipped.
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
    let anchor = parse_iso_as_utc(&run_id);
    let (samples, class): (Vec<Value>, Option<&'static str>) =
        tokio::task::spawn_blocking(move || {
            let samples = requests
                .iter()
                .map(|(ts, key)| {
                    grid::sample_point(&dir, "nowcast", ts, glat, glon, key.as_deref())
                })
                .collect();
            let class = anchor.and_then(|t| point_class(&dir, &input_layer(), t, glat, glon));
            (samples, class)
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
        let mut point = json!({
            "timestamp": py_str(obj.get("timestamp").unwrap_or(&Value::Null)),
            "lead_minutes": obj.get("lead_minutes").cloned().unwrap_or(Value::Null),
            "dbz": dbz,
            "precipitation_mm_h": dbz.map_or_default(dbz_to_rain_rate),
        });
        if let Some(class) = class {
            point["precip_class"] = json!(class);
        }
        points.push(point);
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
    state
        .nowcast_cache
        .insert(cache_key, respond::encode(&body));
    Ok(PointResult {
        body: with_skill(body, state).await,
        max_age: 30,
    })
}

/// Search radius for the echoes that set a point's class: about how far an echo moves in the
/// nowcast hour.
pub const CLASS_RADIUS_KM: f64 = 30.0;
/// Echoes weaker than this do not vote. It is near the clients' snow start threshold
/// (`SNOW_START_IN_HR`) and skips virga, which PrecipFlag files as "no precipitation" (rain).
pub const CLASS_ECHO_DBZ: f32 = 15.0;
/// A winter class wins with at least 1 / this share of the votes. PrecipFlag files echoes that do
/// not reach the ground as rain, and ahead of the 2025-01-06 Washington snow they outnumbered snow.
pub const WINTER_SHARE_DEN: usize = 3;

/// The grid layer the nowcast reads its observed dBZ from (as `radar_core::grid_dump::GridConfig`).
fn input_layer() -> String {
    radar_core::env::var("NOWCAST_GRID_INPUT_LAYER").unwrap_or_else(|| NOWCAST_INPUT_LAYER.into())
}

/// The class of the echoes within [`CLASS_RADIUS_KM`] of the point, in the observed class and dBZ
/// dumps nearest the run `anchor`: the most voted winter class when it has a [`WINTER_SHARE_DEN`]
/// share, else rain. `None` without both dumps or without an echo near the point.
#[must_use]
pub fn point_class(
    grid_dir: &Path,
    input_layer: &str,
    anchor: DateTime<Utc>,
    lat: f64,
    lon: f64,
) -> Option<&'static str> {
    let types = Dump::open(&nearest_observed_types(grid_dir, anchor)?)?;
    let echo = Dump::open(&nearest_grid_meta(
        grid_dir,
        input_layer,
        anchor,
        MAX_OBSERVED_TYPE_DELTA,
    )?)?;
    let m = &types.meta;
    let (row, col) = nearest_cell(m, lat, lon)?;
    let dlat = (m.lat_max - m.lat_min) / (m.height - 1) as f64;
    let dlon = (m.lon_max - m.lon_min) / (m.width - 1) as f64;
    let km_lat = 111.2;
    let km_lon = km_lat * lat.to_radians().cos().max(0.01);
    let reach_r = (CLASS_RADIUS_KM / (km_lat * dlat)).ceil() as usize;
    let reach_c = (CLASS_RADIUS_KM / (km_lon * dlon)).ceil() as usize;
    let (c0, c1) = (
        col.saturating_sub(reach_c),
        (col + reach_c).min(m.width - 1),
    );
    let mut votes = [0usize; 4];
    for r in row.saturating_sub(reach_r)..=(row + reach_r).min(m.height - 1) {
        let cell_lat = m.lat_max - r as f64 * dlat;
        let classes = types.row_span(r, c0, c1)?;
        for (c, class) in (c0..=c1).zip(classes) {
            let cell_lon = m.lon_min + c as f64 * dlon;
            let (dy, dx) = ((cell_lat - lat) * km_lat, (cell_lon - lon) * km_lon);
            if dy.hypot(dx) > CLASS_RADIUS_KM || !(0.0..=3.0).contains(&class) {
                continue;
            }
            let Some((er, ec)) = nearest_cell(&echo.meta, cell_lat, cell_lon) else {
                continue;
            };
            if echo.value(er, ec).is_some_and(|dbz| dbz >= CLASS_ECHO_DBZ) {
                votes[class.round() as usize] += 1;
            }
        }
    }
    let total: usize = votes.iter().sum();
    // `max_by_key` keeps the last maximum, so walk backwards: a tie goes to the lower code.
    let winter = (1..votes.len()).rev().max_by_key(|&i| votes[i])?;
    let class = if votes[winter] > 0 && votes[winter] * WINTER_SHARE_DEN >= total {
        winter
    } else {
        0
    };
    (total > 0).then_some(radar_tiler::typed::TYPE_CLASSES[class])
}

/// A grid dump read a few cells at a time.
struct Dump {
    meta: GridMeta,
    file: File,
}

impl Dump {
    fn open(meta_path: &Path) -> Option<Self> {
        let meta = read_meta(meta_path).ok()?;
        if meta.height < 2 || meta.width < 2 {
            return None;
        }
        let bin = meta_path.parent()?.join(&meta.data_file);
        Some(Self {
            meta,
            file: File::open(bin).ok()?,
        })
    }

    /// Cells `c0..=c1` of row `r`.
    fn row_span(&self, r: usize, c0: usize, c1: usize) -> Option<Vec<f32>> {
        let mut buf = vec![0u8; (c1 + 1 - c0) * 4];
        let offset = ((r * self.meta.width + c0) * 4) as u64;
        self.file.read_exact_at(&mut buf, offset).ok()?;
        Some(
            buf.chunks_exact(4)
                .map(|b| f32::from_le_bytes([b[0], b[1], b[2], b[3]]))
                .collect(),
        )
    }

    fn value(&self, r: usize, c: usize) -> Option<f32> {
        self.row_span(r, c, c)?
            .first()
            .copied()
            .filter(|v| v.is_finite())
    }
}

/// Append the current skill headline to a body as its last key.
async fn with_skill(mut body: Value, state: &AppState) -> Value {
    let skill = skill_summary(state).await;
    if let Some(map) = body.as_object_mut() {
        map.insert("skill".into(), headline(&skill.value));
    }
    body
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
    let skill = skill_summary(&state).await;
    respond::json(
        StatusCode::OK,
        &skill.value,
        Some(&respond::public_max_age(30)),
    )
    .into_response()
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

    /// Write a north-first 15x15 grid over 38.5..39.2 N, 77.4..76.7 W (0.05 degree cells).
    fn write(dir: &Path, layer: &str, ts: &str, f: impl Fn(f64, f64) -> f32) {
        use radar_core::grid_dump::{GridConfig, GridInput, write_grid};
        let lats: Vec<f64> = (0..15).map(|i| 39.2 - f64::from(i) * 0.05).collect();
        let lons: Vec<f64> = (0..15).map(|i| -77.4 + f64::from(i) * 0.05).collect();
        let data: Vec<f32> = lats
            .iter()
            .flat_map(|&la| lons.iter().map(move |&lo| (la, lo)))
            .map(|(la, lo)| f(la, lo))
            .collect();
        let cfg = GridConfig::with_dir(dir);
        write_grid(&cfg, layer, ts, &GridInput::new(&data, &lats, &lons, "x")).unwrap();
    }

    #[test]
    fn point_class_is_the_class_most_nearby_echoes_have() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path();
        let anchor: DateTime<Utc> = "2025-01-06T05:20:00Z".parse().unwrap();
        let (lat, lon) = (38.85, -77.03);
        assert_eq!(point_class(dir, "in", anchor, lat, lon), None, "no dumps");
        // Snow to the west, rain far to the east; the point itself is dry.
        let ts = "2025-01-06T05:18:00+00:00";
        write(dir, "radar-type", ts, |_, lo| {
            f32::from(u8::from(lo < -77.1))
        });
        assert_eq!(point_class(dir, "in", anchor, lat, lon), None, "no dBZ");
        write(dir, "in", ts, |_, lo| {
            if lo < -77.1 {
                20.0
            } else if lo > -76.8 {
                35.0
            } else {
                f32::NAN
            }
        });
        assert_eq!(point_class(dir, "in", anchor, lat, lon), Some("snow"));
        // Virga (a weak echo filed as rain) over the point does not vote.
        write(dir, "in", ts, |_, lo| if lo < -77.1 { 20.0 } else { 10.0 });
        assert_eq!(point_class(dir, "in", anchor, lat, lon), Some("snow"));
        // Dumps older than the hold window are not used.
        let late = anchor + chrono::Duration::minutes(45);
        assert_eq!(point_class(dir, "in", late, lat, lon), None);
        // No echo near the point: no class.
        write(dir, "in", ts, |_, _| 3.0);
        assert_eq!(point_class(dir, "in", anchor, lat, lon), None);
    }

    #[test]
    fn point_class_needs_a_third_of_the_echoes_for_winter() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path();
        let ts = "2025-01-06T05:18:00+00:00";
        let anchor: DateTime<Utc> = "2025-01-06T05:20:00Z".parse().unwrap();
        let class = |types: fn(f64) -> f32| {
            write(dir, "radar-type", ts, move |la, _| types(la));
            point_class(dir, "in", anchor, 38.85, -77.05)
        };
        // Echo everywhere but the point's own row.
        write(dir, "in", ts, |la, _| {
            if (la - 38.85).abs() < 0.01 { 0.0 } else { 25.0 }
        });
        // Half mix: mix.
        assert_eq!(class(|la| if la > 38.85 { 0.0 } else { 2.0 }), Some("mix"));
        // One far row of mix: rain.
        assert_eq!(
            class(|la| if (la - 38.6).abs() < 0.01 { 2.0 } else { 0.0 }),
            Some("rain")
        );
        // Snow and ice tie: the lower code, snow.
        assert_eq!(class(|la| if la > 38.85 { 1.0 } else { 3.0 }), Some("snow"));
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
