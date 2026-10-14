//! `WatchStormWorkflow` (spec 4.11). One entity run per (user, storm cell), lifetime hours to days.
//!
//! Replay rules that must hold (spec 10.2, 4.11.5):
//! - `patched()` is evaluated at the same points, in the same order, as the Python code.
//! - The version-boundary check reads `patched` only when `continue_requested` is already true.
//! - Each loop iteration: optional alert push, compare, optional detect and change push, then the wait.
//!
//! Deviation: the Python `__init__` raises on an unknown handoff schema. The SDK `#[init]` cannot fail, so
//! the check runs at the top of `run` and fails the execution with a non-retryable error. Nothing else
//! runs before that check, so the observable result is the same.

use std::collections::BTreeMap;
use std::pin::pin;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use radar_types::payloads::push::{FanOutPushInput, FanOutPushResult};
use radar_types::payloads::watch_storm::{
    CompareFramesInput, CompareFramesResult, DetectChangeInput, DetectChangeResult,
    MAX_ITERATIONS_PER_RUN, MAX_RUN_DURATION_S, POLL_INTERVAL_S, UnsupportedHandoffSchema,
    WATCH_STORM_HANDOFF_SCHEMA_VERSION, WatchStormHandoff, WatchStormInput, WatchStormState,
};
use radar_types::retry::{COMPARE_RADAR_FRAMES, DETECT_STORM_CHANGE, FAN_OUT_PUSH_TO_USER};
use radar_types::wire::{
    PATCH_WATCH_STORM_ALERT_QUEUE_V1, PATCH_WATCH_STORM_HANDOFF_V1,
    PATCH_WATCH_STORM_VERSION_BOUNDARY_V1,
};
use temporalio_macros::{workflow, workflow_methods};
use temporalio_sdk::workflows::select;
use temporalio_sdk::{
    CancellableFuture, ContinueAsNewOptions, ContinueAsNewVersioningBehavior, SyncWorkflowContext,
    WorkflowContext, WorkflowContextView, WorkflowResult,
};

use super::options;
use crate::activities::alerts::AlertsActivities;

/// Wait between polls, unless an unpin or alert wakes the loop earlier (`POLL_S`).
pub const POLL_WAIT: Duration = Duration::from_secs(POLL_INTERVAL_S);

/// Title for a push, by change kind (spec 4.11.4). Unknown kinds, including `""`, map to "Storm update".
pub fn title_for(kind: &str) -> &'static str {
    match kind {
        "intensifying" => "Storm intensifying",
        "dissipating" => "Storm weakening",
        "severe" => "Severe storm cell",
        "alert" => "Severe weather alert",
        _ => "Storm update",
    }
}

/// Collapse ID for a change push: `<workflow_id>:<last_frame_ts>:<kind>` (spec 4.11.4, 8.6).
pub fn change_collapse_id(workflow_id: &str, last_frame_ts: Option<&str>, kind: &str) -> String {
    format!("{workflow_id}:{}:{kind}", last_frame_ts.unwrap_or("None"))
}

/// Collapse ID for an alert push: `<workflow_id>:alert:<alert_id>`.
pub fn alert_collapse_id(workflow_id: &str, alert_id: &str) -> String {
    format!("{workflow_id}:alert:{alert_id}")
}

/// `extra` payload for a change push.
pub fn change_extra(storm_cell_id: &str, kind: &str) -> BTreeMap<String, String> {
    BTreeMap::from([
        ("storm_cell_id".to_owned(), storm_cell_id.to_owned()),
        ("kind".to_owned(), kind.to_owned()),
    ])
}

/// `extra` payload for an alert push.
pub fn alert_extra(storm_cell_id: &str, alert_id: &str) -> BTreeMap<String, String> {
    BTreeMap::from([
        ("storm_cell_id".to_owned(), storm_cell_id.to_owned()),
        ("alert_id".to_owned(), alert_id.to_owned()),
        ("kind".to_owned(), "alert".to_owned()),
    ])
}

/// Queue an alert signal. New executions (patched) append every alert. Old histories without the marker
/// keep only the most recent alert (`pending.clear()` then append).
pub fn enqueue_alert(pending: &mut Vec<String>, alert_id: String, alert_queue_patched: bool) {
    if !alert_queue_patched {
        pending.clear();
    }
    pending.push(alert_id);
}

/// Sticky continue-as-new latch (spec 4.11.2).
pub fn latch_continue(current: bool, suggested: bool, version_changed: bool) -> bool {
    current || suggested || version_changed
}

