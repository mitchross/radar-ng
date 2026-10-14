//! Open-meteo sync activity, registered only on the open-meteo pool (spec section 4.9).

use std::sync::Arc;

use radar_aux::open_meteo::{OpenMeteoConfig, open_meteo_sync};
use radar_types::payloads::open_meteo::{OpenMeteoSyncArgs, OpenMeteoSyncResult};
use temporalio_macros::activities;
use temporalio_sdk::activities::{ActivityContext, ActivityError};

use crate::support::{classified, json_heartbeat, with_cancel};

/// Activity implementer for the open-meteo pool.
pub struct OpenMeteoActivities {
    cfg: OpenMeteoConfig,
}

impl OpenMeteoActivities {
    /// Wrap the open-meteo configuration (binary path, data directory, heartbeat period).
    pub fn new(cfg: OpenMeteoConfig) -> Self {
        Self { cfg }
    }
}

#[activities]
impl OpenMeteoActivities {
    /// `open_meteo_sync`: run the sync subprocess. Non-zero exit is retryable unless the sibling crate
    /// marks the failure as terminal.
    #[activity(name = "open_meteo_sync")]
    pub async fn sync(
        self: Arc<Self>,
        ctx: ActivityContext,
        args: OpenMeteoSyncArgs,
    ) -> Result<OpenMeteoSyncResult, ActivityError> {
        let beat = json_heartbeat(&ctx);
        with_cancel(&ctx, async {
            open_meteo_sync(&self.cfg, &args, beat)
                .await
                .map_err(|err| classified(err.message, err.retryable))
        })
        .await
    }
}
