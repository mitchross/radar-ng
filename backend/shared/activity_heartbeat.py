"""Helpers for keeping Temporal activity heartbeats alive during thread work."""

from __future__ import annotations

import asyncio
from collections.abc import Callable
from contextlib import suppress
from typing import TypeVar

from temporalio import activity

T = TypeVar("T")
HeartbeatDetails = dict | Callable[[], dict]


async def run_sync_with_heartbeat(
    func: Callable[..., T],
    *args: object,
    heartbeat_every: float,
    heartbeat_details: HeartbeatDetails,
) -> T:
    """Run blocking work in a thread while heartbeating from the activity loop."""
    work = asyncio.create_task(asyncio.to_thread(func, *args))

    async def _beat() -> None:
        while not work.done():
            await asyncio.sleep(heartbeat_every)
            if not work.done():
                details = heartbeat_details() if callable(heartbeat_details) else heartbeat_details
                activity.heartbeat(details)

    beat = asyncio.create_task(_beat())
    try:
        return await work
    finally:
        beat.cancel()
        with suppress(asyncio.CancelledError):
            await beat
