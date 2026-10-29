//! HRRR activities (spec §11.2, §11.3, §12.2): find run, horizon, process one forecast hour, publish a
//! run coherently, mark processed, cleanup. Split into `state` (pure rules), `publish`, `dump`, `render`,
//! `packed` and `activities`.

pub mod activities;
pub mod dump;
pub mod packed;
pub mod publish;
pub mod render;
pub mod state;

use chrono::{DateTime, Duration as ChronoDuration, Utc};
use radar_grib::hrrr::parse_run_id;

/// `ProcessedSet` file for HRRR runs (spec §9.2).
pub const HRRR_PROCESSED_FILE: &str = "ingest-hrrr.json";
/// `ProcessedSet` capacity for HRRR runs.
pub const HRRR_PROCESSED_MAX: usize = 200;
/// Heartbeat cadence for the HRRR download and render.
pub const HRRR_BEAT: std::time::Duration = std::time::Duration::from_secs(30);
/// Zooms rendered for HRRR (spec §6.4).
pub const HRRR_ZOOMS: [u32; 3] = [4, 5, 6];
/// Categories for `precip-type` (spec §5.5).
pub const PRECIP_TYPE_CATEGORIES: [(i64, &str); 4] = [
    (1, "rain"),
    (2, "snow"),
    (3, "freezing_rain"),
    (4, "ice_pellets"),
];

/// Valid time of forecast hour `fhr` of `init` (`run_init + fhr h`).
pub fn valid_time(init: DateTime<Utc>, fhr: u32) -> String {
    radar_grib::mrms::format_valid_time(init + ChronoDuration::hours(fhr as i64))
}

/// Init time of a run id, or an error.
pub fn run_init(run_id: &str) -> anyhow::Result<DateTime<Utc>> {
    parse_run_id(run_id).ok_or_else(|| anyhow::anyhow!("invalid HRRR run id {run_id:?}"))
}

/// `run_issued_at`: the init time as a valid-time string.
pub fn run_issued_at(run_id: &str) -> anyhow::Result<String> {
    Ok(radar_grib::mrms::format_valid_time(run_init(run_id)?))
}

/// Horizon of a run (spec §12.2 `hrrr_horizon_for_run`).
pub fn horizon_for(run_id: &str, forecast_hours: u32, extended_hours: u32) -> anyhow::Result<u32> {
    let init = run_init(run_id)?;
    use chrono::Timelike;
    Ok(radar_grib::hrrr::horizon_for_run(
        init.hour(),
        forecast_hours,
        extended_hours,
    ))
}
