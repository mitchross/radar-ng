//! NWS active alerts (spec-ingest-aux section 10, Python `storm_watch_activities.py` alert activities).
//!
//! - [`fetch_nws_active_alerts`]: fetch and diff against the seen set. Does **not** commit the seen set.
//! - [`mark_alerts_seen`]: commit the IDs whose watch signal succeeded (at-least-once).
//! - [`signal_matching_storm_watches`]: signal every running storm watch whose center is inside the alert polygon.
//!   The Temporal calls are behind the [`WatchSignaler`] trait, which the worker crate implements.

mod geo;
mod state;

use std::collections::BTreeSet;
use std::path::PathBuf;
use std::time::Duration;

use anyhow::Context;
use radar_core::env::var;
use radar_core::pycompat::{is_falsy, py_str};
use radar_types::payloads::alerts::{
    AlertForSignal, FetchAlertsResult, SignalWatchesInput, SignalWatchesResult,
};
use serde_json::{Map, Value};

pub use geo::point_in_geojson;
pub use state::{
    MAX_SEEN, SEEN_FILE, SNAPSHOT_FILE, commit_handled, load_active_snapshot, load_seen, save_seen,
    write_active_snapshot,
};

/// Default feed (`NWS_ALERTS_URL`).
pub const DEFAULT_ALERTS_URL: &str = "https://api.weather.gov/alerts/active?status=actual";
/// Default `User-Agent` (`NWS_USER_AGENT`). This is the Python code default. Production sets a real contact (spec
/// 13, D-8c).
pub const DEFAULT_USER_AGENT: &str = "(radar-ng, mitch@example.com)";
/// Timeout of the feed request (`timeout=20`).
pub const FETCH_TIMEOUT: Duration = Duration::from_secs(20);

/// Where the alert state lives and which feed to read.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AlertsConfig {
    /// `STATE_DIR`.
    pub state_dir: PathBuf,
    /// `NWS_ALERTS_URL`.
    pub alerts_url: String,
    /// `NWS_USER_AGENT`.
    pub user_agent: String,
}

impl AlertsConfig {
    /// Defaults for `state_dir`.
    #[must_use]
    pub fn new(state_dir: impl Into<PathBuf>) -> Self {
        Self {
            state_dir: state_dir.into(),
            alerts_url: DEFAULT_ALERTS_URL.to_owned(),
            user_agent: DEFAULT_USER_AGENT.to_owned(),
        }
    }

    /// Read `STATE_DIR`, `NWS_ALERTS_URL` and `NWS_USER_AGENT`. Empty values use the defaults.
    #[must_use]
    pub fn from_env() -> Self {
        let state_dir = radar_core::env::path_var("STATE_DIR", "/data/state");
        let mut cfg = Self::new(state_dir);
        if let Some(url) = var("NWS_ALERTS_URL") {
            cfg.alerts_url = url;
        }
        if let Some(agent) = var("NWS_USER_AGENT") {
            cfg.user_agent = agent;
        }
        cfg
    }
}

/// Build an [`AlertForSignal`] from one GeoJSON feature (spec 10.1, Python `_alert_from_feature`).
///
/// The ID is `properties.id` when that is truthy, otherwise `feature.id`. A falsy ID gives `None`. The geometry is
/// the feature's `geometry` when it is an object, otherwise `{}`. A non-object `properties` is treated as `{}`, a
/// deliberate change from the Python `AttributeError` (see README).
#[must_use]
pub fn alert_from_feature(feature: &Value) -> Option<AlertForSignal> {
    let feature = feature.as_object()?;
    let props = match feature.get("properties") {
        Some(Value::Object(props)) => Some(props),
        _ => None,
    };
    let from_props = props.and_then(|p| p.get("id")).filter(|v| !is_falsy(v));
    let candidate = from_props.or_else(|| feature.get("id"))?;
    if is_falsy(candidate) {
        return None;
    }
    let geometry: Map<String, Value> = match feature.get("geometry") {
        Some(Value::Object(geometry)) => geometry.clone(),
        _ => Map::new(),
    };
    Some(AlertForSignal {
        alert_id: py_str(candidate),
        geometry,
    })
}

