//! Winter ingest activities (`nbm-snow`, `snow-depth`, `snowfall-24h`, `snow-chance`). Every call names its product first.

use std::sync::Arc;

use chrono::Utc;
use radar_ingest::IngestCtx;
use radar_ingest::winter::{Product, activities as winter};
use radar_types::payloads::hrrr::{ForecastHourResult, HrrrCleanupResult};
use radar_types::payloads::winter::WinterFindResult;
use temporalio_macros::activities;
use temporalio_sdk::activities::{ActivityContext, ActivityError};

use crate::support::{json_heartbeat, with_cancel};

/// Activity implementer for the winter products (`hrrr` role, and `legacy`/`all`).
pub struct WinterActivities {
    ingest: IngestCtx,
}

impl WinterActivities {
    /// Wrap a shared ingest context.
    pub fn new(ingest: IngestCtx) -> Self {
        Self { ingest }
    }
}

fn product(name: &str) -> Result<Product, ActivityError> {
    Product::from_name(name).map_err(ActivityError::from)
}

#[activities]
impl WinterActivities {
    /// `winter_find_latest_run`: newest run with every planned hour posted, and those hours.
    #[activity(name = "winter_find_latest_run")]
    pub async fn find_latest_run(
        self: Arc<Self>,
        ctx: ActivityContext,
        product_name: String,
    ) -> Result<WinterFindResult, ActivityError> {
        let product = product(&product_name)?;
        with_cancel(
            &ctx,
            winter::find_latest_run(&self.ingest, product, Utc::now()),
        )
        .await
    }

    /// `winter_process_hour`: render every layer for one forecast hour. Payloads: product, run ID, hour.
    #[activity(name = "winter_process_hour")]
    pub async fn process_hour(
        self: Arc<Self>,
        ctx: ActivityContext,
        product_name: String,
        run_id: String,
        fhr: i64,
    ) -> Result<ForecastHourResult, ActivityError> {
        let product = product(&product_name)?;
        let beat = json_heartbeat(&ctx);
        with_cancel(
            &ctx,
            winter::process_hour(&self.ingest, product, &run_id, fhr, &beat),
        )
        .await
    }

    /// `winter_publish_run`: publish the coherent prefix; returns the hours published.
    #[activity(name = "winter_publish_run")]
    pub async fn publish_run(
        self: Arc<Self>,
        ctx: ActivityContext,
        product_name: String,
        run_id: String,
        results: Vec<ForecastHourResult>,
    ) -> Result<i64, ActivityError> {
        let product = product(&product_name)?;
        with_cancel(
            &ctx,
            winter::publish_run(&self.ingest, product, &run_id, &results),
        )
        .await
    }

    /// `winter_mark_processed`: mark a complete run as processed.
    #[activity(name = "winter_mark_processed")]
    pub async fn mark_processed(
        self: Arc<Self>,
        ctx: ActivityContext,
        product_name: String,
        run_id: String,
    ) -> Result<(), ActivityError> {
        let product = product(&product_name)?;
        with_cancel(&ctx, winter::mark_processed(&self.ingest, product, &run_id)).await
    }

    /// `winter_cleanup`: remove superseded run directories and record caches past the retention.
    #[activity(name = "winter_cleanup")]
    pub async fn cleanup(
        self: Arc<Self>,
        ctx: ActivityContext,
        product_name: String,
    ) -> Result<HrrrCleanupResult, ActivityError> {
        let product = product(&product_name)?;
        with_cancel(&ctx, winter::cleanup(&self.ingest, product)).await
    }
}
