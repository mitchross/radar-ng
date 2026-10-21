//! HTTP-side checks: API latency, tile sampling, pipeline freshness, and the client playback sim.
//!
//! Port of `tools/debug_harness/checks.py`. Everything goes through the public serving path, the
//! same way the app does, so the numbers include the full hop (`radar api`, not just the handler).
//!
//! Deliberate deviation: the tile URL template is filled with the frame's `path` (which defaults to
//! its timestamp). The Python formatted only `{timestamp}`, but the manifest advertises `{path}`, so
//! the Python harness raised `KeyError` on the live stack. See `README.md`.

use std::f64::consts::PI;

use chrono::{DateTime, Duration as ChronoDuration, Utc};
use futures::StreamExt;
use radar_core::pycompat::{Json, is_falsy};
use serde_json::Value;

use crate::model::{
    Check, Fetch, Http, Status, age_str, float_str, get_manifest, parse_iso, percentiles,
    py_display, quote_ts, seconds_between, stats_json,
};

/// Default probe location: the CONUS centre, inside every layer's coverage.
pub const DEFAULT_LAT: f64 = 39.5;
/// Default probe longitude.
pub const DEFAULT_LON: f64 = -98.35;

/// Client playback tick in milliseconds (`PLAYBACK_MS` in `TimelineBar.tsx`).
pub const TICK_BUDGET_MS: f64 = 420.0;

/// `Cache-Control` `radar api` serves on every `/tiles/*` path: frame paths are unique per frame.
pub const TILE_CACHE: &str = "max-age=86400";

/// Tile template used when the manifest does not carry one.
pub const DEFAULT_TILE_TEMPLATE: &str = "/tiles/{layer}/{palette}/{timestamp}/{z}/{x}/{y}.png";

/// One API endpoint probe: `(name, path, budget_ms)`.
pub fn api_endpoints() -> Vec<(&'static str, String, i64)> {
    vec![
        ("manifest", "/api/manifest.json".into(), 1500),
        ("health", "/api/health".into(), 1500),
        ("livez", "/api/livez".into(), 500),
        ("metrics", "/api/metrics".into(), 2000),
        ("lightning", "/api/lightning".into(), 1500),
        ("storms", "/api/storms".into(), 1500),
        ("tropical", "/api/tropical".into(), 1500),
        (
            "forecast",
            format!(
                "/api/forecast/{}/{}",
                float_str(DEFAULT_LAT),
                float_str(DEFAULT_LON)
            ),
            5000,
        ),
        ("wind-field", "/api/wind-field/latest".into(), 3000),
    ]
}

/// Freshness expectation for one layer (`LAYER_EXPECTATIONS`).
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Expectation {
    /// `latest` is a past timestamp; alarm on its age.
    Observed {
        /// Age above which the layer warns, seconds.
        warn_s: f64,
        /// Age above which the layer fails, seconds.
        fail_s: f64,
        /// Expected seconds between frames (completeness denominator).
        cadence_s: f64,
    },
    /// Timestamps are future valid times; alarm when the horizon collapses.
    Forecast {
        /// Minimum horizon ahead of now, seconds.
        min_horizon_s: f64,
        /// Documented refresh cadence, seconds (informational).
        #[allow(dead_code)]
        refresh_s: f64,
    },
}

/// Layer name to expectation, in the Python dict's order.
pub const LAYER_EXPECTATIONS: &[(&str, Expectation)] = &[
    (
        "radar",
        Expectation::Observed {
            warn_s: 600.0,
            fail_s: 1800.0,
            cadence_s: 120.0,
        },
    ),
    (
        "radar-composite",
        Expectation::Observed {
            warn_s: 600.0,
            fail_s: 1800.0,
            cadence_s: 120.0,
        },
    ),
    (
        "nowcast",
        Expectation::Forecast {
            min_horizon_s: 600.0,
            refresh_s: 120.0,
        },
    ),
    (
        "radar-hrrr",
        Expectation::Forecast {
            min_horizon_s: 3600.0,
            refresh_s: 900.0,
        },
    ),
    (
        "temperature",
        Expectation::Forecast {
            min_horizon_s: 3600.0,
            refresh_s: 900.0,
        },
    ),
    (
        "dewpoint",
        Expectation::Forecast {
            min_horizon_s: 3600.0,
            refresh_s: 900.0,
        },
    ),
    (
        "humidity",
        Expectation::Forecast {
            min_horizon_s: 3600.0,
            refresh_s: 900.0,
        },
    ),
    (
        "wind",
        Expectation::Forecast {
            min_horizon_s: 3600.0,
            refresh_s: 900.0,
        },
    ),
    (
        "cape",
        Expectation::Forecast {
            min_horizon_s: 3600.0,
            refresh_s: 900.0,
        },
    ),
    (
        "precip-type",
        Expectation::Forecast {
            min_horizon_s: 3600.0,
            refresh_s: 900.0,
        },
    ),
    (
        "precip-accum",
        Expectation::Forecast {
            min_horizon_s: 3600.0,
            refresh_s: 900.0,
        },
    ),
    (
        "cloud",
        Expectation::Forecast {
            min_horizon_s: 3600.0,
            refresh_s: 900.0,
        },
    ),
];

