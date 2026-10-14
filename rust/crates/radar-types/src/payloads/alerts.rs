//! NWS alert payloads (`PollAlertsWorkflow` and the alert activities).
//!
//! Source: `temporal/workflows/poll_alerts.py`, `backend/api/api/storm_watch_activities.py`.

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

use super::codec::null_as_empty_object;

/// A GeoJSON geometry as a JSON object. `{}` means no geometry (zoneless alert).
pub type GeometryObject = Map<String, Value>;

/// `AlertForSignal`: one new alert with its geometry.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct AlertForSignal {
    /// NWS alert ID.
    pub alert_id: String,
    /// GeoJSON geometry, `{}` when absent or `null`.
    #[serde(default, deserialize_with = "null_as_empty_object")]
    pub geometry: GeometryObject,
}

/// `FetchAlertsResult`: output of `fetch_nws_active_alerts`.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct FetchAlertsResult {
    /// Size of the active alert set.
    pub alert_count: i64,
    /// IDs not seen before.
    #[serde(default)]
    pub new_alert_ids: Vec<String>,
    /// Details of the new alerts, in the same order as `new_alert_ids`.
    #[serde(default)]
    pub new_alerts: Vec<AlertForSignal>,
}

/// `SignalWatchesInput`: input to `signal_matching_storm_watches`.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct SignalWatchesInput {
    /// Alert to match against watch points.
    pub alert_id: String,
    /// Alert geometry, `{}` when absent.
    #[serde(default, deserialize_with = "null_as_empty_object")]
    pub geometry: GeometryObject,
}

/// `SignalWatchesResult`: output of `signal_matching_storm_watches`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SignalWatchesResult {
    /// Number of watches that were signalled.
    pub matched: i64,
}

/// `MarkAlertsSeenInput`: input to `mark_alerts_seen`. Called even with an empty list.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct MarkAlertsSeenInput {
    /// Alert IDs to commit to the seen set.
    #[serde(default)]
    pub handled_ids: Vec<String>,
}

/// `PollAlertsResult`: result of `PollAlertsWorkflow`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PollAlertsResult {
    /// Active alert count (`FetchAlertsResult.alert_count`).
    pub total: i64,
    /// Length of `new_ids`, including zoneless and failed alerts.
    pub new: i64,
    /// Sum of matched watches across alerts.
    #[serde(default)]
    pub signaled_watches: i64,
    /// Alerts whose signal activity failed as a whole.
    #[serde(default)]
    pub failed_alerts: i64,
    /// All new alert IDs.
    #[serde(default)]
    pub new_ids: Vec<String>,
}
