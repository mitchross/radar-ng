//! `GET /api/wind-field/{timestamp}`: decimated, int8-scaled U/V wind (spec 5.6).
//!
//! Resolution: a sanitized timestamp (or the newest timestamp present in both `wind_u` and
//! `wind_v`, for `latest` or when the pair is missing). Both metadata files and both `.bin` files
//! are read whole. Samples are taken on a stride so the output is about 240 by 120, and each
//! channel is scaled to `-128..126` with NaN as `-128`.
//!
//! Defect fixes (spec 16): D18, a `v` grid whose dimensions differ from `u`, or a data file too
//! short for its metadata, answers `grid_read_failed`. It no longer raises (500).

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use axum::extract::{Path as AxPath, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use bytes::Bytes;
use radar_core::pycompat::py_int;
use radar_core::time::round_half_even;
use serde_json::{Value, json};

use crate::respond;
use crate::state::AppState;

/// Target decimated width and height (`target_w`, `target_h`).
const TARGET_W: usize = 240;
const TARGET_H: usize = 120;

/// What the handler needs: the JSON body and whether it may carry `Cache-Control: max-age=300`.
pub struct WindOutcome {
    /// The encoded response body.
    pub body: Bytes,
    /// True for a successful (cacheable) response.
    pub cacheable: bool,
}

fn sanitize(raw: &str) -> String {
    raw.chars()
        .filter(|c| c.is_alphanumeric() || ":-_+.T".contains(*c))
        .collect()
}

/// Stems of `*.meta.json` in `dir`, with `.meta` removed as the Python `replace(".meta", "")` does.
fn meta_stems(dir: &Path) -> BTreeSet<String> {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return BTreeSet::new();
    };
    entries
        .filter_map(Result::ok)
        .filter_map(|e| {
            let name = e.file_name().to_string_lossy().into_owned();
            let stem = name.strip_suffix(".json")?;
            if !stem.ends_with(".meta") {
                return None;
            }
            Some(stem.replace(".meta", ""))
        })
        .collect()
}

/// `_grid_binary_path`: the sibling `data_file`, or `<name>.bin` for legacy metadata.
fn bin_path_for(meta_path: &Path, meta: &Value) -> Result<PathBuf, ()> {
    let parent = meta_path.parent().ok_or(())?;
    match meta.get("data_file").and_then(Value::as_str) {
        Some(file) if !file.is_empty() => {
            if file.contains('/') || file.contains('\\') || file == "." || file == ".." {
                return Err(());
            }
            Ok(parent.join(file))
        }
        _ => {
            let name = meta_path
                .file_name()
                .ok_or(())?
                .to_string_lossy()
                .into_owned();
            Ok(parent.join(name.replace(".meta.json", ".bin")))
        }
    }
}

fn read_meta(path: &Path) -> Result<Value, ()> {
    let text = std::fs::read_to_string(path).map_err(|_| ())?;
    serde_json::from_str(&text).map_err(|_| ())
}

fn float_of(meta: &Value, key: &str) -> Result<f64, ()> {
    meta.get(key).and_then(Value::as_f64).ok_or(())
}

fn scale(value: f64, low: f64, span: f64) -> i64 {
    if value.is_finite() {
        round_half_even((value - low) / span * 254.0 - 127.0) as i64
    } else {
        -128
    }
}

