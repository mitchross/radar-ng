//! Temporal control-plane checks: schedules, recent failures, stuck runs, and retrying activities.
//!
//! Port of `tools/debug_harness/temporal_checks.py`. The expected schedule set comes from
//! `radar-types`, the same table the seeder uses. The Python fell back to a hard-coded list of 10 that
//! had already drifted (it lacked two schedules), so that fallback is gone.

use std::collections::BTreeSet;
use std::time::SystemTime;

use chrono::{DateTime, Duration as ChronoDuration, Utc};
use futures::StreamExt;
use radar_core::pycompat::Json;
use radar_types::schedules::all_schedules;
use temporalio_client::schedules::ListSchedulesOptions;
use temporalio_client::{
    Client, ClientOptions, ConnectionOptions, NamespacedClient, RpcOptions, UntypedWorkflow, Url,
    WorkflowDescribeOptions, WorkflowExecutionInfo, WorkflowExecutionStatus, WorkflowHandle,
    WorkflowListOptions,
};
use temporalio_common::protos::temporal::api::common::v1::Payloads;
use temporalio_common::protos::temporal::api::history::v1::history_event::Attributes;
use temporalio_common::protos::temporal::api::workflow::v1::PendingActivityInfo;

use crate::model::{Check, Status, age_str, py_display, py_repr_list, seconds_between};

/// How far past a schedule's cadence its last action may be before it counts as stalled.
pub const RECENT_ACTION_SLACK: f64 = 3.0;

/// Connect timeout for the frontend, so an unreachable address fails instead of hanging.
const CONNECT_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(10);

/// A running or recent workflow execution, as the list queries return it.
#[derive(Debug, Clone)]
struct Execution {
    id: String,
    run_id: String,
    workflow_type: String,
    start_time: Option<SystemTime>,
}