/// Web Mercator tile containing `(lat, lon)` at zoom `z` (`_slippy`).
pub fn slippy(lat: f64, lon: f64, z: u32) -> (i64, i64) {
    let lat_r = lat.to_radians();
    let n = 2f64.powi(z as i32);
    let x = ((lon + 180.0) / 360.0 * n) as i64;
    let y = ((1.0 - lat_r.tan().asinh() / PI) / 2.0 * n) as i64;
    (x, y)
}

/// Fill a tile URL template. `{path}` takes the frame path; `{timestamp}` is accepted for older
/// manifests. Both are percent-encoded like the app does.
pub fn tile_url(
    server: &str,
    template: &str,
    layer: &str,
    palette: &str,
    path: &str,
    (z, x, y): (u32, i64, i64),
) -> String {
    let encoded = quote_ts(path);
    let filled = template
        .replace("{layer}", layer)
        .replace("{palette}", palette)
        .replace("{path}", &encoded)
        .replace("{timestamp}", &encoded)
        .replace("{z}", &z.to_string())
        .replace("{x}", &x.to_string())
        .replace("{y}", &y.to_string());
    format!("{server}{filled}")
}

/// The manifest's tile template, or the default.
fn template_of(manifest: &Value) -> String {
    manifest
        .get("tile_url_template")
        .and_then(Value::as_str)
        .unwrap_or(DEFAULT_TILE_TEMPLATE)
        .to_string()
}

/// The layer's own tile template when it has one, else the manifest-wide template.
fn layer_template(layer: &Value, default: &str) -> String {
    layer
        .get("tile_url_template")
        .and_then(Value::as_str)
        .unwrap_or(default)
        .to_string()
}

/// The `max_zoom` the manifest advertises for the frame at `timestamp`, if any.
fn frame_max_zoom(layer: &Value, timestamp: &str) -> Option<u32> {
    layer
        .get("frames")
        .and_then(Value::as_array)?
        .iter()
        .find(|f| f.get("timestamp").and_then(Value::as_str) == Some(timestamp))?
        .get("max_zoom")
        .and_then(Value::as_u64)
        .and_then(|z| u32::try_from(z).ok())
}

/// Timestamps of a layer, as strings, in manifest order.
fn layer_timestamps(layer: &Value) -> Vec<String> {
    layer
        .get("timestamps")
        .and_then(Value::as_array)
        .map(|items| {
            items
                .iter()
                .filter_map(Value::as_str)
                .map(str::to_string)
                .collect()
        })
        .unwrap_or_default()
}

/// First palette of a layer, or `classic`.
fn first_palette(layer: &Value) -> String {
    layer
        .get("palettes")
        .and_then(Value::as_array)
        .and_then(|p| p.first())
        .and_then(Value::as_str)
        .unwrap_or("classic")
        .to_string()
}

/// The frame's tile path: the frame's `path` when present, else the timestamp.
fn frame_path(layer: &Value, timestamp: &str) -> String {
    layer
        .get("frames")
        .and_then(Value::as_array)
        .and_then(|frames| {
            frames
                .iter()
                .find(|f| f.get("timestamp").and_then(Value::as_str) == Some(timestamp))
        })
        .and_then(|f| f.get("path"))
        .and_then(Value::as_str)
        .filter(|p| !p.is_empty())
        .unwrap_or(timestamp)
        .to_string()
}

