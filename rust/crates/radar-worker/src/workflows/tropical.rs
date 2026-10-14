//! `IngestTropicalWorkflow` (spec 4.6). Schedule `ingest-tropical`.

use radar_types::payloads::tropical::TropicalResult;
use radar_types::retry::TROPICAL_FETCH_AND_PUBLISH;
use temporalio_macros::{workflow, workflow_methods};
use temporalio_sdk::{WorkflowContext, WorkflowResult};

use super::options;
use crate::activities::aux::AuxActivities;

/// Workflow type `IngestTropicalWorkflow`. One activity call.
#[workflow]
#[derive(Default)]
pub struct IngestTropicalWorkflow;

#[workflow_methods]
impl IngestTropicalWorkflow {
    /// Fetch and publish the storm feed.
    #[run]
    pub async fn run(ctx: &mut WorkflowContext<Self>) -> WorkflowResult<TropicalResult> {
        let result = ctx
            .execute_activity(
                AuxActivities::fetch_and_publish,
                (),
                options(&TROPICAL_FETCH_AND_PUBLISH),
            )
            .await?;
        Ok(result)
    }
}
