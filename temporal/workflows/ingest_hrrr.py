"""IngestHrrrWorkflow — replaces the legacy `ingest-hrrr` CronJob.

Schedule: every 15 minutes (HRRR runs land hourly, but we poll faster so a
delayed run is picked up promptly). OverlapPolicy.SKIP guards against a
slow run piling up while a long forecast pass is still in flight.

Pipeline:
  1. Find latest available HRRR run (HEAD f01 file), or the run we already
     published partially if it is still young enough to finish
  2. If not marked processed: process forecast hours 1..horizon
     (18 default, 48 for 00z/06z/12z/18z extended runs). Hours already on
     disk return immediately; hours NOAA hasn't uploaded yet come back empty.
  3. Publish the consecutive f01.. prefix (`complete: false` until it spans
     the horizon); mark the run processed only once it is complete
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
        publishable_prefix,
    )


RETENTION_HOURS = 12
# Whole-CONUS pyramids for 3 palettes per hour; 4 keeps the render pool from starving the
# worker's other activities (the legacy single-pod worker has 4 activity slots).
FORECAST_CONCURRENCY = 4
# One hour = ~15 s subset download + ~1-3 min render. Already-rendered hours return in ms.
HOUR_START_TO_CLOSE = timedelta(minutes=8)
HOUR_SCHEDULE_TO_CLOSE = timedelta(minutes=12)
HOUR_HEARTBEAT_TIMEOUT = timedelta(seconds=90)

_DEFAULT_RETRY = RetryPolicy(
    initial_interval=timedelta(seconds=1),
    backoff_coefficient=2.0,
    maximum_interval=timedelta(seconds=100),
    maximum_attempts=5,
)
# Missing hours are not errors (the activity returns empty, no retry), so two
# attempts only cover genuine transport/render faults.
_FORECAST_RETRY = RetryPolicy(
    initial_interval=timedelta(seconds=2),
    backoff_coefficient=2.0,
    maximum_interval=timedelta(seconds=120),
    maximum_attempts=2,
)


@dataclass
class IngestHrrrResult:
    run_id: str | None
    skipped_already_processed: bool
    forecast_hours_processed: int
    cleanup: HrrrCleanupResult | None = None
    layers_per_hour: list[list[str]] = field(default_factory=list)
    published_layers: list[str] = field(default_factory=list)
    complete: bool = False


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
                        start_to_close_timeout=HOUR_START_TO_CLOSE,
                        # Total budget across retries + queue wait, so a stuck hour
                        # cannot pin the run while OverlapPolicy.SKIP drops newer fires.
                        schedule_to_close_timeout=HOUR_SCHEDULE_TO_CLOSE,
                        heartbeat_timeout=HOUR_HEARTBEAT_TIMEOUT,
                        retry_policy=_FORECAST_RETRY,
                    )
                except ActivityError:
                    # One sick forecast hour must not fail the run and cancel its
                    # siblings mid-render (asyncio.gather propagates the first error).
                    # The next 15-min fire re-polls the same run and only that hour re-renders.
                    workflow.logger.warning(
                        "HRRR f%02d failed after retries for run %s — continuing without it",
                        fhr, find.run_id,
                    )
                    return ForecastHourResult(fhr=fhr)

        results = await asyncio.gather(*(process_hour(fhr) for fhr in range(1, horizon + 1)))
        results = sorted(results, key=lambda r: r.fhr)
        layers_per_hour = [r.rendered_layers for r in results]
        # Progressive publication: advertise f01..fN as far as reflectivity exists
        # consecutively. NOAA uploads hours over ~30-60 min; a gap or a pending
        # hour ends the prefix and the next fire extends it.
        prefix = len(publishable_prefix(results))
        complete = prefix == horizon

        if prefix == 0:
            workflow.logger.info(
                "HRRR run %s has no publishable hours yet (f01 pending); not publishing",
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

        if complete:
            # Only a complete run is "processed"; a partial one is re-polled by
            # the next fire (the finder prefers it while it is < 3 h old).
            await workflow.execute_activity(
                hrrr_mark_processed, find.run_id,
                start_to_close_timeout=timedelta(seconds=30),
                retry_policy=_DEFAULT_RETRY,
            )
        else:
            workflow.logger.info(
                "HRRR run %s published partially: %d/%d hours; will extend on the next fire",
                find.run_id, prefix, horizon,
            )

        cleanup = await self._cleanup()
        return IngestHrrrResult(
            run_id=find.run_id,
            skipped_already_processed=False,
            forecast_hours_processed=prefix,
            cleanup=cleanup,
            layers_per_hour=layers_per_hour,
            published_layers=published_layers,
            complete=complete,
        )

    async def _cleanup(self) -> HrrrCleanupResult:
        return await workflow.execute_activity(
            hrrr_cleanup, RETENTION_HOURS,
            start_to_close_timeout=timedelta(minutes=5),
            retry_policy=_DEFAULT_RETRY,
        )
