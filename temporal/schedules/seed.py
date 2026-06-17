"""Idempotent Temporal Schedule seeding.

Called from `temporal/worker.py` on every worker startup (also runnable
as a one-off via `python -m temporal.schedules.seed` for local debugging).
For each scheduled workflow, attempts `client.create_schedule(...)` and
falls through to `update(...)` if the schedule already exists. HA
replicas racing is harmless: both converge on the same desired state.

All schedules use `OverlapPolicy.SKIP` (slow run does not queue) and a
`CatchupWindow=1h` (no thundering-herd backfill on worker recovery).

Defines TWO ingest-mrms schedules (base + composite) driving the same
workflow with different inputs, replacing both the legacy
`ingest-mrms` CronJob AND the `ingest-radar-composite` Deployment.
"""

from __future__ import annotations

import asyncio
import os
from dataclasses import dataclass, field
from datetime import timedelta
from typing import Any

from temporalio.client import (
    Client,
    Schedule,
    ScheduleActionStartWorkflow,
    ScheduleAlreadyRunningError,
    ScheduleIntervalSpec,
    ScheduleOverlapPolicy,
    SchedulePolicy,
    ScheduleSpec,
    ScheduleUpdate,
)
from temporalio.service import RPCError, RPCStatusCode


TASK_QUEUE = "radar-ng"

# RPC status codes worth retrying while seeding at worker startup. The
# dominant case: a Temporal server that just (re)started has history shards
# still warming up, so it rejects RPCs with "shard status unknown"
# (UNAVAILABLE) for seconds-to-minutes. Previously a single such error
# propagated out of _main() and killed the worker (exitCode 1), so k8s
# crash-looped every replica until the shards settled — and any scheduled
# run landing in that window (e.g. nowcast) could fail in the churn.
_RETRYABLE_RPC_CODES = frozenset({
    RPCStatusCode.UNAVAILABLE,
    RPCStatusCode.DEADLINE_EXCEEDED,
    RPCStatusCode.RESOURCE_EXHAUSTED,
    RPCStatusCode.ABORTED,
    RPCStatusCode.INTERNAL,
    RPCStatusCode.UNKNOWN,
})


@dataclass
class ScheduleDef:
    schedule_id: str
    workflow_name: str
    workflow_input: list[Any] = field(default_factory=list)
    interval: timedelta | None = None


SCHEDULES: list[ScheduleDef] = [
    # MRMS base reflectivity (QC) — every 2 min
    ScheduleDef(
        "ingest-mrms-base", "IngestMrmsWorkflow",
        workflow_input=[{"mrms_prefix": "CONUS/MergedBaseReflectivityQC_00.50", "layer_name": "radar"}],
        interval=timedelta(minutes=2),
    ),
    # MRMS composite reflectivity (full atmosphere) — every 2 min
    ScheduleDef(
        "ingest-mrms-composite", "IngestMrmsWorkflow",
        workflow_input=[{"mrms_prefix": "CONUS/MergedReflectivityComposite_00.50", "layer_name": "radar-composite"}],
        interval=timedelta(minutes=2),
    ),
    # HRRR forecast — every 15 min
    ScheduleDef("ingest-hrrr", "IngestHrrrWorkflow", interval=timedelta(minutes=15)),
    # Lightning WS consumer — every 60 min (workflow runs activity for ~50 min)
    ScheduleDef("ingest-lightning", "IngestLightningWorkflow", interval=timedelta(minutes=60)),
    # NHC tropical cyclones — every 1 hour
    ScheduleDef("ingest-tropical", "IngestTropicalWorkflow", interval=timedelta(hours=1)),
    # pysteps nowcast — every 2 min
    ScheduleDef("nowcast", "NowcastWorkflow", interval=timedelta(minutes=2)),
    # Tile + grid cleanup — every 1 hour
    ScheduleDef("tile-cleanup", "TileCleanupWorkflow", interval=timedelta(hours=1)),
    # NWS active alerts — every 5 min
    ScheduleDef("poll-alerts", "PollAlertsWorkflow", interval=timedelta(minutes=5)),
    # Open-meteo GFS sync — every 6h. The legacy CronJob used "30 */6 * * *"
    # to align with GFS run lag, but Temporal SKIP overlap + --past-days=2
    # backfill make exact wall-clock alignment unnecessary; freshness is
    # bounded by the 6h interval regardless.
    ScheduleDef(
        "open-meteo-sync-gfs", "OpenMeteoSyncWorkflow",
        # ncep_gfs013 (0.13° surface), NOT ncep_gfs025: open-meteo restructured
        # its S3 open-data so ncep_gfs025 now holds only upper-air/pressure-level
        # fields — surface vars (temperature_2m, dew_point_2m, …) moved to
        # ncep_gfs013. Syncing gfs025 silently fetched no surface data, which is
        # why the 7-day forecast went all-null (~2026-06-13).
        workflow_input=[{
            "model": "ncep_gfs013",
            "variables": "temperature_2m,dew_point_2m,apparent_temperature,relative_humidity_2m,wind_speed_10m,wind_direction_10m,wind_gusts_10m,weather_code,precipitation,precipitation_probability,surface_pressure,uv_index",
            "past_days": 2,
        }],
        interval=timedelta(hours=6),
    ),
    # Open-meteo HRRR sync — every 1h.
    ScheduleDef(
        "open-meteo-sync-hrrr", "OpenMeteoSyncWorkflow",
        workflow_input=[{
            "model": "ncep_hrrr_conus",
            "variables": "temperature_2m,dew_point_2m,apparent_temperature,relative_humidity_2m,wind_speed_10m,wind_direction_10m,wind_gusts_10m,weather_code,precipitation,precipitation_probability,surface_pressure",
            "past_days": 1,
        }],
        interval=timedelta(hours=1),
    ),
]


