"""IngestLightningWorkflow — wraps the long-lived Blitzortung WS consumer.

Lightning is a stream, not a poll. We run it as a single long activity
that pumps strikes into the GeoJSON file every 2s for ~50 minutes, then
exits cleanly. Schedule fires every 60 min with OverlapPolicy.SKIP so the
next workflow only starts once the previous activity has exited.

If the worker pod dies mid-stream, Temporal redispatches the activity to
another worker. Buffer state is lost across runs; rolling 15-min retention
means we recover within 15 min.
"""

from __future__ import annotations

from datetime import timedelta

from temporalio import workflow
from temporalio.common import RetryPolicy

with workflow.unsafe.imports_passed_through():
    from backend.ingest_lightning.activities import (
        LightningRunResult,
        lightning_consume_stream,
    )


RUN_DURATION_S = 50 * 60  # 50 min — leaves headroom inside the 60-min schedule

_RETRY = RetryPolicy(
    initial_interval=timedelta(seconds=5),
    backoff_coefficient=2.0,
    maximum_interval=timedelta(seconds=120),
    maximum_attempts=3,
)


@workflow.defn(name="IngestLightningWorkflow")
class IngestLightningWorkflow:
    @workflow.run
    async def run(self) -> LightningRunResult:
        return await workflow.execute_activity(
            lightning_consume_stream, RUN_DURATION_S,
            start_to_close_timeout=timedelta(seconds=RUN_DURATION_S + 60),
            heartbeat_timeout=timedelta(seconds=90),
            retry_policy=_RETRY,
        )
