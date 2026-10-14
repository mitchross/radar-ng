//! Nowcast payloads (`NowcastWorkflow` and `nowcast_run`).
//!
//! Source: `temporal/workflows/nowcast.py`, `backend/nowcast/activities.py`.

use serde::{Deserialize, Serialize};

use super::codec::finite_f64;

/// `NowcastResult`: output of `nowcast_run` and `NowcastWorkflow`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct NowcastResult {
    /// False when the run was skipped or failed (see spec-nowcast section 2.4).
    pub ran: bool,
    /// Timestamp of the newest input grid, ISO 8601, or `null`.
    #[serde(default)]
    pub anchor_ts: Option<String>,
    /// Number of lead times produced (`horizon / step`), `0` when not run.
    #[serde(default)]
    pub leadtimes: i64,
    /// Sorted palettes that produced at least one tile.
    #[serde(default)]
    pub palettes: Vec<String>,
    /// Run duration in seconds, rounded to two decimal places.
    #[serde(default, serialize_with = "finite_f64")]
    pub duration_s: f64,
    /// Earlier runs that predicted this anchor and were scored against it.
    #[serde(default)]
    pub skill_scored: i64,
}
