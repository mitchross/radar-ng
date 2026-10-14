//! `RegisterPushTokenWorkflow` and `DeletePushTokenWorkflow` (spec 4.12). Compatibility only: the API no
//! longer starts them, but retained histories must still replay.

use radar_types::payloads::push::{PushTokenInput, RegisterPushTokenInput};
use radar_types::retry::{DELETE_PUSH_TOKEN, PERSIST_PUSH_TOKEN};
use temporalio_macros::{workflow, workflow_methods};
use temporalio_sdk::{WorkflowContext, WorkflowResult};

use super::options;
use crate::activities::alerts::AlertsActivities;

/// Workflow type `RegisterPushTokenWorkflow`. Output is `null`.
#[workflow]
#[derive(Default)]
pub struct RegisterPushTokenWorkflow;

#[workflow_methods]
impl RegisterPushTokenWorkflow {
    /// Persist one device token.
    #[run]
    pub async fn run(
        ctx: &mut WorkflowContext<Self>,
        input: RegisterPushTokenInput,
    ) -> WorkflowResult<()> {
        let args = PushTokenInput {
            user_id: input.user_id,
            token: input.token,
            platform: input.platform,
        };
        ctx.execute_activity(
            AlertsActivities::persist_push_token,
            args,
            options(&PERSIST_PUSH_TOKEN),
        )
        .await?;
        Ok(())
    }
}

/// Workflow type `DeletePushTokenWorkflow`. Output is the number of rows deleted.
#[workflow]
#[derive(Default)]
pub struct DeletePushTokenWorkflow;

#[workflow_methods]
impl DeletePushTokenWorkflow {
    /// Delete one device token by value.
    #[run]
    pub async fn run(ctx: &mut WorkflowContext<Self>, token: String) -> WorkflowResult<usize> {
        let deleted = ctx
            .execute_activity(
                AlertsActivities::delete_push_token,
                token,
                options(&DELETE_PUSH_TOKEN),
            )
            .await?;
        Ok(deleted)
    }
}
