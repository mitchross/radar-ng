"""Schedule watchdog: kick Temporal Schedules whose server-side timer was lost.

The Temporal server occasionally drops the `temporal-sys-scheduler:<id>`
workflow's timer, so a schedule sits with `next_action_times[0]` in the past,
no running action, and never fires again. Any RPC that reaches the scheduler
workflow (a `describe` is enough) wakes it, and `trigger` starts the overdue
run immediately. The seeding worker runs `watch_schedules` alongside the
Worker; a schedule is "wedged" when its next fire is more than two intervals
overdue and nothing is running. Triggers are rate-limited per schedule and
every RPC error is swallowed and logged — this loop must never take the
worker down with it.
"""

from __future__ import annotations

import asyncio
from collections.abc import Callable, Sequence
from datetime import datetime, timedelta, timezone

from loguru import logger
from temporalio.client import Client, ScheduleOverlapPolicy

from temporal.schedules.seed import ScheduleDef

CHECK_EVERY = timedelta(seconds=60)
RPC_TIMEOUT = timedelta(seconds=15)
MIN_TRIGGER_GAP = timedelta(minutes=5)
# A fire is "overdue" once it is this many intervals late. Two keeps jitter,
# a slow workflow start, and clock skew out of the wedged verdict.
OVERDUE_INTERVALS = 2


def is_wedged(
    next_action_times: Sequence[datetime],
    running_actions: Sequence[object],
    interval: timedelta,
    now: datetime,
) -> bool:
    """True when the next fire is >2 intervals in the past and nothing is running."""
    if not next_action_times or running_actions:
        return False
    return next_action_times[0] < now - OVERDUE_INTERVALS * interval


async def check_schedule(
    client: Client,
    s: ScheduleDef,
    last_triggered: dict[str, datetime],
    *,
    now: datetime | None = None,
    rpc_timeout: timedelta = RPC_TIMEOUT,
    min_trigger_gap: timedelta = MIN_TRIGGER_GAP,
) -> bool:
    """Describe one schedule and trigger it if wedged. Returns True if triggered."""
    if s.interval is None:
        return False
    now = now or datetime.now(timezone.utc)
    handle = client.get_schedule_handle(s.schedule_id)
    desc = await handle.describe(rpc_timeout=rpc_timeout)
    if desc.schedule.state.paused:
        return False
    info = desc.info
    if not is_wedged(info.next_action_times, info.running_actions, s.interval, now):
        return False
    last = last_triggered.get(s.schedule_id)
    if last is not None and now - last < min_trigger_gap:
        return False
    logger.error(
        "schedule {} wedged (next fire {} overdue, no running action); triggering",
        s.schedule_id,
        now - info.next_action_times[0],
    )
    await handle.trigger(overlap=ScheduleOverlapPolicy.SKIP, rpc_timeout=rpc_timeout)
    last_triggered[s.schedule_id] = now
    return True


async def watch_schedules(
    client: Client,
    schedules: Sequence[ScheduleDef],
    *,
    check_every: timedelta = CHECK_EVERY,
    clock: Callable[[], datetime] | None = None,
) -> None:
    """Run forever: check every schedule each `check_every`, never raise."""
    last_triggered: dict[str, datetime] = {}
    while True:
        await asyncio.sleep(check_every.total_seconds())
        for s in schedules:
            try:
                await check_schedule(
                    client, s, last_triggered, now=clock() if clock else None
                )
            except asyncio.CancelledError:
                raise
            except Exception as exc:  # noqa: BLE001 — watchdog must outlive any RPC failure
                logger.warning("schedule watchdog: {} check failed: {!r}", s.schedule_id, exc)
