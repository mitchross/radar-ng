//! Schedule seeding (spec section 6): create or update the twelve schedules from
//! `radar_types::schedules::all_schedules()` on worker start.
//!
//! Schedules are written with raw `WorkflowService` RPCs. The high-level `ScheduleAction` builder does
//! not expose `workflow_execution_timeout` or `catchup_window` on the policy, both of which the spec
//! requires. Compare-before-update uses [`comparable`], a projection of the protobuf form onto the fields
//! this port controls. Server-filled defaults are normalised away.

use std::collections::BTreeMap;
use std::time::Duration;

use futures::future::join_all;
use radar_types::schedules::{
    OverlapPolicy, ScheduleDef, action_workflow_id, all_schedules, schedule_action_queue,
};
use serde_json::Value;
use temporalio_client::{Client, NamespacedClient};
use temporalio_common::protos::temporal::api::common::v1::{Payload, Payloads, WorkflowType};
use temporalio_common::protos::temporal::api::enums::v1::{ScheduleOverlapPolicy, TaskQueueKind};
use temporalio_common::protos::temporal::api::schedule::v1::{
    IntervalSpec, Schedule, ScheduleAction, SchedulePolicies, ScheduleSpec, ScheduleState,
    schedule_action,
};
use temporalio_common::protos::temporal::api::taskqueue::v1::TaskQueue;
use temporalio_common::protos::temporal::api::workflow::v1::NewWorkflowExecutionInfo;
use temporalio_common::protos::temporal::api::workflowservice::v1::{
    CreateScheduleRequest, DescribeScheduleRequest, UpdateScheduleRequest,
};
use tonic::Code;

/// Per-RPC timeout for every schedule call (`_RPC_TIMEOUT`).
pub const RPC_TIMEOUT: Duration = Duration::from_secs(15);
/// Passes over the still-retryable definitions.
pub const MAX_ATTEMPTS: u32 = 10;
/// First backoff delay.
pub const BASE_DELAY: Duration = Duration::from_secs(1);
/// Backoff ceiling.
pub const MAX_DELAY: Duration = Duration::from_secs(20);

/// Failure of one definition after the retry loop ended.
#[derive(Debug, thiserror::Error)]
pub enum SeedError {
    /// `max_attempts` was below one.
    #[error("max_attempts must be at least 1")]
    InvalidAttempts,
    /// One or more definitions failed permanently or exhausted their retries. Maps schedule ID to label.
    #[error("schedule seed failed: {}", errors_summary(.0))]
    Failed(BTreeMap<String, String>),
}

/// What happened to one definition.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Outcome {
    /// Created.
    Created,
    /// Existed and differed; updated.
    Updated,
    /// Existed and matched; no update sent.
    Unchanged,
}

/// Status code classification (spec 6.4). `NOT_FOUND` is retryable because a create can race a delete.
pub fn is_retryable_code(code: Code) -> bool {
    matches!(
        code,
        Code::Cancelled
            | Code::Unavailable
            | Code::DeadlineExceeded
            | Code::ResourceExhausted
            | Code::Aborted
            | Code::Internal
            | Code::Unknown
            | Code::NotFound
    )
}

/// `max_attempts < 1` is rejected before any RPC is sent (spec 6.4, invariant 17).
pub fn validate_attempts(max_attempts: u32) -> Result<(), SeedError> {
    if max_attempts < 1 {
        Err(SeedError::InvalidAttempts)
    } else {
        Ok(())
    }
}

/// Backoff before pass `attempt + 1` (1-based): `min(max, base * 2^(attempt-1))`.
pub fn backoff_delay(attempt: u32, base: Duration, max: Duration) -> Duration {
    let exponent = attempt.saturating_sub(1).min(31);
    let factor = 1u32.checked_shl(exponent).unwrap_or(u32::MAX);
    base.checked_mul(factor).map_or(max, |d| d.min(max))
}

/// Failure of one RPC or local timeout, classified without message text.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RpcFailure {
    /// The server returned a gRPC status.
    Status(Code),
    /// A non-RPC error (for example a payload encoding failure). Always permanent.
    Other,
}