def _spec_for(s: ScheduleDef) -> Schedule:
    if s.interval is None:
        raise ValueError(f"schedule {s.schedule_id} has no interval")
    spec = ScheduleSpec(intervals=[ScheduleIntervalSpec(every=s.interval)])
    return Schedule(
        action=ScheduleActionStartWorkflow(
            s.workflow_name,
            *s.workflow_input,
            id=f"sched-{s.schedule_id}",
            task_queue=TASK_QUEUE,
        ),
        spec=spec,
        policy=SchedulePolicy(
            overlap=ScheduleOverlapPolicy.SKIP,
            catchup_window=timedelta(hours=1),
        ),
    )


async def seed(client: Client) -> None:
    for s in SCHEDULES:
        spec = _spec_for(s)
        try:
            await client.create_schedule(s.schedule_id, spec)
            print(f"[seed] created schedule {s.schedule_id}")
        except ScheduleAlreadyRunningError:
            handle = client.get_schedule_handle(s.schedule_id)
            await handle.update(lambda _: ScheduleUpdate(schedule=spec))
            print(f"[seed] updated schedule {s.schedule_id}")


async def seed_with_retry(
    client: Client,
    *,
    max_attempts: int = 10,
    base_delay: float = 1.0,
    max_delay: float = 20.0,
) -> None:
    """Run seed() with bounded exponential backoff over transient RPC errors.

    seed() is idempotent (create-or-update per schedule_id), so re-running the
    whole pass after a partial failure is safe — already-created schedules
    fall through to the update path. Non-retryable errors and exhausted
    attempts re-raise, so a genuinely-down Temporal still fails startup and
    lets k8s restart the pod rather than parking a worker with no schedules.

    Default budget (~10 attempts, 1s→20s capped) covers ~110s of shard
    warmup, comfortably more than a normal Temporal restart needs.
    """
    for attempt in range(1, max_attempts + 1):
        try:
            await seed(client)
            return
        except RPCError as exc:
            if exc.status not in _RETRYABLE_RPC_CODES or attempt == max_attempts:
                raise
            delay = min(max_delay, base_delay * 2 ** (attempt - 1))
            print(
                f"[seed] transient RPC error ({exc.status.name}) on attempt "
                f"{attempt}/{max_attempts}: {exc.message!r}; retrying in {delay:.0f}s"
            )
            await asyncio.sleep(delay)


async def _main() -> None:
    target = os.environ.get("TEMPORAL_ADDRESS", "localhost:7233")
    namespace = os.environ.get("TEMPORAL_NAMESPACE", "default")
    client = await Client.connect(target, namespace=namespace)
    await seed_with_retry(client)


if __name__ == "__main__":
    asyncio.run(_main())
