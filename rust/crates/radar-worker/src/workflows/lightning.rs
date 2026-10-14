//! `IngestLightningWorkflow` (spec 4.5). Schedule `ingest-lightning`.

use radar_types::payloads::lightning::{LightningRunResult, RUN_DURATION_S};
use radar_types::retry::LIGHTNING_CONSUME_STREAM;
use temporalio_macros::{workflow, workflow_methods};
use temporalio_sdk::{WorkflowContext, WorkflowResult};

use super::options;
use crate::activities::aux::AuxActivities;

/// Workflow type `IngestLightningWorkflow`. One activity call for the hour-long stream.
#[workflow]
#[derive(Default)]
pub struct IngestLightningWorkflow;

#[workflow_methods]
impl IngestLightningWorkflow {
    /// Consume the strike stream for `RUN_DURATION_S` seconds.
    #[run]
    pub async fn run(ctx: &mut WorkflowContext<Self>) -> WorkflowResult<LightningRunResult> {
        let result = ctx
            .execute_activity(
                AuxActivities::consume_stream,
                RUN_DURATION_S,
                options(&LIGHTNING_CONSUME_STREAM),
            )
            .await?;
        Ok(result)
    }
}
