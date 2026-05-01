"""IngestMrmsWorkflow — replaces the legacy `ingest-mrms` CronJob.

Schedule: every 2 minutes, OverlapPolicy.SKIP (a slow run does not queue),
CatchupWindow=1h (no thundering-herd backfill on worker recovery).

Pipeline per run:
  1. List unprocessed MRMS keys (newest-first, capped at BACKLOG_PER_CYCLE)
  2. For each key: download + decode + render tiles + write grid + detect storms
  3. Mark each successful frame in the on-disk ProcessedSet so a restart
     does not re-render
  4. Sweep tiles + grids older than retention
"""

from __future__ import annotations

from dataclasses import dataclass
from datetime import timedelta

from temporalio import workflow
from temporalio.common import RetryPolicy

with workflow.unsafe.imports_passed_through():
    from backend.ingest_mrms.activities import (
        CleanupResult,
        ListKeysResult,
        ProcessFrameResult,
        mrms_cleanup,
        mrms_list_unprocessed_keys,
        mrms_mark_processed,
        mrms_process_frame,
    )


RETENTION_HOURS = 4


_DEFAULT_RETRY = RetryPolicy(
    initial_interval=timedelta(seconds=1),
    backoff_coefficient=2.0,
    maximum_interval=timedelta(seconds=100),
    maximum_attempts=5,
)

_FRAME_RETRY = RetryPolicy(
    initial_interval=timedelta(seconds=2),
    backoff_coefficient=2.0,
    maximum_interval=timedelta(seconds=60),
    maximum_attempts=3,
)


@dataclass
class IngestMrmsResult:
    backlog_total: int
    rendered_count: int
    cleanup: CleanupResult | None


@workflow.defn(name="IngestMrmsWorkflow")
class IngestMrmsWorkflow:
    @workflow.run
    async def run(self) -> IngestMrmsResult:
        listing: ListKeysResult = await workflow.execute_activity(
            mrms_list_unprocessed_keys,
            start_to_close_timeout=timedelta(seconds=60),
            retry_policy=_DEFAULT_RETRY,
        )

        if not listing.keys:
            workflow.logger.info("no new MRMS frames; backlog=%d", listing.backlog_total)
            cleanup = await self._cleanup()
            return IngestMrmsResult(backlog_total=listing.backlog_total, rendered_count=0, cleanup=cleanup)

        rendered = 0
        for key in listing.keys:
            result: ProcessFrameResult = await workflow.execute_activity(
                mrms_process_frame,
                key,
                start_to_close_timeout=timedelta(minutes=10),
                heartbeat_timeout=timedelta(seconds=90),
                retry_policy=_FRAME_RETRY,
            )
            if result.rendered:
                await workflow.execute_activity(
                    mrms_mark_processed,
                    key,
                    start_to_close_timeout=timedelta(seconds=30),
                    retry_policy=_DEFAULT_RETRY,
                )
                rendered += 1
            else:
                workflow.logger.warning("frame skipped: %s", key)

        cleanup = await self._cleanup()
        return IngestMrmsResult(
            backlog_total=listing.backlog_total,
            rendered_count=rendered,
            cleanup=cleanup,
        )

    async def _cleanup(self) -> CleanupResult:
        return await workflow.execute_activity(
            mrms_cleanup,
            RETENTION_HOURS,
            start_to_close_timeout=timedelta(minutes=5),
            retry_policy=_DEFAULT_RETRY,
        )
