//! `GET /api/snow/{lat}/{lon}`: snow at one spot (design section 5, "Snow at your spot").
//!
//! One small answer per 0.1° cell: hourly snowfall for 72 h from the self-hosted Open-Meteo
//! (`snowfall`, already synced), plus point samples of the winter layers (`snow-depth`,
//! `snowfall-24h`, `snowfall-forecast`, `ice-forecast`, `snow-chance-*`). Every field is optional: a
//! missing source drops its field, never the answer. Complete answers are cached for [`TTL_S`] in a
//! byte-capped cache; an answer without the hourly block is served `no-store` and not cached.

use std::path::Path;
use std::time::Duration;

use axum::extract::{Path as UrlPath, State};
use axum::http::StatusCode;
use axum::response::Response;
use bytes::Bytes;
use chrono::{DateTime, NaiveDateTime, SecondsFormat, TimeDelta, Utc};
use radar_core::pycompat::float_repr;
use radar_core::time::{parse_iso_as_utc, round_digits};
use serde_json::{Map, Value, json};

use crate::forecast::cell_key;
use crate::grid::sample_point;
use crate::params::Issues;
use crate::respond;
use crate::state::AppState;
use crate::upstream;

/// Cache lifetime of one cell's answer.
pub const TTL_S: u64 = 900;
/// Hours of hourly snowfall returned (the card shows 48; 72 decides whether it shows).
pub const HOURS: usize = 72;
/// Horizons of the NBM totals, in hours.
pub const HORIZONS_H: [i64; 3] = [24, 48, 72];
/// Observed layers older than this are left out.
pub const OBSERVED_MAX_AGE_H: i64 = 48;
/// WPC thresholds, in inches, and their layers.
pub const CHANCE_LAYERS: [(u32, &str); 3] = [
    (1, "snow-chance-1"),
    (4, "snow-chance-4"),
    (8, "snow-chance-8"),
];
const TIMEOUT: Duration = Duration::from_secs(15);
/// NBM frames are 6 h apart.
const NBM_STEP_H: i64 = 6;

/// `GET /api/snow/{lat}/{lon}`.
pub async fn get_snow(
    State(state): State<AppState>,
    UrlPath((lat_raw, lon_raw)): UrlPath<(String, String)>,
) -> Response {
    let mut issues = Issues::new();
    let lat = issues.float_in("path", "lat", Some(&lat_raw), -90.0, 90.0);
    let lon = issues.float_in("path", "lon", Some(&lon_raw), -180.0, 180.0);
    if let Err(resp) = issues.finish() {
        return resp;
    }
    let (Some(lat), Some(lon)) = (lat, lon) else {
        return respond::internal_error();
    };
    let (glat, glon, key) = cell_key(lat, lon);
    // A partial answer (no hourly block) is the error side, so it is served but never cached.
    let got = state
        .snow_cache
        .try_get_with(key, async {
            let (body, complete) = build(&state, glat, glon).await;
            if complete { Ok(body) } else { Err(body) }
        })
        .await;
    match got {
        Ok(body) => {
            respond::json_bytes(StatusCode::OK, body, Some(&respond::public_max_age(TTL_S)))
        }
        Err(body) => respond::json_bytes(StatusCode::OK, (*body).clone(), Some("no-store")),
    }
}

/// The encoded answer for a rounded cell and whether it is complete enough to cache.
async fn build(state: &AppState, glat: f64, glon: f64) -> (Bytes, bool) {
    let now = Utc::now();
    let hourly = fetch_hourly(state, glat, glon).await;
    let layers = crate::manifest::cached_manifest(state)
        .await
        .ok()
        .and_then(|m| m.value.get("layers").cloned());
    let dir = state.config.grid_dir.clone();
    let complete = hourly.is_some();
    let body = tokio::task::spawn_blocking(move || {
        answer(hourly.as_ref(), layers.as_ref(), &dir, glat, glon, now)
    })
    .await
    .unwrap_or_else(|_| answer(None, None, Path::new("/nonexistent"), glat, glon, now));
    (respond::encode(&body), complete)
}

/// Hourly snowfall in inches, one value per hour from the current hour.
#[derive(Debug, Clone, PartialEq)]
pub struct Hourly {
    /// Hour stamps (the value is the snow that fell in the hour ending here, Open-Meteo style).
    pub time: Vec<DateTime<Utc>>,
    /// Inches, `None` where Open-Meteo has no value.
    pub snowfall_in: Vec<Option<f64>>,
}

