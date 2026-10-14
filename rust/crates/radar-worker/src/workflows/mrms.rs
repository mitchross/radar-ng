//! `IngestMrmsWorkflow` (spec 4.2). Schedules `ingest-mrms-base` and `ingest-mrms-composite`.

use radar_types::payloads::mrms::{
    CleanupInput, CleanupResult, IngestMrmsArgs, IngestMrmsResult, MarkProcessedInput,
    ProcessFrameInput, ProcessFrameResult, RETENTION_HOURS,
};
use radar_types::retry::{
    MRMS_CLEANUP, MRMS_LIST_UNPROCESSED_KEYS, MRMS_MARK_PROCESSED, MRMS_PROCESS_FRAME,
};
use temporalio_macros::{workflow, workflow_methods};
use temporalio_sdk::{WorkflowContext, WorkflowResult};

use super::{OptionalArg, bounded, options};
use crate::activities::mrms::MrmsActivities;

/// Concurrent frame renders per run (Python `Semaphore(2)`).
pub const FRAME_CONCURRENCY: usize = 2;

/// Placeholder for a frame whose activity failed. It is not marked processed, so the next run retries it.
pub fn unrendered(key: String) -> ProcessFrameResult {
    ProcessFrameResult {
        key,
        timestamp: String::new(),
        rendered: false,
        palettes: Vec::new(),
        duration_s: 0.0,
    }
}

/// Keys to mark processed, in result order (only rendered frames).
pub fn keys_to_mark(results: &[ProcessFrameResult]) -> Vec<String> {
    results
        .iter()
        .filter(|r| r.rendered)
        .map(|r| r.key.clone())
        .collect()
}

/// Workflow type `IngestMrmsWorkflow`.
#[workflow]
#[derive(Default)]
pub struct IngestMrmsWorkflow;

#[workflow_methods]
impl IngestMrmsWorkflow {
    /// List pending keys, render them with bounded concurrency, mark the rendered ones in key order,
    /// then clean up. Only a failed frame is swallowed; list, mark and cleanup failures fail the run.
    #[run]
    pub async fn run(
        ctx: &mut WorkflowContext<Self>,
        input: OptionalArg<IngestMrmsArgs>,
    ) -> WorkflowResult<IngestMrmsResult> {
        // Python: `args = args or IngestMrmsArgs()`. Schedules without input start with no payload at all.
        let args = input.0.unwrap_or_default();
        let layer = args.layer_name.clone();
        let listing = ctx
            .execute_activity(
                MrmsActivities::list_unprocessed_keys,
                args,
                options(&MRMS_LIST_UNPROCESSED_KEYS),
            )
            .await?;

        if listing.keys.is_empty() {
            let cleanup = cleanup(ctx, &layer).await?;
            return Ok(IngestMrmsResult {
                layer,
                backlog_total: listing.backlog_total,
                rendered_count: 0,
                cleanup: Some(cleanup),
            });
        }

        let ctx_ref: &WorkflowContext<Self> = ctx;
        let frame_layer = layer.clone();
        let results = bounded(listing.keys, FRAME_CONCURRENCY, |key: String| {
            let layer = frame_layer.clone();
            async move { process_key(ctx_ref, key, layer).await }
        })
        .await;

        for key in keys_to_mark(&results) {
            ctx.execute_activity(
                MrmsActivities::mark_processed,
                MarkProcessedInput {
                    key,
                    layer_name: layer.clone(),
                },
                options(&MRMS_MARK_PROCESSED),
            )
            .await?;
        }

        let rendered_count = results.iter().filter(|r| r.rendered).count() as i64;
        let cleanup = cleanup(ctx, &layer).await?;
        Ok(IngestMrmsResult {
            layer,
            backlog_total: listing.backlog_total,
            rendered_count,
            cleanup: Some(cleanup),
        })
    }
}

async fn process_key(
    ctx: &WorkflowContext<IngestMrmsWorkflow>,
    key: String,
    layer: String,
) -> ProcessFrameResult {
    let input = ProcessFrameInput {
        key: key.clone(),
        layer_name: layer,
    };
    match ctx
        .execute_activity(
            MrmsActivities::process_frame,
            input,
            options(&MRMS_PROCESS_FRAME),
        )
        .await
    {
        Ok(result) => result,
        Err(err) => {
            tracing::warn!(key = %key, error = %err, "mrms frame failed; left unmarked");
            unrendered(key)
        }
    }
}

async fn cleanup(
    ctx: &WorkflowContext<IngestMrmsWorkflow>,
    layer: &str,
) -> WorkflowResult<CleanupResult> {
    let input = CleanupInput {
        layer_name: layer.to_owned(),
        retention_hours: RETENTION_HOURS,
    };
    Ok(ctx
        .execute_activity(MrmsActivities::cleanup, input, options(&MRMS_CLEANUP))
        .await?)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn result(key: &str, rendered: bool) -> ProcessFrameResult {
        ProcessFrameResult {
            key: key.into(),
            timestamp: "t".into(),
            rendered,
            palettes: vec![],
            duration_s: 1.0,
        }
    }

    #[test]
    fn only_rendered_frames_are_marked_in_key_order() {
        let results = vec![result("a", true), result("b", false), result("c", true)];
        assert_eq!(
            keys_to_mark(&results),
            vec!["a".to_string(), "c".to_string()]
        );
    }

    #[test]
    fn failed_frame_placeholder_is_unrendered() {
        let placeholder = unrendered("k".into());
        assert!(!placeholder.rendered);
        assert_eq!(placeholder.key, "k");
    }
}
