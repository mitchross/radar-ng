//! Grid readers for the inspector, the nowcast point and the wind field (spec 9).
//!
//! Metadata parsing uses `radar_core::grid_read::read_meta`. Sampling reads only the four cells
//! around the point, as the Python does, with positioned reads (`pread`). `radar_core`'s
//! `GridDump::sample` needs the whole grid in memory, which is too costly per request for a
//! multi-megabyte HRRR grid. The arithmetic below copies spec 9.2 operation for operation, and a
//! test checks it against `GridDump::sample` on the same grid.
//!
//! Deliberate deviations (spec 16): NaN coordinates answer `out_of_bounds` (D7), a short data
//! file answers `read_error` (not a 500), and a degenerate grid (fewer than two rows or columns)
//! answers `meta_invalid`.

use std::fs::File;
use std::os::unix::fs::FileExt;
use std::path::{Path, PathBuf};

use axum::extract::{Path as AxPath, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use radar_core::grid_read::{GridMeta, read_meta};
use serde_json::{Value, json};

use crate::params::Issues;
use crate::respond;
use crate::state::AppState;

/// Keep only the characters in `allowed` (plus alphanumerics), as the Python sanitizers do.
fn sanitize(text: &str, extra: &str) -> String {
    text.chars()
        .filter(|c| c.is_alphanumeric() || extra.contains(*c))
        .collect()
}

/// `_grid_metadata_path(layer, timestamp, grid_key)`. `Err(())` where Python raises `ValueError`.
///
/// # Errors
/// Returns `Err(())` for a run-scoped key with an empty, `.`, `..` or illegal part.
pub fn metadata_path(
    grid_dir: &Path,
    layer: &str,
    timestamp: &str,
    grid_key: Option<&str>,
) -> Result<PathBuf, ()> {
    let safe_layer: String = layer
        .chars()
        .filter(|c| c.is_alphanumeric() || *c == '-' || *c == '_')
        .collect();
    let relative: String = match grid_key {
        None => sanitize(timestamp, ":-_+.T"),
        Some(key) => {
            if key.is_empty() {
                return Err(());
            }
            let parts: Vec<&str> = key.split('/').collect();
            for part in &parts {
                if part.is_empty()
                    || *part == "."
                    || *part == ".."
                    || !part
                        .chars()
                        .all(|c| c.is_alphanumeric() || ":-_+.T".contains(c))
                {
                    return Err(());
                }
            }
            parts.join("/")
        }
    };
    let base = if relative.is_empty() {
        grid_dir.join(&safe_layer)
    } else {
        grid_dir.join(&safe_layer).join(&relative)
    };
    let name = base.file_name().ok_or(())?.to_string_lossy().into_owned();
    let parent = base.parent().ok_or(())?;
    Ok(parent.join(format!("{name}.meta.json")))
}

fn reason(reason: &str, layer: &str, timestamp: &str) -> Value {
    json!({ "ok": false, "reason": reason, "layer": layer, "timestamp": timestamp })
}

/// `_sample_grid_point`: bilinear sample of one point, as the exact Python dict.
#[must_use]
pub fn sample_point(
    grid_dir: &Path,
    layer: &str,
    timestamp: &str,
    lat: f64,
    lon: f64,
    grid_key: Option<&str>,
) -> Value {
    let Ok(meta_path) = metadata_path(grid_dir, layer, timestamp, grid_key) else {
        return reason("grid_missing", layer, timestamp);
    };
    let meta = match read_meta(&meta_path) {
        Ok(m) => m,
        Err(radar_core::grid_read::GridReadError::MetaMissing(_)) => {
            return reason("grid_missing", layer, timestamp);
        }
        Err(err) => {
            tracing::warn!("inspect metadata read failed: {err}");
            return json!({ "ok": false, "reason": "meta_invalid" });
        }
    };
    let bin_path = meta_path
        .parent()
        .unwrap_or(Path::new("."))
        .join(&meta.data_file);
    if !bin_path.exists() {
        return json!({ "ok": false, "reason": "meta_invalid" });
    }
    // Degenerate grids (fewer than two rows or columns, or zero extent) are invalid metadata. This
    // is checked before the bounds test so that a broken grid never answers out_of_bounds.
    if meta.height < 2
        || meta.width < 2
        || meta.lon_max == meta.lon_min
        || meta.lat_max == meta.lat_min
    {
        return json!({ "ok": false, "reason": "meta_invalid" });
    }
    // Out of range, or NaN (every comparison with NaN is false, so test for "inside" instead).
    let inside =
        lat >= meta.lat_min && lat <= meta.lat_max && lon >= meta.lon_min && lon <= meta.lon_max;
    if !inside {
        return json!({ "ok": false, "reason": "out_of_bounds", "lat": lat, "lon": lon });
    }
    match bilinear(&bin_path, &meta, lat, lon) {
        Ok(Some(value)) => json!({
            "ok": true,
            "value": value,
            "unit": meta.unit,
            "layer": layer,
            "timestamp": timestamp,
            "lat": lat,
            "lon": lon,
        }),
        Ok(None) => json!({ "ok": true, "value": null, "unit": meta.unit, "reason": "no_data" }),
        Err(()) => json!({ "ok": false, "reason": "read_error" }),
    }
}

/// The four-cell read and the spec 9.2 arithmetic. `Ok(None)` is `no_data`; `Err` is `read_error`.
fn bilinear(bin_path: &Path, meta: &GridMeta, lat: f64, lon: f64) -> Result<Option<f64>, ()> {
    let (h, w) = (meta.height, meta.width);
    if h < 2 || w < 2 || meta.lon_max == meta.lon_min || meta.lat_max == meta.lat_min {
        return Err(());
    }
    let fx = (lon - meta.lon_min) / (meta.lon_max - meta.lon_min) * (w as f64 - 1.0);
    let fy = (meta.lat_max - lat) / (meta.lat_max - meta.lat_min) * (h as f64 - 1.0);
    let x0 = clamp_index(fx, w);
    let y0 = clamp_index(fy, h);
    let dx = fx - x0 as f64;
    let dy = fy - y0 as f64;

    let file = File::open(bin_path).map_err(|_| ())?;
    let read_cell = |ix: usize, iy: usize| -> Result<f64, ()> {
        let mut buf = [0u8; 4];
        let offset = ((iy * w + ix) * 4) as u64;
        file.read_exact_at(&mut buf, offset).map_err(|_| ())?;
        Ok(f64::from(f32::from_le_bytes(buf)))
    };
    let v00 = read_cell(x0, y0)?;
    let v10 = read_cell(x0 + 1, y0)?;
    let v01 = read_cell(x0, y0 + 1)?;
    let v11 = read_cell(x0 + 1, y0 + 1)?;

    let fill = meta.fill;
    let is_fill = |v: f64| v.is_nan() || (v - fill).abs() < 1e-6;
    let valid: Vec<f64> = [v00, v10, v01, v11]
        .into_iter()
        .filter(|v| !is_fill(*v))
        .collect();
    if valid.is_empty() {
        return Ok(None);
    }
    let v0 = if !(is_fill(v00) || is_fill(v10)) {
        v00 * (1.0 - dx) + v10 * dx
    } else {
        valid[0]
    };
    let v1 = if !(is_fill(v01) || is_fill(v11)) {
        v01 * (1.0 - dx) + v11 * dx
    } else {
        valid[valid.len() - 1]
    };
    Ok(Some(v0 * (1.0 - dy) + v1 * dy))
}

/// `max(0, min(int(f), n - 2))`. `int()` truncates toward zero, which `as i64` also does.
fn clamp_index(f: f64, n: usize) -> usize {
    let upper = n as i64 - 2;
    let truncated = f as i64;
    truncated.min(upper).max(0) as usize
}

/// `GET /api/inspect/{layer}/{timestamp}/{lat}/{lon}`: always 200, never cached.
pub async fn inspect_point(
    State(state): State<AppState>,
    AxPath((layer, timestamp, lat_raw, lon_raw)): AxPath<(String, String, String, String)>,
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
    let dir = state.config.grid_dir.clone();
    let body =
        tokio::task::spawn_blocking(move || sample_point(&dir, &layer, &timestamp, lat, lon, None))
            .await
            .unwrap_or_else(|_| json!({ "ok": false, "reason": "read_error" }));
    respond::json(StatusCode::OK, &body, None).into_response()
}

#[cfg(test)]
mod tests {
    use super::*;
    use radar_core::grid_read::read_grid;
    use std::io::Write;

    /// The layer directory every test grid lives in (`sample_point` is called with layer `x`).
    fn layer_dir(dir: &Path) -> PathBuf {
        let layer = dir.join("x");
        std::fs::create_dir_all(&layer).expect("layer dir");
        layer
    }

    /// Write a 3x4 grid with a known layout: value = row*10 + col, north row first.
    fn write_grid(dir: &Path, name: &str, fill: Option<f32>, values: &[f32]) -> PathBuf {
        let dir = layer_dir(dir);
        let meta_path = dir.join(format!("{name}.meta.json"));
        let bin_path = dir.join(format!("{name}.bin"));
        let mut f = File::create(&bin_path).expect("bin");
        for v in values {
            f.write_all(&v.to_le_bytes()).expect("write");
        }
        let fill_json = fill.map_or("null".to_string(), |v| format!("{v:?}"));
        let meta = format!(
            "{{\"height\":3,\"width\":4,\"lat_min\":40.0,\"lat_max\":42.0,\"lon_min\":-86.0,\"lon_max\":-83.0,\"unit\":\"dBZ\",\"fill\":{fill_json},\"stride\":1,\"data_file\":\"{name}.bin\"}}"
        );
        std::fs::write(&meta_path, meta).expect("meta");
        meta_path
    }

    fn sample_values() -> Vec<f32> {
        (0..3)
            .flat_map(|r| (0..4).map(move |c| (r * 10 + c) as f32))
            .collect()
    }

    #[test]
    fn metadata_path_legacy_and_run_scoped() {
        let dir = Path::new("/g");
        assert_eq!(
            metadata_path(dir, "radar", "2026-10-13T12:00:00+00:00", None).expect("ok"),
            PathBuf::from("/g/radar/2026-10-13T12:00:00+00:00.meta.json")
        );
        assert_eq!(
            metadata_path(dir, "nowcast", "t", Some("runs/a/b")).expect("ok"),
            PathBuf::from("/g/nowcast/runs/a/b.meta.json")
        );
        assert!(metadata_path(dir, "nowcast", "t", Some("runs/../x")).is_err());
        assert!(metadata_path(dir, "nowcast", "t", Some("")).is_err());
        assert!(metadata_path(dir, "nowcast", "t", Some("runs//x")).is_err());
    }

    #[test]
    fn point_at_cell_centre_and_corners() {
        let tmp = tempfile::tempdir().expect("tmp");
        write_grid(tmp.path(), "g", None, &sample_values());
        let meta = layer_dir(tmp.path()).join("g.meta.json");
        let g = read_grid(&meta).expect("grid");
        // NW corner (row 0, col 0) is exactly 0.0; the grid edge is lat_max, lon_min.
        let v = sample_point(tmp.path(), "x", "g", 42.0, -86.0, None);
        assert_eq!(v["value"], json!(0.0));
        assert_eq!(v["unit"], json!("dBZ"));
        // The Rust sampler and radar-core's whole-grid sampler agree on an interior point.
        let expected = match g.sample(41.3, -84.6).expect("sample") {
            radar_core::grid_read::Sample::Value(x) => x,
            other => panic!("unexpected {other:?}"),
        };
        let got = sample_point(tmp.path(), "x", "g", 41.3, -84.6, None)["value"]
            .as_f64()
            .expect("number");
        assert!(
            (got - expected).abs() < 1e-9,
            "got {got}, expected {expected}"
        );
    }

    #[test]
    fn out_of_bounds_and_nan_are_soft_failures() {
        let tmp = tempfile::tempdir().expect("tmp");
        write_grid(tmp.path(), "g", None, &sample_values());
        let far = sample_point(tmp.path(), "x", "g", 44.0, -85.0, None);
        assert_eq!(far["reason"], json!("out_of_bounds"));
        let nan = sample_point(tmp.path(), "x", "g", f64::NAN, -85.0, None);
        assert_eq!(nan["reason"], json!("out_of_bounds"));
        assert!(nan["lat"].is_null(), "NaN echoes as null");
    }

    #[test]
    fn missing_and_invalid_metadata() {
        let tmp = tempfile::tempdir().expect("tmp");
        let missing = sample_point(tmp.path(), "x", "nope", 41.0, -85.0, None);
        assert_eq!(missing["reason"], json!("grid_missing"));
        std::fs::write(layer_dir(tmp.path()).join("bad.meta.json"), "{not json").expect("write");
        let bad = sample_point(tmp.path(), "x", "bad", 41.0, -85.0, None);
        assert_eq!(bad["reason"], json!("meta_invalid"));
    }

    #[test]
    fn fill_quirk_matches_spec_example() {
        // Spec 9.2 quirk (D19): with the NW corner filled, the top edge uses the first valid value of
        // the cell (1.0) instead of a blend. Grid cells: NW fill, NE 1, SW 10, SE 11. At (41.8, -85.5)
        // dx = 0.5 and dy = 0.2: v0 = 1.0, v1 = 10 * 0.5 + 11 * 0.5 = 10.5, result = 0.8 * 1 + 0.2 * 10.5.
        let tmp = tempfile::tempdir().expect("tmp");
        let mut values = sample_values();
        values[0] = -9999.0; // NW corner (row 0, col 0)
        write_grid(tmp.path(), "f", Some(-9999.0), &values);
        let v = sample_point(tmp.path(), "x", "f", 41.8, -85.5, None);
        let got = v["value"].as_f64().expect("number");
        assert!((got - 2.9).abs() < 1e-9, "got {got}");
    }

    #[test]
    fn all_fill_is_no_data() {
        let tmp = tempfile::tempdir().expect("tmp");
        write_grid(tmp.path(), "n", Some(-9999.0), &[-9999.0; 12]);
        let v = sample_point(tmp.path(), "x", "n", 41.0, -85.0, None);
        assert_eq!(v["ok"], json!(true));
        assert!(v["value"].is_null());
        assert_eq!(v["reason"], json!("no_data"));
    }

    #[test]
    fn short_data_file_is_read_error() {
        let tmp = tempfile::tempdir().expect("tmp");
        write_grid(tmp.path(), "s", None, &sample_values());
        let bin = layer_dir(tmp.path()).join("s.bin");
        std::fs::write(&bin, [0u8; 8]).expect("truncate");
        let v = sample_point(tmp.path(), "x", "s", 41.0, -85.0, None);
        assert_eq!(v["reason"], json!("read_error"));
    }
}