async fn fetch_hourly(state: &AppState, glat: f64, glon: f64) -> Option<Hourly> {
    let query: Vec<(&str, String)> = vec![
        ("latitude", float_repr(glat)),
        ("longitude", float_repr(glon)),
        ("hourly", "snowfall".into()),
        ("forecast_hours", HOURS.to_string()),
        ("precipitation_unit", "inch".into()),
        ("timezone", "GMT".into()),
    ];
    let fetched = upstream::get(
        &state.http,
        &state.config.open_meteo_base,
        &query,
        &[],
        TIMEOUT,
    )
    .await
    .ok()?;
    if !fetched.is_success() {
        return None;
    }
    parse_hourly(&fetched.json()?)
}

/// The Open-Meteo `hourly.snowfall` block, in inches. `None` when it is missing or malformed.
#[must_use]
pub fn parse_hourly(body: &Value) -> Option<Hourly> {
    let hourly = body.get("hourly")?;
    let times = hourly.get("time")?.as_array()?;
    let values = hourly.get("snowfall")?.as_array()?;
    if times.is_empty() || times.len() != values.len() {
        return None;
    }
    // Asked for inches; convert if a server answers in its cm default.
    let unit = body
        .pointer("/hourly_units/snowfall")
        .and_then(Value::as_str)
        .unwrap_or("inch");
    let scale = match unit {
        "inch" => 1.0,
        "cm" => 1.0 / 2.54,
        "mm" => 1.0 / 25.4,
        _ => return None,
    };
    let mut time = Vec::with_capacity(times.len());
    for t in times {
        let t = NaiveDateTime::parse_from_str(t.as_str()?, "%Y-%m-%dT%H:%M").ok()?;
        time.push(t.and_utc());
    }
    let snowfall_in = values
        .iter()
        .take(HOURS)
        .map(|v| {
            v.as_f64()
                .filter(|v| v.is_finite())
                .map(|v| round_digits((v * scale).max(0.0), 2))
        })
        .collect();
    time.truncate(HOURS);
    Some(Hourly { time, snowfall_in })
}

fn iso(t: DateTime<Utc>) -> String {
    t.to_rfc3339_opts(SecondsFormat::Secs, true)
}

/// The response body. Pure apart from reading grid files, so the tests drive it directly.
#[must_use]
pub fn answer(
    hourly: Option<&Hourly>,
    layers: Option<&Value>,
    grid_dir: &Path,
    lat: f64,
    lon: f64,
    now: DateTime<Utc>,
) -> Value {
    let mut out = Map::new();
    out.insert("lat".into(), json!(lat));
    out.insert("lon".into(), json!(lon));
    out.insert("generated_at".into(), json!(iso(now)));
    let mut sources: Vec<&str> = Vec::new();

    if let Some(h) = hourly {
        let time: Vec<String> = h.time.iter().copied().map(iso).collect();
        out.insert(
            "hourly".into(),
            json!({ "time": time, "snowfall_in": h.snowfall_in }),
        );
        let snowy = |i: &usize| h.snowfall_in[*i].is_some_and(|v| v > 0.0);
        let first = (0..h.time.len()).find(snowy);
        let last = (0..h.time.len()).rev().find(snowy);
        if let (Some(a), Some(b)) = (first, last) {
            out.insert("start".into(), json!(iso(h.time[a] - TimeDelta::hours(1))));
            out.insert("end".into(), json!(iso(h.time[b])));
        }
        sources.push("open-meteo");
    }

    let sample = |layer: &str, ts: &str| -> Option<f64> {
        let v = sample_point(grid_dir, layer, ts, lat, lon, None);
        v.get("value")
            .and_then(Value::as_f64)
            .filter(|v| v.is_finite())
    };
    let frames = |layer: &str| frames_of(layers, layer);

    // Observed: the newest frame, if it is recent.
    for (layer, key, source) in [
        ("snow-depth", "on_ground", "snodas"),
        ("snowfall-24h", "past_24h", "nohrsc"),
    ] {
        let Some((ts, at)) = frames(layer).into_iter().max_by_key(|(_, at)| *at) else {
            continue;
        };
        if now - at > TimeDelta::hours(OBSERVED_MAX_AGE_H) {
            continue;
        }
        if let Some(v) = sample(layer, &ts) {
            out.insert(format!("{key}_in"), json!(round_digits(v.max(0.0), 2)));
            out.insert(format!("{key}_at"), json!(iso(at)));
            sources.push(source);
        }
    }

    // NBM running totals: the frame at now + N h minus the frame at now.
    let snow = frames("snowfall-forecast");
    let mut nbm = false;
    for h in HORIZONS_H {
        if let Some(v) = window_total(&snow, now, h, &sample, "snowfall-forecast") {
            out.insert(format!("nbm_{h}h_in"), json!(v));
            nbm = true;
        }
    }
    let ice = frames("ice-forecast");
    if let Some(v) = window_total(&ice, now, 72, &sample, "ice-forecast") {
        out.insert("ice_72h_in".into(), json!(v));
        nbm = true;
    }
    if nbm {
        if let Some(issued) = issued_at(layers, "snowfall-forecast") {
            out.insert("nbm_issued_at".into(), json!(issued));
        }
        sources.push("nbm");
    }

    // WPC: the highest chance over the 24 h windows ending in the next 72 h.
    let mut chance = Vec::new();
    for (inches, layer) in CHANCE_LAYERS {
        let best = frames(layer)
            .into_iter()
            .filter(|(_, at)| *at > now && *at <= now + TimeDelta::hours(72))
            .filter_map(|(ts, at)| sample(layer, &ts).map(|v| (v, at)))
            .max_by(|a, b| a.0.total_cmp(&b.0));
        if let Some((pct, at)) = best {
            chance.push(json!({
                "inches": inches,
                "pct": pct.clamp(0.0, 100.0).round(),
                "window_end": iso(at),
            }));
        }
    }
    if !chance.is_empty() {
        out.insert("chance".into(), Value::Array(chance));
        sources.push("wpc");
    }

    out.insert("sources".into(), json!(sources));
    Value::Object(out)
}

