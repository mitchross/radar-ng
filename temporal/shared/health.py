"""Worker liveness via a touched file.

The SDK retries failed polls forever and silently: when cluster DNS died the
worker sat for four hours unable to reach the frontend, with no restart. This
loop calls the frontend's gRPC health check every ``HEALTH_EVERY`` and touches
``TEMPORAL_HEALTH_FILE`` (default ``/tmp/temporal-healthy``) only on success,
so a stale file means "cannot reach Temporal".

Intended k8s livenessProbe (fails after ~3 min without a successful check)::

    livenessProbe:
      exec:
        command: ["sh", "-c", "test $(find /tmp/temporal-healthy -mmin -3)"]
      initialDelaySeconds: 60
      periodSeconds: 30
      failureThreshold: 2
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


def touch(path: Path = HEALTH_FILE) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    path.touch()


async def check_once(client: Client, *, timeout: timedelta = HEALTH_RPC_TIMEOUT) -> bool:
    """One frontend health RPC; False (never raises) on any failure."""
    try:
        return bool(await client.service_client.check_health(timeout=timeout))
    except asyncio.CancelledError:
        raise
    except Exception as exc:  # noqa: BLE001 — a probe failure is data, not a crash
        logger.warning("temporal health check failed: {!r}", exc)
        return False


async def health_file_loop(
    client: Client,
    *,
    path: Path = HEALTH_FILE,
    every: timedelta = HEALTH_EVERY,
) -> None:
    """Run forever: touch `path` after each successful health check."""
    while True:
        if await check_once(client):
            try:
                touch(path)
            except OSError as exc:
                logger.error("cannot touch health file {}: {!r}", path, exc)
        await asyncio.sleep(every.total_seconds())