impl RpcFailure {
    /// Whether the definition is retried on the next pass.
    pub fn is_retryable(&self) -> bool {
        match self {
            RpcFailure::Status(code) => is_retryable_code(*code),
            RpcFailure::Other => false,
        }
    }

    /// Safe label for logs: `RPCError[CODE]` or `Error`. Never includes message text.
    pub fn label(&self) -> String {
        match self {
            RpcFailure::Status(code) => format!("RPCError[{}]", code_name(*code)),
            RpcFailure::Other => "Error".to_owned(),
        }
    }
}

fn code_name(code: Code) -> &'static str {
    match code {
        Code::Cancelled => "CANCELLED",
        Code::Unavailable => "UNAVAILABLE",
        Code::DeadlineExceeded => "DEADLINE_EXCEEDED",
        Code::ResourceExhausted => "RESOURCE_EXHAUSTED",
        Code::Aborted => "ABORTED",
        Code::Internal => "INTERNAL",
        Code::Unknown => "UNKNOWN",
        Code::NotFound => "NOT_FOUND",
        Code::DataLoss => "DATA_LOSS",
        Code::AlreadyExists => "ALREADY_EXISTS",
        Code::InvalidArgument => "INVALID_ARGUMENT",
        Code::PermissionDenied => "PERMISSION_DENIED",
        Code::Unauthenticated => "UNAUTHENTICATED",
        Code::FailedPrecondition => "FAILED_PRECONDITION",
        Code::OutOfRange => "OUT_OF_RANGE",
        Code::Unimplemented => "UNIMPLEMENTED",
        Code::Ok => "OK",
    }
}

/// `id:Label, id:Label` in sorted-ID order (spec 6.5 log format).
pub fn errors_summary(errors: &BTreeMap<String, String>) -> String {
    errors
        .iter()
        .map(|(id, label)| format!("{id}:{label}"))
        .collect::<Vec<_>>()
        .join(",")
}

fn duration(seconds: u64) -> prost_wkt_types::Duration {
    prost_wkt_types::Duration {
        seconds: seconds as i64,
        nanos: 0,
    }
}

/// Canonical JSON bytes for an argument: compact, with object keys sorted at every depth. This is
/// `json.dumps(value, sort_keys=True)` as the Python schedule seeder writes it (F8). Byte-equal output lets the
/// Python and Rust seeders write the same bytes, so neither rewrites the other's schedules.
#[must_use]
pub fn canonical_json(value: &Value) -> Vec<u8> {
    serde_json::to_vec(&sort_keys(value)).unwrap_or_default()
}

/// A copy of `value` with every object's keys in sorted order. `serde_json` keeps insertion order here
/// (`preserve_order`), so the sort is explicit.
fn sort_keys(value: &Value) -> Value {
    match value {
        Value::Object(map) => {
            let mut keys: Vec<&String> = map.keys().collect();
            keys.sort();
            let mut out = serde_json::Map::new();
            for key in keys {
                out.insert(key.clone(), sort_keys(&map[key]));
            }
            Value::Object(out)
        }
        Value::Array(items) => Value::Array(items.iter().map(sort_keys).collect()),
        other => other.clone(),
    }
}

/// JSON payload with the `json/plain` encoding, as the Python default converter writes it. The data uses
/// [`canonical_json`].
pub fn json_payload(value: &Value) -> Payload {
    Payload {
        metadata: BTreeMap::from([("encoding".to_owned(), b"json/plain".to_vec())])
            .into_iter()
            .collect(),
        data: canonical_json(value),
        external_payloads: Vec::new(),
    }
}

fn overlap_proto(policy: OverlapPolicy) -> ScheduleOverlapPolicy {
    match policy {
        OverlapPolicy::Skip => ScheduleOverlapPolicy::Skip,
    }
}

