//! `EnsureStaticDataWorkflow` (design `weather-extras.md` section 6). Schedule `ensure-static-data`
//! passes `"normals"`.
//!
//! One activity call. The activity checks the product's marker, so a daily run costs one `stat` once the
//! data exists.

use radar_types::payloads::static_data::StaticDataResult;
use radar_types::retry::STATIC_DATA_ENSURE;
use temporalio_macros::{workflow, workflow_methods};
use temporalio_sdk::{WorkflowContext, WorkflowResult};

use super::options;
use crate::activities::aux::AuxActivities;

/// Workflow type `EnsureStaticDataWorkflow`.
#[workflow]
#[derive(Default)]
pub struct EnsureStaticDataWorkflow;

#[workflow_methods]
impl EnsureStaticDataWorkflow {
    /// Build one static-data product if it is missing.
    #[run]
    pub async fn run(
        ctx: &mut WorkflowContext<Self>,
        product: String,
    ) -> WorkflowResult<StaticDataResult> {
        let result = ctx
            .execute_activity(
                AuxActivities::static_data_ensure,
                product,
                options(&STATIC_DATA_ENSURE),
            )
            .await?;
        Ok(result)
    }
}