/// Latency percentiles per endpoint against a budget. `/api/health` answers 503 by design when the
/// radar is stale, so an all-503 result counts as degraded, not down.
pub async fn check_api(http: &Http, server: &str, samples: usize) -> Vec<Check> {
    let mut checks = Vec::new();
    for (name, path, budget_ms) in api_endpoints() {
        let url = format!("{server}{path}");
        let mut fetches: Vec<Fetch> = Vec::with_capacity(samples);
        for _ in 0..samples {
            fetches.push(http.get(&url).await);
        }
        let errors: Vec<&Fetch> = fetches.iter().filter(|f| !f.ok()).collect();
        let degraded =
            name == "health" && !errors.is_empty() && errors.iter().all(|f| f.status == 503);
        let times: Vec<f64> = fetches
            .iter()
            .filter(|f| f.ok() || degraded)
            .map(|f| f.elapsed_ms)
            .collect();
        let stats = percentiles(&times);
        if !errors.is_empty() && !degraded {
            let messages: Vec<Json> = errors
                .iter()
                .map(|f| Json::Str(f.error.clone().unwrap_or_default()))
                .collect();
            checks.push(
                Check::new(
                    format!("api.{name}"),
                    Status::Fail,
                    format!(
                        "{}/{samples} failed ({}) — {url}",
                        errors.len(),
                        errors[0].error.as_deref().unwrap_or("None")
                    ),
                )
                .with_data(Json::Object(vec![
                    ("stats".into(), stats_json(stats)),
                    ("errors".into(), Json::Array(messages)),
                ])),
            );
            continue;
        }
        let p95 = stats.map_or(0.0, |s| s.p95_ms);
        let status = if p95 <= budget_ms as f64 {
            Status::Ok
        } else {
            Status::Warn
        };
        let note = if degraded { " [503 degraded]" } else { "" };
        let p50_text = stats.map_or_else(|| "None".to_string(), |s| float_str(s.p50_ms));
        let p95_text = if stats.is_some() {
            float_str(p95)
        } else {
            "0".to_string()
        };
        checks.push(
            Check::new(
                format!("api.{name}"),
                status,
                format!("p50 {p50_text}ms / p95 {p95_text}ms (budget {budget_ms}ms){note}"),
            )
            .with_data(Json::Object(vec![
                ("stats".into(), stats_json(stats)),
                ("budget_ms".into(), Json::Int(budget_ms)),
            ])),
        );
    }
    checks
}

