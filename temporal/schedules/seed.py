"""Idempotent Temporal Schedule seeding.

Called from `temporal/worker.py` on every worker startup (also runnable
as a one-off via `python -m temporal.schedules.seed` for local debugging).
For each scheduled workflow, attempts `client.create_schedule(...)` and
falls through to `update(...)` if the schedule already exists. HA
replicas racing is harmless: both converge on the same desired state.

All schedules use `OverlapPolicy.SKIP` (slow run does not queue). Each
ScheduleDef also carries its own `max_runtime` (workflow execution timeout,
sized to a few multiples of the cadence so one stuck run cannot block many
fires), a `catchup_window` (short for frequent schedules so worker recovery
does not replay a burst of stale fires) and, on the 2-min schedules, a
`jitter` so three workflows do not start on the same second.

Defines TWO ingest-mrms schedules (base + composite) driving the same
workflow with different inputs, replacing both the legacy
`ingest-mrms` CronJob AND the `ingest-radar-composite` Deployment.
"""

from __future__ import annotations

import asyncio
import dataclasses
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
    ScheduleUpdateInput,
)
from temporalio.service import RPCError, RPCStatusCode

from temporal.task_queues import (
    ALERTS_TASK_QUEUE,
    AUX_TASK_QUEUE,
    HRRR_TASK_QUEUE,
    MRMS_TASK_QUEUE,
    NOWCAST_TASK_QUEUE,
    LEGACY_TASK_QUEUE,
)


# RPC status codes worth retrying while seeding at worker startup. The
# dominant case: a Temporal server that just (re)started has history shards
# still warming up, so it rejects RPCs with "shard status unknown"
# (UNAVAILABLE) for seconds-to-minutes. Previously a single such error
# propagated out of _main() and killed the worker (exitCode 1), so k8s
# crash-looped every replica until the shards settled — and any scheduled
# run landing in that window (e.g. nowcast) could fail in the churn.
_RETRYABLE_RPC_CODES = frozenset(
    {
        # The Rust SDK bridge reports some client-side "Timeout expired" calls as
        # CANCELLED (code 1) instead of DEADLINE_EXCEEDED. Treat both as the same
        # bounded startup transient so schedule seeding cannot crash-loop a worker.
        RPCStatusCode.CANCELLED,
        RPCStatusCode.UNAVAILABLE,
        RPCStatusCode.DEADLINE_EXCEEDED,
        RPCStatusCode.RESOURCE_EXHAUSTED,
        RPCStatusCode.ABORTED,
        RPCStatusCode.INTERNAL,
        RPCStatusCode.UNKNOWN,
        # A create can observe ALREADY_EXISTS immediately before a concurrent
        # delete wins. The following update then sees NOT_FOUND; retry the
        # whole create-or-update reconciliation so the next pass can create.
        RPCStatusCode.NOT_FOUND,
    }
)


@dataclass
class ScheduleDef:
    schedule_id: str
    workflow_name: str
    max_runtime: timedelta
    workflow_input: list[Any] = field(default_factory=list)
    interval: timedelta | None = None
    task_queue: str = AUX_TASK_QUEUE
    catchup_window: timedelta = timedelta(hours=1)
    jitter: timedelta | None = None


# Shared knobs for the 2-min schedules: a missed hour of 2-min fires is
# worthless (fresher data supersedes it), and jitter spreads their CPU peaks.
_FAST_CATCHUP = timedelta(minutes=5)
_FAST_JITTER = timedelta(seconds=20)


