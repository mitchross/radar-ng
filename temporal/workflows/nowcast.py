"""NowcastWorkflow — replaces the legacy `nowcast` CronJob.

Schedule: every 2 minutes (matches the MRMS cadence so the freshest grid
gets a forecast immediately). OverlapPolicy.SKIP guarantees a slow pysteps
run isn't compounded by a parallel run.

Single CPU-heavy activity: load grids → optical flow + S-PROG → render.
Per the spec: 2 attempts max (deterministic for the same input — retrying
won't change the result if the first try failed for code reasons).
"""

from __future__ import annotations

from datetime import timedelta

from temporalio import workflow
from temporalio.common import RetryPolicy

with workflow.unsafe.imports_passed_through():
    from backend.nowcast.activities import NowcastResult, nowcast_run


_RETRY = RetryPolicy(
    initial_interval=timedelta(seconds=5),
    backoff_coefficient=2.0,
    maximum_interval=timedelta(seconds=60),
    maximum_attempts=2,
)


@workflow.defn(name="NowcastWorkflow")
class NowcastWorkflow:
    @workflow.run
    async def run(self) -> NowcastResult:
        return await workflow.execute_activity(
            nowcast_run,
            start_to_close_timeout=timedelta(minutes=15),
            heartbeat_timeout=timedelta(seconds=120),
            retry_policy=_RETRY,
        )
