//! `RadarDeploymentSmokeWorkflow` (spec 4.1). Registered on every role; run by the controller before promotion.

use radar_types::payloads::smoke::DeploymentSmokeResult;
use radar_types::retry::RADAR_DEPLOYMENT_SMOKE;
use temporalio_macros::{workflow, workflow_methods};
use temporalio_sdk::{WorkflowContext, WorkflowResult};

use super::options;
use crate::activities::smoke::SmokeActivities;

/// Workflow type `RadarDeploymentSmokeWorkflow`. No input, no signals, no timers.
#[workflow]
#[derive(Default)]
pub struct RadarDeploymentSmokeWorkflow;

#[workflow_methods]
impl RadarDeploymentSmokeWorkflow {
    /// Run the smoke activity once and return its `{ok, role}` result.
    #[run]
    pub async fn run(ctx: &mut WorkflowContext<Self>) -> WorkflowResult<DeploymentSmokeResult> {
        let result = ctx
            .execute_activity(
                SmokeActivities::radar_deployment_smoke,
                (),
                options(&RADAR_DEPLOYMENT_SMOKE),
            )
            .await?;
        Ok(result)
    }
}