/// The feature list of a feed body. Python `payload.get("features", []) or []`: a missing or falsy value is empty.
/// A truthy non-list is an error. A non-object body is an error.
fn features_of(payload: &Value) -> anyhow::Result<&[Value]> {
    let object = payload
        .as_object()
        .context("NWS response is not a JSON object")?;
    match object.get("features") {
        None => Ok(&[]),
        Some(value) if is_falsy(value) => Ok(&[]),
        Some(Value::Array(items)) => Ok(items),
        Some(_) => anyhow::bail!("NWS response features is not a list"),
    }
}

/// Fetch the active alert feed and diff it against the seen set (spec 10.1).
///
/// The feed is requested with `User-Agent` and `Accept: */*`, with a 20-second timeout and no redirects. A non-2xx
/// status is an error. Each feature is read through [`alert_from_feature`]. A feature that is not an object is
/// skipped, and `alert_count` still counts it. `new_alert_ids` and `new_alerts` keep feed order, and duplicate IDs
/// appear twice, as in Python. The active snapshot is written for `mark_alerts_seen`. A failed snapshot write is
/// logged and ignored.
pub async fn fetch_nws_active_alerts(config: &AlertsConfig) -> anyhow::Result<FetchAlertsResult> {
    let client = reqwest::Client::builder()
        .user_agent(config.user_agent.clone())
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .context("building NWS HTTP client")?;
    let response = client
        .get(&config.alerts_url)
        .timeout(FETCH_TIMEOUT)
        .header(reqwest::header::ACCEPT, "*/*")
        .send()
        .await
        .context("requesting NWS active alerts")?
        .error_for_status()
        .context("NWS active alerts returned an error status")?;
    let payload: Value = response
        .json()
        .await
        .context("NWS active alerts body is not JSON")?;
    let features = features_of(&payload)?;

    let state_dir = config.state_dir.clone();
    let seen = tokio::task::spawn_blocking({
        let state_dir = state_dir.clone();
        move || load_seen(&state_dir)
    })
    .await
    .context("loading alerts_seen.json")?;

    let mut active: BTreeSet<String> = BTreeSet::new();
    let mut new_alert_ids = Vec::new();
    let mut new_alerts = Vec::new();
    for feature in features {
        let Some(alert) = alert_from_feature(feature) else {
            continue;
        };
        active.insert(alert.alert_id.clone());
        if !seen.contains(&alert.alert_id) {
            new_alert_ids.push(alert.alert_id.clone());
            new_alerts.push(alert);
        }
    }

    let write_result =
        tokio::task::spawn_blocking(move || write_active_snapshot(&state_dir, &active))
            .await
            .context("writing alerts_active_snapshot.json task")?;
    if let Err(err) = write_result {
        tracing::warn!(event = "active_snapshot_write_failed", error = %err);
    }

    Ok(FetchAlertsResult {
        alert_count: features.len() as i64,
        new_alert_ids,
        new_alerts,
    })
}

/// Commit `handled_ids` to `alerts_seen.json` (spec 10.3). Idempotent. Called with an empty list too.
pub async fn mark_alerts_seen(config: &AlertsConfig, handled_ids: &[String]) -> anyhow::Result<()> {
    let state_dir = config.state_dir.clone();
    let handled = handled_ids.to_vec();
    tokio::task::spawn_blocking(move || commit_handled(&state_dir, &handled))
        .await
        .context("mark_alerts_seen task")?
        .context("writing alerts_seen.json")
}

/// A running storm watch's center, from its `getCurrentState` query (`lat` and `lng`).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct WatchCenter {
    /// Latitude, degrees.
    pub lat: f64,
    /// Longitude, degrees.
    pub lng: f64,
}

/// Temporal access that [`signal_matching_storm_watches`] needs. The worker crate implements it with the Temporal
/// client.
///
/// TODO(integration): the worker crate implements this over the Temporal client (list running `WatchStormWorkflow`
/// executions, query `getCurrentState`, signal `alertMatchSignal`). No Temporal types appear in this crate. Any error from one watch is logged and skipped. An error from [`WatchSignaler::list_running_watches`]
/// fails the whole call.
#[async_trait::async_trait]
pub trait WatchSignaler: Send + Sync {
    /// Workflow IDs of every running `WatchStormWorkflow`. The implementation pages through the results.
    async fn list_running_watches(&self) -> anyhow::Result<Vec<String>>;

