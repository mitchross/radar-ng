//! Storm-cell detection (spec-shared 6.3 and 6.4).
//!
//! `detect_storms` thresholds a grid, labels 4-connected components, drops small
//! ones, sorts by peak reflectivity, caps the count and attaches tracking state
//! from the previous frame.

use std::cmp::Ordering;
use std::time::{SystemTime, UNIX_EPOCH};

use serde_json::Value;

use crate::config::StormConfig;
use crate::doc::{CellFeature, StormsDoc};
use crate::geo::{KM_PER_DEG_LAT, KM_PER_DEG_LON_EQUATOR};
use crate::grid::Grid;
use crate::label::{Component, label_components};
use crate::numeric::radians;
use crate::tracking::{Candidate, assign_tracking};

/// Detect and track storm cells in one frame, using the current wall clock for `generated_at`.
///
/// `previous` is the parsed previous `storms.json`, or `None` on the first frame.
#[must_use]
pub fn detect_storms(
    grid: &Grid<'_>,
    timestamp: &str,
    previous: Option<&Value>,
    cfg: &StormConfig,
) -> StormsDoc {
    detect_storms_with_clock(grid, timestamp, previous, cfg, now_epoch_seconds())
}

/// Same as [`detect_storms`] but with an explicit wall clock (epoch seconds).
///
/// Use this for reproducible output. `wall_now` is the value written to
/// `generated_at` and, only when `timestamp` does not parse, the "now" used for
/// history retention.
#[must_use]
pub fn detect_storms_with_clock(
    grid: &Grid<'_>,
    timestamp: &str,
    previous: Option<&Value>,
    cfg: &StormConfig,
    wall_now: f64,
) -> StormsDoc {
    let (height, width) = (grid.height(), grid.width());
    let values = grid.values();
    // Python compares a float32 array to a float64 threshold, i.e. after widening. NaN is false.
    let mask: Vec<bool> = values
        .iter()
        .map(|&v| f64::from(v) >= cfg.threshold_dbz)
        .collect();
    let components = label_components(&mask, values, height, width);

    let mut candidates: Vec<Candidate> = Vec::new();
    if !components.is_empty() {
        let geo = GridGeometry::new(grid);
        for comp in &components {
            if usize::try_from(comp.size).map_or(true, |s| s < cfg.min_pixels) {
                continue;
            }
            candidates.push(geo.candidate(comp, cfg));
        }
    }
    // Stable sort: equal peaks keep scipy label order, as Python's reverse sort does.
    candidates.sort_by(|a, b| {
        b.peak_dbz
            .partial_cmp(&a.peak_dbz)
            .unwrap_or(Ordering::Equal)
    });
    candidates.truncate(cfg.max_storms);

    // Python returns early, without tracking, when no pixel passes the threshold. The
    // counter is still carried forward so IDs are not reused later (D3).
    let no_components = components.is_empty();
    let (features, next_cell_id) = assign_tracking(&candidates, previous, timestamp, wall_now, cfg);
    build_doc(
        features,
        timestamp,
        wall_now,
        cfg,
        next_cell_id,
        no_components,
    )
}

fn build_doc(
    features: Vec<CellFeature>,
    timestamp: &str,
    wall_now: f64,
    cfg: &StormConfig,
    next_cell_id: i64,
    short_shape: bool,
) -> StormsDoc {
    if short_shape {
        return StormsDoc {
            kind: "FeatureCollection".to_owned(),
            features,
            generated_at: wall_now,
            timestamp: timestamp.to_owned(),
            threshold_dbz: None,
            cell_count: None,
            next_cell_id,
        };
    }
    StormsDoc {
        kind: "FeatureCollection".to_owned(),
        generated_at: wall_now,
        timestamp: timestamp.to_owned(),
        threshold_dbz: Some(cfg.threshold_dbz),
        cell_count: Some(features.len()),
        features,
        next_cell_id,
    }
}

/// Grid-wide quantities shared by every component (spec-shared 6.3 steps 5 and 6).
struct GridGeometry {
    height: usize,
    width: usize,
    lats: Vec<f64>,
    lons_norm: Vec<f64>,
    lat_top: f64,
    lat_bottom: f64,
    lon_left: f64,
    lon_right: f64,
    dlat_km: f64,
    dlon_deg: f64,
}

impl GridGeometry {
    fn new(grid: &Grid<'_>) -> Self {
        let lats = grid.lats().to_vec();
        let lons_norm: Vec<f64> = grid
            .lons()
            .iter()
            .map(|&lon| if lon > 180.0 { lon - 360.0 } else { lon })
            .collect();
        let dlat_km = if lats.len() > 1 {
            (lats[1] - lats[0]).abs() * KM_PER_DEG_LAT
        } else {
            1.0
        };
        let dlon_deg = if lons_norm.len() > 1 {
            (lons_norm[1] - lons_norm[0]).abs()
        } else {
            0.01
        };
        Self {
            height: grid.height(),
            width: grid.width(),
            lat_top: lats[0],
            lat_bottom: lats[lats.len() - 1],
            lon_left: lons_norm[0],
            lon_right: lons_norm[lons_norm.len() - 1],
            lats,
            lons_norm,
            dlat_km,
            dlon_deg,
        }
    }

    /// Area of one grid cell at `lat` (km^2), with the same operation order as Python.
    fn cell_km2(&self, lat: f64) -> f64 {
        self.dlat_km * self.dlon_deg * KM_PER_DEG_LON_EQUATOR * radians(lat).cos()
    }

    fn candidate(&self, comp: &Component, cfg: &StormConfig) -> Candidate {
        let size = comp.size as f64;
        let row = comp.sum_row as f64 / size;
        let col = comp.sum_col as f64 / size;
        let row_den = (self.height as f64 - 1.0).max(1.0);
        let col_den = (self.width as f64 - 1.0).max(1.0);
        let lat = self.lat_top + (self.lat_bottom - self.lat_top) * (row / row_den);
        let lon = self.lon_left + (self.lon_right - self.lon_left) * (col / col_den);
        Candidate {
            lon,
            lat,
            peak_dbz: f64::from(comp.peak),
            area_km2: size * self.cell_km2(lat),
            pixel_count: comp.size,
            bbox: self.component_bbox(comp, cfg.prefetch_padding_km),
        }
    }

    /// Padded `[west, south, east, north]` (`_component_bbox`). No rounding.
    fn component_bbox(&self, comp: &Component, padding_km: f64) -> [f64; 4] {
        let row_start = comp.row_start;
        let row_stop = comp.row_stop.saturating_sub(1).max(row_start);
        let col_start = comp.col_start;
        let col_stop = comp.col_stop.saturating_sub(1).max(col_start);

        let (lat_a, lat_b) = (self.lats[row_start], self.lats[row_stop]);
        let (lon_a, lon_b) = (self.lons_norm[col_start], self.lons_norm[col_stop]);
        let (south, north) = (lat_a.min(lat_b), lat_a.max(lat_b));
        let (west, east) = (lon_a.min(lon_b), lon_a.max(lon_b));

        let center_lat = (south + north) / 2.0;
        let lat_padding = padding_km / KM_PER_DEG_LAT;
        let lon_scale = (KM_PER_DEG_LON_EQUATOR * radians(center_lat).cos()).max(1e-6);
        let lon_padding = padding_km / lon_scale;
        [
            west - lon_padding,
            south - lat_padding,
            east + lon_padding,
            north + lat_padding,
        ]
    }
}

/// Current wall clock in epoch seconds (Python `time.time()`).
#[must_use]
pub fn now_epoch_seconds() -> f64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0.0, |d| d.as_secs_f64())
}
