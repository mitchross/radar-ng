#!/usr/bin/env python3
"""Build the served fixture for the API load test (bench 5).

Writes one tree that both servers read unchanged:

  FIXTURE/tiles/radar/<palette>/<frame ts>/<z>/<x>/<y>.png   pyramid, 8 frames (TILE_DIR)
  FIXTURE/state/manifest.json                                  production manifest schema (STATE_DIR)
  FIXTURE/grids/nowcast/runs/<run>/<ts>.*                      run-scoped nowcast grids (GRID_DIR)

Tiles: rendered by the Rust tile runner (bench_tile, same pyramid as bench 1: 3500x7000 MRMS
scene t0, z4-7, classic/muted/vivid). Every frame reuses that tree through hardlinks (no extra
disk). A second scene (t1) was tried and dropped: it has a different set of non-empty tiles
(576 vs 585), so some mix URLs would 404 for reasons unrelated to the servers.

Nowcast: the bench 4 Rust forecast dump (12 leads, 875x1750, f32), leads 5/10/15 min written
through the production backend.shared.grid_dump.write_grid, so the Python and Rust servers
sample the same run-scoped files.

Timestamps are relative to now so the grids are fresh when the server reads them.
"""

from __future__ import annotations

import argparse
import datetime as dt
import json
import os
import shutil
import subprocess
import sys
from pathlib import Path

HERE = Path(__file__).resolve().parent
BENCH = HERE.parent
RADAR_NG = Path(os.environ.get("RADAR_NG", "."))
PALETTES = ["classic", "muted", "vivid"]
N_FRAMES = 8
FRAME_STEP_MIN = 2
NOWCAST_LEADS = [5, 10, 15]
NOWCAST_STEP_MIN = 5
NOWCAST_H, NOWCAST_W = 875, 1750
NOWCAST_LEADS_AVAILABLE = 12


def ts_fmt(t: dt.datetime) -> str:
    return t.strftime("%Y%m%dT%H%M%SZ")


def render_scene(bench_tile: Path, inputs: Path, scratch: Path, which: str) -> Path:
    """Render one MRMS scene (t0 or t1) with the Rust tile runner and return its tree.

    bench_tile always reads mrms_3500x7000_t0.f32, so the t1 scene gets its own input
    directory whose t0 entry is a symlink to the t1 file (no change to the runner).
    """
    src_dir = scratch / f"in_{which}"
    if src_dir.exists():
        shutil.rmtree(src_dir)
    src_dir.mkdir(parents=True)
    (src_dir / "mrms_3500x7000_t0.f32").symlink_to(inputs / f"mrms_3500x7000_{which}.f32")
    for name in ("lats_3500.f64", "lons_7000.f64", "manifest.json"):
        (src_dir / name).symlink_to(inputs / name)
    out = scratch / f"render_{which}"
    if out.exists():
        shutil.rmtree(out)
    out.mkdir(parents=True)
    subprocess.run(
        [str(bench_tile), "--input-dir", str(src_dir), "--out-root", str(out),
         "--iters", "1", "--warmup", "0", "--json", str(out / "render.json")],
        check=True,
    )
    tree = out / "iter0"
    if not tree.is_dir():
        raise SystemExit(f"bench_tile did not keep iter0 under {out}")
    return tree


def link_tree(src: Path, dst: Path) -> None:
    import os as _os
    dst.parent.mkdir(parents=True, exist_ok=True)
    shutil.copytree(src, dst, copy_function=_os.link)