/// Continue-as-new condition (spec 4.11.3). `boundary_patched` is evaluated only when the latch is set and
/// the first two terms are false, so the patch marker is read at the same points as Python.
pub fn should_rollover(
    polls_this_run: u32,
    now: SystemTime,
    deadline: SystemTime,
    continue_requested: bool,
    boundary_patched: impl FnOnce() -> bool,
) -> bool {
    polls_this_run >= MAX_ITERATIONS_PER_RUN
        || now >= deadline
        || (continue_requested && boundary_patched())
}

/// Input for the handoff continue-as-new: the original input with its handoff replaced by the live state.
pub fn handoff_input(
    original: &WatchStormInput,
    state: &WatchStormState,
    pending: &[String],
) -> WatchStormInput {
    let mut next = original.clone();
    next.handoff = Some(WatchStormHandoff {
        schema_version: WATCH_STORM_HANDOFF_SCHEMA_VERSION,
        state: state.clone(),
        pending_alert_ids: pending.to_vec(),
    });
    next
}

/// Reject a handoff whose schema version this build does not understand.
pub fn check_handoff(input: &WatchStormInput) -> Result<(), UnsupportedHandoffSchema> {
    match &input.handoff {
        Some(handoff) => handoff.check_schema_version(),
        None => Ok(()),
    }
}

/// Fan-out `sent` count, or 0 when the fan-out failed after retries.
pub fn sent_count(result: Option<&FanOutPushResult>) -> i64 {
    result.map_or(0, |r| r.sent)
}

fn epoch_seconds(time: SystemTime) -> f64 {
    time.duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs_f64())
        .unwrap_or(0.0)
}

/// Workflow type `WatchStormWorkflow`.
#[workflow]
pub struct WatchStormWorkflow {
    /// Start input as delivered (kept for the continue-as-new paths).
    input: WatchStormInput,
    /// Business state, restored from the handoff when present.
    storm: WatchStormState,
    /// Alert IDs waiting for a push, drained FIFO one per iteration.
    pending: Vec<String>,
    /// Set by `unpinSignal`. Ends the run after the current iteration.
    unpinned: bool,
    /// Sticky latch set by continue-as-new suggestions and version changes.
    continue_requested: bool,
}

#[workflow_methods]
impl WatchStormWorkflow {
    /// Restore state from the handoff, if any. Runs before any signal is delivered.
    #[init]
    fn new(_ctx: &WorkflowContextView, input: WatchStormInput) -> Self {
        let (storm, pending) = match input.handoff.clone() {
            Some(handoff) => (handoff.state, handoff.pending_alert_ids),
            None => (WatchStormState::default(), Vec::new()),
        };
        Self {
            input,
            storm,
            pending,
            unpinned: false,
            continue_requested: false,
        }
    }