/// Tile sampling for every manifest layer at z4 to z8 around the probe point: latency, 404 rate,
/// and the `Cache-Control` split the serving path promises.
pub async fn check_tiles(
    http: &Http,
    server: &str,
    lat: f64,
    lon: f64,
    per_zoom: usize,
) -> Vec<Check> {
    let (mf, manifest) = get_manifest(http, server).await;
    let Some(manifest) = manifest else {
        return vec![Check::new(
            "tiles.manifest",
            Status::Fail,
            format!(
                "cannot load manifest: {}",
                mf.error.as_deref().unwrap_or("None")
            ),
        )];
    };
    let default_template = template_of(&manifest);
    let layers = sorted_layers(&manifest);
    let side = ((per_zoom as f64).sqrt() as i64).max(1);
    let mut checks = Vec::new();
    for (layer_name, layer) in layers {
        let timestamps = layer_timestamps(layer);
        if timestamps.is_empty() {
            checks.push(Check::new(
                format!("tiles.{layer_name}"),
                Status::Warn,
                "no timestamps in manifest",
            ));
            continue;
        }
        let ts = layer
            .get("latest")
            .and_then(Value::as_str)
            .map_or_else(|| timestamps[timestamps.len() - 1].clone(), str::to_string);
        let palette = first_palette(layer);
        let path = frame_path(layer, &ts);
        // Proxied layers (satellite) carry their own template and a lower zoom cap.
        let template = layer_template(layer, &default_template);
        let max_zoom = frame_max_zoom(layer, &ts).unwrap_or(8).min(8);

        let mut urls = Vec::new();
        for z in 4..=max_zoom {
            let (cx, cy) = slippy(lat, lon, z);
            for dy in 0..side {
                for dx in 0..side {
                    let x = cx + dx - side / 2;
                    let y = cy + dy - side / 2;
                    urls.push(tile_url(
                        server,
                        &template,
                        layer_name,
                        &palette,
                        &path,
                        (z, x, y),
                    ));
                }
            }
        }
        let fetches = fetch_all(http, urls, 8).await;
        let total = fetches.len();

        let hits: Vec<&Fetch> = fetches.iter().filter(|f| f.ok()).collect();
        let missing = fetches.iter().filter(|f| f.status == 404).count();
        let broken: Vec<&Fetch> = fetches
            .iter()
            .filter(|f| !f.ok() && f.status != 404)
            .collect();
        let hit_times: Vec<f64> = hits.iter().map(|f| f.elapsed_ms).collect();
        let stats = percentiles(&hit_times);

        let want = TILE_CACHE;
        let bad_cache: Vec<&&Fetch> = hits
            .iter()
            .filter(|f| !f.header("cache-control").contains(want))
            .collect();

        let (status, detail) = if !broken.is_empty() {
            (
                Status::Fail,
                format!(
                    "{}/{total} errored ({})",
                    broken.len(),
                    broken[0].error.as_deref().unwrap_or("None")
                ),
            )
        } else if hits.is_empty() {
            (
                Status::Fail,
                format!("0/{total} tiles found for latest={ts}"),
            )
        } else if let Some(first_bad) = bad_cache.first() {
            let got = if first_bad.headers.contains_key("cache-control") {
                first_bad.header("cache-control").to_string()
            } else {
                "<none>".to_string()
            };
            (
                Status::Warn,
                format!("cache-control mismatch: want '{want}', got '{got}'"),
            )
        } else {
            let p95 = stats.map_or(0.0, |s| s.p95_ms);
            let status = if p95 <= 800.0 {
                Status::Ok
            } else {
                Status::Warn
            };
            let avg = hits.iter().map(|f| f.bytes).sum::<usize>() / hits.len().max(1);
            let p50 = stats.map_or_else(|| "None".to_string(), |s| float_str(s.p50_ms));
            (
                status,
                format!(
                    "{} hits / {missing} 404 · p50 {p50}ms p95 {}ms · avg {avg}B · cache '{want}' ok",
                    hits.len(),
                    float_str(p95)
                ),
            )
        };
        checks.push(
            Check::new(format!("tiles.{layer_name}"), status, detail).with_data(Json::Object(
                vec![
                    ("timestamp".into(), Json::Str(ts.clone())),
                    ("palette".into(), Json::Str(palette.clone())),
                    ("stats".into(), stats_json(stats)),
                    ("hits".into(), Json::Int(hits.len() as i64)),
                    ("missing_404".into(), Json::Int(missing as i64)),
                    ("broken".into(), Json::Int(broken.len() as i64)),
                ],
            )),
        );
    }
    checks
}

/// Data freshness per layer, read from the same manifest the app draws frames from.
pub async fn check_pipeline(http: &Http, server: &str) -> Vec<Check> {
    let health = http.get_json(&format!("{server}/api/health")).await;
    let manifest = get_manifest(http, server).await;
    let lightning = http.get_json(&format!("{server}/api/lightning")).await;
    pipeline_checks(
        Utc::now(),
        into_result(health),
        into_result(manifest),
        into_result(lightning),
    )
}

/// Turn a fetched `(fetch, body)` pair into `Result`, carrying the fetch error text on failure.
fn into_result((fetch, body): (Fetch, Option<Value>)) -> Result<Value, String> {
    body.ok_or_else(|| fetch.error.unwrap_or_else(|| "None".to_string()))
}