/// Run the control-plane checks against a Temporal frontend.
pub async fn check_temporal(
    address: &str,
    namespace: &str,
    failure_window_h: i64,
    stuck_after_h: i64,
    now: DateTime<Utc>,
) -> Vec<Check> {
    let client = match connect(address, namespace).await {
        Ok(c) => c,
        Err(e) => {
            return vec![Check::new(
                "temporal.connect",
                Status::Fail,
                format!("cannot reach {address} ({namespace}): {e}"),
            )];
        }
    };
    let mut checks = vec![Check::new(
        "temporal.connect",
        Status::Ok,
        format!("connected to {address} namespace={namespace}"),
    )];

    // -- schedules: existence, pause state, recent-action recency --------------
    let expected: Vec<(String, u64)> = all_schedules()
        .into_iter()
        .map(|s| (s.id.to_string(), s.interval_s))
        .collect();
    let mut seen: BTreeSet<String> = BTreeSet::new();
    let mut stream = client.list_schedules(ListSchedulesOptions::builder().build());
    while let Some(entry) = stream.next().await {
        match entry {
            Ok(summary) => {
                seen.insert(summary.schedule_id().to_string());
            }
            Err(e) => {
                checks.push(Check::new(
                    "temporal.schedules",
                    Status::Fail,
                    format!("list_schedules failed: {e}"),
                ));
                return checks;
            }
        }
    }

    let missing: Vec<String> = {
        let mut m: Vec<String> = expected
            .iter()
            .map(|(id, _)| id.clone())
            .filter(|id| !seen.contains(id))
            .collect();
        m.sort();
        m
    };
    if missing.is_empty() {
        checks.push(Check::new(
            "temporal.schedules",
            Status::Ok,
            format!("all {} expected schedules present", expected.len()),
        ));
    } else {
        checks.push(Check::new(
            "temporal.schedules",
            Status::Fail,
            format!(
                "{} expected schedule(s) missing: {} (worker seed never ran? check worker logs for '[seed]')",
                missing.len(),
                py_repr_list(&missing)
            ),
        ));
    }

    for sid in &seen {
        checks.push(check_one_schedule(&client, sid, &expected, now).await);
    }

    // -- recent failed, timed-out or terminated workflows -------------------------
    let cutoff = (now - ChronoDuration::hours(failure_window_h))
        .format("%Y-%m-%dT%H:%M:%SZ")
        .to_string();
    let mut failures: Vec<Execution> = Vec::new();
    for state in ["Failed", "TimedOut", "Terminated"] {
        let query = format!("ExecutionStatus = '{state}' AND StartTime > '{cutoff}'");
        match list_executions(&client, &query).await {
            Ok(found) => failures.extend(found),
            Err(e) => {
                checks.push(Check::new(
                    "temporal.failures",
                    Status::Warn,
                    format!("visibility query for {state} failed: {e}"),
                ));
                break;
            }
        }
    }

    if failures.is_empty() {
        checks.push(Check::new(
            "temporal.failures",
            Status::Ok,
            format!("no failed runs in last {failure_window_h}h"),
        ));
    } else {
        failures.sort_by_key(|w| std::cmp::Reverse(w.start_time));
        let by_type = count_by_type(failures.iter().map(|w| w.workflow_type.as_str()));
        // Python sorts by count (descending) and keeps first-seen order for ties.
        let mut ranked = by_type.clone();
        ranked.sort_by_key(|(_, n)| std::cmp::Reverse(*n));
        let summary = ranked
            .iter()
            .map(|(t, n)| format!("{t}×{n}"))
            .collect::<Vec<_>>()
            .join(", ");
        let status = if failures.len() < 10 {
            Status::Warn
        } else {
            Status::Fail
        };
        checks.push(
            Check::new(
                "temporal.failures",
                status,
                format!(
                    "{} failed/timed-out runs in last {failure_window_h}h: {summary}",
                    failures.len()
                ),
            )
            .with_data(Json::Object(vec![
                ("count".into(), Json::Int(failures.len() as i64)),
                (
                    "by_type".into(),
                    Json::Object(
                        by_type
                            .iter()
                            .map(|(t, n)| (t.clone(), Json::Int(*n as i64)))
                            .collect(),
                    ),
                ),
            ])),
        );
        // The terminal failure message is usually the whole diagnosis: which activity, which error.
        for wf in failures.iter().take(5) {
            let msg = terminal_failure_message(&client, wf).await;
            let age = seconds_between(now, start_or_epoch(wf.start_time));
            checks.push(Check::new(
                format!("temporal.failure.{}", wf.id),
                Status::Warn,
                format!(
                    "{} started {} ago{}",
                    wf.workflow_type,
                    age_str(age),
                    msg.map(|m| format!(": {m}")).unwrap_or_default()
                ),
            ));
        }
    }

    // -- running workflows: stuck long-runners and retrying activities ---------------
    let running = match list_executions(&client, "ExecutionStatus = 'Running'").await {
        Ok(running) => {
            let mut stuck: Vec<String> = Vec::new();
            for wf in &running {
                let run_h = seconds_between(now, start_or_epoch(wf.start_time)) / 3600.0;
                if run_h > stuck_after_h as f64 {
                    stuck.push(format!("{}({}) {run_h:.1}h", wf.workflow_type, wf.id));
                }
            }
            if stuck.is_empty() {
                checks.push(Check::new(
                    "temporal.stuck",
                    Status::Ok,
                    format!("no workflows running > {stuck_after_h}h"),
                ));
            } else {
                let head: Vec<String> = stuck.iter().take(5).cloned().collect();
                checks.push(Check::new(
                    "temporal.stuck",
                    Status::Warn,
                    format!(
                        "{} workflow(s) running > {stuck_after_h}h: {}",
                        stuck.len(),
                        head.join("; ")
                    ),
                ));
            }
            running
        }
        Err(e) => {
            checks.push(Check::new(
                "temporal.running",
                Status::Warn,
                format!("visibility query failed: {e}"),
            ));
            Vec::new()
        }
    };

    // Activities retrying inside otherwise healthy runs are the early warning for the failures above.
    let mut retrying = 0usize;
    for wf in running.iter().take(20) {
        let handle = run_handle(&client, &wf.id, &wf.run_id);
        let Ok(desc) = handle.describe(WorkflowDescribeOptions::default()).await else {
            continue;
        };
        for act in &desc.raw_description.pending_activities {
            if act.attempt <= 1 {
                continue;
            }
            retrying += 1;
            let attempt_name = act.activity_type.as_ref().map_or("", |t| t.name.as_str());
            let max = if act.maximum_attempts == 0 {
                "∞".to_string()
            } else {
                act.maximum_attempts.to_string()
            };
            let mut detail = format!("{attempt_name} attempt {}/{max}", act.attempt);
            let failure_msg = act
                .last_failure
                .as_ref()
                .map(|f| f.message.clone())
                .unwrap_or_default();
            if !failure_msg.is_empty() {
                detail.push_str(&format!(", last failure: {failure_msg}"));
            }
            if let Some(hb) = heartbeat_summary(act) {
                detail.push_str(&format!(", heartbeat: {hb}"));
            }
            checks.push(Check::new(
                format!("temporal.retrying.{}", wf.id),
                Status::Warn,
                detail,
            ));
        }
    }
    if retrying == 0 {
        checks.push(Check::new(
            "temporal.retrying",
            Status::Ok,
            "no activities on attempt > 1",
        ));
    }

    checks
}