    /// Poll loop: compare frames each iteration, push on change, and roll over when the history grows large,
    /// the run is old, or the worker version changes. Returns the final state when unpinned.
    #[run]
    pub async fn run(ctx: &mut WorkflowContext<Self>) -> WorkflowResult<WatchStormState> {
        let input = ctx.state(|s| s.input.clone());
        if let Err(err) = check_handoff(&input) {
            return Err(temporalio_sdk::ApplicationFailure::non_retryable(err.to_string()).into());
        }

        let started = ctx.workflow_time().unwrap_or(UNIX_EPOCH);
        let deadline = started + Duration::from_secs(MAX_RUN_DURATION_S);
        let mut polls_this_run: u32 = 0;
        ctx.state_mut(|s| {
            s.storm.user_id = input.user_id.clone();
            s.storm.storm_cell_id = input.storm_cell_id.clone();
            s.storm.lat = input.lat;
            s.storm.lng = input.lng;
        });
        tracing::info!(workflow_id = %ctx.workflow_id(), "watch start");

        loop {
            if ctx.state(|s| s.unpinned) {
                break;
            }
            observe(ctx);

            let now = ctx.workflow_time().unwrap_or(UNIX_EPOCH);
            let continue_requested = ctx.state(|s| s.continue_requested);
            if should_rollover(polls_this_run, now, deadline, continue_requested, || {
                ctx.patched(PATCH_WATCH_STORM_VERSION_BOUNDARY_V1)
            }) {
                if ctx.patched(PATCH_WATCH_STORM_HANDOFF_V1) {
                    let waiting: &WorkflowContext<Self> = ctx;
                    waiting
                        .wait_condition(|_: &Self| waiting.all_handlers_finished())
                        .await?;
                    if ctx.state(|s| s.unpinned) {
                        break;
                    }
                    let (storm, pending) = ctx.state(|s| (s.storm.clone(), s.pending.clone()));
                    let next = handoff_input(&input, &storm, &pending);
                    // `ContinueAsNewOptions` is non-exhaustive: start from the default and set the field.
                    let mut options = ContinueAsNewOptions::default();
                    options.initial_versioning_behavior =
                        Some(ContinueAsNewVersioningBehavior::AutoUpgrade);
                    let never = ctx.continue_as_new(next, options)?;
                    match never {}
                }
                // Legacy path (no handoff marker): restarts from the original input and loses state.
                let never = ctx.continue_as_new(input.clone(), ContinueAsNewOptions::default())?;
                match never {}
            }

            if let Some(alert_id) = ctx.state_mut(|s| {
                if s.pending.is_empty() {
                    None
                } else {
                    Some(s.pending.remove(0))
                }
            }) {
                push_alert(ctx, &input, &alert_id).await;
                observe(ctx);
            }

            let (lat, lng, prev_timestamp, prev_max_dbz) = ctx.state(|s| {
                (
                    s.storm.lat,
                    s.storm.lng,
                    s.storm.last_frame_ts.clone(),
                    s.storm.last_max_dbz,
                )
            });
            let cmp: CompareFramesResult = ctx
                .execute_activity(
                    AlertsActivities::compare_radar_frames,
                    CompareFramesInput {
                        lat,
                        lng,
                        prev_timestamp,
                        prev_max_dbz,
                    },
                    options(&COMPARE_RADAR_FRAMES),
                )
                .await?;
            observe(ctx);

            ctx.state_mut(|s| s.storm.poll_count += 1);
            polls_this_run += 1;

            if cmp.sampled {
                ctx.state_mut(|s| {
                    s.storm.last_frame_ts = Some(cmp.curr_timestamp.clone());
                    s.storm.last_max_dbz = Some(cmp.curr_max_dbz);
                });
                let detect = DetectChangeInput {
                    has_prev: cmp.has_prev,
                    curr_max_dbz: cmp.curr_max_dbz,
                    prev_max_dbz: cmp.prev_max_dbz,
                    max_dbz_delta: cmp.max_dbz_delta,
                };
                let change: DetectChangeResult = ctx
                    .execute_activity(
                        AlertsActivities::detect_storm_change,
                        detect,
                        options(&DETECT_STORM_CHANGE),
                    )
                    .await?;
                observe(ctx);
                if !change.kind.is_empty() {
                    push_change(ctx, &input, &change).await;
                    observe(ctx);
                }
            }

            wait_for_poll(ctx).await;
        }

        tracing::info!(workflow_id = %ctx.workflow_id(), "watch unpinned");
        Ok(ctx.state(|s| s.storm.clone()))
    }

    /// Stop the loop after the current iteration.
    #[signal(name = "unpinSignal")]
    fn unpin(&mut self, _ctx: &mut SyncWorkflowContext<Self>) {
        self.unpinned = true;
    }

    /// Wake the loop so it can observe a continue request. Carries no business work.
    #[signal(name = "wakeUpSignal")]
    fn wake_up(&mut self, ctx: &mut SyncWorkflowContext<Self>) {
        self.continue_requested = latch_continue(
            self.continue_requested,
            ctx.continue_as_new_suggested(),
            ctx.target_worker_deployment_version_changed(),
        );
    }

    /// Queue an alert for a push. Patched executions queue every alert; older histories keep the last one.
    #[signal(name = "alertMatchSignal")]
    pub fn alert_match(&mut self, ctx: &mut SyncWorkflowContext<Self>, alert_id: String) {
        enqueue_alert(
            &mut self.pending,
            alert_id,
            ctx.patched(PATCH_WATCH_STORM_ALERT_QUEUE_V1),
        );
        self.continue_requested = latch_continue(
            self.continue_requested,
            ctx.continue_as_new_suggested(),
            ctx.target_worker_deployment_version_changed(),
        );
    }

    /// Current business state, including the watch point used by alert matching.
    #[query(name = "getCurrentState")]
    pub fn get_current_state(&self, _ctx: &WorkflowContextView) -> WatchStormState {
        self.storm.clone()
    }
}

/// Sample the continue-as-new signals and latch them (spec 4.11.2 `observe_continue_request`).
fn observe(ctx: &WorkflowContext<WatchStormWorkflow>) {
    ctx.state_mut(|s| {
        s.continue_requested = latch_continue(
            s.continue_requested,
            ctx.continue_as_new_suggested(),
            ctx.target_worker_deployment_version_changed(),
        );
    });
}