/// Pure freshness logic over already-fetched documents (`check_pipeline` after the I/O).
pub fn pipeline_checks(
    now: DateTime<Utc>,
    health: Result<Value, String>,
    manifest: Result<Value, String>,
    lightning: Result<Value, String>,
) -> Vec<Check> {
    let mut checks = Vec::new();

    match &health {
        Ok(h) => {
            let status = if h.get("status").and_then(Value::as_str) == Some("ok") {
                Status::Ok
            } else {
                Status::Fail
            };
            let reasons = h.get("reasons");
            let reasons_text = match reasons {
                Some(v) if !is_falsy(v) => py_display(Some(v)),
                _ => "[]".to_string(),
            };
            let nowcast_status = h.get("nowcast").and_then(|n| n.get("status"));
            checks.push(
                Check::new(
                    "pipeline.health",
                    status,
                    format!(
                        "status={} mrms_age={}s reasons={reasons_text} nowcast={}",
                        py_display(h.get("status")),
                        py_display(h.get("mrms_age_s")),
                        py_display(nowcast_status),
                    ),
                )
                .with_data(Json::Object(vec![("health".into(), Json::from(h))])),
            );
        }
        Err(e) => checks.push(Check::new(
            "pipeline.health",
            Status::Fail,
            format!("/api/health unreachable: {e}"),
        )),
    }

    let manifest = match manifest {
        Ok(m) => m,
        Err(e) => {
            checks.push(Check::new(
                "pipeline.manifest",
                Status::Fail,
                format!("cannot load manifest: {e}"),
            ));
            return checks;
        }
    };

    for (layer_name, exp) in LAYER_EXPECTATIONS {
        let Some(layer) = manifest.get("layers").and_then(|l| l.get(*layer_name)) else {
            checks.push(Check::new(
                format!("pipeline.{layer_name}"),
                Status::Warn,
                "layer absent from manifest",
            ));
            continue;
        };
        let mut stamps: Vec<DateTime<Utc>> = layer_timestamps(layer)
            .iter()
            .filter_map(|t| parse_iso(t))
            .collect();
        stamps.sort();
        let Some(&last) = stamps.last() else {
            checks.push(Check::new(
                format!("pipeline.{layer_name}"),
                Status::Fail,
                "no parseable timestamps",
            ));
            continue;
        };

        match *exp {
            Expectation::Observed {
                warn_s,
                fail_s,
                cadence_s,
            } => {
                let age = seconds_between(now, last);
                let mut status = if age <= warn_s {
                    Status::Ok
                } else if age <= fail_s {
                    Status::Warn
                } else {
                    Status::Fail
                };
                let window_start = now - ChronoDuration::hours(1);
                let got = stamps.iter().filter(|t| **t >= window_start).count();
                let want = (3600.0 / cadence_s) as i64;
                let comp = got as f64 / want as f64;
                if status == Status::Ok && comp < 0.8 {
                    status = Status::Warn;
                }
                checks.push(
                    Check::new(
                        format!("pipeline.{layer_name}"),
                        status,
                        format!(
                            "latest {} old · {got}/{want} frames in trailing hour ({:.0}%) · {} total",
                            age_str(age),
                            comp * 100.0,
                            stamps.len()
                        ),
                    )
                    .with_data(Json::Object(vec![
                        ("age_s".into(), Json::Int(age as i64)),
                        ("frames_hour".into(), Json::Int(got as i64)),
                        ("expected_hour".into(), Json::Int(want)),
                    ])),
                );
            }
            Expectation::Forecast { min_horizon_s, .. } => {
                let horizon = seconds_between(last, now);
                let status = if horizon >= min_horizon_s {
                    Status::Ok
                } else if horizon > 0.0 {
                    Status::Warn
                } else {
                    Status::Fail
                };
                checks.push(
                    Check::new(
                        format!("pipeline.{layer_name}"),
                        status,
                        format!(
                            "forecast horizon +{} (want ≥ +{}) · {} valid times",
                            age_str(horizon.max(0.0)),
                            age_str(min_horizon_s),
                            stamps.len()
                        ),
                    )
                    .with_data(Json::Object(vec![
                        ("horizon_s".into(), Json::Int(horizon as i64)),
                        ("timestamps".into(), Json::Int(stamps.len() as i64)),
                    ])),
                );
            }
        }
    }

    let mut unknown: Vec<String> = manifest
        .get("layers")
        .and_then(Value::as_object)
        .map(|l| l.keys().cloned().collect())
        .unwrap_or_default();
    unknown.retain(|name| !LAYER_EXPECTATIONS.iter().any(|(known, _)| known == name));
    unknown.sort();
    if !unknown.is_empty() {
        checks.push(Check::new(
            "pipeline.unknown-layers",
            Status::Warn,
            format!(
                "layers with no freshness expectation: {} (add them to LAYER_EXPECTATIONS in crates/radar-debug/src/checks.rs)",
                crate::model::py_repr_list(&unknown)
            ),
        ));
    }

    match lightning {
        Ok(l) => {
            let generated = l
                .get("generated_at")
                .filter(|v| !is_falsy(v))
                .and_then(Value::as_f64);
            let n = l
                .get("features")
                .and_then(Value::as_array)
                .map_or(0, Vec::len);
            let age = generated.map(|g| now.timestamp() as f64 - g);
            let stale = age.is_some_and(|a| a > 120.0);
            let detail = match age {
                Some(a) => format!("{n} strikes, written {} ago", age_str(a)),
                None => format!("{n} strikes, no generated_at (service down or quiet)"),
            };
            let status = if stale || age.is_none() {
                Status::Warn
            } else {
                Status::Ok
            };
            checks.push(Check::new("pipeline.lightning", status, detail));
        }
        Err(e) => checks.push(Check::new(
            "pipeline.lightning",
            Status::Fail,
            format!("unreachable: {e}"),
        )),
    }

    checks
}

