//! Air-quality (AQM) ingest activities (spec section 4.4). Chunk renders heartbeat every
//! `cfg.heartbeat_every`.

use std::sync::Arc;

use radar_aux::aqm::{
    AqmConfig, aqm_cleanup, aqm_find_latest_run, aqm_mark_processed, aqm_publish_run,
    aqm_render_chunk,
};
use radar_types::payloads::airquality::{AqmChunkResult, AqmCleanupResult, AqmFindRunResult};
use temporalio_macros::activities;
use temporalio_sdk::activities::{ActivityContext, ActivityError};

use crate::support::{json_heartbeat, with_cancel};

/// Activity implementer for the AQM render and publish steps (`aux` role, and `legacy`/`all`).
pub struct AqmActivities {
    cfg: AqmConfig,
    http: reqwest::Client,
}

impl AqmActivities {
    /// Wrap the AQM configuration and the shared HTTP client.
    pub fn new(cfg: AqmConfig, http: reqwest::Client) -> Self {
        Self { cfg, http }
    }
}

#[activities]
impl AqmActivities {
    /// `aqm_find_latest_run`: newest forecast run on the source bucket.
    #[activity(name = "aqm_find_latest_run")]
    pub async fn find_latest_run(
        self: Arc<Self>,
        ctx: ActivityContext,
    ) -> Result<AqmFindRunResult, ActivityError> {
        with_cancel(&ctx, aqm_find_latest_run(&self.cfg, &self.http)).await
    }

    /// `aqm_render_chunk`: render one layer's message chunk. Three payloads: run ID, layer, start message.
    #[activity(name = "aqm_render_chunk")]
    pub async fn render_chunk(
        self: Arc<Self>,
        ctx: ActivityContext,
        run_id: String,
        layer: String,
        start_msg: i64,
    ) -> Result<AqmChunkResult, ActivityError> {
        let beat = json_heartbeat(&ctx);
        with_cancel(
            &ctx,
            aqm_render_chunk(&self.cfg, &self.http, &run_id, &layer, start_msg, beat),
        )
        .await
    }

    /// `aqm_publish_run`: publish every chunk result; returns the published layer names.
    #[activity(name = "aqm_publish_run")]
    pub async fn publish_run(
        self: Arc<Self>,
        ctx: ActivityContext,
        run_id: String,
        results: Vec<AqmChunkResult>,
    ) -> Result<Vec<String>, ActivityError> {
        with_cancel(&ctx, aqm_publish_run(&self.cfg, &run_id, &results)).await
    }

    /// `aqm_mark_processed`: mark a run as processed.
    #[activity(name = "aqm_mark_processed")]
    pub async fn mark_processed(
        self: Arc<Self>,
        ctx: ActivityContext,
        run_id: String,
    ) -> Result<(), ActivityError> {
        with_cancel(&ctx, aqm_mark_processed(&self.cfg, &run_id)).await
    }

    /// `aqm_cleanup`: retention sweep with the given window in hours.
    #[activity(name = "aqm_cleanup")]
    pub async fn cleanup(
        self: Arc<Self>,
        ctx: ActivityContext,
        retention_hours: i64,
    ) -> Result<AqmCleanupResult, ActivityError> {
        with_cancel(&ctx, aqm_cleanup(&self.cfg, retention_hours)).await
    }
}