/// Connect to the frontend at `address` (`host:port` or a full URL) in `namespace`.
async fn connect(address: &str, namespace: &str) -> Result<Client, String> {
    let full = if address.contains("://") {
        address.to_string()
    } else {
        format!("http://{address}")
    };
    let url = Url::parse(&full).map_err(|e| format!("invalid address: {e}"))?;
    let conn = ConnectionOptions::new(url)
        .connect_timeout(CONNECT_TIMEOUT)
        .build();
    let opts = ClientOptions::new(namespace.to_string()).build();
    Client::connect(conn, opts).await.map_err(|e| e.to_string())
}

/// One schedule: pause state, recency against its cadence, and the last run's result.
async fn check_one_schedule(
    client: &Client,
    sid: &str,
    expected: &[(String, u64)],
    now: DateTime<Utc>,
) -> Check {
    let name = format!("temporal.schedule.{sid}");
    let desc = match client
        .get_schedule_handle(sid)
        .describe(RpcOptions::default())
        .await
    {
        Ok(d) => d,
        Err(e) => return Check::new(name, Status::Fail, format!("describe failed: {e}")),
    };
    if desc.paused() {
        let note = desc.note().unwrap_or("no note");
        return Check::new(name, Status::Warn, format!("PAUSED ({note})"));
    }

    let recent = desc.recent_actions();
    let interval = expected
        .iter()
        .find(|(id, _)| id == sid)
        .map(|(_, s)| *s as f64);
    let Some(last) = recent.last() else {
        return Check::new(name, Status::Warn, "no actions recorded yet");
    };
    let last_time = last.actual_time.or(last.schedule_time);
    let mut status = Status::Ok;
    let mut detail = match last_time {
        Some(t) => format!(
            "last run {} ago",
            age_str(seconds_between(now, DateTime::<Utc>::from(t)))
        ),
        None => "last run time unknown".to_string(),
    };
    if let (Some(t), Some(every)) = (last_time, interval) {
        let last_age = seconds_between(now, DateTime::<Utc>::from(t));
        if last_age > every * RECENT_ACTION_SLACK {
            // SKIP overlap legitimately drops triggers during one long run, but 3x the cadence with
            // nothing started means stalled.
            status = Status::Fail;
            detail.push_str(&format!(
                " — expected every {} (stalled or worker down)",
                age_str(every)
            ));
        }
    }

    let handle = run_handle(client, &last.workflow_id, &last.run_id);
    match handle.describe(WorkflowDescribeOptions::default()).await {
        Ok(wf_desc) => {
            let label = status_name(wf_desc.status());
            match wf_desc.status() {
                WorkflowExecutionStatus::Failed
                | WorkflowExecutionStatus::TimedOut
                | WorkflowExecutionStatus::Terminated => {
                    status = Status::Fail;
                    detail.push_str(&format!(", last result {label}"));
                }
                WorkflowExecutionStatus::Running => detail.push_str(", currently running"),
                _ => detail.push_str(&format!(", last result {label}")),
            }
        }
        Err(_) => detail.push_str(", result unknown"),
    }
    Check::new(name, status, detail)
}

/// The workflow handle pinned to one run.
fn run_handle(
    client: &Client,
    workflow_id: &str,
    run_id: &str,
) -> WorkflowHandle<Client, UntypedWorkflow> {
    let info = WorkflowExecutionInfo::builder()
        .namespace(client.namespace())
        .workflow_id(workflow_id)
        .run_id(run_id)
        .build();
    WorkflowHandle::new(client.clone(), info)
}

/// Run the list query and collect every match.
async fn list_executions(client: &Client, query: &str) -> Result<Vec<Execution>, String> {
    let mut stream = client.list_workflows(query, WorkflowListOptions::builder().build());
    let mut out = Vec::new();
    while let Some(item) = stream.next().await {
        let wf = item.map_err(|e| e.to_string())?;
        out.push(Execution {
            id: wf.id().to_string(),
            run_id: wf.run_id().to_string(),
            workflow_type: wf.workflow_type().to_string(),
            start_time: wf.start_time(),
        });
    }
    Ok(out)
}

