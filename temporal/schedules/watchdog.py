"""Read-only observation for Temporal Schedules whose timer may have stalled.

Temporal's scheduler timer can be dead-lettered while the Schedule itself
continues to describe successfully.  That leaves ``next_action_times[0]`` in
the past, no running action, and no new executions.  This observer detects
that shape and emits a structured critical event for an operator and the
alerting pipeline.

Observation is deliberately non-authoritative: it never triggers, updates,
deletes, or terminates anything.  A process remembers the exact overdue timer
it has reported so the one-minute loop does not flood logs.  That suppression
is intentionally lost on restart and is not shared between replicas; durable
deduplication belongs in the alerting system, not in Temporal state.
"""

from __future__ import annotations

import asyncio
from collections.abc import Callable, MutableMapping, Sequence
from datetime import datetime, timedelta, timezone

from loguru import logger
from temporalio.client import Client

from temporal.schedules.seed import ScheduleDef

CHECK_EVERY = timedelta(seconds=60)
RPC_TIMEOUT = timedelta(seconds=15)
STALL_EVENT = "TEMPORAL_SCHEDULE_STALLED"
# A fire is "overdue" once it is this many intervals late. Two keeps jitter,
# a slow workflow start, and clock skew out of the stalled verdict.
OVERDUE_INTERVALS = 2

# schedule ID -> the exact next-action timestamp already reported by this
# process.  Callers own the mapping so a fresh process naturally starts with
# no durable suppression state.
ReportedStalls = MutableMapping[str, datetime]


def is_wedged(
    next_action_times: Sequence[datetime],
    running_actions: Sequence[object],
    interval: timedelta,
    now: datetime,
) -> bool:
    """Return whether a Schedule looks stalled from one read-only snapshot."""
    if not next_action_times or running_actions:
        return False
    return next_action_times[0] < now - OVERDUE_INTERVALS * interval


async def check_schedule(
    client: Client,
    schedule: ScheduleDef,
    reported_stalls: ReportedStalls,
    *,
    now: datetime | None = None,
    rpc_timeout: timedelta = RPC_TIMEOUT,
) -> bool:
    """Describe one Schedule and emit at most one event per stalled timer.

    Returns ``True`` only when this call emitted a new critical event.  There
    is no ``await`` between checking and recording the timer fingerprint, so
    concurrent checks sharing ``reported_stalls`` cannot double-report in one
    asyncio process.
    """
    if schedule.interval is None:
        return False

    now = now or datetime.now(timezone.utc)
    handle = client.get_schedule_handle(schedule.schedule_id)
    description = await handle.describe(rpc_timeout=rpc_timeout)

    if description.schedule.state.paused:
        # An explicit operator pause is not a stall. If it is later resumed,
        # allow a still-overdue timer to produce a fresh event.
        reported_stalls.pop(schedule.schedule_id, None)
        return False

    info = description.info
    if not is_wedged(
        info.next_action_times,
        info.running_actions,
        schedule.interval,
        now,
    ):
        # A future timer proves that the previously reported timer advanced.
        # Keep suppression while an action is merely running: a manual action
        # does not prove the durable scheduler timer recovered.
        if info.next_action_times and info.next_action_times[0] > now:
            reported_stalls.pop(schedule.schedule_id, None)
        return False

    stalled_action_time = info.next_action_times[0]
    if reported_stalls.get(schedule.schedule_id) == stalled_action_time:
        return False

    # Record before logging. Loguru is synchronous, and this makes duplicate
    # concurrent checks converge before either can yield again.
    reported_stalls[schedule.schedule_id] = stalled_action_time
    overdue = now - stalled_action_time
    live_action = getattr(description.schedule, "action", None)
    workflow_name = getattr(live_action, "workflow", schedule.workflow_name)
    task_queue = getattr(live_action, "task_queue", schedule.task_queue)
    logger.bind(
        event=STALL_EVENT,
        schedule_id=schedule.schedule_id,
        workflow_name=workflow_name,
        task_queue=task_queue,
        next_action_time=stalled_action_time.isoformat(),
        overdue_seconds=overdue.total_seconds(),
        interval_seconds=schedule.interval.total_seconds(),
        automatic_recovery=False,
        operator_action="inspect_temporal_timer_dlq",
    ).critical(
        "{}: schedule {} is overdue with no running action; observer made no mutation",
        STALL_EVENT,
        schedule.schedule_id,
    )
    return True


async def _check_one(
    client: Client,
    schedule: ScheduleDef,
    reported_stalls: ReportedStalls,
    *,
    now: datetime | None,
) -> None:
    """Isolate one Schedule's read failure from every other observation."""
    try:
        await check_schedule(client, schedule, reported_stalls, now=now)
    except asyncio.CancelledError:
        raise
    except Exception as exc:  # noqa: BLE001 - the observer must outlive an RPC failure
        logger.bind(
            event="TEMPORAL_SCHEDULE_OBSERVATION_FAILED",
            schedule_id=schedule.schedule_id,
        ).warning("schedule observation failed: {!r}", exc)


async def check_schedules_once(
    client: Client,
    schedules: Sequence[ScheduleDef],
    reported_stalls: ReportedStalls,
    *,
    now: datetime | None = None,
) -> None:
    """Observe a snapshot concurrently so one slow Schedule cannot serialize it."""
    await asyncio.gather(
        *(
            _check_one(client, schedule, reported_stalls, now=now)
            for schedule in schedules
        )
    )


async def watch_schedules(
    client: Client,
    schedules: Sequence[ScheduleDef],
    *,
    check_every: timedelta = CHECK_EVERY,
    clock: Callable[[], datetime] | None = None,
) -> None:
    """Run forever, observing every Schedule without mutating Temporal state."""
    reported_stalls: dict[str, datetime] = {}
    while True:
        await asyncio.sleep(check_every.total_seconds())
        await check_schedules_once(
            client,
            schedules,
            reported_stalls,
            now=clock() if clock else None,
        )
