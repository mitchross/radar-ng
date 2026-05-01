"""OpenMeteoSyncWorkflow — replaces the two open-meteo-sync CronJobs.

Two schedules drive this workflow:
  - `open-meteo-sync-gfs`  every 6h at :30  (matches GFS run cadence)
  - `open-meteo-sync-hrrr` every 1h at :45  (matches HRRR run cadence)

The workflow itself is a single activity that creates a Kubernetes Job
running the upstream `ghcr.io/open-meteo/open-meteo:latest` image, then
polls until the Job terminates. Worker pod needs RBAC to create + watch
jobs in the radar-ng namespace.
"""

from __future__ import annotations

from datetime import timedelta

from temporalio import workflow
from temporalio.common import RetryPolicy

with workflow.unsafe.imports_passed_through():
    from backend.open_meteo_sync.activities import (
        OpenMeteoSyncArgs,
        OpenMeteoSyncResult,
        open_meteo_sync_via_k8s_job,
    )


_RETRY = RetryPolicy(
    initial_interval=timedelta(seconds=10),
    backoff_coefficient=2.0,
    maximum_interval=timedelta(minutes=5),
    maximum_attempts=3,
)


@workflow.defn(name="OpenMeteoSyncWorkflow")
class OpenMeteoSyncWorkflow:
    @workflow.run
    async def run(self, args: OpenMeteoSyncArgs) -> OpenMeteoSyncResult:
        return await workflow.execute_activity(
            open_meteo_sync_via_k8s_job, args,
            start_to_close_timeout=timedelta(minutes=35),
            heartbeat_timeout=timedelta(seconds=90),
            retry_policy=_RETRY,
        )
