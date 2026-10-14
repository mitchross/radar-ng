//! MRMS ingest activities. Names and payloads are fixed by spec section 4.2 and `radar_types::wire`.

use std::sync::Arc;

use radar_ingest::{IngestCtx, mrms};
use radar_types::payloads::mrms::{
    CleanupInput, CleanupResult, IngestMrmsArgs, ListKeysResult, MarkProcessedInput,
    ProcessFrameInput, ProcessFrameResult,
};
use temporalio_macros::activities;
use temporalio_sdk::activities::{ActivityContext, ActivityError};

use crate::support::{activity_info, json_heartbeat, with_cancel};

/// Activity implementer for the MRMS pool (`mrms` role, and `legacy`/`all`).
pub struct MrmsActivities {
    ingest: IngestCtx,
}

impl MrmsActivities {
    /// Wrap a shared ingest context.
    pub fn new(ingest: IngestCtx) -> Self {
        Self { ingest }
    }
}

#[activities]
impl MrmsActivities {
    /// `mrms_list_unprocessed_keys`: newest-first keys still needing a render, capped per cycle.
    #[activity(name = "mrms_list_unprocessed_keys")]
    pub async fn list_unprocessed_keys(
        self: Arc<Self>,
        ctx: ActivityContext,
        args: IngestMrmsArgs,
    ) -> Result<ListKeysResult, ActivityError> {
        with_cancel(&ctx, mrms::list_unprocessed_keys(&self.ingest, &args)).await
    }

    /// `mrms_process_frame`: download, render and publish one frame. Heartbeats while working.
    #[activity(name = "mrms_process_frame")]
    pub async fn process_frame(
        self: Arc<Self>,
        ctx: ActivityContext,
        input: ProcessFrameInput,
    ) -> Result<ProcessFrameResult, ActivityError> {
        let info = activity_info(&ctx);
        let beat = json_heartbeat(&ctx);
        with_cancel(
            &ctx,
            mrms::process_frame(&self.ingest, &info, &input, &beat),
        )
        .await
    }

    /// `mrms_mark_processed`: record a rendered key in the processed set.
    #[activity(name = "mrms_mark_processed")]
    pub async fn mark_processed(
        self: Arc<Self>,
        ctx: ActivityContext,
        input: MarkProcessedInput,
    ) -> Result<(), ActivityError> {
        with_cancel(&ctx, mrms::mark_processed(&self.ingest, &input)).await
    }

    /// `mrms_cleanup`: remove tiles and grids older than the retention window.
    #[activity(name = "mrms_cleanup")]
    pub async fn cleanup(
        self: Arc<Self>,
        ctx: ActivityContext,
        input: CleanupInput,
    ) -> Result<CleanupResult, ActivityError> {
        with_cancel(&ctx, mrms::cleanup(&self.ingest, &input)).await
    }
}
