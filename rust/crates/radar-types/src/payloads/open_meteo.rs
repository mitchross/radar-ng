//! Open-Meteo sync payloads (`OpenMeteoSyncWorkflow` and `open_meteo_sync`).
//!
//! Source: `temporal/workflows/open_meteo_sync.py`, `backend/open_meteo_sync/activities.py`.

use serde::{Deserialize, Serialize};

use super::codec::{default_one, finite_f64};

/// `OpenMeteoSyncArgs`: input to `OpenMeteoSyncWorkflow` and `open_meteo_sync`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct OpenMeteoSyncArgs {
    /// Open-Meteo model ID, for example `ncep_gfs013` or `ncep_hrrr_conus`.
    pub model: String,
    /// Comma-separated variable list (no spaces).
    pub variables: String,
    /// Days of backfill passed as `--past-days`.
    #[serde(default = "default_one")]
    pub past_days: i64,
}

/// `OpenMeteoSyncResult`: output of `open_meteo_sync` and `OpenMeteoSyncWorkflow`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct OpenMeteoSyncResult {
    /// Model that was synced.
    pub model: String,
    /// True when the subprocess exited successfully.
    pub succeeded: bool,
    /// Duration in seconds, rounded to one decimal place.
    #[serde(serialize_with = "finite_f64")]
    pub duration_s: f64,
    /// Subprocess exit code.
    #[serde(default)]
    pub return_code: i64,
    /// Tail of stderr on failure, `""` on success.
    #[serde(default)]
    pub stderr_tail: String,
}
