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
from datetime import datetime, timedelta, timezone
from typing import Any

from temporalio.client import (
    Client,
    Schedule,
    ScheduleActionExecutionStartWorkflow,
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
_RETRYABLE_RPC_CODES = frozenset({
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
})


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
        "ingest-mrms-base", "IngestMrmsWorkflow",
        max_runtime=timedelta(minutes=6),
        workflow_input=[{"mrms_prefix": "CONUS/MergedBaseReflectivityQC_00.50", "layer_name": "radar"}],
        interval=timedelta(minutes=2),
        task_queue=MRMS_TASK_QUEUE,
        catchup_window=_FAST_CATCHUP, jitter=_FAST_JITTER,
    ),
    # MRMS composite reflectivity (full atmosphere) — every 2 min
    ScheduleDef(
        "ingest-mrms-composite", "IngestMrmsWorkflow",
        max_runtime=timedelta(minutes=6),
        workflow_input=[{"mrms_prefix": "CONUS/MergedReflectivityComposite_00.50", "layer_name": "radar-composite"}],
        interval=timedelta(minutes=2),
        task_queue=MRMS_TASK_QUEUE,
        catchup_window=_FAST_CATCHUP, jitter=_FAST_JITTER,
    ),
    # HRRR forecast — every 15 min
    ScheduleDef(
        "ingest-hrrr", "IngestHrrrWorkflow",
        max_runtime=timedelta(minutes=45),
        interval=timedelta(minutes=15), task_queue=HRRR_TASK_QUEUE,
    ),
    # NAQFC air quality (PM2.5 + ozone) — cycles land twice daily; poll every
    # 30 min so a fresh cycle is picked up promptly. Non-new runs are a HEAD.
    ScheduleDef(
        "ingest-airquality", "IngestAirQualityWorkflow",
        max_runtime=timedelta(minutes=75),
        interval=timedelta(minutes=30),
    ),
    # Lightning WS consumer — every 60 min (workflow runs activity for ~50 min)
    ScheduleDef(
        "ingest-lightning", "IngestLightningWorkflow",
        max_runtime=timedelta(minutes=56), interval=timedelta(minutes=60),
    ),
    # NHC tropical cyclones — every 1 hour
    ScheduleDef(
        "ingest-tropical", "IngestTropicalWorkflow",
        max_runtime=timedelta(minutes=5), interval=timedelta(hours=1),
    ),
    # pysteps nowcast — every 2 min
    ScheduleDef(
        "nowcast", "NowcastWorkflow",
        max_runtime=timedelta(minutes=12),
        interval=timedelta(minutes=2), task_queue=NOWCAST_TASK_QUEUE,
        catchup_window=_FAST_CATCHUP, jitter=_FAST_JITTER,
    ),
    # Tile + grid cleanup — every 1 hour
    ScheduleDef(
        "tile-cleanup", "TileCleanupWorkflow",
        max_runtime=timedelta(minutes=15), interval=timedelta(hours=1),
    ),
    # NWS active alerts — every 5 min
    ScheduleDef(
        "poll-alerts", "PollAlertsWorkflow",
        max_runtime=timedelta(minutes=4, seconds=30),
        interval=timedelta(minutes=5), task_queue=ALERTS_TASK_QUEUE,
        catchup_window=timedelta(minutes=10),
    ),
    # Open-meteo GFS sync — every 6h. The legacy CronJob used "30 */6 * * *"
    # to align with GFS run lag, but Temporal SKIP overlap + --past-days=2
    # backfill make exact wall-clock alignment unnecessary; freshness is
    # bounded by the 6h interval regardless.
    ScheduleDef(
        "open-meteo-sync-gfs", "OpenMeteoSyncWorkflow",
        max_runtime=timedelta(minutes=60),
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
        max_runtime=timedelta(minutes=55),
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
    spec = ScheduleSpec(intervals=[ScheduleIntervalSpec(every=s.interval)], jitter=s.jitter)
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


# Per-RPC budget for schedule operations. A healthy create/update/delete
# answers in well under a second; a scheduler workflow wedged server-side
# (post cluster-rebuild state, 2026-08-25 `nowcast` incident) hangs the RPC
# until the client's default deadline instead. A short explicit timeout turns
# "wedged" into a detectable signal rather than a 100s+ stall per attempt.
_RPC_TIMEOUT = timedelta(seconds=15)

# Consecutive per-schedule timeouts before we conclude the schedule's backing
# temporal-sys-scheduler workflow is wedged and recreate it. Transient server
# churn clears in 1-2 attempts; a wedge never does.
_WEDGE_ATTEMPTS = 3

# The Rust SDK bridge surfaces client-side deadline hits as CANCELLED
# ("Timeout expired"), not just DEADLINE_EXCEEDED.
_TIMEOUT_CODES = frozenset({
    RPCStatusCode.CANCELLED,
    RPCStatusCode.DEADLINE_EXCEEDED,
})


async def _apply(client: Client, s: ScheduleDef, spec: Schedule) -> None:
    """Create-or-update one schedule, preserving live paused state."""
    try:
        await client.create_schedule(s.schedule_id, spec, rpc_timeout=_RPC_TIMEOUT)
        print(f"[seed] created schedule {s.schedule_id}")
    except ScheduleAlreadyRunningError:
        handle = client.get_schedule_handle(s.schedule_id)

        def _update(inp: ScheduleUpdateInput, desired: Schedule = spec) -> ScheduleUpdate:
            # Preserve the live state (paused flag + note): replacing the
            # whole Schedule with the desired spec silently UN-paused
            # schedules on every worker restart/deploy — pausing radar
            # ingest during an incident didn't survive the next rollout.
            return ScheduleUpdate(
                schedule=dataclasses.replace(desired, state=inp.description.schedule.state)
            )

        await handle.update(_update, rpc_timeout=_RPC_TIMEOUT)
        await _terminate_stale_actions(client, s, handle)
        print(f"[seed] updated schedule {s.schedule_id}")


async def _terminate_stale_actions(client: Client, s: ScheduleDef, handle: Any) -> None:
    """Close schedule actions that outlived the same bound used for new runs."""
    description = await handle.describe(rpc_timeout=_RPC_TIMEOUT)
    cutoff = datetime.now(timezone.utc) - s.max_runtime
    for action in description.info.running_actions:
        if not isinstance(action, ScheduleActionExecutionStartWorkflow):
            continue
        workflow_handle = client.get_workflow_handle(action.workflow_id)
        workflow_description = await workflow_handle.describe(
            rpc_timeout=_RPC_TIMEOUT
        )
        if workflow_description.start_time >= cutoff:
            continue
        try:
            await workflow_handle.terminate(
                reason=(
                    f"radar-ng schedule {s.schedule_id}: execution exceeded "
                    f"{s.max_runtime}"
                ),
                rpc_timeout=_RPC_TIMEOUT,
            )
            print(f"[seed] terminated stale action {action.workflow_id}")
        except RPCError as exc:
            if exc.status not in {
                RPCStatusCode.NOT_FOUND,
                RPCStatusCode.FAILED_PRECONDITION,
            }:
                raise


async def _recreate_wedged(client: Client, s: ScheduleDef, spec: Schedule) -> None:
    """Self-heal a schedule whose scheduler workflow no longer answers RPCs.

    Observed after the 2026-08-24 cluster rebuild: every RPC against the
    `nowcast` schedule (describe/update/delete — even from admintools) hung to
    deadline while the other 15 schedules answered instantly, so seeding died
    on it and crash-looped the worker 100+ times. The wedged state does not
    clear on its own; delete + recreate is the recovery. Losing the paused
    flag on that one schedule is acceptable — an unresponsive schedule cannot
    report its state anyway.
    """
    print(f"[seed] recreating wedged schedule {s.schedule_id}…")
    handle = client.get_schedule_handle(s.schedule_id)
    try:
        await handle.delete(rpc_timeout=_RPC_TIMEOUT)
    except RPCError as exc:
        if exc.status == RPCStatusCode.NOT_FOUND:
            pass  # already gone — just recreate
        elif exc.status in _TIMEOUT_CODES:
            # Delete rides through the same wedged workflow; terminate the
            # backing system workflow directly, then delete the husk.
            wf = client.get_workflow_handle(f"temporal-sys-scheduler:{s.schedule_id}")
            try:
                await wf.terminate(
                    reason="radar-ng seed: schedule RPCs wedged; recreating",
                    rpc_timeout=_RPC_TIMEOUT,
                )
            except RPCError as texc:
                if texc.status != RPCStatusCode.NOT_FOUND:
                    raise
            try:
                await handle.delete(rpc_timeout=_RPC_TIMEOUT)
            except RPCError as dexc:
                if dexc.status != RPCStatusCode.NOT_FOUND:
                    raise
        else:
            raise
    try:
        await client.create_schedule(s.schedule_id, spec, rpc_timeout=_RPC_TIMEOUT)
    except ScheduleAlreadyRunningError:
        # HA race: another replica won the delete+create; converged either way.
        print(f"[seed] schedule {s.schedule_id} already recreated by a peer")
        return
    print(f"[seed] recreated wedged schedule {s.schedule_id}")


async def seed(client: Client) -> None:
    """Single seeding pass over all schedules (no retries). Kept for the
    one-off `python -m temporal.schedules.seed` debugging path."""
    for s in SCHEDULES:
        await _apply(client, s, _spec_for(s))


async def seed_with_retry(
    client: Client,
    *,
    max_attempts: int = 10,
    base_delay: float = 1.0,
    max_delay: float = 20.0,
) -> None:
    """Seed all schedules, retrying transients and self-healing wedges.

    Per-schedule create-or-update is idempotent, so only the schedules that
    failed are retried on later passes. Two failure classes are handled:

    - Transient RPC errors (shard warmup after a Temporal restart, etc.):
      bounded exponential backoff, ~110s total budget, then re-raise so a
      genuinely-down Temporal still fails startup and k8s restarts the pod.
    - A wedged schedule (its scheduler workflow hangs every RPC to deadline,
      forever): after _WEDGE_ATTEMPTS consecutive timeouts on the SAME
      schedule, delete + recreate it instead of crash-looping the worker.
    """
    timeout_streak: dict[str, int] = {}
    pending = list(SCHEDULES)
    for attempt in range(1, max_attempts + 1):
        failed: list[ScheduleDef] = []
        last_exc: RPCError | None = None
        for s in pending:
            spec = _spec_for(s)
            try:
                await _apply(client, s, spec)
                timeout_streak.pop(s.schedule_id, None)
            except RPCError as exc:
                if exc.status not in _RETRYABLE_RPC_CODES:
                    raise
                if exc.status in _TIMEOUT_CODES:
                    streak = timeout_streak.get(s.schedule_id, 0) + 1
                    timeout_streak[s.schedule_id] = streak
                    if streak >= _WEDGE_ATTEMPTS:
                        await _recreate_wedged(client, s, spec)
                        timeout_streak.pop(s.schedule_id, None)
                        continue
                last_exc = exc
                failed.append(s)
                print(
                    f"[seed] transient RPC error ({exc.status.name}) on "
                    f"{s.schedule_id}, attempt {attempt}/{max_attempts}: "
                    f"{exc.message!r}"
                )
        if not failed:
            return
        if attempt == max_attempts:
            raise last_exc if last_exc else RuntimeError("schedule seeding failed")
        delay = min(max_delay, base_delay * 2 ** (attempt - 1))
        print(
            f"[seed] {len(failed)} schedule(s) still failing after attempt "
            f"{attempt}/{max_attempts}; retrying in {delay:.0f}s"
        )
        await asyncio.sleep(delay)
        pending = failed


async def _main() -> None:
    target = os.environ.get("TEMPORAL_ADDRESS", "localhost:7233")
    namespace = os.environ.get("TEMPORAL_NAMESPACE", "default")
    client = await Client.connect(target, namespace=namespace)
    await seed_with_retry(client)


if __name__ == "__main__":
    asyncio.run(_main())
