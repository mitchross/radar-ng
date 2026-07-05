"""CLI entrypoint. See package docstring / docs/debug-harness.md."""

from __future__ import annotations

import argparse
import asyncio
import os
import pathlib
import sys
import time

from . import __version__
from .checks import (
    DEFAULT_LAT,
    DEFAULT_LON,
    check_api,
    check_client_sim,
    check_pipeline,
    check_tiles,
)
from .core import Check, eprint, exit_code, render_checks
from .disk_checks import check_disk
from .temporal_checks import check_temporal


def _parser() -> argparse.ArgumentParser:
    # Shared flags live on a parent parser attached to every subcommand, so
    # both `doctor --server X` and plain `--help` discovery work naturally.
    common = argparse.ArgumentParser(add_help=False)
    common.add_argument(
        "--server",
        default=os.environ.get("RADAR_DEBUG_SERVER", "http://localhost:8080"),
        help="tile-server base URL, the same one the app uses (env RADAR_DEBUG_SERVER)",
    )
    common.add_argument(
        "--temporal-address",
        default=os.environ.get("TEMPORAL_ADDRESS", "localhost:7233"),
        help="Temporal frontend host:port (env TEMPORAL_ADDRESS)",
    )
    common.add_argument(
        "--namespace",
        default=os.environ.get("TEMPORAL_NAMESPACE", "default"),
        help="Temporal namespace (env TEMPORAL_NAMESPACE)",
    )
    common.add_argument("--json", action="store_true", help="machine-readable output")
    common.add_argument("--lat", type=float, default=DEFAULT_LAT, help="probe latitude")
    common.add_argument("--lon", type=float, default=DEFAULT_LON, help="probe longitude")

    p = argparse.ArgumentParser(
        prog="python -m tools.debug_harness",
        description="radar-ng debug harness — inspect a live stack for performance and health issues.",
    )
    p.add_argument("--version", action="version", version=__version__)

    sub = p.add_subparsers(dest="command", required=True)
    sub.add_parser("doctor", parents=[common],
                   help="full sweep: api + tiles + pipeline + client + temporal")
    api = sub.add_parser("api", parents=[common], help="API endpoint latency percentiles")
    api.add_argument("--samples", type=int, default=5)
    sub.add_parser("tiles", parents=[common], help="tile sampling: latency, 404s, cache headers")
    sub.add_parser("pipeline", parents=[common], help="per-layer data freshness vs schedule cadence")
    cl = sub.add_parser("client", parents=[common], help="simulate the app's radar playback fetch pattern")
    cl.add_argument("--zoom", type=int, default=7)
    cl.add_argument("--frames", type=int, default=5)
    cl.add_argument("--layer", default="radar")
    tp = sub.add_parser("temporal", parents=[common], help="schedules, recent failures, stuck workflows")
    tp.add_argument("--failure-window-h", type=int, default=6)
    sub.add_parser(
        "disk", parents=[common],
        help="PVC usage, orphaned .tmp renders, manifest-vs-disk drift (run inside a pod)",
    )
    watch = sub.add_parser("watch", parents=[common], help="run doctor on a loop")
    watch.add_argument("--interval", type=int, default=60, help="seconds between sweeps")
    watch.add_argument("--skip-temporal", action="store_true")
    return p


def _run_doctor(args: argparse.Namespace, *, skip_temporal: bool = False) -> list[Check]:
    sections: list[tuple[str, list[Check]]] = [
        ("pipeline — data freshness", check_pipeline(args.server)),
        ("api — endpoint latency", check_api(args.server)),
        ("tiles — serving path", check_tiles(args.server, lat=args.lat, lon=args.lon)),
        ("client — playback simulation", check_client_sim(args.server, lat=args.lat, lon=args.lon)),
    ]
    # Disk checks only make sense where the data volumes are mounted; skip
    # silently on a laptop pointed at a remote server.
    if any(pathlib.Path(os.environ.get(v, d)).is_dir() for v, d in
           (("TILE_DIR", "/data/tiles"), ("STATE_DIR", "/data/state"))):
        sections.append(("disk — volumes and state", check_disk()))
    if not skip_temporal:
        sections.append((
            "temporal — control plane",
            asyncio.run(check_temporal(args.temporal_address, args.namespace)),
        ))
    all_checks: list[Check] = []
    for title, checks in sections:
        render_checks(title, checks, as_json=args.json)
        all_checks.extend(checks)
    return all_checks


def main(argv: list[str] | None = None) -> int:
    args = _parser().parse_args(argv)

    if args.command == "doctor":
        checks = _run_doctor(args)
        if not args.json:
            fails = sum(1 for c in checks if c.status == "fail")
            warns = sum(1 for c in checks if c.status == "warn")
            print(f"\n{len(checks)} checks · {fails} fail · {warns} warn")
        return exit_code(checks)

    if args.command == "api":
        checks = check_api(args.server, samples=args.samples)
        render_checks("api — endpoint latency", checks, as_json=args.json)
        return exit_code(checks)

    if args.command == "tiles":
        checks = check_tiles(args.server, lat=args.lat, lon=args.lon)
        render_checks("tiles — serving path", checks, as_json=args.json)
        return exit_code(checks)

    if args.command == "pipeline":
        checks = check_pipeline(args.server)
        render_checks("pipeline — data freshness", checks, as_json=args.json)
        return exit_code(checks)

    if args.command == "client":
        checks = check_client_sim(
            args.server, lat=args.lat, lon=args.lon,
            zoom=args.zoom, frames=args.frames, layer=args.layer,
        )
        render_checks("client — playback simulation", checks, as_json=args.json)
        return exit_code(checks)

    if args.command == "disk":
        checks = check_disk()
        render_checks("disk — volumes and state", checks, as_json=args.json)
        return exit_code(checks)

    if args.command == "temporal":
        checks = asyncio.run(check_temporal(
            args.temporal_address, args.namespace,
            failure_window_h=args.failure_window_h,
        ))
        render_checks("temporal — control plane", checks, as_json=args.json)
        return exit_code(checks)

    if args.command == "watch":
        try:
            while True:
                started = time.time()
                print(f"\n──── sweep @ {time.strftime('%H:%M:%S')} ────")
                _run_doctor(args, skip_temporal=args.skip_temporal)
                elapsed = time.time() - started
                time.sleep(max(0, args.interval - elapsed))
        except KeyboardInterrupt:
            eprint("\nstopped")
            return 0

    return 2


if __name__ == "__main__":
    sys.exit(main())
