//! Schedule watchdog (spec section 7). A read-only observer that runs only on the seeding pool.
//!
//! It never calls trigger, update, delete or terminate. A stall is reported once per process for each
//! exact timer value. Suppression is in memory, so a restart or another replica can report again.

use std::collections::HashMap;
use std::sync::Mutex;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use chrono::{DateTime, Utc};
use futures::future::join_all;
use radar_types::schedules::{ScheduleDef, all_schedules};
use temporalio_client::schedules::ScheduleDescriptionAction;
use temporalio_client::{Client, RpcOptions};

/// Time between sweeps (`CHECK_EVERY`).
pub const CHECK_EVERY: Duration = Duration::from_secs(60);
/// Per-describe timeout (`RPC_TIMEOUT`).
pub const RPC_TIMEOUT: Duration = Duration::from_secs(15);
/// A timer more than this many intervals overdue is a stall (`OVERDUE_INTERVALS`).
pub const OVERDUE_INTERVALS: u32 = 2;

/// Stall predicate (spec 7.1): true only when the next timer is strictly older than two intervals, there
/// is no running action, and at least one next time is known.
pub fn is_wedged(
    next_action_times: &[SystemTime],
    running_actions: usize,
    interval: Duration,
    now: SystemTime,
) -> bool {
    let Some(first) = next_action_times.first() else {
        return false;
    };
    if running_actions > 0 {
        return false;
    }
    let overdue = interval
        .checked_mul(OVERDUE_INTERVALS)
        .and_then(|span| first.checked_add(span));
    overdue.is_some_and(|limit| limit < now)
}

/// What one observation of a schedule says.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Observation {
    /// Paused by an operator. Not a stall.
    Paused,
    /// Not wedged. `future` is the next timer when it lies in the future.
    Healthy { future: Option<SystemTime> },
    /// Wedged on the timer `next`.
    Stalled { next: SystemTime },
}

/// Classify one describe result (spec 7.2 steps 1 to 3).
pub fn observe(
    paused: bool,
    next_action_times: &[SystemTime],
    running_actions: usize,
    interval: Duration,
    now: SystemTime,
) -> Observation {
    if paused {
        return Observation::Paused;
    }
    if is_wedged(next_action_times, running_actions, interval, now) {
        return Observation::Stalled {
            next: next_action_times[0],
        };
    }
    let future = next_action_times.first().copied().filter(|t| *t > now);
    Observation::Healthy { future }
}

/// Apply an observation to the per-process suppression map. Returns true when an event should be emitted.
///
/// A paused schedule or a future timer clears suppression. A running action neither clears it nor emits.
/// The same timer is reported once. Recording happens before the caller logs.
pub fn apply_observation(
    reported: &mut HashMap<String, SystemTime>,
    schedule_id: &str,
    observation: &Observation,
) -> bool {
    match observation {
        Observation::Paused => {
            reported.remove(schedule_id);
            false
        }
        Observation::Healthy { future: Some(_) } => {
            reported.remove(schedule_id);
            false
        }
        Observation::Healthy { future: None } => false,
        Observation::Stalled { next } => {
            if reported.get(schedule_id) == Some(next) {
                return false;
            }
            reported.insert(schedule_id.to_owned(), *next);
            true
        }
    }
}

fn iso(time: SystemTime) -> String {
    let dt: DateTime<Utc> = time.into();
    dt.to_rfc3339()
}

/// Log a failed observation as WARNING with a safe label (spec 7.2). Never carries message text.
fn observation_failed(schedule_id: &str, label: &str) {
    tracing::warn!(
        event = "TEMPORAL_SCHEDULE_OBSERVATION_FAILED",
        schedule_id = schedule_id,
        error = label,
        operator_action = "inspect_schedule_observation",
        "schedule observation failed"
    );
}

/// Describe one schedule and emit `TEMPORAL_SCHEDULE_STALLED` when it is newly wedged. Observation errors are
/// logged as WARNING with a safe label and never propagate.
pub async fn check_schedule(
    client: &Client,
    def: &ScheduleDef,
    reported: &Mutex<HashMap<String, SystemTime>>,
    now: SystemTime,
) {
    let interval = Duration::from_secs(def.interval_s);
    let handle = client.get_schedule_handle(def.id);
    let description =
        match tokio::time::timeout(RPC_TIMEOUT, handle.describe(RpcOptions::default())).await {
            Ok(Ok(description)) => description,
            Ok(Err(_)) => {
                observation_failed(def.id, "Error");
                return;
            }
            Err(_) => {
                observation_failed(def.id, "RPCError[DEADLINE_EXCEEDED]");
                return;
            }
        };

    let next = description.future_action_times();
    let running = description.running_actions().len();
    let observation = observe(description.paused(), &next, running, interval, now);
    let emit = {
        let mut map = reported
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        apply_observation(&mut map, def.id, &observation)
    };
    if let (true, Observation::Stalled { next }) = (emit, &observation) {
        let (workflow_name, task_queue) = match description.action() {
            ScheduleDescriptionAction::StartWorkflow(action) => (
                action.workflow_type().to_owned(),
                action.task_queue().to_owned(),
            ),
            // The action enum is non-exhaustive. Any other action type is still reported as stalled.
            _ => ("unknown".to_owned(), String::new()),
        };
        let overdue = now.duration_since(*next).map(|d| d.as_secs()).unwrap_or(0);
        tracing::error!(
            event = "TEMPORAL_SCHEDULE_STALLED",
            schedule_id = def.id,
            workflow_name = %workflow_name,
            task_queue = %task_queue,
            next_action_time = %iso(*next),
            overdue_seconds = overdue,
            interval_seconds = def.interval_s,
            automatic_recovery = false,
            operator_action = "inspect_temporal_timer_dlq",
            "TEMPORAL_SCHEDULE_STALLED schedule {} next action {} overdue {}s (workflow {} on {}); automatic recovery is off, inspect the timer and its dead-letter queue",
            def.id,
            iso(*next),
            overdue,
            workflow_name,
            task_queue
        );
    }
}