/// Replay the app's radar playback access pattern: a 3x3 viewport at `zoom` for the last `frames`
/// frames, judged against the 420 ms playback tick. A cold client needs each frame within one tick.
pub async fn check_client_sim(
    http: &Http,
    server: &str,
    lat: f64,
    lon: f64,
    zoom: u32,
    frames: usize,
    layer: &str,
) -> Vec<Check> {
    let (mf, manifest) = get_manifest(http, server).await;
    let Some(manifest) = manifest else {
        return vec![Check::new(
            "client.manifest",
            Status::Fail,
            format!(
                "cannot load manifest: {}",
                mf.error.as_deref().unwrap_or("None")
            ),
        )];
    };
    let lyr = manifest.get("layers").and_then(|l| l.get(layer));
    let timestamps = lyr.map(layer_timestamps).unwrap_or_default();
    let Some(lyr) = lyr.filter(|_| !timestamps.is_empty()) else {
        return vec![Check::new(
            "client.frames",
            Status::Fail,
            format!("layer '{layer}' has no frames"),
        )];
    };

    let template = template_of(&manifest);
    let palette = first_palette(lyr);
    // Python `timestamps[-frames:]`: a zero count selects everything.
    let window: Vec<String> = if frames == 0 || frames >= timestamps.len() {
        timestamps.clone()
    } else {
        timestamps[timestamps.len() - frames..].to_vec()
    };

    let (cx, cy) = slippy(lat, lon, zoom);
    let mut coords = Vec::new();
    for dy in [-1i64, 0, 1] {
        for dx in [-1i64, 0, 1] {
            coords.push((cx + dx, cy + dy));
        }
    }

    let mut checks = Vec::new();
    let mut total_bytes = 0usize;
    let mut frame_times = Vec::new();
    for ts in &window {
        let path = frame_path(lyr, ts);
        let urls: Vec<String> = coords
            .iter()
            .map(|&(x, y)| tile_url(server, &template, layer, &palette, &path, (zoom, x, y)))
            .collect();
        let fetches = fetch_all(http, urls, 6).await;
        // Parallel fetch: the slowest tile gates the frame.
        let frame_ms = fetches
            .iter()
            .map(|f| f.elapsed_ms)
            .fold(f64::MIN, f64::max);
        let misses = fetches.iter().filter(|f| !f.ok()).count();
        total_bytes += fetches.iter().map(|f| f.bytes).sum::<usize>();
        frame_times.push(frame_ms);
        let status = if frame_ms <= TICK_BUDGET_MS && misses == 0 {
            Status::Ok
        } else {
            Status::Warn
        };
        checks.push(
            Check::new(
                format!("client.frame[{ts}]"),
                status,
                format!(
                    "viewport z{zoom} loaded in {:.0}ms ({}/{} tiles)",
                    frame_ms,
                    fetches.len() - misses,
                    fetches.len()
                ),
            )
            .with_data(Json::Object(vec![
                (
                    "frame_ms".into(),
                    Json::Float(radar_core::time::round_digits(frame_ms, 1)),
                ),
                ("misses".into(), Json::Int(misses as i64)),
            ])),
        );
    }

    let worst_ms = frame_times.iter().copied().fold(f64::MIN, f64::max);
    let verdict = if worst_ms <= TICK_BUDGET_MS {
        Status::Ok
    } else {
        Status::Warn
    };
    let tail = if verdict == Status::Ok {
        "cold playback should be smooth"
    } else {
        "cold clients will see blank frames on the first loop (server-cached replays are fine)"
    };
    checks.push(
        Check::new(
            "client.playback",
            verdict,
            format!(
                "worst frame {:.0}ms vs {:.0}ms playback tick · {}KB for {} frames — {tail}",
                worst_ms,
                TICK_BUDGET_MS,
                total_bytes / 1024,
                window.len()
            ),
        )
        .with_data(Json::Object(vec![
            (
                "worst_frame_ms".into(),
                Json::Float(radar_core::time::round_digits(worst_ms, 1)),
            ),
            ("budget_ms".into(), Json::Float(TICK_BUDGET_MS)),
            ("total_kb".into(), Json::Int((total_bytes / 1024) as i64)),
        ])),
    );
    checks
}

