//! `Level2PoolWorkflow`: one per pool, polls only the stations with live tile demand.
//! No schedule starts it and no station has its own schedule; the worker starts it by fixed ID.

use std::collections::BTreeMap;
use std::time::Duration;

use radar_level2::poller::{POLL_INTERVAL_S, backoff_s};
use radar_types::payloads::level2::{
    Level2PollInput, Level2PollResult, Level2PoolInput, Level2Tick,
};
use radar_types::retry::{LEVEL2_POLL_STATION, LEVEL2_TICK};
use temporalio_macros::{workflow, workflow_methods};
use temporalio_sdk::{
    ContinueAsNewOptions, ContinueAsNewVersioningBehavior, WorkflowContext, WorkflowResult,
};

use super::{bounded, options};
use crate::activities::level2::Level2Activities;

/// Passes per run before continue-as-new, which keeps history far under the event limit.
pub const MAX_PASSES_PER_RUN: u32 = 60;
/// Wait while the gate is off.
pub const IDLE_WAIT_S: u64 = 60;
/// Stations polled at once. The pool-wide cap is eight.
pub const POLL_CONCURRENCY: usize = 8;

/// Next failure count for a station after one poll.
pub fn next_failures(previous: u32, result: &Result<Level2PollResult, String>) -> u32 {
    match result {
        Ok(r) if r.status == "numeric_ready" => 0,
        Ok(r) if r.status == "skipped" => previous,
        _ => previous.saturating_add(1),
    }
}

/// Stations due this pass: those whose failure streak has not been waited out.
pub fn due(failures: &BTreeMap<String, u32>, stations: &[String], pass: u32) -> Vec<String> {
    stations
        .iter()
        .filter(|s| {
            let n = failures.get(*s).copied().unwrap_or(0);
            let every = (backoff_s(n) / POLL_INTERVAL_S).max(1) as u32;
            pass.is_multiple_of(every)
        })
        .cloned()
        .collect()
}

/// Workflow type `Level2PoolWorkflow`.
#[workflow]
#[derive(Default)]
pub struct Level2PoolWorkflow;

#[workflow_methods]
impl Level2PoolWorkflow {
    /// Tick, poll the demanded stations, wait, and roll over before history grows.
    #[run]
    pub async fn run(
        ctx: &mut WorkflowContext<Self>,
        input: Level2PoolInput,
    ) -> WorkflowResult<()> {
        let mut failures: BTreeMap<String, u32> = input.failures;
        for pass in 0..MAX_PASSES_PER_RUN {
            let tick: Level2Tick = ctx
                .execute_activity(Level2Activities::tick, (), options(&LEVEL2_TICK))
                .await?;
            if !tick.enabled {
                ctx.timer(Duration::from_secs(IDLE_WAIT_S)).await;
            } else {
                failures.retain(|s, _| tick.stations.contains(s));
                let stations = due(&failures, &tick.stations, pass);
                let results = bounded(stations.clone(), POLL_CONCURRENCY, |station: String| {
                    let call = ctx.execute_activity(
                        Level2Activities::poll_station,
                        Level2PollInput { station },
                        options(&LEVEL2_POLL_STATION),
                    );
                    async move { call.await.map_err(|e| e.to_string()) }
                })
                .await;
                for (station, result) in stations.iter().zip(&results) {
                    let n = next_failures(failures.get(station).copied().unwrap_or(0), result);
                    failures.insert(station.clone(), n);
                }
                ctx.timer(Duration::from_secs(POLL_INTERVAL_S)).await;
            }
            if ctx.continue_as_new_suggested() || ctx.target_worker_deployment_version_changed() {
                break;
            }
        }
        let mut next = ContinueAsNewOptions::default();
        next.initial_versioning_behavior = Some(ContinueAsNewVersioningBehavior::AutoUpgrade);
        ctx.continue_as_new(Level2PoolInput { failures }, next)?
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ok(status: &str) -> Result<Level2PollResult, String> {
        Ok(Level2PollResult {
            status: status.into(),
            reason: None,
        })
    }

    #[test]
    fn failures_reset_on_success_and_hold_on_skip() {
        assert_eq!(next_failures(3, &ok("numeric_ready")), 0);
        assert_eq!(next_failures(3, &ok("skipped")), 3);
        assert_eq!(next_failures(3, &ok("fallback")), 4);
        assert_eq!(next_failures(3, &Err("boom".into())), 4);
    }

    #[test]
    fn backed_off_stations_are_polled_less_often() {
        let stations = vec!["KGRR".to_owned(), "KDTX".to_owned()];
        let mut failures = BTreeMap::new();
        failures.insert("KDTX".to_owned(), 2);
        assert_eq!(due(&failures, &stations, 1), vec!["KGRR".to_owned()]);
        assert_eq!(due(&failures, &stations, 4), stations);
    }
}
