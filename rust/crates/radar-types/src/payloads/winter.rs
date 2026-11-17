//! Winter ingest payloads (`IngestWinterWorkflow` and the `winter_*` activities).
//!
//! The workflow argument is the product name (`nbm-snow`). Hours reuse the HRRR
//! [`ForecastHourResult`](super::hrrr::ForecastHourResult) and cleanup counters.

use serde::{Deserialize, Serialize};

use super::hrrr::HrrrCleanupResult;

/// Output of `winter_find_latest_run`.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct WinterFindResult {
    /// Run ID `YYYYMMDD_HH`, or `null` when no run is available or the product is off.
    pub run_id: Option<String>,
    /// Whether the run is already fully processed.
    #[serde(default)]
    pub already_processed: bool,
    /// Forecast hours to process, ascending.
    #[serde(default)]
    pub hours: Vec<i64>,
}

/// Result of `IngestWinterWorkflow`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct IngestWinterResult {
    /// The product the run was for.
    pub product: String,
    /// Run ID, or `null`.
    pub run_id: Option<String>,
    /// True when the run was skipped because it was already processed.
    #[serde(default)]
    pub skipped_already_processed: bool,
    /// Hours published for every layer of the product (the coherent prefix).
    #[serde(default)]
    pub frames_published: i64,
    /// True when every planned hour was published.
    #[serde(default)]
    pub complete: bool,
    /// Cleanup counters, `null` when cleanup did not run.
    #[serde(default)]
    pub cleanup: Option<HrrrCleanupResult>,
}