/// The schedule definition in protobuf form (spec 6.1, 6.2, 10.5). `use_isolated` selects the action queue
/// through the shared routing rule.
pub fn desired_schedule(def: &ScheduleDef, use_isolated: bool) -> Schedule {
    let spec = ScheduleSpec {
        interval: vec![IntervalSpec {
            interval: Some(duration(def.interval_s)),
            phase: None,
        }],
        jitter: def.jitter_s.map(duration),
        ..Default::default()
    };
    let input = if def.args.is_empty() {
        None
    } else {
        Some(Payloads {
            payloads: def.args.iter().map(json_payload).collect(),
        })
    };
    let info = NewWorkflowExecutionInfo {
        workflow_id: action_workflow_id(def.id),
        workflow_type: Some(WorkflowType {
            name: def.workflow_type.to_owned(),
        }),
        task_queue: Some(TaskQueue {
            name: schedule_action_queue(def, use_isolated).to_owned(),
            kind: TaskQueueKind::Normal as i32,
            normal_name: String::new(),
        }),
        input,
        workflow_execution_timeout: Some(duration(def.execution_timeout_s)),
        ..Default::default()
    };
    Schedule {
        spec: Some(spec),
        action: Some(ScheduleAction {
            action: Some(schedule_action::Action::StartWorkflow(info)),
        }),
        policies: Some(SchedulePolicies {
            overlap_policy: overlap_proto(def.overlap) as i32,
            catchup_window: Some(duration(def.catchup_window_s)),
            pause_on_failure: def.pause_on_failure,
            keep_original_workflow_id: false,
        }),
        state: Some(ScheduleState::default()),
    }
}

/// Comparison projection of a schedule. Two schedules are equal here exactly when the update would not
/// change anything this port controls. Live `paused`, `note` and limited-action state are excluded, as the
/// Python callback preserves them.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Comparable {
    /// `(interval, phase)` in seconds. A missing phase is 0.
    pub intervals: Vec<(i64, i64)>,
    /// Jitter in seconds (0 when unset).
    pub jitter_s: i64,
    /// Timezone name with `""` and `"UTC"` treated as the same value.
    pub timezone: String,
    /// Action workflow ID.
    pub workflow_id: String,
    /// Workflow type name.
    pub workflow_type: String,
    /// Action task queue name.
    pub task_queue: String,
    /// Each argument, parsed from its payload data. Compared as JSON values, so key order and whitespace
    /// do not matter (F8). A payload that is not JSON is kept as its lossy text.
    pub input: Vec<Value>,
    /// Execution timeout in seconds.
    pub execution_timeout_s: i64,
    /// Overlap policy; unspecified is treated as the server default, SKIP.
    pub overlap: i32,
    /// Catch-up window in seconds.
    pub catchup_s: i64,
    /// Pause-on-failure flag.
    pub pause_on_failure: bool,
}

fn secs(d: Option<&prost_wkt_types::Duration>) -> i64 {
    d.map_or(0, |d| d.seconds)
}

/// One argument's payload data as a JSON value (lossy text when it is not JSON).
fn parsed_argument(data: &[u8]) -> Value {
    serde_json::from_slice(data)
        .unwrap_or_else(|_| Value::String(String::from_utf8_lossy(data).into_owned()))
}

