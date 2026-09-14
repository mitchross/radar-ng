"""Bounded controller gate, executed on the candidate's task queue and version."""

from datetime import timedelta
from temporalio import workflow
from temporalio.common import RetryPolicy, VersioningBehavior


@workflow.defn(versioning_behavior=VersioningBehavior.PINNED)
class RadarDeploymentSmokeWorkflow:
    @workflow.run
    async def run(self) -> dict:
        return await workflow.execute_activity(
            "radar_deployment_smoke",
            schedule_to_close_timeout=timedelta(seconds=90),
            start_to_close_timeout=timedelta(seconds=30),
            retry_policy=RetryPolicy(maximum_attempts=2),
        )
