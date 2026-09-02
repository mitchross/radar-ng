"""Benchmark: legacy RGBA-per-palette render vs render-once indexed pipeline.

Synthetic MRMS-sized grid (3500 x 7000 float32, CONUS bbox) through both
paths for the shipped palettes; prints wall time, tile counts and bytes.

    uv run --no-project --python 3.12 --with 'numpy<2' --with scipy --with Pillow \
        --with pyproj python -m backend.scripts.bench_render_once [--zooms 4,5,6,7] [--skip-old]
"""

from __future__ import annotations

import argparse
import json
import os
import platform
import shutil
import tempfile
import time
import tracemalloc
from pathlib import Path

import numpy as np

from backend.shared import tiler
from backend.shared.tiler import (
    apply_color_table,
    render_frame_palettes,
    render_tiles_atomic,
)

PALETTES_DIR = Path(__file__).resolve().parents[1] / "shared" / "palettes"


def synthetic_grid(
    h: int = 3500, w: int = 7000, seed: int = 1
) -> tuple[np.ndarray, np.ndarray, np.ndarray]:
    """Blobby echoes over ~30 % of CONUS; NaN elsewhere (MRMS-like coverage)."""
    rng = np.random.default_rng(seed)
    lats = np.linspace(20.0, 55.0, h)
    lons = np.linspace(-130.0, -60.0, w)
    data = np.full((h, w), np.nan, dtype=np.float32)
    yy = np.arange(h, dtype=np.float32)[:, None]
    xx = np.arange(w, dtype=np.float32)[None, :]
    for _ in range(40):
        cy, cx = rng.uniform(300, h - 300), rng.uniform(300, w - 300)
        ry, rx = rng.uniform(60, 400), rng.uniform(60, 600)
        peak = rng.uniform(25, 70)
        d = ((yy - cy) / ry) ** 2 + ((xx - cx) / rx) ** 2
        inside = d < 1.0
        vals = (peak * (1.0 - np.sqrt(d, where=inside, out=np.ones_like(d)))).astype(
            np.float32
        )
        data = np.where(inside & (vals > np.nan_to_num(data, nan=-1.0)), vals, data)
    return data, lats, lons


def _du(path: Path) -> tuple[int, int]:
    files = list(path.rglob("*.png"))
    return len(files), sum(f.stat().st_size for f in files)


def run_old(data, lats, lons, tables, out: Path, zooms) -> dict:
    t0 = time.perf_counter()
    tracemalloc.start()
    for name, table in tables.items():
        rgba = apply_color_table(data, table)
        render_tiles_atomic(rgba, lats, lons, str(out / name / "ts"), zooms)
        del rgba
    _, peak = tracemalloc.get_traced_memory()
    tracemalloc.stop()
    n, size = _du(out)
    return {
        "wall_s": time.perf_counter() - t0,
        "tiles": n,
        "bytes": size,
        "py_peak_mb": peak / 2**20,
    }


def run_new(data, lats, lons, tables, out: Path, zooms) -> dict:
    t0 = time.perf_counter()
    tracemalloc.start()
    render_frame_palettes(
        data, lats, lons, tables, {n: str(out / n / "ts") for n in tables}, zooms
    )
    _, peak = tracemalloc.get_traced_memory()
    tracemalloc.stop()
    n, size = _du(out)
    return {
        "wall_s": time.perf_counter() - t0,
        "tiles": n,
        "bytes": size,
        "py_peak_mb": peak / 2**20,
    }


def main() -> None:
    ap = argparse.ArgumentParser()
    ap.add_argument("--zooms", default="4,5,6,7")
    ap.add_argument("--shape", default="3500x7000")
    ap.add_argument("--skip-old", action="store_true")
    ap.add_argument(
        "--repeat-new",
        type=int,
        default=2,
        help="second run shows the warm geometry cache",
    )
    args = ap.parse_args()
    zooms = [int(z) for z in args.zooms.split(",")]
    h, w = (int(v) for v in args.shape.split("x"))

    tables = {
        p: json.loads((PALETTES_DIR / f"{p}.json").read_text())["reflectivity"]
        for p in ("classic", "muted", "vivid")
    }
    t0 = time.perf_counter()
    data, lats, lons = synthetic_grid(h, w)
    print(
        f"machine: {platform.node()} {platform.machine()} cpus={os.cpu_count()} numpy={np.__version__}"
    )
    print(
        f"grid: {h}x{w} float32 ({data.nbytes / 2**20:.0f} MB), {np.isfinite(data).mean() * 100:.0f}% covered, "
        f"zooms={zooms}, palettes={list(tables)}, png_level={tiler.PNG_COMPRESS_LEVEL}  (synth {time.perf_counter() - t0:.1f}s)"
    )

    root = Path(tempfile.mkdtemp(prefix="bench-render-once-"))
    try:
        if not args.skip_old:
            r = run_old(data, lats, lons, tables, root / "old", zooms)
            print(
                f"old  (RGBA x{len(tables)} palettes, serial): {r['wall_s']:6.2f}s  tiles={r['tiles']}  "
                f"bytes={r['bytes'] / 2**20:.1f}MB  py_peak={r['py_peak_mb']:.0f}MB"
            )
        for i in range(args.repeat_new):
            tiler.clear_tile_geometry_cache() if i == 0 else None
            r = run_new(data, lats, lons, tables, root / f"new{i}", zooms)
            label = "cold geometry cache" if i == 0 else "warm geometry cache"
            print(
                f"new  (indexed, {label:19s}): {r['wall_s']:6.2f}s  tiles={r['tiles']}  "
                f"bytes={r['bytes'] / 2**20:.1f}MB  py_peak={r['py_peak_mb']:.0f}MB"
            )
    finally:
        shutil.rmtree(root, ignore_errors=True)


if __name__ == "__main__":
    main()