/// Project a schedule onto [`Comparable`].
pub fn comparable(schedule: &Schedule) -> Comparable {
    let spec = schedule.spec.clone().unwrap_or_default();
    let (workflow_id, workflow_type, task_queue, input, execution_timeout_s) =
        match schedule.action.as_ref().and_then(|a| a.action.as_ref()) {
            Some(schedule_action::Action::StartWorkflow(info)) => (
                info.workflow_id.clone(),
                info.workflow_type
                    .as_ref()
                    .map(|w| w.name.clone())
                    .unwrap_or_default(),
                info.task_queue
                    .as_ref()
                    .map(|t| t.name.clone())
                    .unwrap_or_default(),
                info.input
                    .as_ref()
                    .map(|p| {
                        p.payloads
                            .iter()
                            .map(|x| parsed_argument(&x.data))
                            .collect()
                    })
                    .unwrap_or_default(),
                secs(info.workflow_execution_timeout.as_ref()),
            ),
            None => (String::new(), String::new(), String::new(), Vec::new(), 0),
        };
    let policies = schedule.policies.unwrap_or_default();
    let overlap = if policies.overlap_policy == ScheduleOverlapPolicy::Unspecified as i32 {
        ScheduleOverlapPolicy::Skip as i32
    } else {
        policies.overlap_policy
    };
    let timezone = match spec.timezone_name.as_str() {
        "UTC" => String::new(),
        other => other.to_owned(),
    };
    Comparable {
        intervals: spec
            .interval
            .iter()
            .map(|i| (secs(i.interval.as_ref()), secs(i.phase.as_ref())))
            .collect(),
        jitter_s: secs(spec.jitter.as_ref()),
        timezone,
        workflow_id,
        workflow_type,
        task_queue,
        input,
        execution_timeout_s,
        overlap,
        catchup_s: secs(policies.catchup_window.as_ref()),
        pause_on_failure: policies.pause_on_failure,
    }
}

/// Whether the live schedule already matches the desired definition.
pub fn schedules_match(live: &Schedule, desired: &Schedule) -> bool {
    comparable(live) == comparable(desired)
}

/// Apply one definition: create it, or if it already exists compare with the live copy and update only
/// when different. An `ALREADY_EXISTS` create falls through to the update path.
pub async fn apply(
    client: &Client,
    namespace: &str,
    def: &ScheduleDef,
    use_isolated: bool,
) -> Result<Outcome, RpcFailure> {
    let desired = desired_schedule(def, use_isolated);
    let identity = client.identity();
    let mut service = client.connection().workflow_service();

    let create = CreateScheduleRequest {
        namespace: namespace.to_owned(),
        schedule_id: def.id.to_owned(),
        schedule: Some(desired.clone()),
        identity: identity.clone(),
        request_id: uuid::Uuid::new_v4().to_string(),
        ..Default::default()
    };
    match rpc(service.create_schedule(tonic::Request::new(create))).await {
        Ok(_) => return Ok(Outcome::Created),
        Err(RpcFailure::Status(Code::AlreadyExists)) => {}
        Err(err) => return Err(err),
    }

    // Python's ScheduleHandle.update() performs this describe internally before its callback runs.
    let described = rpc(
        service.describe_schedule(tonic::Request::new(DescribeScheduleRequest {
            namespace: namespace.to_owned(),
            schedule_id: def.id.to_owned(),
        })),
    )
    .await?
    .into_inner();
    let current = described.schedule.clone().unwrap_or_default();
    let mut wanted = desired;
    wanted.state = current.state.clone();
    if schedules_match(&current, &wanted) {
        return Ok(Outcome::Unchanged);
    }

    let update = UpdateScheduleRequest {
        namespace: namespace.to_owned(),
        schedule_id: def.id.to_owned(),
        schedule: Some(wanted),
        conflict_token: described.conflict_token,
        identity,
        request_id: uuid::Uuid::new_v4().to_string(),
        ..Default::default()
    };
    rpc(service.update_schedule(tonic::Request::new(update))).await?;
    Ok(Outcome::Updated)
}

/// Await one RPC under [`RPC_TIMEOUT`]. A local timeout is reported as `DEADLINE_EXCEEDED`.
async fn rpc<T, F>(call: F) -> Result<T, RpcFailure>
where
    F: std::future::Future<Output = Result<T, tonic::Status>>,
{
    match tokio::time::timeout(RPC_TIMEOUT, call).await {
        Ok(Ok(value)) => Ok(value),
        Ok(Err(status)) => Err(RpcFailure::Status(status.code())),
        Err(_) => Err(RpcFailure::Status(Code::DeadlineExceeded)),
    }
}

