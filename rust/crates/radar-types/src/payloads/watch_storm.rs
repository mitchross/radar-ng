//! WatchStorm payloads (`WatchStormWorkflow`, its handoff, and the frame and change activities).
//!
//! Source: `temporal/workflows/watch_storm.py`, `backend/api/api/storm_watch_activities.py`.

use serde::{Deserialize, Serialize};
use thiserror::Error;

use super::codec::{finite_f64, finite_opt_f64};

/// Seconds between polls (`POLL_S`); also the `wait_condition` timeout.
pub const POLL_INTERVAL_S: u64 = 60;
/// Loop iterations per run before continue-as-new (`MAX_FRAMES_PER_RUN`; counts iterations, not frames).
pub const MAX_ITERATIONS_PER_RUN: u32 = 1000;
/// Maximum run duration before continue-as-new (`MAX_RUN_DURATION`, 24 h).
pub const MAX_RUN_DURATION_S: u64 = 24 * 60 * 60;
/// The only accepted `WatchStormHandoff.schema_version`.
pub const WATCH_STORM_HANDOFF_SCHEMA_VERSION: i64 = 1;

/// Error for a handoff with an unsupported schema version. Non-retryable in Python.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
#[error("Unsupported storm-watch handoff schema")]
pub struct UnsupportedHandoffSchema {
    /// The version found in the handoff.
    pub found: i64,
}

/// `WatchStormState`: business state, returned by the `getCurrentState` query and by unpin.
///
/// All fields have the Python defaults (strings `""`, floats `0.0`, optionals `null`, counts `0`).
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct WatchStormState {
    /// Owner of the watch.
    pub user_id: String,
    /// Storm cell being watched.
    pub storm_cell_id: String,
    /// Watch point latitude.
    #[serde(serialize_with = "finite_f64")]
    pub lat: f64,
    /// Watch point longitude.
    #[serde(serialize_with = "finite_f64")]
    pub lng: f64,
    /// Timestamp of the last sampled frame.
    pub last_frame_ts: Option<String>,
    /// Max dBZ of the last sampled frame.
    #[serde(serialize_with = "finite_opt_f64")]
    pub last_max_dbz: Option<f64>,
    /// Kind of the last change notification (`intensifying`, `dissipating`, `severe`, `alert`).
    pub last_change_kind: Option<String>,
    /// Epoch seconds of the last notification attempt.
    #[serde(serialize_with = "finite_opt_f64")]
    pub last_notified_at: Option<f64>,
    /// Lifetime poll count (across continue-as-new).
    pub poll_count: i64,
    /// Lifetime count of sent notifications (not deliveries; see spec-temporal D6).
    pub push_count: i64,
}

/// `WatchStormInput`: input to `WatchStormWorkflow`. The API omits `handoff`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct WatchStormInput {
    /// Owner of the watch.
    pub user_id: String,
    /// Storm cell being watched.
    pub storm_cell_id: String,
    /// Watch point latitude.
    #[serde(serialize_with = "finite_f64")]
    pub lat: f64,
    /// Watch point longitude.
    #[serde(serialize_with = "finite_f64")]
    pub lng: f64,
    /// Carried state from a previous run, `null` on a fresh start.
    #[serde(default)]
    pub handoff: Option<WatchStormHandoff>,
}

/// `WatchStormHandoff`: state carried across continue-as-new (`patch watch-storm-handoff-v1`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct WatchStormHandoff {
    /// Must be [`WATCH_STORM_HANDOFF_SCHEMA_VERSION`].
    #[serde(default = "default_schema_version")]
    pub schema_version: i64,
    /// Business state.
    #[serde(default)]
    pub state: WatchStormState,
    /// Alert IDs not yet drained when the run ended.
    #[serde(default)]
    pub pending_alert_ids: Vec<String>,
}

fn default_schema_version() -> i64 {
    WATCH_STORM_HANDOFF_SCHEMA_VERSION
}

impl WatchStormHandoff {
    /// Rejects an unknown schema version, as `WatchStormWorkflow.__init__` does.
    ///
    /// Returns [`UnsupportedHandoffSchema`] for any version other than 1. The
    /// caller should fail the execution as non-retryable.
    pub fn check_schema_version(&self) -> Result<(), UnsupportedHandoffSchema> {
        if self.schema_version == WATCH_STORM_HANDOFF_SCHEMA_VERSION {
            Ok(())
        } else {
            Err(UnsupportedHandoffSchema {
                found: self.schema_version,
            })
        }
    }
}

/// `CompareFramesInput`: input to `compare_radar_frames`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CompareFramesInput {
    /// Watch point latitude.
    #[serde(serialize_with = "finite_f64")]
    pub lat: f64,
    /// Watch point longitude.
    #[serde(serialize_with = "finite_f64")]
    pub lng: f64,
    /// Timestamp of the previously sampled frame.
    pub prev_timestamp: Option<String>,
    /// Max dBZ of the previously sampled frame.
    #[serde(serialize_with = "finite_opt_f64")]
    pub prev_max_dbz: Option<f64>,
}

/// `CompareFramesResult`: output of `compare_radar_frames`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CompareFramesResult {
    /// True when a new frame was sampled.
    pub sampled: bool,
    /// Timestamp of the sampled frame.
    #[serde(default)]
    pub curr_timestamp: String,
    /// Max dBZ at the watch point in the sampled frame.
    #[serde(default, serialize_with = "finite_f64")]
    pub curr_max_dbz: f64,
    /// Previous max dBZ, as echoed or derived by the activity.
    #[serde(default, serialize_with = "finite_f64")]
    pub prev_max_dbz: f64,
    /// True when a previous sample existed.
    #[serde(default)]
    pub has_prev: bool,
    /// Current minus previous max dBZ.
    #[serde(default, serialize_with = "finite_f64")]
    pub max_dbz_delta: f64,
}

/// `DetectChangeInput`: input to `detect_storm_change`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DetectChangeInput {
    /// True when a previous sample existed.
    pub has_prev: bool,
    /// Current max dBZ.
    #[serde(serialize_with = "finite_f64")]
    pub curr_max_dbz: f64,
    /// Previous max dBZ.
    #[serde(serialize_with = "finite_f64")]
    pub prev_max_dbz: f64,
    /// Change in max dBZ.
    #[serde(serialize_with = "finite_f64")]
    pub max_dbz_delta: f64,
}

/// `DetectChangeResult`: output of `detect_storm_change`. `kind` is `""` when nothing changed.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DetectChangeResult {
    /// Change kind (`intensifying`, `dissipating`, `severe`, or `""`).
    pub kind: String,
    /// Human-readable summary, `""` when nothing changed.
    pub summary: String,
}
