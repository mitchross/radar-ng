//! `IngestWinterWorkflow`. Schedules `ingest-nbm-snow` (`nbm-snow`: `snowfall-forecast`, `ice-forecast`),
//! `ingest-snow-depth` (`snow-depth`), `ingest-snowfall-24h` (`snowfall-24h`) and `ingest-snow-chance`
//! (`snow-chance`: `snow-chance-1`, `-4`, `-8`).
//!
//! Same steps and activity options as `IngestGlobalModelWorkflow`: find the run, process its hours with
//! bounded concurrency, publish the coherent prefix, mark the run processed only when complete, clean up.

use radar_types::payloads::hrrr::{ForecastHourResult, HrrrCleanupResult};
use radar_types::payloads::winter::IngestWinterResult;
use radar_types::retry::{
    HRRR_CLEANUP, HRRR_FIND_LATEST_RUN, HRRR_MARK_PROCESSED, HRRR_PROCESS_FORECAST_HOUR,
    HRRR_PUBLISH_RUN,
};
use temporalio_macros::{workflow, workflow_methods};
use temporalio_sdk::{WorkflowContext, WorkflowResult};

use super::global_model::is_complete;
use super::{bounded, options};
use crate::activities::winter::WinterActivities;

/// Concurrent forecast-hour renders, as HRRR.
pub const HOUR_CONCURRENCY: usize = 4;

/// Workflow type `IngestWinterWorkflow`. The argument is the product name.
#[workflow]
#[derive(Default)]
pub struct IngestWinterWorkflow;

#[workflow_methods]
impl IngestWinterWorkflow {
    /// Process the newest complete run of one winter product.
    #[run]
    pub async fn run(
        ctx: &mut WorkflowContext<Self>,
        product: String,
    ) -> WorkflowResult<IngestWinterResult> {
        let find = ctx
            .execute_activity(
                WinterActivities::find_latest_run,
                product.clone(),
                options(&HRRR_FIND_LATEST_RUN),
            )
            .await?;
        let mut result = IngestWinterResult {
            product: product.clone(),
            run_id: find.run_id.clone(),
            skipped_already_processed: find.already_processed,
            frames_published: 0,
            complete: false,
            cleanup: None,
        };
        let Some(run_id) = find.run_id.clone().filter(|_| !find.already_processed) else {
            result.cleanup = Some(cleanup(ctx, &product).await?);
            return Ok(result);
        };

        let ctx_ref: &WorkflowContext<Self> = ctx;
        let (hour_product, hour_run) = (product.clone(), run_id.clone());
        let results: Vec<ForecastHourResult> =
            bounded(find.hours.clone(), HOUR_CONCURRENCY, |fhr: i64| {
                let (product, run) = (hour_product.clone(), hour_run.clone());
                async move {
                    match ctx_ref
                        .execute_activity(
                            WinterActivities::process_hour,
                            (product, run, fhr),
                            options(&HRRR_PROCESS_FORECAST_HOUR),
                        )
                        .await
                    {
                        Ok(result) => result,
                        Err(err) => {
                            tracing::warn!(fhr, error = %err, "winter hour failed; continuing");
                            ForecastHourResult::failed(fhr)
                        }
                    }
                }
            })
            .await;

        let published = ctx
            .execute_activity(
                WinterActivities::publish_run,
                (product.clone(), run_id.clone(), results),
                options(&HRRR_PUBLISH_RUN),
            )
            .await?;
        result.frames_published = published;
        result.complete = is_complete(published, &find.hours);
        if result.complete {
            ctx.execute_activity(
                WinterActivities::mark_processed,
                (product.clone(), run_id),
                options(&HRRR_MARK_PROCESSED),
            )
            .await?;
        }
        result.cleanup = Some(cleanup(ctx, &product).await?);
        Ok(result)
    }
}

async fn cleanup(
    ctx: &WorkflowContext<IngestWinterWorkflow>,
    product: &str,
) -> WorkflowResult<HrrrCleanupResult> {
    Ok(ctx
        .execute_activity(
            WinterActivities::cleanup,
            product.to_string(),
            options(&HRRR_CLEANUP),
        )
        .await?)
}
