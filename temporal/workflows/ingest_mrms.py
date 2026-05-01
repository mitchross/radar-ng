"""IngestMrmsWorkflow — replaces the legacy `ingest-mrms` CronJob AND
the legacy `ingest-radar-composite` Deployment.

Same workflow, two schedules:
  - `ingest-mrms-base` → MergedBaseReflectivityQC_00.50, layer `radar`
  - `ingest-mrms-composite` → MergedReflectivityComposite_00.50, layer `radar-composite`

Schedule fires every 2 minutes. OverlapPolicy.SKIP guards against pile-up.
ProcessedSet state is per-layer so the two schedules don't race.

Pipeline per run:
  1. List unprocessed MRMS keys (newest-first, capped at BACKLOG_PER_CYCLE)
  2. For each key: download + decode + render tiles + write grid + detect storms
  3. Mark each successful frame in the on-disk ProcessedSet
  4. Sweep tiles + grids older than retention
"""

from __future__ import annotations

from dataclasses import dataclass
from datetime import timedelta

from temporalio import workflow
from temporalio.common import RetryPolicy

with workflow.unsafe.imports_passed_through():
    from backend.ingest_mrms.activities import (
        CleanupInput,
        CleanupResult,
        IngestMrmsArgs,
        ListKeysResult,
        MarkProcessedInput,
        ProcessFrameInput,
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
    layer: str
    backlog_total: int
    rendered_count: int
    cleanup: CleanupResult | None = None


@workflow.defn(name="IngestMrmsWorkflow")
class IngestMrmsWorkflow:
    @workflow.run
    async def run(self, args: IngestMrmsArgs | None = None) -> IngestMrmsResult:
        # Schedules without input land here with args=None — fall back to
        # defaults (the QC-applied base reflectivity).
        args = args or IngestMrmsArgs()
        listing: ListKeysResult = await workflow.execute_activity(
            mrms_list_unprocessed_keys, args,
            start_to_close_timeout=timedelta(seconds=60),
            retry_policy=_DEFAULT_RETRY,
        )

        if not listing.keys:
            workflow.logger.info("no new %s frames; backlog=%d", args.layer_name, listing.backlog_total)
            cleanup = await self._cleanup(args.layer_name)
            return IngestMrmsResult(
                layer=args.layer_name, backlog_total=listing.backlog_total,
                rendered_count=0, cleanup=cleanup,
            )

        rendered = 0
        for key in listing.keys:
            r: ProcessFrameResult = await workflow.execute_activity(
                mrms_process_frame, ProcessFrameInput(key=key, layer_name=args.layer_name),
                start_to_close_timeout=timedelta(minutes=10),
                heartbeat_timeout=timedelta(seconds=90),
                retry_policy=_FRAME_RETRY,
            )
            if r.rendered:
                await workflow.execute_activity(
                    mrms_mark_processed, MarkProcessedInput(key=key, layer_name=args.layer_name),
                    start_to_close_timeout=timedelta(seconds=30),
                    retry_policy=_DEFAULT_RETRY,
                )
                rendered += 1
            else:
                workflow.logger.warning("frame skipped: %s", key)

        cleanup = await self._cleanup(args.layer_name)
        return IngestMrmsResult(
            layer=args.layer_name,
            backlog_total=listing.backlog_total,
            rendered_count=rendered,
            cleanup=cleanup,
        )

    async def _cleanup(self, layer_name: str) -> CleanupResult:
        return await workflow.execute_activity(
            mrms_cleanup, CleanupInput(layer_name=layer_name, retention_hours=RETENTION_HOURS),
            start_to_close_timeout=timedelta(minutes=5),
            retry_policy=_DEFAULT_RETRY,
        )