/// `(timestamp, valid time)` of every frame of a manifest layer.
fn frames_of(layers: Option<&Value>, layer: &str) -> Vec<(String, DateTime<Utc>)> {
    let Some(entry) = layers.and_then(|l| l.get(layer)) else {
        return Vec::new();
    };
    let stamps: Vec<String> = match entry.get("frames").and_then(Value::as_array) {
        Some(frames) => frames
            .iter()
            .filter_map(|f| {
                f.get("timestamp")
                    .and_then(Value::as_str)
                    .map(str::to_owned)
            })
            .collect(),
        None => entry
            .get("timestamps")
            .and_then(Value::as_array)
            .map(|a| {
                a.iter()
                    .filter_map(Value::as_str)
                    .map(str::to_owned)
                    .collect()
            })
            .unwrap_or_default(),
    };
    stamps
        .into_iter()
        .filter_map(|s| parse_iso_as_utc(&s).map(|t| (s, t)))
        .collect()
}

fn issued_at(layers: Option<&Value>, layer: &str) -> Option<String> {
    let frames = layers?.get(layer)?.get("frames")?.as_array()?;
    let issued = frames.first()?.get("issued_at")?.as_str()?;
    parse_iso_as_utc(issued).map(iso)
}

/// Snow (or ice) still to fall from `now` to `now + hours`, from a running-total layer. The
/// baseline is the newest frame at or before `now` (zero before the first frame); the target is the
/// newest frame at or before `now + hours`, and it must reach within one frame step of it.
fn window_total(
    frames: &[(String, DateTime<Utc>)],
    now: DateTime<Utc>,
    hours: i64,
    sample: &impl Fn(&str, &str) -> Option<f64>,
    layer: &str,
) -> Option<f64> {
    let end = now + TimeDelta::hours(hours);
    let target = frames
        .iter()
        .filter(|(_, at)| *at > now && *at <= end)
        .max_by_key(|(_, at)| *at)?;
    if end - target.1 >= TimeDelta::hours(NBM_STEP_H) {
        return None;
    }
    let base = match frames
        .iter()
        .filter(|(_, at)| *at <= now)
        .max_by_key(|(_, at)| *at)
    {
        Some((ts, _)) => sample(layer, ts)?,
        None => 0.0,
    };
    let total = sample(layer, &target.0)?;
    Some(round_digits((total - base).max(0.0), 2))
}

#[cfg(test)]
mod tests {
    use std::fs::File;
    use std::io::Write;

    use chrono::TimeZone;

    use super::*;

    /// Live self-hosted Open-Meteo answer for 39.7, -105.9 on 2026-11-18 (inches).
    const OPEN_METEO: &str = include_str!("../testdata/snow/open-meteo-39.7_-105.9.json");

    fn now() -> DateTime<Utc> {
        Utc.with_ymd_and_hms(2026, 11, 18, 4, 30, 0).unwrap()
    }