/// The failure of the workflow's terminal `WorkflowExecutionFailed` event, with its cause.
async fn terminal_failure_message(client: &Client, wf: &Execution) -> Option<String> {
    let handle = run_handle(client, &wf.id, &wf.run_id);
    let history = handle.fetch_history(temporalio_client::WorkflowFetchHistoryOptions::default());
    let events = history.into_events().await.ok()?;
    for event in events.iter().rev() {
        if let Some(Attributes::WorkflowExecutionFailedEventAttributes(attrs)) = &event.attributes
            && let Some(failure) = attrs.failure.as_ref().filter(|f| !f.message.is_empty())
        {
            let inner = failure
                .cause
                .as_deref()
                .filter(|c| !c.message.is_empty())
                .map(|c| format!(" ← {}", c.message))
                .unwrap_or_default();
            return Some(format!("{}{inner}", failure.message));
        }
    }
    None
}

/// The first heartbeat payload of a pending activity, rendered as Python would print it, at most 120 chars.
fn heartbeat_summary(act: &PendingActivityInfo) -> Option<String> {
    let details: &Payloads = act.heartbeat_details.as_ref()?;
    let first = details.payloads.first()?;
    let value: serde_json::Value = serde_json::from_slice(&first.data).ok()?;
    let text: String = py_display(Some(&value)).chars().take(120).collect();
    (!text.is_empty()).then_some(text)
}

/// Start time, or the Unix epoch when the server did not send one (treated as very old).
fn start_or_epoch(t: Option<SystemTime>) -> DateTime<Utc> {
    DateTime::<Utc>::from(t.unwrap_or(SystemTime::UNIX_EPOCH))
}

/// Count occurrences keeping first-seen order (Python dict insertion order).
fn count_by_type<'a>(types: impl Iterator<Item = &'a str>) -> Vec<(String, usize)> {
    let mut out: Vec<(String, usize)> = Vec::new();
    for t in types {
        match out.iter_mut().find(|(name, _)| name == t) {
            Some((_, n)) => *n += 1,
            None => out.push((t.to_string(), 1)),
        }
    }
    out
}

/// Python enum name of a workflow status (`RUNNING`, `CONTINUED_AS_NEW`, ...).
fn status_name(status: WorkflowExecutionStatus) -> &'static str {
    match status {
        WorkflowExecutionStatus::Unspecified => "UNSPECIFIED",
        WorkflowExecutionStatus::Running => "RUNNING",
        WorkflowExecutionStatus::Completed => "COMPLETED",
        WorkflowExecutionStatus::Failed => "FAILED",
        WorkflowExecutionStatus::Canceled => "CANCELED",
        WorkflowExecutionStatus::Terminated => "TERMINATED",
        WorkflowExecutionStatus::ContinuedAsNew => "CONTINUED_AS_NEW",
        WorkflowExecutionStatus::TimedOut => "TIMED_OUT",
        WorkflowExecutionStatus::Paused => "PAUSED",
        _ => "UNKNOWN",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn count_by_type_keeps_first_seen_order() {
        let v = count_by_type(["B", "A", "B", "C", "A", "B"].into_iter());
        assert_eq!(
            v,
            vec![
                ("B".to_string(), 3),
                ("A".to_string(), 2),
                ("C".to_string(), 1)
            ]
        );
    }

    #[test]
    fn ranking_is_stable_for_ties() {
        // Python: sorted(items, key=lambda kv: -kv[1]) keeps insertion order for ties.
        let mut ranked = count_by_type(["X", "Y", "Z", "Y"].into_iter());
        ranked.sort_by_key(|(_, n)| std::cmp::Reverse(*n));
        assert_eq!(
            ranked,
            vec![
                ("Y".to_string(), 2),
                ("X".to_string(), 1),
                ("Z".to_string(), 1)
            ]
        );
    }

    #[test]
    fn status_names_match_python_enum() {
        assert_eq!(
            status_name(WorkflowExecutionStatus::ContinuedAsNew),
            "CONTINUED_AS_NEW"
        );
        assert_eq!(status_name(WorkflowExecutionStatus::TimedOut), "TIMED_OUT");
    }

    #[test]
    fn expected_schedules_come_from_radar_types() {
        let ids: Vec<String> = all_schedules().iter().map(|s| s.id.to_string()).collect();
        // The Python fallback list omitted these two; the Rust set must include them.
        assert!(ids.iter().any(|i| i == "ingest-airquality"));
        assert!(ids.iter().any(|i| i == "open-meteo-sync-gfs025"));
        assert_eq!(ids.len(), 15);
    }

    #[test]
    fn stalled_threshold_is_three_times_cadence() {
        assert_eq!(RECENT_ACTION_SLACK, 3.0);
    }
}
