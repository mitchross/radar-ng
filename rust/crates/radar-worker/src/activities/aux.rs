//! Auxiliary ingest activities: lightning stream, tropical feed, tile cleanup (spec sections 4.5, 4.6, 4.8).

use std::sync::Arc;
use std::time::Duration;

use radar_alerts::lightning::{LightningConfig, LightningError, lightning_consume_stream};
use radar_aux::tile_cleanup::{TileCleanupConfig, tile_cleanup_sweep};
use radar_aux::tropical::{TropicalConfig, tropical_fetch_and_publish};
use radar_types::payloads::cleanup::TileCleanupResult;
use radar_types::payloads::lightning::LightningRunResult;
use radar_types::payloads::tropical::TropicalResult;
use temporalio_macros::activities;
use temporalio_sdk::activities::{ActivityContext, ActivityError};
use tokio_util::sync::CancellationToken;

use crate::support::{json_heartbeat, with_cancel};

/// Activity implementer for lightning, tropical and tile-cleanup (`aux` role, and `legacy`/`all`).
pub struct AuxActivities {
    http: reqwest::Client,
    tropical: TropicalConfig,
}

impl AuxActivities {
    /// Build from the shared HTTP client and the tropical configuration.
    pub fn new(http: reqwest::Client, tropical: TropicalConfig) -> Self {
        Self { http, tropical }
    }
}

#[activities]
impl AuxActivities {
    /// `lightning_consume_stream`: run the strike stream for `duration_s` seconds. Cancellation stops the
    /// stream at its next await point. Heartbeats carry the stream counters every 30 s.
    #[activity(name = "lightning_consume_stream")]
    pub async fn consume_stream(
        self: Arc<Self>,
        ctx: ActivityContext,
        duration_s: i64,
    ) -> Result<LightningRunResult, ActivityError> {
        let config = LightningConfig::from_env()?;
        let duration = Duration::from_secs(u64::try_from(duration_s).unwrap_or(0));
        let token = CancellationToken::new();
        let finished = CancellationToken::new();
        let (bridge, watcher, done) = (token.clone(), ctx.clone(), finished.clone());
        tokio::spawn(async move {
            tokio::select! {
                () = watcher.cancelled() => bridge.cancel(),
                () = done.cancelled() => {}
            }
        });
        let beat = json_heartbeat(&ctx);
        let outcome =
            lightning_consume_stream(config, duration, token, move |details| beat(details)).await;
        finished.cancel();
        match outcome {
            Ok(result) => Ok(result),
            Err(LightningError::Cancelled) => Err(ActivityError::cancelled()),
            Err(LightningError::Other(err)) => Err(err.into()),
        }
    }

    /// `tropical_fetch_and_publish`: fetch the storm feed and publish its GeoJSON.
    ///
    /// TODO(integration): `radar_aux::tropical::fetch_gis` builds its job closure with an elided `&str`
    /// tuple element, so its future is not `Send` for every lifetime and the activity macro rejects this
    /// method. Fix the closure annotation in radar-aux, then drop the blocking-thread hop here. Until then the
    /// future runs to completion on a blocking thread. Cancelling the activity stops waiting on it, but the
    /// thread finishes on its own, bounded by the feed and GIS HTTP timeouts.
    #[activity(name = "tropical_fetch_and_publish")]
    pub async fn fetch_and_publish(
        self: Arc<Self>,
        ctx: ActivityContext,
    ) -> Result<TropicalResult, ActivityError> {
        let this = Arc::clone(&self);
        let run = tokio::task::spawn_blocking(move || {
            tokio::runtime::Handle::current()
                .block_on(tropical_fetch_and_publish(&this.tropical, &this.http))
        });
        let work = async move {
            let joined = run
                .await
                .map_err(|err| anyhow::anyhow!("tropical task failed: {err}"))?;
            joined.map_err(anyhow::Error::from)
        };
        with_cancel(&ctx, work).await
    }

    /// `tile_cleanup_sweep`: sweep expired tiles and grids across all layers.
    #[activity(name = "tile_cleanup_sweep")]
    pub async fn tile_cleanup(
        self: Arc<Self>,
        ctx: ActivityContext,
    ) -> Result<TileCleanupResult, ActivityError> {
        let config = TileCleanupConfig::from_env()?;
        with_cancel(&ctx, tile_cleanup_sweep(&config)).await
    }
}
