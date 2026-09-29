"""Separate local event-loop liveness from Temporal connectivity readiness.

Neither file proves that the SDK is polling. Inspect SDK poll metrics and
controller registration when connectivity succeeds but work stops progressing.
"""

from __future__ import annotations

import asyncio
import os
from datetime import timedelta
from pathlib import Path

from loguru import logger
from temporalio.client import Client

HEALTH_EVERY = timedelta(seconds=30)
HEALTH_RPC_TIMEOUT = timedelta(seconds=10)
HEALTH_FILE = Path(os.environ.get("TEMPORAL_HEALTH_FILE", "/tmp/temporal-healthy"))
ALIVE_FILE = Path(os.environ.get("TEMPORAL_ALIVE_FILE", "/tmp/worker-alive"))


def touch(path: Path = HEALTH_FILE) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    path.touch()


HEALTH_RETRY_DELAY = timedelta(seconds=1)


async def check_once(
    client: Client,
    *,
    timeout: timedelta = HEALTH_RPC_TIMEOUT,
    retries: int = 1,
    retry_delay: timedelta = HEALTH_RETRY_DELAY,
) -> bool:
    """Frontend health RPC, retried once; False (never raises) on any failure.

    ~2% of calls on the shared worker connection end in a transient client-side
    Cancelled; warn only when the retry fails too, so a real outage still shows.
    """
    for attempt in range(retries + 1):
        try:
            return bool(await client.service_client.check_health(timeout=timeout))
        except asyncio.CancelledError:
            raise
        except Exception as exc:  # noqa: BLE001 — a probe failure is data, not a crash
            if attempt == retries:
                logger.warning("temporal health check failed: {!r}", exc)
                return False
            logger.debug("temporal health check retrying after: {!r}", exc)
            await asyncio.sleep(retry_delay.total_seconds())
    return False


async def health_file_loop(
    client: Client,
    *,
    path: Path = HEALTH_FILE,
    alive_path: Path = ALIVE_FILE,
    every: timedelta = HEALTH_EVERY,
) -> None:
    """Refresh liveness each iteration, readiness only after a successful RPC."""
    while True:
        try:
            touch(alive_path)
        except OSError as exc:
            logger.error("cannot touch liveness file {}: {!r}", alive_path, exc)
        if await check_once(client):
            try:
                touch(path)
            except OSError as exc:
                logger.error("cannot touch health file {}: {!r}", path, exc)
        await asyncio.sleep(every.total_seconds())
