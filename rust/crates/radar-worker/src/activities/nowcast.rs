//! Nowcast activity (spec section 4.7). Single attempt by design (`NOWCAST_RETRY`).

use std::sync::Arc;

use radar_nowcast::NowcastConfig;
use radar_nowcast::run::nowcast_run;
use radar_types::payloads::nowcast::NowcastResult;
use temporalio_macros::activities;
use temporalio_sdk::activities::{ActivityContext, ActivityError};

use crate::support::{unit_heartbeat, with_cancel};

/// Activity implementer for the nowcast pool (`nowcast` role, and `legacy`/`all`).
pub struct NowcastActivities {
    cfg: Arc<NowcastConfig>,
}

impl NowcastActivities {
    /// Wrap the nowcast configuration.
    pub fn new(cfg: NowcastConfig) -> Self {
        Self { cfg: Arc::new(cfg) }
    }
}

#[activities]
impl NowcastActivities {
    /// `nowcast_run`: produce the nowcast frames and return the run summary.
    #[activity(name = "nowcast_run")]
    pub async fn run(
        self: Arc<Self>,
        ctx: ActivityContext,
    ) -> Result<NowcastResult, ActivityError> {
        let beat = unit_heartbeat(&ctx);
        let output = with_cancel(&ctx, nowcast_run(self.cfg.clone(), beat)).await?;
        Ok(output.result)
    }
}
