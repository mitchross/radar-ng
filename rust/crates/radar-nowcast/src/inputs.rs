//! Reading grid dumps (spec-nowcast sections 3.3, 4.1, 5).
//!
//! Metadata and data come from `radar_core::grid_read`. This module adds the radar-ng rules on top:
//! the Python `_load_grid` fill normalization, the linspace axes, and the input-layer fallback.

use std::fs;
use std::path::{Path, PathBuf};

use chrono::{DateTime, Utc};
use radar_core::grid_read::{self, GridMeta};
use radar_core::time::parse_iso_as_utc;
use radar_tiler::GridGeo;

use crate::config::NowcastConfig;
use crate::field::Field2;
use crate::select::{Candidate, select_input_grids};

/// Fill used when the metadata has no `fill` key (spec 3.3).
pub const DEFAULT_FILL: f64 = -9999.0;

/// A grid read from disk with its axes, values in dBZ, no-data as `-9999.0`.
#[derive(Debug, Clone)]
pub struct LoadedGrid {
    /// `height x width` values, row 0 north.
    pub values: Field2<f32>,
    /// Latitudes, descending (north first), `height` long.
    pub lats: Vec<f64>,
    /// Longitudes, ascending, `width` long.
    pub lons: Vec<f64>,
    /// Parsed metadata.
    pub meta: GridMeta,
}

/// `numpy.linspace(start, stop, n)` with the same last-element rule.
pub fn linspace(start: f64, stop: f64, n: usize) -> Vec<f64> {
    match n {
        0 => Vec::new(),
        1 => vec![start],
        _ => {
            let step = (stop - start) / (n - 1) as f64;
            let mut out: Vec<f64> = (0..n).map(|i| i as f64 * step + start).collect();
            out[n - 1] = stop;
            out
        }
    }
}

/// Load `<key>.meta.json` and its data (Python `_load_grid`). Any read or parse failure returns `None`,
/// and the grid is skipped, as Python does.
pub fn load_grid(meta_path: &Path) -> Option<LoadedGrid> {
    let dump = match grid_read::read_grid(meta_path) {
        Ok(d) => d,
        Err(err) => {
            tracing::warn!(path = %meta_path.display(), error = %err, "grid_load_failed");
            return None;
        }
    };
    let meta = dump.meta.clone();
    let fill = if meta.fill.is_nan() {
        DEFAULT_FILL
    } else {
        meta.fill
    };
    let mut values = dump.values;
    for v in &mut values {
        if (f64::from(*v) - fill).abs() < 1e-3 {
            *v = DEFAULT_FILL as f32;
        }
    }
    let values = Field2::new(meta.width, meta.height, values).ok()?;
    let lats = linspace(meta.lat_max, meta.lat_min, meta.height);
    let lons = linspace(meta.lon_min, meta.lon_max, meta.width);
    Some(LoadedGrid {
        values,
        lats,
        lons,
        meta,
    })
}

/// Load only the values of a grid (for scoring), with the same fill rule.
pub fn load_values(meta_path: &Path) -> Option<(Field2<f32>, GridMeta)> {
    load_grid(meta_path).map(|g| (g.values, g.meta))
}

/// Directory of the input layer: `GRID_DIR/<layer>`, falling back to `GRID_DIR/radar` only when the
/// layer directory is absent and the layer is not already `radar` (spec 2.3).
pub fn input_layer_dir(cfg: &NowcastConfig) -> PathBuf {
    let dir = cfg.paths.grid_dir.join(&cfg.input_layer);
    if !dir.exists() && cfg.input_layer != "radar" {
        return cfg.paths.grid_dir.join("radar");
    }
    dir
}

/// All `*.meta.json` in `dir`, sorted by file name (Python `sorted(glob)`), as candidates.
pub fn list_metas(dir: &Path) -> Vec<Candidate<PathBuf>> {
    let Ok(read) = fs::read_dir(dir) else {
        return Vec::new();
    };
    let mut names: Vec<String> = read
        .flatten()
        .filter_map(|e| e.file_name().to_str().map(str::to_string))
        .filter(|n| n.ends_with(".meta.json"))
        .collect();
    names.sort();
    names
        .into_iter()
        .map(|name| {
            let stem = name.trim_end_matches(".meta.json").to_string();
            let time = parse_iso_as_utc(&stem);
            Candidate {
                item: dir.join(&name),
                stem,
                time,
            }
        })
        .collect()
}

/// `_list_recent_grids`: the candidates in the input layer, then `select_input_grids` (spec 4.1).
pub fn recent_input_grids(cfg: &NowcastConfig) -> Vec<Candidate<PathBuf>> {
    let metas = list_metas(&input_layer_dir(cfg));
    select_input_grids(&metas, cfg.input_step_min, cfg.input_frames)
}

/// Time of a stem, as used for matching (Python `datetime.fromisoformat`).
pub fn stem_time(stem: &str) -> Option<DateTime<Utc>> {
    parse_iso_as_utc(stem)
}

/// Axes of a regular grid from its metadata, for the tiler.
pub fn regular_geo(lats: &[f64], lons: &[f64]) -> GridGeo {
    GridGeo::Regular {
        lats: lats.to_vec(),
        lons: lons.to_vec(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn linspace_matches_numpy_endpoints_and_spacing() {
        let l = linspace(54.99, 20.01, 4);
        assert_eq!(l[0], 54.99);
        assert_eq!(l[3], 20.01);
        assert!((l[1] - 43.33).abs() < 1e-9, "{}", l[1]);
        assert_eq!(linspace(1.0, 2.0, 1), vec![1.0]);
        assert!(linspace(1.0, 2.0, 0).is_empty());
    }
}
