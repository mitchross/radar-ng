//! The ice class on observed radar: which HRRR `precip-type` grid colours an MRMS frame, shared by
//! the MRMS ingest (tiles) and the API (inspector) so both apply the same rule.

use std::path::{Path, PathBuf};

use chrono::{DateTime, Duration, Utc};

use crate::grid_read::nearest_grid_meta;

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
