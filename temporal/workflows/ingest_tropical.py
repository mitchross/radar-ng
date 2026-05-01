"""IngestTropicalWorkflow — replaces the legacy `ingest-tropical` CronJob.

Schedule: every 1 hour, OverlapPolicy.SKIP. NHC updates active storm data
roughly every 6 hours during a system, more frequently as a storm develops;
1h cadence is enough freshness without hammering the feed.
"""

from __future__ import annotations

from datetime import timedelta

from temporalio import workflow
from temporalio.common import RetryPolicy

with workflow.unsafe.imports_passed_through():
    from backend.ingest_tropical.activities import (
        TropicalResult,
        tropical_fetch_and_publish,
    )


_RETRY = RetryPolicy(
    initial_interval=timedelta(seconds=2),
    backoff_coefficient=2.0,
    maximum_interval=timedelta(seconds=100),
    maximum_attempts=5,
)


@workflow.defn(name="IngestTropicalWorkflow")
class IngestTropicalWorkflow:
    @workflow.run
    async def run(self) -> TropicalResult:
        return await workflow.execute_activity(
            tropical_fetch_and_publish,
            start_to_close_timeout=timedelta(seconds=60),
            retry_policy=_RETRY,
        )