    /// A 3x4 grid over 40..42 N, 86..83 W with one value everywhere.
    fn grid(dir: &Path, layer: &str, stamp: &str, value: f32, unit: &str) {
        let d = dir.join(layer);
        std::fs::create_dir_all(&d).unwrap();
        let mut f = File::create(d.join(format!("{stamp}.bin"))).unwrap();
        for _ in 0..12 {
            f.write_all(&value.to_le_bytes()).unwrap();
        }
        std::fs::write(
            d.join(format!("{stamp}.meta.json")),
            format!("{{\"height\":3,\"width\":4,\"lat_min\":40.0,\"lat_max\":42.0,\"lon_min\":-86.0,\"lon_max\":-83.0,\"unit\":\"{unit}\",\"fill\":-9999.0,\"stride\":1,\"data_file\":\"{stamp}.bin\"}}"),
        )
        .unwrap();
    }

    #[test]
    fn real_open_meteo_hours_become_inches_with_start_and_end() {
        let h = parse_hourly(&serde_json::from_str(OPEN_METEO).unwrap()).unwrap();
        assert_eq!(h.time.len(), HOURS);
        assert_eq!(h.snowfall_in[0], Some(0.61));
        let body = answer(Some(&h), None, Path::new("/none"), 39.7, -105.9, now());
        assert_eq!(body["start"], "2026-11-18T03:00:00Z");
        assert_eq!(body["end"], "2026-11-21T03:00:00Z");
        assert_eq!(body["hourly"]["time"][0], "2026-11-18T04:00:00Z");
        assert_eq!(body["sources"], json!(["open-meteo"]));
    }

    #[test]
    fn cm_answers_are_converted_and_bad_blocks_are_none() {
        let cm = json!({
            "hourly_units": {"snowfall": "cm"},
            "hourly": {"time": ["2026-11-18T05:00"], "snowfall": [2.54]},
        });
        assert_eq!(parse_hourly(&cm).unwrap().snowfall_in, vec![Some(1.0)]);
        let short = json!({"hourly": {"time": ["2026-11-18T05:00"], "snowfall": []}});
        assert!(parse_hourly(&short).is_none());
        assert!(parse_hourly(&json!({"error": true})).is_none());
    }

    #[test]
    fn every_source_missing_is_still_a_valid_answer() {
        let body = answer(None, None, Path::new("/none"), 41.0, -85.0, now());
        assert_eq!(body["lat"], 41.0);
        assert_eq!(body["sources"], json!([]));
        for k in [
            "hourly",
            "start",
            "on_ground_in",
            "past_24h_in",
            "nbm_24h_in",
            "ice_72h_in",
            "chance",
        ] {
            assert!(body.get(k).is_none(), "{k}");
        }
        // Layers listed in the manifest but with no grid files drop their fields too.
        let layers =
            json!({"snow-depth": {"frames": [{"timestamp": "2026-11-18T06:00:00+00:00"}]}});
        let body = answer(None, Some(&layers), Path::new("/none"), 41.0, -85.0, now());
        assert!(body.get("on_ground_in").is_none());
    }

    /// Winter grids and manifest layers for a snowy spot at 41.0, -85.0.
    fn winter_fixture(dir: &Path) -> Value {
        let depth = "2026-11-17T06:00:00+00:00";
        let fell = "2026-11-18T00:00:00+00:00";
        grid(dir, "snow-depth", depth, 6.2, "in");
        grid(dir, "snowfall-24h", fell, 1.5, "in");
        // NBM run 03Z: running totals at +6 h steps.
        let nbm = [
            ("2026-11-18T09:00:00+00:00", 0.5, 0.0),
            ("2026-11-18T15:00:00+00:00", 2.0, 0.05),
            ("2026-11-18T21:00:00+00:00", 3.0, 0.08),
            ("2026-11-19T03:00:00+00:00", 4.0, 0.1),
            ("2026-11-19T09:00:00+00:00", 4.5, 0.1),
            ("2026-11-19T15:00:00+00:00", 4.8, 0.1),
            ("2026-11-19T21:00:00+00:00", 5.0, 0.1),
            ("2026-11-20T03:00:00+00:00", 5.0, 0.1),
            ("2026-11-20T09:00:00+00:00", 5.0, 0.1),
            ("2026-11-20T15:00:00+00:00", 5.1, 0.11),
            ("2026-11-20T21:00:00+00:00", 5.2, 0.12),
            ("2026-11-21T03:00:00+00:00", 5.25, 0.12),
        ];
        for (ts, s, i) in nbm {
            grid(dir, "snowfall-forecast", ts, s, "in");
            grid(dir, "ice-forecast", ts, i, "in");
        }
        grid(dir, "snow-chance-4", "2026-11-19T00:00:00+00:00", 35.0, "%");
        grid(dir, "snow-chance-4", "2026-11-19T06:00:00+00:00", 60.0, "%");
        grid(dir, "snow-chance-4", "2026-11-18T00:00:00+00:00", 90.0, "%"); // already past
        let frames = |list: &[&str]| json!({"frames": list.iter().map(|t| json!({"timestamp": t, "issued_at": "2026-11-18T03:00:00+00:00"})).collect::<Vec<_>>()});
        let nbm_ts: Vec<&str> = nbm.iter().map(|n| n.0).collect();
        json!({
            "snow-depth": frames(&[depth]),
            "snowfall-24h": frames(&[fell]),
            "snowfall-forecast": frames(&nbm_ts),
            "ice-forecast": frames(&nbm_ts),
            "snow-chance-4": frames(&["2026-11-18T00:00:00+00:00", "2026-11-19T00:00:00+00:00", "2026-11-19T06:00:00+00:00"]),
        })
    }

