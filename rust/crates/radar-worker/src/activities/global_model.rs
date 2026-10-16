//! Global-model ingest activities (`radar-gfs`, `uv`). Every call names its layer first.

use std::sync::Arc;

use chrono::Utc;
use radar_ingest::IngestCtx;
use radar_ingest::global_model::{Product, activities as global};
use radar_types::payloads::global_model::GlobalModelFindResult;
use radar_types::payloads::hrrr::{ForecastHourResult, HrrrCleanupResult};
use temporalio_macros::activities;
use temporalio_sdk::activities::{ActivityContext, ActivityError};

use crate::support::{json_heartbeat, with_cancel};

/// Activity implementer for the global-model layers (`hrrr` role, and `legacy`/`all`).
pub struct GlobalModelActivities {
    ingest: IngestCtx,
}

impl GlobalModelActivities {
    /// Wrap a shared ingest context.
    pub fn new(ingest: IngestCtx) -> Self {
        Self { ingest }
    }
}

fn product(layer: &str) -> Result<Product, ActivityError> {
    Product::from_layer(layer).map_err(ActivityError::from)
}

#[activities]
impl GlobalModelActivities {
    /// `global_model_find_latest_run`: newest run with every planned hour posted, and those hours.
    #[activity(name = "global_model_find_latest_run")]
    pub async fn find_latest_run(
        self: Arc<Self>,
        ctx: ActivityContext,
        layer: String,
    ) -> Result<GlobalModelFindResult, ActivityError> {
        let product = product(&layer)?;
        with_cancel(
            &ctx,
            global::find_latest_run(&self.ingest, product, Utc::now()),
        )
        .await
    }

    /// `global_model_process_hour`: render one forecast hour. Three payloads: layer, run ID, hour.
    #[activity(name = "global_model_process_hour")]
    pub async fn process_hour(
        self: Arc<Self>,
        ctx: ActivityContext,
        layer: String,
        run_id: String,
        fhr: i64,
    ) -> Result<ForecastHourResult, ActivityError> {
        let product = product(&layer)?;
        let beat = json_heartbeat(&ctx);
        with_cancel(
            &ctx,
            global::process_hour(&self.ingest, product, &run_id, fhr, &beat),
        )
        .await
    }

    /// `global_model_publish_run`: publish the coherent prefix; returns the frames published.
    #[activity(name = "global_model_publish_run")]
    pub async fn publish_run(
        self: Arc<Self>,
        ctx: ActivityContext,
        layer: String,
        run_id: String,
        results: Vec<ForecastHourResult>,
    ) -> Result<i64, ActivityError> {
        let product = product(&layer)?;
        with_cancel(
            &ctx,
            global::publish_run(&self.ingest, product, &run_id, &results),
        )
        .await
    }

    /// `global_model_mark_processed`: mark a complete run as processed.
    #[activity(name = "global_model_mark_processed")]
    pub async fn mark_processed(
        self: Arc<Self>,
        ctx: ActivityContext,
        layer: String,
        run_id: String,
    ) -> Result<(), ActivityError> {
        let product = product(&layer)?;
        with_cancel(&ctx, global::mark_processed(&self.ingest, product, &run_id)).await
    }

    /// `global_model_cleanup`: remove superseded run directories past the layer's retention.
    #[activity(name = "global_model_cleanup")]
    pub async fn cleanup(
        self: Arc<Self>,
        ctx: ActivityContext,
        layer: String,
    ) -> Result<HrrrCleanupResult, ActivityError> {
        let product = product(&layer)?;
        with_cancel(&ctx, global::cleanup(&self.ingest, product)).await
    }
}
