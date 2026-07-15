"""IngestHrrrWorkflow — replaces the legacy `ingest-hrrr` CronJob.

Schedule: every 15 minutes (HRRR runs land hourly, but we poll faster so a
delayed run is picked up promptly). OverlapPolicy.SKIP guards against a
slow run piling up while a long forecast pass is still in flight.

Pipeline:
  1. Find latest available HRRR run (HEAD f01 file)
  2. If new (not in ProcessedSet): process forecast hours 1..horizon
     (18 default, 48 for 00z/06z/12z/18z extended runs)
  3. Mark run processed
  4. Cleanup old HRRR-layer tiles + grids
"""

from __future__ import annotations

import asyncio
from dataclasses import dataclass, field
from datetime import timedelta

from temporalio import workflow
from temporalio.common import RetryPolicy
from temporalio.exceptions import ActivityError

with workflow.unsafe.imports_passed_through():
    from backend.ingest_hrrr.activities import (
        FindRunResult,
        ForecastHourResult,
        HrrrCleanupResult,
        hrrr_cleanup,
        hrrr_find_latest_run,
        hrrr_horizon_for_run,
        hrrr_mark_processed,
        hrrr_publish_run,
        hrrr_process_forecast_hour,
    )


RETENTION_HOURS = 12
FORECAST_CONCURRENCY = 8

_DEFAULT_RETRY = RetryPolicy(
    initial_interval=timedelta(seconds=1),
    backoff_coefficient=2.0,
    maximum_interval=timedelta(seconds=100),
    maximum_attempts=5,
)
_FORECAST_RETRY = RetryPolicy(
    initial_interval=timedelta(seconds=2),
    backoff_coefficient=2.0,
    maximum_interval=timedelta(seconds=120),
    maximum_attempts=3,
)


@dataclass
class IngestHrrrResult:
    run_id: str | None
    skipped_already_processed: bool
    forecast_hours_processed: int
    cleanup: HrrrCleanupResult | None = None
    layers_per_hour: list[list[str]] = field(default_factory=list)
    published_layers: list[str] = field(default_factory=list)


@workflow.defn(name="IngestHrrrWorkflow")
class IngestHrrrWorkflow:
    @workflow.run
    async def run(self) -> IngestHrrrResult:
        find: FindRunResult = await workflow.execute_activity(
            hrrr_find_latest_run,
            start_to_close_timeout=timedelta(seconds=120),
            retry_policy=_DEFAULT_RETRY,
        )

        if find.run_id is None:
            workflow.logger.info("no HRRR run available")
            cleanup = await self._cleanup()
            return IngestHrrrResult(run_id=None, skipped_already_processed=False, forecast_hours_processed=0, cleanup=cleanup)

        if find.already_processed:
            workflow.logger.info("HRRR run already processed: %s", find.run_id)
            cleanup = await self._cleanup()
            return IngestHrrrResult(
                run_id=find.run_id, skipped_already_processed=True,
                forecast_hours_processed=0, cleanup=cleanup,
            )

        horizon: int = await workflow.execute_activity(
            hrrr_horizon_for_run, find.run_id,
            start_to_close_timeout=timedelta(seconds=10),
            retry_policy=_DEFAULT_RETRY,
        )
        workflow.logger.info("HRRR run %s horizon=%dh", find.run_id, horizon)

        sem = asyncio.Semaphore(FORECAST_CONCURRENCY)

        async def process_hour(fhr: int) -> ForecastHourResult:
            async with sem:
                try:
                    return await workflow.execute_activity(
                        hrrr_process_forecast_hour, args=[find.run_id, fhr],
                        start_to_close_timeout=timedelta(minutes=20),
                        # Total budget across retries + queue wait. Without it,
                        # 3 × 20-min attempts can pin one run while the SKIP
                        # overlap policy drops every newer trigger — same
                        # failure mode fixed in ingest_mrms.
                        schedule_to_close_timeout=timedelta(minutes=30),
                        heartbeat_timeout=timedelta(seconds=180),
                        retry_policy=_FORECAST_RETRY,
                    )
                except ActivityError:
                    # One sick forecast hour must not fail the run and cancel
                    # its siblings mid-render (asyncio.gather propagates the
                    # first error). The gap self-heals: the next HRRR run
                    # (≤1 h away) supersedes this valid time anyway.
                    workflow.logger.warning(
                        "HRRR f%02d failed after retries for run %s — continuing without it",
                        fhr, find.run_id,
                    )
                    return ForecastHourResult(fhr=fhr)

        results = await asyncio.gather(*(process_hour(fhr) for fhr in range(1, horizon + 1)))
        results = sorted(results, key=lambda r: r.fhr)
        layers_per_hour = [r.rendered_layers for r in results]
        succeeded = sum(1 for r in results if r.rendered_layers)

        if succeeded != horizon or any(
            "radar-hrrr" not in result.rendered_layers for result in results
        ):
            # Never expose a mixed or partial run. Immutable run paths can be
            # retried safely; the prior complete manifest remains live.
            workflow.logger.error(
                "HRRR run %s incomplete: %d/%d reflectivity hours; not publishing",
                find.run_id, succeeded, horizon,
            )
            cleanup = await self._cleanup()
            return IngestHrrrResult(
                run_id=find.run_id,
                skipped_already_processed=False,
                forecast_hours_processed=0,
                cleanup=cleanup,
                layers_per_hour=layers_per_hour,
            )

        published_layers: list[str] = await workflow.execute_activity(
            hrrr_publish_run,
            args=[find.run_id, results],
            start_to_close_timeout=timedelta(minutes=2),
            retry_policy=_DEFAULT_RETRY,
        )
        if "radar-hrrr" not in published_layers:
            workflow.logger.error(
                "HRRR run %s failed coherent publication; not marking processed",
                find.run_id,
            )
            cleanup = await self._cleanup()
            return IngestHrrrResult(
                run_id=find.run_id,
                skipped_already_processed=False,
                forecast_hours_processed=0,
                cleanup=cleanup,
                layers_per_hour=layers_per_hour,
            )

        await workflow.execute_activity(
            hrrr_mark_processed, find.run_id,
            start_to_close_timeout=timedelta(seconds=30),
            retry_policy=_DEFAULT_RETRY,
        )

        cleanup = await self._cleanup()
        return IngestHrrrResult(
            run_id=find.run_id,
            skipped_already_processed=False,
            forecast_hours_processed=succeeded,
            cleanup=cleanup,
            layers_per_hour=layers_per_hour,
            published_layers=published_layers,
        )

    async def _cleanup(self) -> HrrrCleanupResult:
        return await workflow.execute_activity(
            hrrr_cleanup, RETENTION_HOURS,
            start_to_close_timeout=timedelta(minutes=5),
            retry_policy=_DEFAULT_RETRY,
        )
