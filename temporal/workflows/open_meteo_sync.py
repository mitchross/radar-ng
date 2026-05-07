"""OpenMeteoSyncWorkflow — replaces the two open-meteo-sync CronJobs.

Two schedules drive this workflow:
  - `open-meteo-sync-gfs`  every 6h   (matches GFS run cadence)
  - `open-meteo-sync-hrrr` every 1h   (matches HRRR run cadence)

The workflow body is a single activity (`open_meteo_sync`) that
subprocess-execs the open-meteo Swift binary inside the dedicated
`radar-ng-open-meteo-worker` pod. No k8s Jobs are created — the
separate worker pool replaces the earlier short-lived-Job pattern.
"""

from __future__ import annotations

from datetime import timedelta

from temporalio import workflow
from temporalio.common import RetryPolicy

with workflow.unsafe.imports_passed_through():
    from backend.open_meteo_sync.activities import (
        OpenMeteoSyncArgs,
        OpenMeteoSyncResult,
        open_meteo_sync,
    )


_RETRY = RetryPolicy(
    initial_interval=timedelta(seconds=10),
    backoff_coefficient=2.0,
    maximum_interval=timedelta(minutes=5),
    maximum_attempts=3,
)

OPEN_METEO_TASK_QUEUE = "radar-ng-open-meteo"


@workflow.defn(name="OpenMeteoSyncWorkflow")
class OpenMeteoSyncWorkflow:
    @workflow.run
    async def run(self, args: OpenMeteoSyncArgs) -> OpenMeteoSyncResult:
        return await workflow.execute_activity(
            open_meteo_sync, args,
            task_queue=OPEN_METEO_TASK_QUEUE,
            start_to_close_timeout=timedelta(minutes=35),
            heartbeat_timeout=timedelta(seconds=90),
            retry_policy=_RETRY,
        )
