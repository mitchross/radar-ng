//! HRRR ingest activities (spec section 4.3). The horizon call is synchronous in the sibling crate.

use std::sync::Arc;

use chrono::Utc;
use radar_ingest::IngestCtx;
use radar_ingest::hrrr::activities as hrrr;
use radar_types::payloads::hrrr::{FindRunResult, ForecastHourResult, HrrrCleanupResult};
use temporalio_macros::activities;
use temporalio_sdk::activities::{ActivityContext, ActivityError};

use crate::support::{activity_info, json_heartbeat, with_cancel};

/// Activity implementer for the HRRR pool (`hrrr` role, and `legacy`/`all`).
pub struct HrrrActivities {
    ingest: IngestCtx,
}

impl HrrrActivities {
    /// Wrap a shared ingest context.
    pub fn new(ingest: IngestCtx) -> Self {
        Self { ingest }
    }
}

#[activities]
impl HrrrActivities {
    /// `hrrr_find_latest_run`: newest run that has data on the source bucket.
    #[activity(name = "hrrr_find_latest_run")]
    pub async fn find_latest_run(
        self: Arc<Self>,
        ctx: ActivityContext,
    ) -> Result<FindRunResult, ActivityError> {
        with_cancel(&ctx, hrrr::find_latest_run(&self.ingest, Utc::now())).await
    }

    /// `hrrr_horizon_for_run`: number of forecast hours to process for a run.
    #[activity(name = "hrrr_horizon_for_run")]
    pub async fn horizon_for_run(
        self: Arc<Self>,
        _ctx: ActivityContext,
        run_id: String,
    ) -> Result<i64, ActivityError> {
        hrrr::horizon_for_run(&self.ingest, &run_id).map_err(ActivityError::from)
    }

    /// `hrrr_process_forecast_hour`: render every layer for one forecast hour. Two payloads: run ID, hour.
    #[activity(name = "hrrr_process_forecast_hour")]
    pub async fn process_forecast_hour(
        self: Arc<Self>,
        ctx: ActivityContext,
        run_id: String,
        fhr: i64,
    ) -> Result<ForecastHourResult, ActivityError> {
        let info = activity_info(&ctx);
        let beat = json_heartbeat(&ctx);
        with_cancel(
            &ctx,
            hrrr::process_forecast_hour(&self.ingest, &info, &run_id, fhr, &beat),
        )
        .await
    }

    /// `hrrr_publish_run`: publish the complete hour list; returns the layers that were published.
    #[activity(name = "hrrr_publish_run")]
    pub async fn publish_run(
        self: Arc<Self>,
        ctx: ActivityContext,
        run_id: String,
        results: Vec<ForecastHourResult>,
    ) -> Result<Vec<String>, ActivityError> {
        with_cancel(&ctx, hrrr::publish_run(&self.ingest, &run_id, &results)).await
    }

    /// `hrrr_mark_processed`: mark a complete run as processed.
    #[activity(name = "hrrr_mark_processed")]
    pub async fn mark_processed(
        self: Arc<Self>,
        ctx: ActivityContext,
        run_id: String,
    ) -> Result<(), ActivityError> {
        with_cancel(&ctx, hrrr::mark_processed(&self.ingest, &run_id)).await
    }

    /// `hrrr_cleanup`: retention sweep with the given window in hours.
    #[activity(name = "hrrr_cleanup")]
    pub async fn cleanup(
        self: Arc<Self>,
        ctx: ActivityContext,
        retention_hours: i64,
    ) -> Result<HrrrCleanupResult, ActivityError> {
        with_cancel(&ctx, hrrr::cleanup(&self.ingest, retention_hours)).await
    }
}