/// Wait up to `POLL_WAIT` for an unpin or a pending alert. Timeout is the normal outcome.
///
/// Command order matches Python's `wait_condition(fn, timeout=POLL_S)` (replay gate, `tests/replay.rs`):
/// Python creates the timer first, which consumes a timer sequence number, and cancels it when the condition
/// wins. The timer command is pushed when `ctx.timer` is called, and dropping the future does not cancel it,
/// so the condition branch must cancel it explicitly:
/// - condition already true (unpin or alert arrived during the iteration): the timer is created and cancelled
///   in the same activation, so Core drops the unsent `StartTimer` and no timer event is recorded;
/// - condition becomes true in a later task: a `CancelTimer` command (`TimerCanceled` event) precedes the next
///   iteration's commands, as in Python.
///
/// Without the cancel, the run recorded a `StartTimer` that Python never sends and failed replay of every
/// Python history in which a signal arrived during an iteration.
async fn wait_for_poll(ctx: &WorkflowContext<WatchStormWorkflow>) {
    let mut timer = pin!(ctx.timer(POLL_WAIT));
    let mut condition =
        pin!(ctx.wait_condition(|s: &WatchStormWorkflow| s.unpinned || !s.pending.is_empty()));
    // `select!` polls top to bottom, so a condition that is already true wins over a timer fired in the same
    // activation, as the Python condition check does.
    select! {
        _ = condition.as_mut() => timer.cancel(),
        _ = timer.as_mut() => {}
    }
}

async fn push_alert(
    ctx: &WorkflowContext<WatchStormWorkflow>,
    input: &WatchStormInput,
    alert_id: &str,
) {
    let workflow_id = ctx.workflow_id().to_owned();
    let request = FanOutPushInput {
        user_id: input.user_id.clone(),
        title: "Severe weather alert near your storm".to_owned(),
        body: "NWS issued an alert in your watched area".to_owned(),
        collapse_id: alert_collapse_id(&workflow_id, alert_id),
        extra: alert_extra(&input.storm_cell_id, alert_id),
    };
    let result = fan_out(ctx, request).await;
    let notified_at = epoch_seconds(ctx.workflow_time().unwrap_or(UNIX_EPOCH));
    ctx.state_mut(|s| {
        s.storm.last_change_kind = Some("alert".to_owned());
        s.storm.last_notified_at = Some(notified_at);
        s.storm.push_count += sent_count(result.as_ref());
    });
}

async fn push_change(
    ctx: &WorkflowContext<WatchStormWorkflow>,
    input: &WatchStormInput,
    change: &DetectChangeResult,
) {
    let workflow_id = ctx.workflow_id().to_owned();
    let last_frame_ts = ctx.state(|s| s.storm.last_frame_ts.clone());
    let request = FanOutPushInput {
        user_id: input.user_id.clone(),
        title: title_for(&change.kind).to_owned(),
        body: change.summary.clone(),
        collapse_id: change_collapse_id(&workflow_id, last_frame_ts.as_deref(), &change.kind),
        extra: change_extra(&input.storm_cell_id, &change.kind),
    };
    let result = fan_out(ctx, request).await;
    let notified_at = epoch_seconds(ctx.workflow_time().unwrap_or(UNIX_EPOCH));
    ctx.state_mut(|s| {
        s.storm.last_change_kind = Some(change.kind.clone());
        s.storm.last_notified_at = Some(notified_at);
        s.storm.push_count += sent_count(result.as_ref());
    });
}