/// Create-or-update every definition with bounded retries (spec 6.4).
///
/// Definitions run concurrently. Only retryable RPC failures are retried, and healthy definitions are not
/// re-applied. Exhaustion reports the still-failing definitions and never deletes or terminates anything.
pub async fn seed_with_retry(
    client: &Client,
    namespace: &str,
    use_isolated: bool,
    max_attempts: u32,
    base_delay: Duration,
    max_delay: Duration,
) -> Result<BTreeMap<String, Outcome>, SeedError> {
    seed_definitions(
        client,
        namespace,
        use_isolated,
        all_schedules(),
        max_attempts,
        base_delay,
        max_delay,
    )
    .await
}

/// [`seed_with_retry`] over an explicit definition list.
pub async fn seed_definitions(
    client: &Client,
    namespace: &str,
    use_isolated: bool,
    definitions: Vec<ScheduleDef>,
    max_attempts: u32,
    base_delay: Duration,
    max_delay: Duration,
) -> Result<BTreeMap<String, Outcome>, SeedError> {
    validate_attempts(max_attempts)?;
    let mut pending = definitions;
    let mut outcomes = BTreeMap::new();
    let mut permanent: BTreeMap<String, String> = BTreeMap::new();
    let mut last_label: BTreeMap<String, String> = BTreeMap::new();

    for attempt in 1..=max_attempts {
        let results = join_all(
            pending
                .iter()
                .map(|def| apply(client, namespace, def, use_isolated)),
        )
        .await;
        let mut retry = Vec::new();
        for (def, result) in pending.into_iter().zip(results) {
            match result {
                Ok(outcome) => {
                    tracing::info!(schedule_id = def.id, outcome = ?outcome, "schedule reconciled");
                    outcomes.insert(def.id.to_owned(), outcome);
                }
                Err(failure) if failure.is_retryable() => {
                    tracing::warn!(
                        event = "TEMPORAL_SCHEDULE_RECONCILIATION_RETRY",
                        schedule_id = def.id,
                        error = %failure.label(),
                        attempt,
                        "schedule reconciliation will retry"
                    );
                    last_label.insert(def.id.to_owned(), failure.label());
                    retry.push(def);
                }
                Err(failure) => {
                    tracing::error!(
                        event = "TEMPORAL_SCHEDULE_RECONCILIATION_ERROR",
                        schedule_id = def.id,
                        error = %failure.label(),
                        retryable = false,
                        "schedule reconciliation failed permanently"
                    );
                    permanent.insert(def.id.to_owned(), failure.label());
                }
            }
        }
        if retry.is_empty() {
            break;
        }
        if attempt == max_attempts {
            for def in &retry {
                let label = last_label
                    .get(def.id)
                    .cloned()
                    .unwrap_or_else(|| "Error".to_owned());
                permanent.insert(def.id.to_owned(), label);
            }
            break;
        }
        tokio::time::sleep(backoff_delay(attempt, base_delay, max_delay)).await;
        pending = retry;
    }

    if permanent.is_empty() {
        Ok(outcomes)
    } else {
        Err(SeedError::Failed(permanent))
    }
}