    /// The watch's center from its `getCurrentState` query. `Ok(None)` when `lat` or `lng` is missing. Python
    /// `float()` failures are returned as `Err`.
    async fn watch_center(&self, workflow_id: &str) -> anyhow::Result<Option<WatchCenter>>;

    /// Send `alertMatchSignal` with the alert ID as its one positional string argument.
    async fn signal_alert_match(&self, workflow_id: &str, alert_id: &str) -> anyhow::Result<()>;
}

/// Signal every running storm watch whose center lies inside the alert geometry (spec 10.4).
///
/// Returns `matched = 0` without touching Temporal when the geometry is empty. A watch that has no center, or
/// whose center is outside, is not signalled. A failure on one watch is logged and not counted, so the alert can
/// still be marked seen (spec Q-15, D7). Only a failure to list the watches fails the call.
pub async fn signal_matching_storm_watches(
    signaler: &dyn WatchSignaler,
    input: &SignalWatchesInput,
) -> anyhow::Result<SignalWatchesResult> {
    if input.geometry.is_empty() {
        return Ok(SignalWatchesResult { matched: 0 });
    }
    let watches = signaler
        .list_running_watches()
        .await
        .context("listing running storm watches")?;
    let mut matched = 0i64;
    for workflow_id in &watches {
        let outcome: anyhow::Result<bool> = async {
            let Some(center) = signaler.watch_center(workflow_id).await? else {
                return Ok(false);
            };
            if !point_in_geojson(center.lng, center.lat, &input.geometry) {
                return Ok(false);
            }
            signaler
                .signal_alert_match(workflow_id, &input.alert_id)
                .await?;
            Ok(true)
        }
        .await;
        match outcome {
            Ok(true) => matched += 1,
            Ok(false) => {}
            Err(err) => {
                tracing::warn!(event = "signal_failed", workflow_id = %workflow_id, error = %format!("{err:#}"));
            }
        }
    }
    Ok(SignalWatchesResult { matched })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn alert_id_prefers_properties_id_over_feature_id() {
        let f = json!({"id": "feature-id", "properties": {"id": "props-id"}});
        assert_eq!(alert_from_feature(&f).unwrap().alert_id, "props-id");
        let g = json!({"id": "feature-id", "properties": {"id": ""}});
        assert_eq!(alert_from_feature(&g).unwrap().alert_id, "feature-id");
        assert!(alert_from_feature(&json!({"properties": {"id": 0}, "id": null})).is_none());
    }

    #[test]
    fn geometry_defaults_to_empty_object() {
        let f = json!({"properties": {"id": "a"}, "geometry": null});
        assert!(alert_from_feature(&f).unwrap().geometry.is_empty());
        let g =
            json!({"properties": {"id": "a"}, "geometry": {"type": "Polygon", "coordinates": []}});
        assert_eq!(alert_from_feature(&g).unwrap().geometry["type"], "Polygon");
    }

    #[test]
    fn features_of_follows_python_truthiness() {
        assert!(features_of(&json!({})).unwrap().is_empty());
        assert!(features_of(&json!({"features": null})).unwrap().is_empty());
        assert!(features_of(&json!({"features": []})).unwrap().is_empty());
        assert_eq!(features_of(&json!({"features": [1, 2]})).unwrap().len(), 2);
        assert!(features_of(&json!({"features": 5})).is_err());
        assert!(features_of(&json!([1])).is_err());
    }

    #[test]
    fn non_string_ids_use_python_str() {
        assert_eq!(
            alert_from_feature(&json!({"properties": {"id": 12345}}))
                .unwrap()
                .alert_id,
            "12345"
        );
        assert_eq!(
            alert_from_feature(&json!({"properties": {"id": true}}))
                .unwrap()
                .alert_id,
            "True"
        );
    }
}
