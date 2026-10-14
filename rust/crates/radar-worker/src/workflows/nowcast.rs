//! `NowcastWorkflow` (spec 4.7). Schedule `nowcast`, and the MRMS kick (spec 5.3).

use radar_types::payloads::nowcast::NowcastResult;
use radar_types::retry::NOWCAST_RUN;
use temporalio_macros::{workflow, workflow_methods};
use temporalio_sdk::{WorkflowContext, WorkflowResult};

use super::options;
use crate::activities::nowcast::NowcastActivities;

/// Workflow type `NowcastWorkflow`. A single attempt is deliberate (spec 4.7).
#[workflow]
#[derive(Default)]
pub struct NowcastWorkflow;

#[workflow_methods]
impl NowcastWorkflow {
    /// Run the nowcast once.
    #[run]
    pub async fn run(ctx: &mut WorkflowContext<Self>) -> WorkflowResult<NowcastResult> {
        let result = ctx
            .execute_activity(NowcastActivities::run, (), options(&NOWCAST_RUN))
            .await?;
        Ok(result)
    }
}