    #[test]
    fn winter_layers_are_sampled_at_the_point() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path();
        let layers = winter_fixture(dir);
        let body = answer(None, Some(&layers), dir, 41.0, -85.0, now());
        assert_eq!(body["on_ground_in"], 6.2);
        assert_eq!(body["on_ground_at"], "2026-11-17T06:00:00Z");
        assert_eq!(body["past_24h_in"], 1.5);
        // No frame before 04:30, so the baseline is the run start (0); +24 h lands on the 03Z frame.
        assert_eq!(body["nbm_24h_in"], 4.0);
        assert_eq!(body["nbm_48h_in"], 5.0);
        assert_eq!(body["nbm_72h_in"], 5.25);
        assert_eq!(body["ice_72h_in"], 0.12);
        assert_eq!(body["nbm_issued_at"], "2026-11-18T03:00:00Z");
        assert_eq!(
            body["chance"],
            json!([{"inches": 4, "pct": 60.0, "window_end": "2026-11-19T06:00:00Z"}])
        );
        assert_eq!(body["sources"], json!(["snodas", "nohrsc", "nbm", "wpc"]));

        // Later in the run the baseline is the frame already passed, and a short run drops 72 h.
        let later = Utc.with_ymd_and_hms(2026, 11, 18, 16, 0, 0).unwrap();
        let body = answer(None, Some(&layers), dir, 41.0, -85.0, later);
        assert_eq!(body["nbm_24h_in"], 2.8);
        assert!(body.get("nbm_72h_in").is_none());

        // Observed frames older than 48 h are left out.
        let stale = Utc.with_ymd_and_hms(2026, 11, 19, 7, 0, 0).unwrap();
        let body = answer(None, Some(&layers), dir, 41.0, -85.0, stale);
        assert!(body.get("on_ground_in").is_none());
        assert_eq!(body["past_24h_in"], 1.5);
    }

    #[test]
    fn a_point_outside_the_grids_drops_the_fields() {
        let tmp = tempfile::tempdir().unwrap();
        grid(
            tmp.path(),
            "snow-depth",
            "2026-11-18T06:00:00+00:00",
            3.0,
            "in",
        );
        let layers =
            json!({"snow-depth": {"frames": [{"timestamp": "2026-11-18T06:00:00+00:00"}]}});
        let body = answer(None, Some(&layers), tmp.path(), 21.3, -157.8, now());
        assert!(body.get("on_ground_in").is_none());
    }

    /// The contract fixture the web and app tests read. `SNOW_SPOT_FIXTURE_WRITE=1` rewrites it.
    #[test]
    fn contract_fixture_matches_the_server() {
        let path = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../../web/src/shared/testdata/snowSpot.contract.json");
        let tmp = tempfile::tempdir().unwrap();
        let layers = winter_fixture(tmp.path());
        let h = parse_hourly(&serde_json::from_str(OPEN_METEO).unwrap()).unwrap();
        let fixture = json!({
            "now": iso(now()),
            "utc_offset_seconds": -25200,
            "time_zone": "America/Denver",
            "full": answer(Some(&h), Some(&layers), tmp.path(), 41.0, -85.0, now()),
            "hourly_only": answer(Some(&h), None, tmp.path(), 39.7, -105.9, now()),
            "layers_only": answer(None, Some(&layers), tmp.path(), 41.0, -85.0, now()),
            "empty": answer(None, None, tmp.path(), 41.0, -85.0, now()),
        });
        let text = format!("{}\n", serde_json::to_string_pretty(&fixture).unwrap());
        if std::env::var_os("SNOW_SPOT_FIXTURE_WRITE").is_some() {
            std::fs::write(&path, &text).unwrap();
        }
        let on_disk: Value =
            serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
        assert_eq!(on_disk, fixture, "rerun with SNOW_SPOT_FIXTURE_WRITE=1");
    }
}
