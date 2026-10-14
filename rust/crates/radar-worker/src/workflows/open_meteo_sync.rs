//! `OpenMeteoSyncWorkflow` (spec 4.9). Schedules `open-meteo-sync-*`. The activity runs on the open-meteo queue.

use radar_types::payloads::open_meteo::{OpenMeteoSyncArgs, OpenMeteoSyncResult};
use radar_types::retry::OPEN_METEO_SYNC;
use radar_types::wire::OPEN_METEO_TASK_QUEUE;
use temporalio_macros::{workflow, workflow_methods};
use temporalio_sdk::{WorkflowContext, WorkflowResult};

use super::options_on_queue;
use crate::activities::open_meteo::OpenMeteoActivities;

/// Workflow type `OpenMeteoSyncWorkflow`.
#[workflow]
#[derive(Default)]
pub struct OpenMeteoSyncWorkflow;

#[workflow_methods]
impl OpenMeteoSyncWorkflow {
    /// Run the sync subprocess on the open-meteo pool.
    #[run]
    pub async fn run(
        ctx: &mut WorkflowContext<Self>,
        args: OpenMeteoSyncArgs,
    ) -> WorkflowResult<OpenMeteoSyncResult> {
        let result = ctx
            .execute_activity(
                OpenMeteoActivities::sync,
                args,
                options_on_queue(&OPEN_METEO_SYNC, OPEN_METEO_TASK_QUEUE),
            )
            .await?;
        Ok(result)
    }
}
