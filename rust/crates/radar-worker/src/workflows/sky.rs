//! `IngestSkyWorkflow` (design `weather-extras.md` section 7). Schedule `ingest-iss` passes `"iss"`.
//!
//! The argument is the product, so later sky products run on the same workflow type.

use radar_types::payloads::sky::SkyResult;
use radar_types::retry::SKY_FETCH_AND_PUBLISH;
use temporalio_macros::{workflow, workflow_methods};
use temporalio_sdk::{WorkflowContext, WorkflowResult};

use super::options;
use crate::activities::aux::AuxActivities;

/// Workflow type `IngestSkyWorkflow`. One activity call, one attempt.
#[workflow]
#[derive(Default)]
pub struct IngestSkyWorkflow;

#[workflow_methods]
impl IngestSkyWorkflow {
    /// Fetch and publish one sky product.
    #[run]
    pub async fn run(
        ctx: &mut WorkflowContext<Self>,
        product: String,
    ) -> WorkflowResult<SkyResult> {
        let result = ctx
            .execute_activity(
                AuxActivities::sky_fetch_and_publish,
                product,
                options(&SKY_FETCH_AND_PUBLISH),
            )
            .await?;
        Ok(result)
    }
}
