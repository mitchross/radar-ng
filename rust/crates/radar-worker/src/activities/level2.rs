//! Level II pool activities: the housekeeping tick and one bounded station poll.

use std::sync::Arc;

use radar_level2::poller::{Poller, enabled};
use radar_types::payloads::level2::{Level2PollInput, Level2PollResult, Level2Tick};
use temporalio_macros::activities;
use temporalio_sdk::activities::{ActivityContext, ActivityError};

use crate::support::with_cancel;

/// Activity implementer for the `level2` pool. Built only when the numeric gate is on.
pub struct Level2Activities {
    poller: Option<Poller>,
}

impl Level2Activities {
    /// With the gate off the pool still polls its queue, and every tick reports `enabled: false`.
    pub fn from_env() -> anyhow::Result<Self> {
        let poller = if enabled()? {
            Some(Poller::from_env()?)
        } else {
            None
        };
        Ok(Self { poller })
    }
}

#[activities]
impl Level2Activities {
    /// `level2_tick`: reap, evict and list demanded stations. Cancellation drops it at the next await.
    #[activity(name = "level2_tick")]
    pub async fn tick(self: Arc<Self>, ctx: ActivityContext) -> Result<Level2Tick, ActivityError> {
        let Some(poller) = &self.poller else {
            return Ok(Level2Tick::default());
        };
        let tick = with_cancel(&ctx, poller.tick()).await?;
        Ok(Level2Tick {
            stations: tick.stations,
            enabled: true,
        })
    }

    /// `level2_poll_station`: one poll. A fallback is a result, not a failure.
    #[activity(name = "level2_poll_station")]
    pub async fn poll_station(
        self: Arc<Self>,
        ctx: ActivityContext,
        input: Level2PollInput,
    ) -> Result<Level2PollResult, ActivityError> {
        let Some(poller) = &self.poller else {
            return Ok(Level2PollResult {
                status: "skipped".into(),
                reason: Some("gate_disabled".into()),
            });
        };
        let outcome = with_cancel(&ctx, poller.poll(&input.station)).await?;
        Ok(Level2PollResult {
            status: outcome.status.into(),
            reason: outcome.reason,
        })
    }
}