SCHEDULES: list[ScheduleDef] = [
    # MRMS base reflectivity (QC) — every 2 min
    ScheduleDef(
        "ingest-mrms-base",
        "IngestMrmsWorkflow",
        max_runtime=timedelta(minutes=6),
        workflow_input=[
            {
                "mrms_prefix": "CONUS/MergedBaseReflectivityQC_00.50",
                "layer_name": "radar",
            }
        ],
        interval=timedelta(minutes=2),
        task_queue=MRMS_TASK_QUEUE,
        catchup_window=_FAST_CATCHUP,
        jitter=_FAST_JITTER,
    ),
    # MRMS composite reflectivity (full atmosphere) — every 2 min
    ScheduleDef(
        "ingest-mrms-composite",
        "IngestMrmsWorkflow",
        max_runtime=timedelta(minutes=6),
        workflow_input=[
            {
                "mrms_prefix": "CONUS/MergedReflectivityComposite_00.50",
                "layer_name": "radar-composite",
            }
        ],
        interval=timedelta(minutes=2),
        task_queue=MRMS_TASK_QUEUE,
        catchup_window=_FAST_CATCHUP,
        jitter=_FAST_JITTER,
    ),
    # HRRR forecast — every 15 min
    ScheduleDef(
        "ingest-hrrr",
        "IngestHrrrWorkflow",
        max_runtime=timedelta(minutes=45),
        interval=timedelta(minutes=15),
        task_queue=HRRR_TASK_QUEUE,
    ),
    # NAQFC air quality (PM2.5 + ozone) — cycles land twice daily; poll every
    # 30 min so a fresh cycle is picked up promptly. Non-new runs are a HEAD.
    ScheduleDef(
        "ingest-airquality",
        "IngestAirQualityWorkflow",
        max_runtime=timedelta(minutes=75),
        interval=timedelta(minutes=30),
    ),
    # Lightning WS consumer — every 60 min (workflow runs activity for ~50 min)
    ScheduleDef(
        "ingest-lightning",
        "IngestLightningWorkflow",
        max_runtime=timedelta(minutes=56),
        interval=timedelta(minutes=60),
    ),
    # NHC tropical cyclones — every 1 hour
    ScheduleDef(
        "ingest-tropical",
        "IngestTropicalWorkflow",
        max_runtime=timedelta(minutes=5),
        interval=timedelta(hours=1),
    ),
    # pysteps nowcast — every 2 min
    ScheduleDef(
        "nowcast",
        "NowcastWorkflow",
        max_runtime=timedelta(minutes=12),
        interval=timedelta(minutes=2),
        task_queue=NOWCAST_TASK_QUEUE,
        catchup_window=_FAST_CATCHUP,
        jitter=_FAST_JITTER,
    ),
    # Tile + grid cleanup — every 1 hour
    ScheduleDef(
        "tile-cleanup",
        "TileCleanupWorkflow",
        max_runtime=timedelta(minutes=15),
        interval=timedelta(hours=1),
    ),
    # NWS active alerts — every 5 min
    ScheduleDef(
        "poll-alerts",
        "PollAlertsWorkflow",
        max_runtime=timedelta(minutes=4, seconds=30),
        interval=timedelta(minutes=5),
        task_queue=ALERTS_TASK_QUEUE,
        catchup_window=timedelta(minutes=10),
    ),
    # Open-meteo GFS sync — every 6h. The legacy CronJob used "30 */6 * * *"
    # to align with GFS run lag, but Temporal SKIP overlap + --past-days=2
    # backfill make exact wall-clock alignment unnecessary; freshness is
    # bounded by the 6h interval regardless.
    ScheduleDef(
        "open-meteo-sync-gfs",
        "OpenMeteoSyncWorkflow",
        max_runtime=timedelta(minutes=60),
        # ncep_gfs013 (0.13° surface), NOT ncep_gfs025: open-meteo restructured
        # its S3 open-data so ncep_gfs025 now holds only upper-air/pressure-level
        # fields — surface vars (temperature_2m, dew_point_2m, …) moved to
        # ncep_gfs013. Syncing gfs025 silently fetched no surface data, which is
        # why the 7-day forecast went all-null (~2026-06-13).
        workflow_input=[
            {
                "model": "ncep_gfs013",
                "variables": "temperature_2m,dew_point_2m,apparent_temperature,relative_humidity_2m,wind_speed_10m,wind_direction_10m,wind_gusts_10m,weather_code,precipitation,precipitation_probability,surface_pressure,uv_index",
                "past_days": 2,
            }
        ],
        interval=timedelta(hours=6),
    ),
    # Open-meteo HRRR sync — every 1h.
    ScheduleDef(
        "open-meteo-sync-hrrr",
        "OpenMeteoSyncWorkflow",
        max_runtime=timedelta(minutes=55),
        workflow_input=[
            {
                "model": "ncep_hrrr_conus",
                "variables": "temperature_2m,dew_point_2m,apparent_temperature,relative_humidity_2m,wind_speed_10m,wind_direction_10m,wind_gusts_10m,weather_code,precipitation,precipitation_probability,surface_pressure",
                "past_days": 1,
            }
        ],
        interval=timedelta(hours=1),
    ),
]


