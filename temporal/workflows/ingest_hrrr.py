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

from dataclasses import dataclass, field
from datetime import timedelta

from temporalio import workflow
from temporalio.common import RetryPolicy

with workflow.unsafe.imports_passed_through():
    from backend.ingest_hrrr.activities import (
        FindRunResult,
        ForecastHourResult,
        HrrrCleanupResult,
        hrrr_cleanup,
        hrrr_find_latest_run,
        hrrr_horizon_for_run,
        hrrr_mark_processed,
        hrrr_process_forecast_hour,
    )


RETENTION_HOURS = 12

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

        layers_per_hour: list[list[str]] = []
        for fhr in range(1, horizon + 1):
            r: ForecastHourResult = await workflow.execute_activity(
                hrrr_process_forecast_hour, args=[find.run_id, fhr],
                start_to_close_timeout=timedelta(minutes=20),
                heartbeat_timeout=timedelta(seconds=180),
                retry_policy=_FORECAST_RETRY,
            )
            layers_per_hour.append(r.rendered_layers)

        await workflow.execute_activity(
            hrrr_mark_processed, find.run_id,
            start_to_close_timeout=timedelta(seconds=30),
            retry_policy=_DEFAULT_RETRY,
        )

        cleanup = await self._cleanup()
        return IngestHrrrResult(
            run_id=find.run_id,
            skipped_already_processed=False,
            forecast_hours_processed=horizon,
            cleanup=cleanup,
            layers_per_hour=layers_per_hour,
        )

    async def _cleanup(self) -> HrrrCleanupResult:
        return await workflow.execute_activity(
            hrrr_cleanup, RETENTION_HOURS,
            start_to_close_timeout=timedelta(minutes=5),
            retry_policy=_DEFAULT_RETRY,
        )
