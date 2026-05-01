"""Open-meteo sync activity — runs the upstream Swift binary as a subprocess.

The activity is registered ONLY by the secondary `radar-ng-open-meteo-worker`
deployment (deploy/k8s/open-meteo-worker-deployment.yaml). That worker's
image is `FROM ghcr.io/open-meteo/open-meteo:latest` with a thin Python +
temporalio layer on top. The main `radar-ng-temporal-worker` does NOT
register this activity — Temporal dispatches it to the open-meteo worker
based on registration.

This is the same "one Temporal worker per concern" pattern news-reader
uses. No Kubernetes Jobs are involved; the binary runs in-process inside
the worker's own pod.
"""

from __future__ import annotations

import asyncio
import os
import subprocess
import time
from collections import deque
from dataclasses import dataclass
from pathlib import Path

from temporalio import activity
from temporalio.exceptions import ApplicationError


# Path to the open-meteo binary inside the open-meteo worker image. Verified
# at /app/openmeteo-api in ghcr.io/open-meteo/open-meteo:latest (Feb 2026).
OPENMETEO_BIN = os.environ.get("OPENMETEO_BIN", "/app/openmeteo-api")
OPENMETEO_DATA_DIR = os.environ.get("OPENMETEO_DATA_DIR", "/app/data")


@dataclass
class OpenMeteoSyncArgs:
    model: str            # "ncep_gfs025" | "ncep_hrrr_conus"
    variables: str        # comma-separated list passed as the third arg
    past_days: int = 1


@dataclass
class OpenMeteoSyncResult:
    model: str
    succeeded: bool
    duration_s: float
    return_code: int = 0
    stderr_tail: str = ""


@activity.defn(name="open_meteo_sync")
async def open_meteo_sync(args: OpenMeteoSyncArgs) -> OpenMeteoSyncResult:
    """Run `openmeteo-api sync <model> <variables> --past-days <n>` and
    stream output. Heartbeats every 30s while the process is running.
    """
    started = time.time()
    cmd = [OPENMETEO_BIN, "sync", args.model, args.variables, "--past-days", str(args.past_days)]
    activity.logger.info("starting open-meteo sync: %s", " ".join(cmd))

    proc = await asyncio.create_subprocess_exec(
        *cmd,
        cwd=str(Path(OPENMETEO_DATA_DIR).parent),
        stdout=asyncio.subprocess.PIPE,
        stderr=asyncio.subprocess.PIPE,
    )

    stderr_tail: deque[str] = deque(maxlen=50)

    async def _drain(stream: asyncio.StreamReader, sink: deque[str] | None) -> None:
        while True:
            line = await stream.readline()
            if not line:
                break
            decoded = line.decode("utf-8", errors="replace").rstrip()
            if sink is not None:
                sink.append(decoded)
            activity.logger.debug(decoded)

    drain_out = asyncio.create_task(_drain(proc.stdout, None))
    drain_err = asyncio.create_task(_drain(proc.stderr, stderr_tail))

    last_heartbeat = 0.0
    while proc.returncode is None:
        try:
            await asyncio.wait_for(proc.wait(), timeout=15)
        except asyncio.TimeoutError:
            pass
        now = time.monotonic()
        if now - last_heartbeat >= 30:
            activity.heartbeat({"model": args.model, "elapsed_s": int(time.time() - started)})
            last_heartbeat = now

    await drain_out
    await drain_err
    duration = round(time.time() - started, 1)

    rc = proc.returncode or 0
    if rc != 0:
        tail = "\n".join(stderr_tail)
        raise ApplicationError(
            f"open-meteo {args.model} sync exited rc={rc}: {tail[-500:]}",
            non_retryable=False,
        )

    activity.logger.info("open-meteo %s sync ok in %.1fs", args.model, duration)
    return OpenMeteoSyncResult(model=args.model, succeeded=True, duration_s=duration, return_code=rc)
