//! HRRR forecast payloads (`IngestHrrrWorkflow` and the `hrrr_*` activities).
//!
//! Source: `temporal/workflows/ingest_hrrr.py`, `backend/ingest_hrrr/activities.py`.

use serde::{Deserialize, Serialize};

use super::codec::finite_f64;

/// `RETENTION_HOURS` passed to `hrrr_cleanup`.
pub const RETENTION_HOURS: i64 = 12;

/// `FindRunResult`: output of `hrrr_find_latest_run`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FindRunResult {
    /// Run ID `YYYYMMDD_HH`, or `null` when no run is available.
    pub run_id: Option<String>,
    /// Whether the run is already fully processed.
    pub already_processed: bool,
}

/// `ForecastHourResult`: output of `hrrr_process_forecast_hour`, and the
/// placeholder the workflow substitutes for a failed hour.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct ForecastHourResult {
    /// Forecast hour.
    pub fhr: i64,
    /// Layers rendered for this hour.
    #[serde(default)]
    pub rendered_layers: Vec<String>,
    /// Wall-clock duration in seconds.
    #[serde(default, serialize_with = "finite_f64")]
    pub duration_s: f64,
    /// Valid time of the hour as an ISO 8601 string, or `""`.
    #[serde(default)]
    pub valid_timestamp: String,
}

impl ForecastHourResult {
    /// The result the workflow records for a failed hour: `ForecastHourResult(fhr=fhr)`.
    pub fn failed(fhr: i64) -> Self {
        Self {
            fhr,
            ..Self::default()
        }
    }
}

/// `HrrrCleanupResult`: output of `hrrr_cleanup` (same shape as the MRMS result).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct HrrrCleanupResult {
    /// Tile directories removed.
    pub tile_dirs_removed: i64,
    /// Grid files removed.
    pub grid_files_removed: i64,
}

/// `IngestHrrrResult`: result of `IngestHrrrWorkflow`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct IngestHrrrResult {
    /// Run ID, or `null`.
    pub run_id: Option<String>,
    /// True when the run was skipped because it was already processed.
    pub skipped_already_processed: bool,
    /// Hours published (the publishable prefix), `0` on early-return paths.
    pub forecast_hours_processed: i64,
    /// Cleanup counters, `null` when cleanup did not run.
    #[serde(default)]
    pub cleanup: Option<HrrrCleanupResult>,
    /// Layers rendered per hour, ordered by `fhr`.
    #[serde(default)]
    pub layers_per_hour: Vec<Vec<String>>,
    /// Layers published by `hrrr_publish_run`.
    #[serde(default)]
    pub published_layers: Vec<String>,
    /// True when every hour in the horizon was published.
    #[serde(default)]
    pub complete: bool,
}
