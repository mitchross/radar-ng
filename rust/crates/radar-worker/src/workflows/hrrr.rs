//! `IngestHrrrWorkflow` (spec 4.3). Schedule `ingest-hrrr`.

use radar_ingest::hrrr::state::publishable_prefix;
use radar_types::payloads::hrrr::{
    ForecastHourResult, HrrrCleanupResult, IngestHrrrResult, RETENTION_HOURS,
};
use radar_types::retry::{
    HRRR_CLEANUP, HRRR_FIND_LATEST_RUN, HRRR_HORIZON_FOR_RUN, HRRR_MARK_PROCESSED,
    HRRR_PROCESS_FORECAST_HOUR, HRRR_PUBLISH_RUN,
};
use temporalio_macros::{workflow, workflow_methods};
use temporalio_sdk::{WorkflowContext, WorkflowResult};

use super::{bounded, options};
use crate::activities::hrrr::HrrrActivities;

/// Concurrent forecast-hour renders (Python `Semaphore(4)`).
pub const FORECAST_CONCURRENCY: usize = 4;
/// Layer whose presence in the publish result marks the run as published.
pub const GATING_LAYER: &str = "radar-hrrr";

/// Outcome of the decision after all hours have been processed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Decision {
    /// No publishable prefix: cleanup only, zero hours reported.
    NothingPublishable,
    /// Publish was attempted, but the gating layer is absent from the published list.
    NotPublished,
    /// Published; `mark` says whether the run is complete and should be marked processed.
    Published { prefix: usize, mark: bool },
}

/// Decide what follows the publish step (spec 4.3 pseudo-code).
pub fn decide(
    results: &[ForecastHourResult],
    horizon: i64,
    published: Option<&[String]>,
) -> Decision {
    let prefix = publishable_prefix(results).len();
    if prefix == 0 {
        return Decision::NothingPublishable;
    }
    let complete = prefix as i64 == horizon;
    match published {
        Some(layers) if layers.iter().any(|l| l == GATING_LAYER) => Decision::Published {
            prefix,
            mark: complete,
        },
        _ => Decision::NotPublished,
    }
}

/// Workflow type `IngestHrrrWorkflow`.
#[workflow]
#[derive(Default)]
pub struct IngestHrrrWorkflow;

