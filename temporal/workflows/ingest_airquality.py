"""IngestAirQualityWorkflow — NOAA NAQFC (AQMv7) PM2.5 + ozone guidance.

Schedule: every 30 minutes. New cycles land only twice a day (06z/12z with a
few hours of product lag), so most runs are a cheap HEAD + skip; polling at
30 min just bounds how stale we are once a cycle does land.

Pipeline:
  1. Find latest available AQMv7 cycle (HEAD on the pm2.5 file)
  2. If new: render both pollutant files in 24-message chunks
     (72 hourly-average messages each = current hour + 3-day forecast)
  3. Publish each complete layer to the manifest atomically
  4. Mark run processed once the primary pm2.5 layer is live
  5. Cleanup superseded run subtrees
"""

from __future__ import annotations

import asyncio
from dataclasses import dataclass, field
from datetime import timedelta

from temporalio import workflow
from temporalio.common import RetryPolicy
from temporalio.exceptions import ActivityError

with workflow.unsafe.imports_passed_through():
    from backend.ingest_airquality.activities import (
        AQM_LAYERS,
        CHUNK_MESSAGES,
        FORECAST_MESSAGES,
        AqmChunkResult,
        AqmCleanupResult,
        AqmFindRunResult,
        aqm_cleanup,
        aqm_find_latest_run,
        aqm_mark_processed,
        aqm_publish_run,
        aqm_render_chunk,
    )


# Two cycles a day: keeping ~1.5 days covers the live run + its predecessor.
RETENTION_HOURS = 36
# Each chunk downloads a 50-90MB file and renders 24 frames × 3 palettes;
# two at a time keeps the aux worker responsive for its other workflows.
CHUNK_CONCURRENCY = 2

_DEFAULT_RETRY = RetryPolicy(
    initial_interval=timedelta(seconds=1),
    backoff_coefficient=2.0,
    maximum_interval=timedelta(seconds=100),
    maximum_attempts=5,
)
_CHUNK_RETRY = RetryPolicy(
    initial_interval=timedelta(seconds=5),
    backoff_coefficient=2.0,
    maximum_interval=timedelta(seconds=120),
    maximum_attempts=3,
)


@dataclass
class IngestAirQualityResult:
    run_id: str | None
    skipped_already_processed: bool
    published_layers: list[str] = field(default_factory=list)
    frames_rendered: int = 0
    cleanup: AqmCleanupResult | None = None


@workflow.defn(name="IngestAirQualityWorkflow")
class IngestAirQualityWorkflow:
    @workflow.run
    async def run(self) -> IngestAirQualityResult:
        find: AqmFindRunResult = await workflow.execute_activity(
            aqm_find_latest_run,
            start_to_close_timeout=timedelta(seconds=120),
            retry_policy=_DEFAULT_RETRY,
        )

        if find.run_id is None:
            workflow.logger.info("no AQM run available")
            cleanup = await self._cleanup()
            return IngestAirQualityResult(
                run_id=None, skipped_already_processed=False, cleanup=cleanup
            )

        if find.already_processed:
            workflow.logger.info("AQM run already processed: %s", find.run_id)
            cleanup = await self._cleanup()
            return IngestAirQualityResult(
                run_id=find.run_id, skipped_already_processed=True, cleanup=cleanup
            )

        sem = asyncio.Semaphore(CHUNK_CONCURRENCY)

        async def render_chunk(layer: str, start_msg: int) -> AqmChunkResult:
            async with sem:
                try:
                    return await workflow.execute_activity(
                        aqm_render_chunk,
                        args=[find.run_id, layer, start_msg],
                        start_to_close_timeout=timedelta(minutes=25),
                        # Bound total time across retries + queue wait so one
                        # sick chunk can't pin the run while SKIP overlap
                        # drops every newer trigger.
                        schedule_to_close_timeout=timedelta(minutes=40),
                        heartbeat_timeout=timedelta(seconds=180),
                        retry_policy=_CHUNK_RETRY,
                    )
                except ActivityError:
                    # A missing chunk only blocks that pollutant's publish;
                    # the run is retried on the next schedule tick because it
                    # is never marked processed without a pm2.5 publish.
                    workflow.logger.warning(
                        "AQM chunk %s@%d failed after retries for run %s",
                        layer, start_msg, find.run_id,
                    )
                    return AqmChunkResult(layer=layer, start_msg=start_msg)

        chunk_starts = list(range(0, FORECAST_MESSAGES, CHUNK_MESSAGES))
        results = await asyncio.gather(
            *(
                render_chunk(layer, start)
                for layer in AQM_LAYERS
                for start in chunk_starts
            )
        )
        frames_rendered = sum(len(r.rendered_timestamps) for r in results)

        published_layers: list[str] = await workflow.execute_activity(
            aqm_publish_run,
            args=[find.run_id, list(results)],
            start_to_close_timeout=timedelta(minutes=2),
            retry_policy=_DEFAULT_RETRY,
        )

        # PM2.5 is the primary product (it's what the Air Quality layer
        # shows). Only a run that published it counts as processed; ozone
        # alone re-runs next tick, where existing tiles make retries cheap.
        if "air-quality" in published_layers:
            await workflow.execute_activity(
                aqm_mark_processed,
                find.run_id,
                start_to_close_timeout=timedelta(seconds=30),
                retry_policy=_DEFAULT_RETRY,
            )
        else:
            workflow.logger.error(
                "AQM run %s incomplete (%d frames); not marking processed",
                find.run_id, frames_rendered,
            )

        cleanup = await self._cleanup()
        return IngestAirQualityResult(
            run_id=find.run_id,
            skipped_already_processed=False,
            published_layers=published_layers,
            frames_rendered=frames_rendered,
            cleanup=cleanup,
        )

    async def _cleanup(self) -> AqmCleanupResult:
        return await workflow.execute_activity(
            aqm_cleanup,
            RETENTION_HOURS,
            start_to_close_timeout=timedelta(minutes=5),
            retry_policy=_DEFAULT_RETRY,
        )
