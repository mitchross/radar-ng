//! `IngestGlobalModelWorkflow`. Schedules `ingest-gfs-radar` (`radar-gfs`) and `ingest-uv` (`uv`).
//!
//! Same steps and activity options as `IngestHrrrWorkflow`: find the run, process its hours with bounded
//! concurrency, publish the coherent prefix, mark the run processed only when complete, clean up.

use radar_types::payloads::global_model::IngestGlobalModelResult;
use radar_types::payloads::hrrr::{ForecastHourResult, HrrrCleanupResult};
use radar_types::retry::{
    HRRR_CLEANUP, HRRR_FIND_LATEST_RUN, HRRR_MARK_PROCESSED, HRRR_PROCESS_FORECAST_HOUR,
    HRRR_PUBLISH_RUN,
};
use temporalio_macros::{workflow, workflow_methods};
use temporalio_sdk::{WorkflowContext, WorkflowResult};

use super::{bounded, options};
use crate::activities::global_model::GlobalModelActivities;

/// Concurrent forecast-hour renders, as HRRR.
pub const HOUR_CONCURRENCY: usize = 4;

/// Whether the run is complete: every planned hour was published.
pub fn is_complete(frames_published: i64, hours: &[i64]) -> bool {
    !hours.is_empty() && frames_published == hours.len() as i64
}

/// Workflow type `IngestGlobalModelWorkflow`. The argument is the layer name.
#[workflow]
#[derive(Default)]
pub struct IngestGlobalModelWorkflow;

#[workflow_methods]
impl IngestGlobalModelWorkflow {
    /// Process the newest complete run of one global-model layer.
    #[run]
    pub async fn run(
        ctx: &mut WorkflowContext<Self>,
        layer: String,
    ) -> WorkflowResult<IngestGlobalModelResult> {
        let find = ctx
            .execute_activity(
                GlobalModelActivities::find_latest_run,
                layer.clone(),
                options(&HRRR_FIND_LATEST_RUN),
            )
            .await?;
        let mut result = IngestGlobalModelResult {
            layer: layer.clone(),
            run_id: find.run_id.clone(),
            skipped_already_processed: find.already_processed,
            frames_published: 0,
            complete: false,
            cleanup: None,
        };
        let Some(run_id) = find.run_id.clone().filter(|_| !find.already_processed) else {
            result.cleanup = Some(cleanup(ctx, &layer).await?);
            return Ok(result);
        };

        let ctx_ref: &WorkflowContext<Self> = ctx;
        let (hour_layer, hour_run) = (layer.clone(), run_id.clone());
        let results: Vec<ForecastHourResult> = bounded(find.hours.clone(), HOUR_CONCURRENCY, |fhr: i64| {
            let (layer, run) = (hour_layer.clone(), hour_run.clone());
            async move {
                match ctx_ref
                    .execute_activity(GlobalModelActivities::process_hour, (layer, run, fhr), options(&HRRR_PROCESS_FORECAST_HOUR))
                    .await
                {
                    Ok(result) => result,
                    Err(err) => {
                        tracing::warn!(fhr, error = %err, "global-model hour failed; continuing");
                        ForecastHourResult::failed(fhr)
                    }
                }
            }
        })
        .await;

        let published = ctx
            .execute_activity(
                GlobalModelActivities::publish_run,
                (layer.clone(), run_id.clone(), results),
                options(&HRRR_PUBLISH_RUN),
            )
            .await?;
        result.frames_published = published;
        result.complete = is_complete(published, &find.hours);
        if result.complete {
            ctx.execute_activity(
                GlobalModelActivities::mark_processed,
                (layer.clone(), run_id),
                options(&HRRR_MARK_PROCESSED),
            )
            .await?;
        }
        result.cleanup = Some(cleanup(ctx, &layer).await?);
        Ok(result)
    }
}

async fn cleanup(
    ctx: &WorkflowContext<IngestGlobalModelWorkflow>,
    layer: &str,
) -> WorkflowResult<HrrrCleanupResult> {
    Ok(ctx
        .execute_activity(
            GlobalModelActivities::cleanup,
            layer.to_string(),
            options(&HRRR_CLEANUP),
        )
        .await?)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn complete_only_when_every_planned_hour_published() {
        assert!(is_complete(3, &[24, 25, 26]));
        assert!(!is_complete(2, &[24, 25, 26]));
        assert!(!is_complete(0, &[]));
    }
}