/// Push fan-out. Failure after retries is logged and swallowed: a push never fails the watch.
async fn fan_out(
    ctx: &WorkflowContext<WatchStormWorkflow>,
    request: FanOutPushInput,
) -> Option<FanOutPushResult> {
    match ctx
        .execute_activity(
            AlertsActivities::fan_out_push_to_user,
            request,
            options(&FAN_OUT_PUSH_TO_USER),
        )
        .await
    {
        Ok(result) => Some(result),
        Err(err) => {
            tracing::warn!(error = %err, "push fan-out failed after retries; watch continues");
            None
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn titles_follow_the_spec_mapping() {
        assert_eq!(title_for("intensifying"), "Storm intensifying");
        assert_eq!(title_for("dissipating"), "Storm weakening");
        assert_eq!(title_for("severe"), "Severe storm cell");
        assert_eq!(title_for("alert"), "Severe weather alert");
        assert_eq!(title_for("appeared"), "Storm update");
        assert_eq!(title_for(""), "Storm update");
    }

    #[test]
    fn collapse_ids_use_the_documented_shapes() {
        assert_eq!(
            change_collapse_id("watch:abc", Some("2026-01-01T00:00Z"), "severe"),
            "watch:abc:2026-01-01T00:00Z:severe"
        );
        assert_eq!(
            change_collapse_id("watch:abc", None, "severe"),
            "watch:abc:None:severe"
        );
        assert_eq!(alert_collapse_id("watch:abc", "A1"), "watch:abc:alert:A1");
    }

    #[test]
    fn extra_payloads_have_exact_keys() {
        let change = change_extra("cell", "intensifying");
        assert_eq!(
            change.keys().map(String::as_str).collect::<Vec<_>>(),
            vec!["kind", "storm_cell_id"]
        );
        let alert = alert_extra("cell", "A1");
        assert_eq!(alert.get("kind").map(String::as_str), Some("alert"));
        assert_eq!(alert.get("alert_id").map(String::as_str), Some("A1"));
    }

    #[test]
    fn new_executions_queue_every_alert() {
        let mut pending = vec!["old".to_string()];
        enqueue_alert(&mut pending, "a".into(), true);
        enqueue_alert(&mut pending, "b".into(), true);
        assert_eq!(pending, vec!["old", "a", "b"]);
    }

    #[test]
    fn legacy_histories_keep_only_the_last_alert() {
        let mut pending = vec!["old".to_string()];
        enqueue_alert(&mut pending, "a".into(), false);
        enqueue_alert(&mut pending, "b".into(), false);
        assert_eq!(pending, vec!["b"]);
    }

    #[test]
    fn continue_latch_is_sticky() {
        assert!(!latch_continue(false, false, false));
        assert!(latch_continue(false, true, false));
        assert!(
            latch_continue(true, false, false),
            "a set latch never clears"
        );
        assert!(latch_continue(false, false, true));
    }

    #[test]
    fn boundary_patch_is_read_only_when_latch_is_set() {
        let now = UNIX_EPOCH + Duration::from_secs(1000);
        let deadline = UNIX_EPOCH + Duration::from_secs(10_000);
        let mut reads = 0;
        let rolled = should_rollover(0, now, deadline, false, || {
            reads += 1;
            true
        });
        assert!(!rolled);
        assert_eq!(
            reads, 0,
            "patched() must not be evaluated when continue_requested is false"
        );

        assert!(should_rollover(0, now, deadline, true, || true));
        assert!(!should_rollover(0, now, deadline, true, || false));
    }

    #[test]
    fn iteration_cap_and_deadline_roll_over_without_patch_reads() {
        let now = UNIX_EPOCH + Duration::from_secs(1000);
        let deadline = UNIX_EPOCH + Duration::from_secs(10_000);
        assert!(should_rollover(
            MAX_ITERATIONS_PER_RUN,
            now,
            deadline,
            false,
            || panic!("not read")
        ));
        assert!(should_rollover(0, deadline, deadline, false, || panic!(
            "not read"
        )));
    }

    #[test]
    fn handoff_carries_state_and_pending_alerts() {
        let original = WatchStormInput {
            user_id: "synthetic-u".into(),
            storm_cell_id: "synthetic-c".into(),
            lat: 0.0,
            lng: 0.0,
            handoff: None,
        };
        let state = WatchStormState {
            push_count: 7,
            last_change_kind: Some("intensifying".into()),
            ..Default::default()
        };
        let next = handoff_input(&original, &state, &["a".into(), "b".into()]);
        let handoff = next.handoff.expect("handoff set");
        assert_eq!(handoff.schema_version, 1);
        assert_eq!(handoff.pending_alert_ids, vec!["a", "b"]);
        assert_eq!(handoff.state.push_count, 7);
    }

    #[test]
    fn unknown_handoff_schema_is_rejected() {
        let mut input = WatchStormInput {
            user_id: "u".into(),
            storm_cell_id: "c".into(),
            lat: 0.0,
            lng: 0.0,
            handoff: Some(WatchStormHandoff {
                schema_version: 2,
                state: WatchStormState::default(),
                pending_alert_ids: vec![],
            }),
        };
        assert!(check_handoff(&input).is_err());
        input.handoff = None;
        assert!(check_handoff(&input).is_ok());
    }

    #[test]
    fn failed_fan_out_counts_as_zero_sent() {
        assert_eq!(sent_count(None), 0);
        assert_eq!(sent_count(Some(&FanOutPushResult { sent: 3 })), 3);
    }
}