/// Startup reconciliation (spec 6.5). Never returns an error and never restarts the worker: failures are
/// logged once as CRITICAL with sorted IDs and safe labels.
pub async fn reconcile_schedules_once(client: &Client, namespace: &str, use_isolated: bool) {
    match seed_with_retry(
        client,
        namespace,
        use_isolated,
        MAX_ATTEMPTS,
        BASE_DELAY,
        MAX_DELAY,
    )
    .await
    {
        Ok(_) => tracing::info!("schedule reconciliation complete"),
        Err(SeedError::Failed(errors)) => {
            let ids: Vec<&str> = errors.keys().map(String::as_str).collect();
            tracing::error!(
                event = "TEMPORAL_SCHEDULE_RECONCILIATION_FAILED",
                schedule_ids = ?ids,
                errors = %errors_summary(&errors),
                operator_action = "inspect_schedule_reconciliation",
                "schedule reconciliation failed"
            );
        }
        Err(SeedError::InvalidAttempts) => {
            tracing::error!(
                event = "TEMPORAL_SCHEDULE_RECONCILIATION_FAILED",
                schedule_ids = ?Vec::<String>::new(),
                errors = "unexpected:ValueError",
                operator_action = "inspect_schedule_reconciliation",
                "schedule reconciliation failed"
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use radar_types::schedules::all_schedules;
    use serde_json::json;

    fn def(id: &str) -> ScheduleDef {
        all_schedules()
            .into_iter()
            .find(|d| d.id == id)
            .expect("known schedule")
    }

    #[test]
    fn retryable_codes_match_the_spec_list() {
        for code in [
            Code::Cancelled,
            Code::Unavailable,
            Code::DeadlineExceeded,
            Code::ResourceExhausted,
            Code::Aborted,
            Code::Internal,
            Code::Unknown,
            Code::NotFound,
        ] {
            assert!(is_retryable_code(code), "{code:?} should be retryable");
        }
        for code in [
            Code::AlreadyExists,
            Code::InvalidArgument,
            Code::PermissionDenied,
            Code::Unauthenticated,
        ] {
            assert!(!is_retryable_code(code), "{code:?} should be permanent");
        }
        assert!(
            !RpcFailure::Other.is_retryable(),
            "non-RPC errors are permanent"
        );
    }

    #[test]
    fn labels_never_include_message_text() {
        assert_eq!(
            RpcFailure::Status(Code::Unavailable).label(),
            "RPCError[UNAVAILABLE]"
        );
        assert_eq!(RpcFailure::Other.label(), "Error");
    }

    #[test]
    fn backoff_follows_the_spec_sequence() {
        let seq: Vec<u64> = (1..=9)
            .map(|a| backoff_delay(a, BASE_DELAY, MAX_DELAY).as_secs())
            .collect();
        assert_eq!(seq, vec![1, 2, 4, 8, 16, 20, 20, 20, 20]);
    }

    #[test]
    fn summary_is_sorted_and_contains_only_ids_and_labels() {
        let mut errors = BTreeMap::new();
        errors.insert("poll-alerts".to_owned(), "RPCError[UNAVAILABLE]".to_owned());
        errors.insert("ingest-hrrr".to_owned(), "Error".to_owned());
        assert_eq!(
            errors_summary(&errors),
            "ingest-hrrr:Error,poll-alerts:RPCError[UNAVAILABLE]"
        );
    }

    #[test]
    fn desired_schedule_matches_the_definition() {
        let d = def("ingest-mrms-base");
        let s = desired_schedule(&d, true);
        let spec = s.spec.as_ref().unwrap();
        assert_eq!(spec.interval[0].interval.as_ref().unwrap().seconds, 60);
        assert_eq!(spec.jitter.as_ref().unwrap().seconds, 20);
        let policies = s.policies.as_ref().unwrap();
        assert_eq!(policies.overlap_policy, ScheduleOverlapPolicy::Skip as i32);
        assert_eq!(policies.catchup_window.as_ref().unwrap().seconds, 300);
        let Some(schedule_action::Action::StartWorkflow(info)) =
            s.action.as_ref().and_then(|a| a.action.clone())
        else {
            panic!("expected StartWorkflow");
        };
        assert_eq!(info.workflow_id, "sched-ingest-mrms-base");
        assert_eq!(
            info.workflow_execution_timeout.as_ref().unwrap().seconds,
            360
        );
        assert_eq!(info.task_queue.as_ref().unwrap().name, "radar-ng-mrms");
        assert_eq!(info.input.as_ref().unwrap().payloads.len(), 1);
    }

    #[test]
    fn legacy_queue_rule_applies_without_isolation() {
        let d = def("ingest-hrrr");
        let Some(schedule_action::Action::StartWorkflow(info)) =
            desired_schedule(&d, false).action.and_then(|a| a.action)
        else {
            panic!("expected StartWorkflow");
        };
        assert_eq!(info.task_queue.unwrap().name, "radar-ng");
        assert!(info.input.is_none(), "empty args send no input");
    }

    #[test]
    fn identical_schedules_compare_equal_and_timezone_utc_is_normalised() {
        let d = def("nowcast");
        let live = desired_schedule(&d, true);
        let mut with_state = live.clone();
        with_state.state = Some(ScheduleState {
            paused: true,
            notes: "operator".into(),
            ..Default::default()
        });
        assert!(
            schedules_match(&with_state, &live),
            "paused and notes are not compared"
        );

        let mut utc = live.clone();
        utc.spec.as_mut().unwrap().timezone_name = "UTC".into();
        assert!(schedules_match(&utc, &live));
    }

    #[test]
    fn argument_key_order_does_not_count_as_a_change() {
        // F8 regression. A live schedule written with unsorted keys (the old Rust bytes) and the Python seeder's
        // sorted bytes describe the same schedule.
        let desired = desired_schedule(&def("ingest-mrms-base"), true);
        let desired_bytes = desired
            .action
            .as_ref()
            .and_then(|a| match a.action.as_ref() {
                Some(schedule_action::Action::StartWorkflow(info)) => {
                    info.input.as_ref().map(|i| i.payloads[0].data.clone())
                }
                None => None,
            })
            .expect("mrms args");
        let mut unsorted = desired.clone();
        let input = unsorted
            .action
            .as_mut()
            .and_then(|a| match a.action.as_mut() {
                Some(schedule_action::Action::StartWorkflow(info)) => info.input.as_mut(),
                None => None,
            });
        let payload = input
            .and_then(|i| i.payloads.first_mut())
            .expect("mrms args");
        let value: Value = serde_json::from_slice(&payload.data).expect("json");
        // Reverse the key order, so the bytes differ from the canonical (sorted) ones.
        let mut reversed = serde_json::Map::new();
        for (k, v) in value.as_object().expect("object").iter().rev() {
            reversed.insert(k.clone(), v.clone());
        }
        payload.data = serde_json::to_vec(&Value::Object(reversed)).expect("json");
        assert_ne!(
            payload.data, desired_bytes,
            "the live bytes are not in canonical order"
        );
        assert!(
            schedules_match(&unsorted, &desired),
            "same JSON value, different key order"
        );

        let mut changed = value.clone();
        changed["layer_name"] = Value::String("other".into());
        let mut other = desired.clone();
        if let Some(schedule_action::Action::StartWorkflow(info)) =
            other.action.as_mut().and_then(|a| a.action.as_mut())
        {
            info.input.as_mut().expect("input").payloads[0].data =
                serde_json::to_vec(&changed).expect("json");
        }
        assert!(
            !schedules_match(&other, &desired),
            "a changed value is a change"
        );
    }

    #[test]
    fn canonical_json_sorts_keys_at_every_depth() {
        let value = json!({"b": {"y": 1, "x": [{"d": 1, "c": 2}]}, "a": 0});
        assert_eq!(
            String::from_utf8(canonical_json(&value)).expect("utf8"),
            r#"{"a":0,"b":{"x":[{"c":2,"d":1}],"y":1}}"#
        );
    }

    #[test]
    fn changed_args_are_detected() {
        let live = desired_schedule(&def("open-meteo-sync-gfs"), true);
        let other = desired_schedule(&def("open-meteo-sync-hrrr"), true);
        assert!(!schedules_match(&live, &other));
    }

    #[test]
    fn overlap_unspecified_is_treated_as_skip() {
        let mut live = desired_schedule(&def("nowcast"), true);
        live.policies.as_mut().unwrap().overlap_policy = ScheduleOverlapPolicy::Unspecified as i32;
        assert!(schedules_match(
            &live,
            &desired_schedule(&def("nowcast"), true)
        ));
    }

    #[test]
    fn zero_attempts_is_rejected() {
        assert!(matches!(
            validate_attempts(0),
            Err(SeedError::InvalidAttempts)
        ));
        assert!(validate_attempts(1).is_ok());
    }

    #[test]
    fn overlap_policy_is_skip_for_every_definition() {
        for d in all_schedules() {
            assert_eq!(
                overlap_proto(d.overlap),
                ScheduleOverlapPolicy::Skip,
                "{}",
                d.id
            );
        }
    }
}
