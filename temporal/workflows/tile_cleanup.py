"""TileCleanupWorkflow — replaces the legacy `tile-cleanup` CronJob.

Schedule: every 1 hour. Sweeps timestamp subtrees older than the per-layer
retention across every layer. Cheap — just stat + rmtree.
"""

from __future__ import annotations

from datetime import timedelta

from temporalio import workflow
from temporalio.common import RetryPolicy

with workflow.unsafe.imports_passed_through():
    from backend.tile_cleanup.activities import (
        TileCleanupResult,
        tile_cleanup_sweep,
    )


_RETRY = RetryPolicy(
    initial_interval=timedelta(seconds=2),
    backoff_coefficient=2.0,
    maximum_interval=timedelta(seconds=60),
    maximum_attempts=3,
)


@workflow.defn(name="TileCleanupWorkflow")
class TileCleanupWorkflow:
    @workflow.run
    async def run(self) -> TileCleanupResult:
        return await workflow.execute_activity(
            tile_cleanup_sweep,
            start_to_close_timeout=timedelta(minutes=5),
            retry_policy=_RETRY,
        )