/// Layers of a manifest sorted by name (`sorted(manifest["layers"].items())`).
fn sorted_layers(manifest: &Value) -> Vec<(&str, &Value)> {
    let mut layers: Vec<(&str, &Value)> = manifest
        .get("layers")
        .and_then(Value::as_object)
        .map(|l| l.iter().map(|(k, v)| (k.as_str(), v)).collect())
        .unwrap_or_default();
    layers.sort_by(|a, b| a.0.cmp(b.0));
    layers
}

/// Fetch many URLs with bounded concurrency, returning results in input order.
async fn fetch_all(http: &Http, urls: Vec<String>, concurrency: usize) -> Vec<Fetch> {
    futures::stream::iter(urls)
        .map(|url| async move { http.get(&url).await })
        .buffered(concurrency)
        .collect()
        .await
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Duration as D;
    use serde_json::json;

    #[test]
    fn slippy_conus_center_at_z4() {
        assert_eq!(slippy(39.5, -98.35, 4), (3, 6));
    }

    #[test]
    fn tile_url_fills_path_and_timestamp_templates() {
        let server = "http://x";
        let ts = "2026-07-04T20:02:00+00:00";
        assert_eq!(
            tile_url(
                server,
                "/tiles/{layer}/{palette}/{path}/{z}/{x}/{y}.png",
                "radar",
                "classic",
                ts,
                (4, 3, 6)
            ),
            "http://x/tiles/radar/classic/2026-07-04T20%3A02%3A00%2B00%3A00/4/3/6.png"
        );
        assert_eq!(
            tile_url(
                server,
                DEFAULT_TILE_TEMPLATE,
                "radar",
                "classic",
                ts,
                (4, 3, 6)
            ),
            "http://x/tiles/radar/classic/2026-07-04T20%3A02%3A00%2B00%3A00/4/3/6.png"
        );
    }

    #[test]
    fn layer_template_and_max_zoom_override_the_defaults() {
        let layer = json!({
            "tile_url_template": "/tiles/satellite/{path}/{z}/{x}/{y}.png",
            "frames": [{"timestamp": "t1", "path": "p1", "max_zoom": 7}, {"timestamp": "t2"}],
        });
        assert_eq!(
            layer_template(&layer, DEFAULT_TILE_TEMPLATE),
            "/tiles/satellite/{path}/{z}/{x}/{y}.png"
        );
        assert_eq!(layer_template(&json!({}), "/d"), "/d");
        assert_eq!(frame_max_zoom(&layer, "t1"), Some(7));
        assert_eq!(frame_max_zoom(&layer, "t2"), None);
    }

    #[test]
    fn frame_path_prefers_explicit_path() {
        let layer = json!({"frames": [{"timestamp": "t1", "path": "runs/a/t1"}]});
        assert_eq!(frame_path(&layer, "t1"), "runs/a/t1");
        assert_eq!(frame_path(&layer, "t2"), "t2");
    }

    /// Mirrors the Python `test_pipeline_freshness_logic`.
    #[test]
    fn pipeline_freshness_logic() {
        let now = parse_iso("2026-10-14T12:00:00+00:00").unwrap();
        let fresh: Vec<String> = (0..30)
            .rev()
            .map(|i| (now - D::minutes(2 * i)).to_rfc3339())
            .collect();
        let stale = vec![(now - D::hours(2)).to_rfc3339()];
        let manifest = json!({
            "layers": {
                "radar": {"timestamps": fresh, "latest": fresh.last().unwrap(), "palettes": ["classic"]},
                "nowcast": {"timestamps": stale, "latest": stale[0]},
            },
            "tile_url_template": "/tiles/{layer}/{palette}/{timestamp}/{z}/{x}/{y}.png",
        });
        let health = json!({"status": "ok", "reasons": [], "nowcast": {"status": "ok"}});
        let lightning = json!({"features": [], "generated_at": now.timestamp() as f64});
        let checks = pipeline_checks(now, Ok(health), Ok(manifest), Ok(lightning));
        let by_name = |n: &str| checks.iter().find(|c| c.name == n).unwrap();

        let radar = by_name("pipeline.radar");
        assert_eq!(radar.status, Status::Ok);
        assert_eq!(
            radar.data.clone(),
            Json::Object(vec![
                ("age_s".into(), Json::Int(0)),
                ("frames_hour".into(), Json::Int(30)),
                ("expected_hour".into(), Json::Int(30)),
            ])
        );
        // A forecast layer whose horizon has collapsed into the past must fail.
        assert_eq!(by_name("pipeline.nowcast").status, Status::Fail);
        // HRRR layers absent from the manifest only warn.
        assert_eq!(by_name("pipeline.temperature").status, Status::Warn);
        assert_eq!(by_name("pipeline.lightning").status, Status::Ok);
    }

    #[test]
    fn unknown_layers_are_reported_sorted() {
        let now = parse_iso("2026-10-14T12:00:00+00:00").unwrap();
        let manifest = json!({"layers": {"zeta": {"timestamps": []}, "alpha": {"timestamps": []}}});
        let checks = pipeline_checks(
            now,
            Ok(json!({"status": "ok"})),
            Ok(manifest),
            Ok(json!({})),
        );
        let unknown = checks
            .iter()
            .find(|c| c.name == "pipeline.unknown-layers")
            .unwrap();
        assert!(
            unknown
                .detail
                .starts_with("layers with no freshness expectation: ['alpha', 'zeta']")
        );
    }

    #[test]
    fn health_reasons_and_failed_health_endpoint() {
        let now = parse_iso("2026-10-14T12:00:00+00:00").unwrap();
        let health = json!({"status": "stale", "reasons": ["mrms"], "mrms_age_s": 1234});
        let checks = pipeline_checks(
            now,
            Ok(health),
            Err("manifest down".into()),
            Err("HTTP 500".into()),
        );
        assert_eq!(checks[0].status, Status::Fail);
        assert_eq!(
            checks[0].detail,
            "status=stale mrms_age=1234s reasons=['mrms'] nowcast=None"
        );
        assert_eq!(checks[1].name, "pipeline.manifest");
        assert_eq!(checks[1].detail, "cannot load manifest: manifest down");

        // A manifest failure ends the run (as in Python), so there is no lightning check after it.
        assert_eq!(checks.len(), 2);

        let checks = pipeline_checks(
            now,
            Err("HTTP 503".into()),
            Err("x".into()),
            Err("y".into()),
        );
        assert_eq!(checks[0].detail, "/api/health unreachable: HTTP 503");
    }

    #[test]
    fn forecast_horizon_zero_reports_zero_seconds() {
        let now = parse_iso("2026-10-14T12:00:00+00:00").unwrap();
        let past = (now - D::minutes(5)).to_rfc3339();
        let manifest = json!({"layers": {"nowcast": {"timestamps": [past]}}});
        let checks = pipeline_checks(
            now,
            Ok(json!({"status": "ok"})),
            Ok(manifest),
            Ok(json!({})),
        );
        let c = checks
            .iter()
            .find(|c| c.name == "pipeline.nowcast")
            .unwrap();
        assert_eq!(c.status, Status::Fail);
        assert_eq!(
            c.detail,
            "forecast horizon +0s (want ≥ +10m00s) · 1 valid times"
        );
    }

    #[test]
    fn expectations_cover_the_layers_the_python_watched() {
        let names: Vec<&str> = LAYER_EXPECTATIONS.iter().map(|(n, _)| *n).collect();
        assert_eq!(names.len(), 12);
        assert_eq!(names[0], "radar");
        assert_eq!(names[11], "cloud");
    }

    #[test]
    fn cache_contract_constants() {
        assert_eq!(TILE_CACHE, "max-age=86400");
    }
}