#[workflow_methods]
impl IngestHrrrWorkflow {
    /// Find the run, process hours 1..=horizon with bounded concurrency, publish the full hour list, and
    /// mark the run processed only when it is complete. Partial publication is intentional (spec 4.3).
    #[run]
    pub async fn run(ctx: &mut WorkflowContext<Self>) -> WorkflowResult<IngestHrrrResult> {
        let find = ctx
            .execute_activity(
                HrrrActivities::find_latest_run,
                (),
                options(&HRRR_FIND_LATEST_RUN),
            )
            .await?;
        let Some(run_id) = find.run_id.clone() else {
            let cleanup = cleanup(ctx).await?;
            return Ok(report(None, false, 0, cleanup, vec![], vec![], false));
        };
        if find.already_processed {
            let cleanup = cleanup(ctx).await?;
            return Ok(report(
                Some(run_id),
                true,
                0,
                cleanup,
                vec![],
                vec![],
                false,
            ));
        }

        let horizon = ctx
            .execute_activity(
                HrrrActivities::horizon_for_run,
                run_id.clone(),
                options(&HRRR_HORIZON_FOR_RUN),
            )
            .await?;

        let ctx_ref: &WorkflowContext<Self> = ctx;
        let hour_run = run_id.clone();
        let results: Vec<ForecastHourResult> = bounded((1..=horizon).collect::<Vec<i64>>(), FORECAST_CONCURRENCY, |fhr: i64| {
            let run = hour_run.clone();
            async move {
                match ctx_ref
                    .execute_activity(HrrrActivities::process_forecast_hour, (run, fhr), options(&HRRR_PROCESS_FORECAST_HOUR))
                    .await
                {
                    Ok(result) => result,
                    Err(err) => {
                        tracing::warn!(fhr, error = %err, "hrrr forecast hour failed; continuing");
                        ForecastHourResult::failed(fhr)
                    }
                }
            }
        })
        .await;

        let layers_per_hour: Vec<Vec<String>> =
            results.iter().map(|r| r.rendered_layers.clone()).collect();
        let prefix = publishable_prefix(&results).len();

        if prefix == 0 {
            let cleanup = cleanup(ctx).await?;
            return Ok(report(
                Some(run_id),
                false,
                0,
                cleanup,
                layers_per_hour,
                vec![],
                false,
            ));
        }

        let published = ctx
            .execute_activity(
                HrrrActivities::publish_run,
                (run_id.clone(), results.clone()),
                options(&HRRR_PUBLISH_RUN),
            )
            .await?;

        let complete = prefix as i64 == horizon;
        match decide(&results, horizon, Some(&published)) {
            Decision::NothingPublishable | Decision::NotPublished => {
                let cleanup = cleanup(ctx).await?;
                Ok(report(
                    Some(run_id),
                    false,
                    0,
                    cleanup,
                    layers_per_hour,
                    vec![],
                    false,
                ))
            }
            Decision::Published { prefix, mark } => {
                if mark {
                    ctx.execute_activity(
                        HrrrActivities::mark_processed,
                        run_id.clone(),
                        options(&HRRR_MARK_PROCESSED),
                    )
                    .await?;
                }
                let cleanup = cleanup(ctx).await?;
                Ok(report(
                    Some(run_id),
                    false,
                    prefix as i64,
                    cleanup,
                    layers_per_hour,
                    published,
                    complete,
                ))
            }
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn report(
    run_id: Option<String>,
    skipped: bool,
    hours: i64,
    cleanup: HrrrCleanupResult,
    layers_per_hour: Vec<Vec<String>>,
    published_layers: Vec<String>,
    complete: bool,
) -> IngestHrrrResult {
    IngestHrrrResult {
        run_id,
        skipped_already_processed: skipped,
        forecast_hours_processed: hours,
        cleanup: Some(cleanup),
        layers_per_hour,
        published_layers,
        complete,
    }
}

async fn cleanup(ctx: &WorkflowContext<IngestHrrrWorkflow>) -> WorkflowResult<HrrrCleanupResult> {
    Ok(ctx
        .execute_activity(
            HrrrActivities::cleanup,
            RETENTION_HOURS,
            options(&HRRR_CLEANUP),
        )
        .await?)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hour(fhr: i64, layers: &[&str]) -> ForecastHourResult {
        ForecastHourResult {
            fhr,
            rendered_layers: layers.iter().map(|s| (*s).to_owned()).collect(),
            duration_s: 1.0,
            valid_timestamp: "t".into(),
        }
    }

    #[test]
    fn nothing_publishable_when_first_hour_failed() {
        let results = vec![ForecastHourResult::failed(1), hour(2, &[GATING_LAYER])];
        assert_eq!(
            decide(&results, 2, Some(&[GATING_LAYER.to_owned()])),
            Decision::NothingPublishable
        );
    }

    #[test]
    fn complete_run_is_marked() {
        let results = vec![hour(1, &[GATING_LAYER]), hour(2, &[GATING_LAYER])];
        assert_eq!(
            decide(&results, 2, Some(&[GATING_LAYER.to_owned()])),
            Decision::Published {
                prefix: 2,
                mark: true
            }
        );
    }

    #[test]
    fn partial_run_publishes_without_marking() {
        let results = vec![hour(1, &[GATING_LAYER]), ForecastHourResult::failed(2)];
        assert_eq!(
            decide(&results, 2, Some(&[GATING_LAYER.to_owned()])),
            Decision::Published {
                prefix: 1,
                mark: false
            }
        );
    }

    #[test]
    fn missing_gating_layer_is_not_published() {
        let results = vec![hour(1, &[GATING_LAYER])];
        assert_eq!(
            decide(&results, 1, Some(&["other".to_owned()])),
            Decision::NotPublished
        );
    }

    #[test]
    fn constants_match_the_spec() {
        assert_eq!(FORECAST_CONCURRENCY, 4);
        assert_eq!(HRRR_PROCESS_FORECAST_HOUR.start_to_close_s, 480);
        assert_eq!(HRRR_PROCESS_FORECAST_HOUR.schedule_to_close_s, Some(720));
        assert_eq!(HRRR_PROCESS_FORECAST_HOUR.heartbeat_s, Some(90));
        assert_eq!(HRRR_PROCESS_FORECAST_HOUR.retry.max_attempts, 2);
        assert!(
            HRRR_PROCESS_FORECAST_HOUR.start_to_close_s
                < HRRR_PROCESS_FORECAST_HOUR.schedule_to_close_s.unwrap()
        );
    }
}