def _spec_for(s: ScheduleDef) -> Schedule:
    if s.interval is None:
        raise ValueError(f"schedule {s.schedule_id} has no interval")
    spec = ScheduleSpec(
        intervals=[ScheduleIntervalSpec(every=s.interval)], jitter=s.jitter
    )
    task_queue = (
        s.task_queue
        if os.environ.get("USE_ISOLATED_TASK_QUEUES") == "1"
        else LEGACY_TASK_QUEUE
    )
    return Schedule(
        action=ScheduleActionStartWorkflow(
            s.workflow_name,
            *s.workflow_input,
            id=f"sched-{s.schedule_id}",
            task_queue=task_queue,
            execution_timeout=s.max_runtime,
        ),
        spec=spec,
        policy=SchedulePolicy(
            overlap=ScheduleOverlapPolicy.SKIP,
            catchup_window=s.catchup_window,
        ),
    )


# Per-RPC budget for schedule operations. A healthy create/update
# answers in well under a second; a scheduler workflow wedged server-side
# (post cluster-rebuild state, 2026-08-25 `nowcast` incident) hangs the RPC
# until the client's default deadline instead. A short explicit timeout turns
# the failure into a bounded reconciliation error rather than a 100s+ stall.
_RPC_TIMEOUT = timedelta(seconds=15)


class ScheduleSeedError(RuntimeError):
    """Aggregate failures from an otherwise isolated reconciliation pass."""

    def __init__(self, failures: dict[str, Exception]) -> None:
        self.failures = failures
        schedule_ids = ", ".join(sorted(failures))
        super().__init__(f"schedule reconciliation failed for: {schedule_ids}")


async def _schedules_semantically_equal(
    client: Client,
    current: Schedule,
    desired: Schedule,
) -> bool:
    """Compare Schedule meaning after SDK payload encoding and normalization.

    A Schedule decoded from Temporal contains raw Payload objects and marks
    its action as ``_from_raw``; direct dataclass equality therefore reports
    a false difference from the equivalent declarative Python inputs. The SDK
    protobuf form removes that representational difference.
    """
    return await current._to_proto(client) == await desired._to_proto(client)


async def _update_for_current_description(
    client: Client,
    inp: ScheduleUpdateInput,
    desired: Schedule,
) -> ScheduleUpdate | None:
    """Build an update from the callback's current state, or return no-op.

    Temporal supplies the description used for this update attempt. Copying
    state here, rather than from a separate pre-update describe, preserves the
    latest pause, note, action limit, and remaining-action count visible to the
    SDK callback. Returning ``None`` prevents an UpdateSchedule mutation when
    the declarative action/spec/policy already match.
    """
    current = inp.description.schedule
    desired_with_current_state = dataclasses.replace(desired, state=current.state)
    if await _schedules_semantically_equal(
        client,
        current,
        desired_with_current_state,
    ):
        return None
    return ScheduleUpdate(schedule=desired_with_current_state)