/// Watch every schedule forever. Each sweep runs the checks concurrently so one slow describe cannot hold up
/// the others.
pub async fn watch_schedules(client: Client, schedules: Vec<ScheduleDef>) {
    let reported = Mutex::new(HashMap::new());
    loop {
        tokio::time::sleep(CHECK_EVERY).await;
        let now = SystemTime::now();
        join_all(
            schedules
                .iter()
                .map(|def| check_schedule(&client, def, &reported, now)),
        )
        .await;
    }
}

/// The schedules the watchdog observes: every definition in `radar_types::schedules::all_schedules`.
pub fn watched_schedules() -> Vec<ScheduleDef> {
    all_schedules()
}

/// Seconds since the Unix epoch, for tests and logs.
pub fn epoch_seconds(time: SystemTime) -> u64 {
    time.duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(secs: u64) -> SystemTime {
        UNIX_EPOCH + Duration::from_secs(secs)
    }

    const INTERVAL: Duration = Duration::from_secs(60);

    #[test]
    fn empty_next_times_are_never_wedged() {
        assert!(!is_wedged(&[], 0, INTERVAL, at(10_000)));
    }

    #[test]
    fn running_action_is_never_wedged() {
        assert!(!is_wedged(&[at(0)], 1, INTERVAL, at(10_000)));
    }

    #[test]
    fn future_timer_is_not_wedged() {
        assert!(!is_wedged(&[at(10_100)], 0, INTERVAL, at(10_000)));
    }

    #[test]
    fn wedged_only_when_strictly_older_than_two_intervals() {
        let now = at(1_000);
        // exactly 2 intervals old: not strictly older
        assert!(!is_wedged(&[at(880)], 0, INTERVAL, now));
        // older by one second
        assert!(is_wedged(&[at(879)], 0, INTERVAL, now));
    }

    #[test]
    fn observation_reports_each_timer_once() {
        let mut reported = HashMap::new();
        let stalled = observe(false, &[at(0)], 0, INTERVAL, at(1_000));
        assert!(apply_observation(&mut reported, "nowcast", &stalled));
        assert!(
            !apply_observation(&mut reported, "nowcast", &stalled),
            "same timer is suppressed"
        );
    }

    #[test]
    fn future_timer_clears_suppression_so_a_later_stall_is_reported() {
        let mut reported = HashMap::new();
        let stalled = observe(false, &[at(0)], 0, INTERVAL, at(1_000));
        assert!(apply_observation(&mut reported, "nowcast", &stalled));
        let healthy = observe(false, &[at(2_000)], 0, INTERVAL, at(1_000));
        assert!(!apply_observation(&mut reported, "nowcast", &healthy));
        assert!(
            apply_observation(&mut reported, "nowcast", &stalled),
            "re-reported after the clearing event"
        );
    }

    #[test]
    fn running_action_neither_clears_nor_emits() {
        let mut reported = HashMap::new();
        let stalled = observe(false, &[at(0)], 0, INTERVAL, at(1_000));
        assert!(apply_observation(&mut reported, "nowcast", &stalled));
        let running = observe(false, &[at(0)], 1, INTERVAL, at(1_000));
        assert!(!apply_observation(&mut reported, "nowcast", &running));
        assert!(
            !apply_observation(&mut reported, "nowcast", &stalled),
            "suppression survives a running action"
        );
    }

    #[test]
    fn paused_schedule_clears_suppression_and_emits_nothing() {
        let mut reported = HashMap::new();
        let stalled = observe(false, &[at(0)], 0, INTERVAL, at(1_000));
        apply_observation(&mut reported, "nowcast", &stalled);
        let paused = observe(true, &[at(0)], 0, INTERVAL, at(1_000));
        assert_eq!(paused, Observation::Paused);
        assert!(!apply_observation(&mut reported, "nowcast", &paused));
        assert!(apply_observation(&mut reported, "nowcast", &stalled));
    }

    #[test]
    fn stall_is_per_schedule() {
        let mut reported = HashMap::new();
        let stalled = observe(false, &[at(0)], 0, INTERVAL, at(1_000));
        assert!(apply_observation(&mut reported, "a", &stalled));
        assert!(apply_observation(&mut reported, "b", &stalled));
    }

    #[test]
    fn watched_set_is_the_full_schedule_list() {
        assert_eq!(watched_schedules().len(), 12);
    }
}
