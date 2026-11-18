//! The ice class on observed radar: which HRRR `precip-type` grid colours an MRMS frame, shared by
//! the MRMS ingest (tiles) and the API (inspector) so both apply the same rule.

use std::path::{Path, PathBuf};

use chrono::{DateTime, Duration, Utc};

use crate::grid_read::{GridDump, nearest_grid_meta};

/// Grid dump layer of the HRRR precip-type category per valid hour (0 none, 1 rain, 2 snow,
/// 3 freezing rain, 4 ice pellets; `radar_grib::units::PRECIP_*`).
pub const PRECIP_TYPE_GRID: &str = "precip-type";
/// Largest gap between an MRMS frame and the HRRR grid used for its ice class.
pub const MAX_ICE_GRID_DELTA: Duration = Duration::minutes(90);
/// Lowest reflectivity that counts as an echo for the ice class (the packed reflectivity floor).
pub const ICE_ECHO_DBZ: f32 = 5.0;

/// Freezing rain (3) or ice pellets (4).
#[must_use]
pub fn is_ice_category(v: f32) -> bool {
    v.is_finite() && matches!(v as i32, 3 | 4)
}

/// The `precip-type` grid valid nearest `t`, at most [`MAX_ICE_GRID_DELTA`] away.
#[must_use]
pub fn nearest_type_grid(grid_dir: &Path, t: DateTime<Utc>) -> Option<PathBuf> {
    nearest_grid_meta(grid_dir, PRECIP_TYPE_GRID, t, MAX_ICE_GRID_DELTA)
}

/// Grid dump layer of each observed MRMS frame's display classes (0 rain, 1 snow, 2 mix, 3 ice).
/// Nowcast leads hold the newest one still.
pub const OBSERVED_TYPE_GRID: &str = "radar-type";
/// Observed class dumps kept on disk.
pub const OBSERVED_TYPE_KEEP: u64 = 3;
/// Largest gap between a nowcast anchor and the observed class dump it holds still.
pub const MAX_OBSERVED_TYPE_DELTA: Duration = Duration::minutes(30);

/// The observed class dump nearest the nowcast anchor `t`, at most [`MAX_OBSERVED_TYPE_DELTA`] away.
#[must_use]
pub fn nearest_observed_types(grid_dir: &Path, t: DateTime<Utc>) -> Option<PathBuf> {
    nearest_grid_meta(grid_dir, OBSERVED_TYPE_GRID, t, MAX_OBSERVED_TYPE_DELTA)
}

/// The class of `dump`'s cell nearest each `(lats[r], lons[c])`, row-major. Cells outside the dump,
/// missing values and classes above `max_class` are 0 (rain).
#[must_use]
pub fn classes_on(dump: &GridDump, lats: &[f64], lons: &[f64], max_class: u8) -> Vec<u8> {
    let m = &dump.meta;
    let mut out = vec![0u8; lats.len() * lons.len()];
    if m.height < 2 || m.width < 2 || dump.values.len() != m.height * m.width {
        return out;
    }
    let index = |f: f64, n: usize| {
        let i = f.round();
        (i >= 0.0 && i < n as f64).then_some(i as usize)
    };
    let fy = (m.height - 1) as f64 / (m.lat_max - m.lat_min);
    let fx = (m.width - 1) as f64 / (m.lon_max - m.lon_min);
    let cols: Vec<Option<usize>> = lons
        .iter()
        .map(|&lon| {
            let lon = if lon > 180.0 { lon - 360.0 } else { lon };
            index((lon - m.lon_min) * fx, m.width)
        })
        .collect();
    for (r, &lat) in lats.iter().enumerate() {
        let Some(row) = index((m.lat_max - lat) * fy, m.height) else {
            continue;
        };
        let src = &dump.values[row * m.width..(row + 1) * m.width];
        let dst = &mut out[r * lons.len()..(r + 1) * lons.len()];
        for (d, col) in dst.iter_mut().zip(&cols) {
            if let Some(col) = *col {
                let v = src[col];
                if v.is_finite() && v >= 0.0 && v <= f32::from(max_class) {
                    *d = v.round() as u8;
                }
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::grid_dump::{GridConfig, GridInput, write_grid};
    use crate::grid_read::read_grid;

    #[test]
    fn observed_classes_resample_by_nearest_cell() {
        let dir = tempfile::tempdir().unwrap();
        let cfg = GridConfig::with_dir(dir.path());
        // 3x4 grid, north first: snow on the top row, ice in one cell, a bad value elsewhere.
        let lats = [42.0, 41.0, 40.0];
        let lons = [-90.0, -89.0, -88.0, -87.0];
        let data = [
            1.0,
            1.0,
            1.0,
            1.0,
            0.0,
            3.0,
            0.0,
            9.0,
            2.0,
            0.0,
            f32::NAN,
            0.0,
        ];
        let ts = "2026-11-17T19:00:00Z";
        let input = GridInput::new(&data, &lats, &lons, "class");
        write_grid(&cfg, OBSERVED_TYPE_GRID, ts, &input).unwrap();
        let t: DateTime<Utc> = ts.parse().unwrap();
        let path = nearest_observed_types(dir.path(), t + Duration::minutes(10)).unwrap();
        assert!(nearest_observed_types(dir.path(), t + Duration::minutes(31)).is_none());
        let dump = read_grid(&path).unwrap();
        // A finer target grid, plus one row and one column outside the dump.
        let out = classes_on(
            &dump,
            &[42.1, 41.1, 40.0, 30.0],
            &[-89.9, -88.8, -87.0, -80.0],
            3,
        );
        assert_eq!(out, [1, 1, 1, 0, 0, 3, 0, 0, 2, 0, 0, 0, 0, 0, 0, 0]);
    }
}