async def _apply(client: Client, s: ScheduleDef, spec: Schedule) -> None:
    """Create-or-update one schedule, preserving live paused state."""
    try:
        await client.create_schedule(s.schedule_id, spec, rpc_timeout=_RPC_TIMEOUT)
        print(f"[seed] created schedule {s.schedule_id}")
    except ScheduleAlreadyRunningError:
        handle = client.get_schedule_handle(s.schedule_id)
        update_required = False

        async def _update(
            inp: ScheduleUpdateInput, desired: Schedule = spec
        ) -> ScheduleUpdate | None:
            nonlocal update_required
            update = await _update_for_current_description(
                client,
                inp,
                desired,
            )
            # The SDK contract allows repeated callback invocation during
            # conflict handling. Reflect the last callback decision rather
            # than remembering a stale earlier one.
            update_required = update is not None
            return update

        await handle.update(_update, rpc_timeout=_RPC_TIMEOUT)
        outcome = "updated" if update_required else "unchanged"
        print(f"[seed] {outcome} schedule {s.schedule_id}")


async def _apply_isolated(client: Client, schedule: ScheduleDef) -> Exception | None:
    """Apply one definition without allowing its failure to cancel peers."""
    try:
        await _apply(client, schedule, _spec_for(schedule))
    except asyncio.CancelledError:
        raise
    except Exception as exc:  # noqa: BLE001 - returned to the aggregate caller
        return exc
    return None


async def _seed_pass(
    client: Client,
    schedules: list[ScheduleDef],
) -> dict[str, Exception]:
    """Reconcile Schedules concurrently and return failures by Schedule ID."""
    results = await asyncio.gather(
        *(_apply_isolated(client, schedule) for schedule in schedules)
    )
    return {
        schedule.schedule_id: result
        for schedule, result in zip(schedules, results, strict=True)
        if result is not None
    }


async def seed(client: Client) -> None:
    """Run one failure-isolated reconciliation pass for local debugging."""
    failures = await _seed_pass(client, SCHEDULES)
    if failures:
        raise ScheduleSeedError(failures)


async def seed_with_retry(
    client: Client,
    *,
    max_attempts: int = 10,
    base_delay: float = 1.0,
    max_delay: float = 20.0,
) -> None:
    """Reconcile all schedules with isolated, bounded transient retries.

    Each pass runs concurrently, so a slow or failed Schedule does not prevent
    healthy definitions from converging. Only retryable Temporal RPC failures
    enter a later pass. Permanent errors are retained for the final aggregate
    error while the remaining Schedules continue. This function never
    deletes a Schedule or terminates a workflow as a recovery mechanism.
    """
    if max_attempts < 1:
        raise ValueError("max_attempts must be at least 1")

    pending = list(SCHEDULES)
    permanent_failures: dict[str, Exception] = {}
    for attempt in range(1, max_attempts + 1):
        pass_failures = await _seed_pass(client, pending)
        retry: list[ScheduleDef] = []
        retry_failures: dict[str, Exception] = {}

        for schedule in pending:
            exc = pass_failures.get(schedule.schedule_id)
            if exc is None:
                continue
            if isinstance(exc, RPCError) and exc.status in _RETRYABLE_RPC_CODES:
                retry.append(schedule)
                retry_failures[schedule.schedule_id] = exc
                print(
                    f"[seed] transient RPC error ({exc.status.name}) on "
                    f"{schedule.schedule_id}, attempt {attempt}/{max_attempts}: "
                    f"{exc.message!r}"
                )
            else:
                permanent_failures[schedule.schedule_id] = exc
                print(
                    f"[seed] permanent reconciliation error on "
                    f"{schedule.schedule_id}: {exc!r}"
                )

        if not retry:
            break
        if attempt == max_attempts:
            permanent_failures.update(retry_failures)
            break

        delay = min(max_delay, base_delay * 2 ** (attempt - 1))
        print(
            f"[seed] {len(retry)} schedule(s) still failing after attempt "
            f"{attempt}/{max_attempts}; retrying in {delay:.0f}s"
        )
        await asyncio.sleep(delay)
        pending = retry

    if permanent_failures:
        raise ScheduleSeedError(permanent_failures)


async def _main() -> None:
    target = os.environ.get("TEMPORAL_ADDRESS", "localhost:7233")
    namespace = os.environ.get("TEMPORAL_NAMESPACE", "default")
    client = await Client.connect(target, namespace=namespace)
    await seed_with_retry(client)


if __name__ == "__main__":
    asyncio.run(_main())