/// The success body, or a `grid_read_failed` / `grid_empty` reason.
fn compute(u_meta_path: &Path, v_meta_path: &Path, ts: &str) -> Result<Value, &'static str> {
    let u_m = read_meta(u_meta_path).map_err(|()| "grid_read_failed")?;
    let v_m = read_meta(v_meta_path).map_err(|()| "grid_read_failed")?;
    let h = py_int(u_m.get("height").unwrap_or(&Value::Null)).ok_or("grid_read_failed")?;
    let w = py_int(u_m.get("width").unwrap_or(&Value::Null)).ok_or("grid_read_failed")?;
    if h < 1 || w < 1 {
        return Err("grid_read_failed");
    }
    let (h, w) = (h as usize, w as usize);
    let v_h = py_int(v_m.get("height").unwrap_or(&Value::Null));
    let v_w = py_int(v_m.get("width").unwrap_or(&Value::Null));
    if v_h != Some(h as i64) || v_w != Some(w as i64) {
        return Err("grid_read_failed"); // D18: the v grid must match the u grid.
    }
    let u_bin = std::fs::read(bin_path_for(u_meta_path, &u_m).map_err(|()| "grid_read_failed")?)
        .map_err(|_| "grid_read_failed")?;
    let v_bin = std::fs::read(bin_path_for(v_meta_path, &v_m).map_err(|()| "grid_read_failed")?)
        .map_err(|_| "grid_read_failed")?;
    let need = h * w * 4;
    if u_bin.len() < need || v_bin.len() < need {
        return Err("grid_read_failed");
    }

    let sx = (w / TARGET_W).max(1);
    let sy = (h / TARGET_H).max(1);
    let out_w = w / sx;
    let out_h = h / sy;
    let sample = |bin: &[u8], index: usize| -> f64 {
        let o = index * 4;
        f64::from(f32::from_le_bytes([
            bin[o],
            bin[o + 1],
            bin[o + 2],
            bin[o + 3],
        ]))
    };
    let mut u_vals = Vec::with_capacity(out_w * out_h);
    let mut v_vals = Vec::with_capacity(out_w * out_h);
    for ry in 0..out_h {
        for rx in 0..out_w {
            let src = (ry * sy) * w + rx * sx;
            u_vals.push(sample(&u_bin, src));
            v_vals.push(sample(&v_bin, src));
        }
    }

    let finite: Vec<(f64, f64)> = u_vals
        .iter()
        .zip(&v_vals)
        .filter(|(u, v)| u.is_finite() && v.is_finite())
        .map(|(u, v)| (*u, *v))
        .collect();
    if finite.is_empty() {
        return Err("grid_empty");
    }
    let u_min = finite.iter().map(|p| p.0).fold(f64::INFINITY, f64::min);
    let u_max = finite.iter().map(|p| p.0).fold(f64::NEG_INFINITY, f64::max);
    let v_min = finite.iter().map(|p| p.1).fold(f64::INFINITY, f64::min);
    let v_max = finite.iter().map(|p| p.1).fold(f64::NEG_INFINITY, f64::max);
    let u_span = (u_max - u_min).max(1e-6);
    let v_span = (v_max - v_min).max(1e-6);
    let u_scaled: Vec<i64> = u_vals.iter().map(|u| scale(*u, u_min, u_span)).collect();
    let v_scaled: Vec<i64> = v_vals.iter().map(|v| scale(*v, v_min, v_span)).collect();

    let lat_max = float_of(&u_m, "lat_max").map_err(|()| "grid_read_failed")?;
    let lat_min = float_of(&u_m, "lat_min").map_err(|()| "grid_read_failed")?;
    let lon_min = float_of(&u_m, "lon_min").map_err(|()| "grid_read_failed")?;
    let lon_max = float_of(&u_m, "lon_max").map_err(|()| "grid_read_failed")?;
    let lat_step = (lat_max - lat_min) / ((h - 1).max(1) as f64);
    let lon_step = (lon_max - lon_min) / ((w - 1).max(1) as f64);
    let out_lat_min = lat_max - (((out_h - 1) * sy) as f64) * lat_step;
    let out_lon_max = lon_min + (((out_w - 1) * sx) as f64) * lon_step;

    Ok(json!({
        "ok": true,
        "timestamp": ts,
        "width": out_w,
        "height": out_h,
        "lat_min": out_lat_min,
        "lat_max": lat_max,
        "lon_min": lon_min,
        "lon_max": out_lon_max,
        "u_min": u_min,
        "u_max": u_max,
        "v_min": v_min,
        "v_max": v_max,
        "u": u_scaled,
        "v": v_scaled,
    }))
}

/// The whole endpoint logic, synchronous. Run it on the blocking pool.
#[must_use]
pub fn wind_field(state: &AppState, raw_timestamp: &str) -> WindOutcome {
    let grid = &state.config.grid_dir;
    let u_dir = grid.join("wind_u");
    let v_dir = grid.join("wind_v");
    let mut ts = sanitize(raw_timestamp);
    let pair_present = u_dir.join(format!("{ts}.meta.json")).exists()
        && v_dir.join(format!("{ts}.meta.json")).exists();
    if ts.eq_ignore_ascii_case("latest") || !pair_present {
        let common: BTreeSet<String> = meta_stems(&u_dir)
            .intersection(&meta_stems(&v_dir))
            .cloned()
            .collect();
        match common.iter().next_back() {
            Some(last) => ts = last.clone(),
            None => {
                return WindOutcome {
                    body: respond::encode(
                        &json!({ "ok": false, "reason": "grid_missing", "timestamp": raw_timestamp }),
                    ),
                    cacheable: false,
                };
            }
        }
    }
    if let Some(hit) = state.wind_cache.get(&ts) {
        return WindOutcome {
            body: hit,
            cacheable: true,
        };
    }
    let u_meta = u_dir.join(format!("{ts}.meta.json"));
    let v_meta = v_dir.join(format!("{ts}.meta.json"));
    match compute(&u_meta, &v_meta, &ts) {
        Ok(body) => {
            let body = respond::encode(&body);
            state.wind_cache.insert(ts, body.clone());
            WindOutcome {
                body,
                cacheable: true,
            }
        }
        Err(reason) => WindOutcome {
            body: respond::encode(&json!({ "ok": false, "reason": reason })),
            cacheable: false,
        },
    }
}

/// `GET /api/wind-field/{timestamp}`.
pub async fn get_wind_field(
    State(state): State<AppState>,
    AxPath(timestamp): AxPath<String>,
) -> Response {
    let st = state.clone();
    let outcome = tokio::task::spawn_blocking(move || wind_field(&st, &timestamp))
        .await
        .unwrap_or_else(|_| WindOutcome {
            body: respond::encode(&json!({ "ok": false, "reason": "grid_read_failed" })),
            cacheable: false,
        });
    let cc = if outcome.cacheable {
        Some(respond::public_max_age(300))
    } else {
        None
    };
    respond::json_bytes(StatusCode::OK, outcome.body, cc.as_deref()).into_response()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scale_maps_range_and_fill() {
        assert_eq!(scale(f64::NAN, 0.0, 1.0), -128);
        assert_eq!(scale(0.0, 0.0, 1.0), -127);
        assert_eq!(scale(1.0, 0.0, 1.0), 127);
        // 0.5 * 254 - 127 = 0 exactly; ties to even.
        assert_eq!(scale(0.5, 0.0, 1.0), 0);
    }

    #[test]
    fn sanitize_keeps_only_timestamp_characters() {
        assert_eq!(
            sanitize("2026-09-29T05:00:00+00:00/../x"),
            "2026-09-29T05:00:00+00:00..x"
        );
    }
}
