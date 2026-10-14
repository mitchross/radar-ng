//! `TileCleanupWorkflow` (spec 4.8). Schedule `tile-cleanup`.

use radar_types::payloads::cleanup::TileCleanupResult;
use radar_types::retry::TILE_CLEANUP_SWEEP;
use temporalio_macros::{workflow, workflow_methods};
use temporalio_sdk::{WorkflowContext, WorkflowResult};

use super::options;
use crate::activities::aux::AuxActivities;

/// Workflow type `TileCleanupWorkflow`. One activity call.
#[workflow]
#[derive(Default)]
pub struct TileCleanupWorkflow;

#[workflow_methods]
impl TileCleanupWorkflow {
    /// Sweep expired tiles and grids.
    #[run]
    pub async fn run(ctx: &mut WorkflowContext<Self>) -> WorkflowResult<TileCleanupResult> {
        let result = ctx
            .execute_activity(
                AuxActivities::tile_cleanup,
                (),
                options(&TILE_CLEANUP_SWEEP),
            )
            .await?;
        Ok(result)
    }
}