def main() -> None:
    ap = argparse.ArgumentParser()
    ap.add_argument("--fixture", type=Path, default=BENCH / "work" / "api_fixture")
    ap.add_argument("--bench-tile", type=Path,
                    default=Path(os.environ.get("CARGO_TARGET_DIR", "rust/target"))
                    / "release" / "bench_tile")
    ap.add_argument("--inputs", type=Path, default=BENCH / "work" / "inputs")
    ap.add_argument("--forecast", type=Path,
                    default=BENCH / "work" / "out" / "nowcast" / f"rust_{NOWCAST_H}x{NOWCAST_W}_fc.f32")
    args = ap.parse_args()
    fx: Path = args.fixture.resolve()
    tiles_root = fx / "tiles"
    state = fx / "state"
    grids = fx / "grids"
    scratch = fx / "scratch"
    for d in (tiles_root, state, grids):
        d.mkdir(parents=True, exist_ok=True)

    # ---- tiles -------------------------------------------------------------
    if not args.bench_tile.exists():
        raise SystemExit(f"bench_tile not built: {args.bench_tile}")
    scene_tree = render_scene(args.bench_tile, args.inputs, scratch, "t0")

    now = dt.datetime.now(dt.timezone.utc)
    latest = now.replace(second=0, microsecond=0)
    latest -= dt.timedelta(minutes=latest.minute % FRAME_STEP_MIN)
    frame_times = [latest - dt.timedelta(minutes=FRAME_STEP_MIN * i) for i in range(N_FRAMES)][::-1]
    frames = []
    for i, t in enumerate(frame_times):
        ts = ts_fmt(t)
        for p in PALETTES:
            link_tree(scene_tree / p / "ts", tiles_root / "radar" / p / ts)
        frames.append({
            "timestamp": ts,
            "path": ts,
            "palettes": list(PALETTES),
        })
    shutil.rmtree(scratch, ignore_errors=True)

    # ---- nowcast grids (through the production writer) ---------------------
    import numpy as np
    os.environ["GRID_DIR"] = str(grids)
    sys.path.insert(0, str(RADAR_NG))
    from backend.shared.grid_dump import write_grid  # noqa: E402
    from backend.shared.manifest import TILE_URL_TEMPLATE, TILE_URL_TEMPLATE_LEGACY  # noqa: E402

    fc = np.fromfile(args.forecast, dtype="<f4")
    per_lead = NOWCAST_H * NOWCAST_W
    if fc.size != NOWCAST_LEADS_AVAILABLE * per_lead:
        raise SystemExit(f"forecast dump has {fc.size} values, expected {NOWCAST_LEADS_AVAILABLE}x{per_lead}")
    fc = fc.reshape(NOWCAST_LEADS_AVAILABLE, NOWCAST_H, NOWCAST_W)
    lats = np.fromfile(args.inputs / "lats_3500.f64", dtype="<f8")[::4]
    lons = np.fromfile(args.inputs / "lons_7000.f64", dtype="<f8")[::4]
    assert lats.size == NOWCAST_H and lons.size == NOWCAST_W, (lats.size, lons.size)

    run = f"bench{ts_fmt(latest)}"
    nowcast_frames = []
    for lead_min in NOWCAST_LEADS:
        lead_idx = lead_min // NOWCAST_STEP_MIN - 1
        t = latest + dt.timedelta(minutes=lead_min)
        ts = ts_fmt(t)
        field = fc[lead_idx].astype(np.float32)
        field = np.where(field <= -900.0, np.nan, field)
        key = f"runs/{run}/{ts}"
        written = write_grid("nowcast", ts, field, lats, lons, "dBZ",
                             fill=float("nan"), max_cells=NOWCAST_H * NOWCAST_W, grid_key=key)
        if written is None:
            raise SystemExit(f"write_grid skipped lead {lead_min}")
        nowcast_frames.append({
            "timestamp": ts,
            "path": ts,
            "palettes": [],
            "lead_minutes": lead_min,
            "grid_key": key,
            "spatial_resolution_km": 4.4,
        })

    # ---- manifest (production schema, as _normalize_frame/_sync_layer_indexes write it) ----
    manifest = {
        "schema_version": 1,
        "updated_at": now.isoformat(),
        "tile_url_template": TILE_URL_TEMPLATE,
        "tile_url_template_legacy": TILE_URL_TEMPLATE_LEGACY,
        "layers": {
            "radar": {
                "latest": frames[-1]["timestamp"],
                "timestamps": [f["timestamp"] for f in frames],
                "palettes": list(PALETTES),
                "unit": "dBZ",
                "frames": frames,
            },
            "nowcast": {
                "run_id": run,
                "latest": nowcast_frames[-1]["timestamp"],
                "timestamps": [f["timestamp"] for f in nowcast_frames],
                "palettes": [],
                "method": "pysteps-sprog",
                "horizon_minutes": NOWCAST_LEADS[-1],
                "step_minutes": NOWCAST_STEP_MIN,
                "frames": nowcast_frames,
            },
        },
    }
    (state / "manifest.json").write_text(json.dumps(manifest, indent=2) + "\n")

    n_png = sum(1 for _ in tiles_root.rglob("*.png"))
    summary = {
        "fixture": str(fx),
        "radar_frames": len(frames),
        "radar_palettes": PALETTES,
        "tile_pngs": n_png,
        "nowcast_frames": len(nowcast_frames),
        "nowcast_run": run,
        "latest_radar": frames[-1]["timestamp"],
    }
    print(json.dumps(summary))


if __name__ == "__main__":
    main()
