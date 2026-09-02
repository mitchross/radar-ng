"""NowcastWorkflow — replaces the legacy `nowcast` CronJob.

Schedule: every 2 minutes (matches the MRMS cadence so the freshest grid
gets a forecast immediately). OverlapPolicy.SKIP guarantees a slow pysteps
run isn't compounded by a parallel run.

Single CPU-heavy activity: load grids → optical flow + S-PROG → render.
One attempt only: the run is deterministic for its input, and a retry of a
~9-min job would just pin the schedule while fresher grids wait.
"""

from __future__ import annotations

from datetime import timedelta

from temporalio import workflow
from temporalio.common import RetryPolicy

with workflow.unsafe.imports_passed_through():
    from backend.nowcast.activities import NowcastResult, nowcast_run


_RETRY = RetryPolicy(maximum_attempts=1)


@workflow.defn(name="NowcastWorkflow")
class NowcastWorkflow:
    @workflow.run
    async def run(self) -> NowcastResult:
        return await workflow.execute_activity(
            nowcast_run,
            # A run takes ~9 min; the 12-min ceiling matches the schedule's
            # max_runtime so a stuck run frees the slot for the next fire.
            start_to_close_timeout=timedelta(minutes=10),
            schedule_to_close_timeout=timedelta(minutes=12),
            # A single leadtime's tile-pyramid render can exceed 60 s on NFS;
            # the activity heartbeats between leadtimes, so 120 s tolerates that.
            heartbeat_timeout=timedelta(seconds=120),
            retry_policy=_RETRY,
        )
