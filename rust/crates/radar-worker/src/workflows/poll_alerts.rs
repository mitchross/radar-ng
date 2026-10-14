//! `PollAlertsWorkflow` (spec 4.10). Schedule `poll-alerts`.

use radar_types::payloads::alerts::{
    AlertForSignal, GeometryObject, MarkAlertsSeenInput, PollAlertsResult, SignalWatchesInput,
    SignalWatchesResult,
};
use radar_types::retry::{
    FETCH_NWS_ACTIVE_ALERTS, MARK_ALERTS_SEEN, SIGNAL_MATCHING_STORM_WATCHES,
};
use temporalio_macros::{workflow, workflow_methods};
use temporalio_sdk::workflows::join_all;
use temporalio_sdk::{WorkflowContext, WorkflowResult};

use super::options;
use crate::activities::alerts::AlertsActivities;

/// Split fetched alerts into IDs handled without a signal (zoneless: empty geometry) and the
/// `(alert_id, geometry)` pairs that must be signalled. Order within each list follows the input.
pub fn partition(new_alerts: &[AlertForSignal]) -> (Vec<String>, Vec<(String, GeometryObject)>) {
    let mut handled = Vec::new();
    let mut to_signal = Vec::new();
    for alert in new_alerts {
        if alert.geometry.is_empty() {
            handled.push(alert.alert_id.clone());
        } else {
            to_signal.push((alert.alert_id.clone(), alert.geometry.clone()));
        }
    }
    (handled, to_signal)
}

/// Workflow type `PollAlertsWorkflow`.
#[workflow]
#[derive(Default)]
pub struct PollAlertsWorkflow;

#[workflow_methods]
impl PollAlertsWorkflow {
    /// Fetch alerts, signal matching watches concurrently, then commit the handled IDs. A failed signal
    /// leaves its alert unhandled, so the next poll retries it.
    #[run]
    pub async fn run(ctx: &mut WorkflowContext<Self>) -> WorkflowResult<PollAlertsResult> {
        let fetched = ctx
            .execute_activity(
                AlertsActivities::fetch_nws_active_alerts,
                (),
                options(&FETCH_NWS_ACTIVE_ALERTS),
            )
            .await?;

        let (mut handled, to_signal) = partition(&fetched.new_alerts);

        let ctx_ref: &WorkflowContext<Self> = ctx;
        let signals = to_signal.into_iter().map(|(alert_id, geometry)| async move {
            let input = SignalWatchesInput { alert_id: alert_id.clone(), geometry };
            match ctx_ref
                .execute_activity(AlertsActivities::signal_matching_storm_watches, input, options(&SIGNAL_MATCHING_STORM_WATCHES))
                .await
            {
                Ok(SignalWatchesResult { matched }) => Some((alert_id, matched)),
                Err(err) => {
                    tracing::warn!(alert_id = %alert_id, error = %err, "signal failed; alert stays unseen");
                    None
                }
            }
        });
        let outcomes: Vec<Option<(String, i64)>> = join_all(signals).await;

        let mut signaled_watches = 0i64;
        let mut failed_alerts = 0i64;
        for outcome in outcomes {
            match outcome {
                None => failed_alerts += 1,
                Some((alert_id, matched)) => {
                    handled.push(alert_id);
                    signaled_watches += matched;
                }
            }
        }

        ctx.execute_activity(
            AlertsActivities::mark_alerts_seen,
            MarkAlertsSeenInput {
                handled_ids: handled,
            },
            options(&MARK_ALERTS_SEEN),
        )
        .await?;

        Ok(PollAlertsResult {
            total: fetched.alert_count,
            new: fetched.new_alert_ids.len() as i64,
            signaled_watches,
            failed_alerts,
            new_ids: fetched.new_alert_ids,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn alert(id: &str, geometry: GeometryObject) -> AlertForSignal {
        AlertForSignal {
            alert_id: id.into(),
            geometry,
        }
    }

    fn polygon() -> GeometryObject {
        serde_json::json!({"type": "Polygon", "coordinates": [[[0, 0], [1, 0], [1, 1], [0, 0]]]})
            .as_object()
            .cloned()
            .unwrap()
    }

    #[test]
    fn zoneless_alerts_are_handled_without_signalling() {
        let alerts = vec![
            alert("a", GeometryObject::new()),
            alert("b", polygon()),
            alert("c", GeometryObject::new()),
        ];
        let (handled, to_signal) = partition(&alerts);
        assert_eq!(handled, vec!["a".to_string(), "c".to_string()]);
        assert_eq!(to_signal.len(), 1);
        assert_eq!(to_signal[0].0, "b");
    }

    #[test]
    fn partition_keeps_input_order() {
        let alerts = vec![alert("z", polygon()), alert("y", polygon())];
        let (_, to_signal) = partition(&alerts);
        let ids: Vec<&str> = to_signal.iter().map(|(id, _)| id.as_str()).collect();
        assert_eq!(ids, vec!["z", "y"]);
    }
}
