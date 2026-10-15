//! Global-model forecast payloads (`IngestGlobalModelWorkflow` and the `global_model_*` activities).
//!
//! The workflow argument is the layer name (`radar-gfs` or `uv`). Hours reuse the HRRR
//! [`ForecastHourResult`](super::hrrr::ForecastHourResult) and cleanup counters.

use serde::{Deserialize, Serialize};

use super::hrrr::HrrrCleanupResult;

/// Output of `global_model_find_latest_run`.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct GlobalModelFindResult {
    /// Run ID `YYYYMMDD_HH`, or `null` when no run is available or the layer is off.
    pub run_id: Option<String>,
    /// Whether the run is already fully processed.
    #[serde(default)]
    pub already_processed: bool,
    /// Forecast hours to process, ascending.
    #[serde(default)]
    pub hours: Vec<i64>,
}

/// Result of `IngestGlobalModelWorkflow`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct IngestGlobalModelResult {
    /// The layer the run was for.
    pub layer: String,
    /// Run ID, or `null`.
    pub run_id: Option<String>,
    /// True when the run was skipped because it was already processed.
    #[serde(default)]
    pub skipped_already_processed: bool,
    /// Frames published (the coherent prefix).
    #[serde(default)]
    pub frames_published: i64,
    /// True when every planned hour was published.
    #[serde(default)]
    pub complete: bool,
    /// Cleanup counters, `null` when cleanup did not run.
    #[serde(default)]
    pub cleanup: Option<HrrrCleanupResult>,
}
