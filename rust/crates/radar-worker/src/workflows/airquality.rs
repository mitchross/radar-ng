//! `IngestAirQualityWorkflow` (spec 4.4). Schedule `ingest-airquality`.

use radar_types::payloads::airquality::{
    AQM_LAYERS, AqmChunkResult, AqmCleanupResult, CHUNK_START_MSGS, IngestAirQualityResult,
    RETENTION_HOURS,
};
use radar_types::retry::{
    AQM_CLEANUP, AQM_FIND_LATEST_RUN, AQM_MARK_PROCESSED, AQM_PUBLISH_RUN, AQM_RENDER_CHUNK,
};
use temporalio_macros::{workflow, workflow_methods};
use temporalio_sdk::{WorkflowContext, WorkflowResult};

use super::{bounded, options};
use crate::activities::aqm::AqmActivities;

/// Concurrent chunk renders (Python `Semaphore(2)`).
pub const CHUNK_CONCURRENCY: usize = 2;
/// Layer whose publication gates the processed mark.
pub const GATING_LAYER: &str = "air-quality";

/// Chunk calls in the layer-major order the replay contract requires (spec 4.4, 10.2).
pub fn chunk_plan() -> Vec<(String, i64)> {
    AQM_LAYERS
        .iter()
        .flat_map(|layer| {
            CHUNK_START_MSGS
                .iter()
                .map(move |start| ((*layer).to_owned(), *start))
        })
        .collect()
}

/// Total frames rendered across all chunk results.
pub fn frames_rendered(results: &[AqmChunkResult]) -> i64 {
    results
        .iter()
        .map(|r| r.rendered_timestamps.len() as i64)
        .sum()
}

/// Whether the processed mark follows this publish result.
pub fn should_mark(published: &[String]) -> bool {
    published.iter().any(|l| l == GATING_LAYER)
}

/// Workflow type `IngestAirQualityWorkflow`.
#[workflow]
#[derive(Default)]
pub struct IngestAirQualityWorkflow;

#[workflow_methods]
impl IngestAirQualityWorkflow {
    /// Find the run, render six chunks with bounded concurrency, publish all results, and mark the run
    /// processed only when the `air-quality` layer was published.
    #[run]
    pub async fn run(ctx: &mut WorkflowContext<Self>) -> WorkflowResult<IngestAirQualityResult> {
        let find = ctx
            .execute_activity(
                AqmActivities::find_latest_run,
                (),
                options(&AQM_FIND_LATEST_RUN),
            )
            .await?;
        let Some(run_id) = find.run_id.clone() else {
            let cleanup = cleanup(ctx).await?;
            return Ok(report(None, false, vec![], 0, Some(cleanup)));
        };
        if find.already_processed {
            let cleanup = cleanup(ctx).await?;
            return Ok(report(Some(run_id), true, vec![], 0, Some(cleanup)));
        }

        let ctx_ref: &WorkflowContext<Self> = ctx;
        let chunk_run = run_id.clone();
        let results: Vec<AqmChunkResult> = bounded(chunk_plan(), CHUNK_CONCURRENCY, |(layer, start): (String, i64)| {
            let run = chunk_run.clone();
            async move {
                match ctx_ref
                    .execute_activity(
                        AqmActivities::render_chunk,
                        (run, layer.clone(), start),
                        options(&AQM_RENDER_CHUNK),
                    )
                    .await
                {
                    Ok(result) => result,
                    Err(err) => {
                        tracing::warn!(layer = %layer, start, error = %err, "aqm chunk failed; continuing");
                        AqmChunkResult::failed(layer, start)
                    }
                }
            }
        })
        .await;

        let frames = frames_rendered(&results);
        let published = ctx
            .execute_activity(
                AqmActivities::publish_run,
                (run_id.clone(), results),
                options(&AQM_PUBLISH_RUN),
            )
            .await?;

        if should_mark(&published) {
            ctx.execute_activity(
                AqmActivities::mark_processed,
                run_id.clone(),
                options(&AQM_MARK_PROCESSED),
            )
            .await?;
        } else {
            tracing::error!(run_id = %run_id, "air-quality layer not published; run stays unprocessed");
        }

        let cleanup = cleanup(ctx).await?;
        Ok(report(
            Some(run_id),
            false,
            published,
            frames,
            Some(cleanup),
        ))
    }
}

fn report(
    run_id: Option<String>,
    skipped: bool,
    published_layers: Vec<String>,
    frames_rendered: i64,
    cleanup: Option<AqmCleanupResult>,
) -> IngestAirQualityResult {
    IngestAirQualityResult {
        run_id,
        skipped_already_processed: skipped,
        published_layers,
        frames_rendered,
        cleanup,
    }
}

async fn cleanup(
    ctx: &WorkflowContext<IngestAirQualityWorkflow>,
) -> WorkflowResult<AqmCleanupResult> {
    Ok(ctx
        .execute_activity(
            AqmActivities::cleanup,
            RETENTION_HOURS,
            options(&AQM_CLEANUP),
        )
        .await?)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn chunk_plan_is_layer_major_with_three_starts() {
        let plan = chunk_plan();
        assert_eq!(plan.len(), 6);
        assert_eq!(plan[0], ("air-quality".to_string(), 0));
        assert_eq!(plan[2], ("air-quality".to_string(), 48));
        assert_eq!(plan[3], ("ozone".to_string(), 0));
        assert_eq!(plan[5], ("ozone".to_string(), 48));
    }

    #[test]
    fn only_air_quality_gates_the_processed_mark() {
        assert!(should_mark(&["air-quality".to_string()]));
        assert!(!should_mark(&["ozone".to_string()]));
    }

    #[test]
    fn frames_are_summed_across_chunks() {
        let results = vec![
            AqmChunkResult {
                layer: "a".into(),
                start_msg: 0,
                rendered_timestamps: vec!["x".into(), "y".into()],
            },
            AqmChunkResult::failed("a", 24),
        ];
        assert_eq!(frames_rendered(&results), 2);
    }
}
