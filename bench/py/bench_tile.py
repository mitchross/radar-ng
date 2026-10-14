#!/usr/bin/env python3
"""Tile render benchmark, Python side: backend.shared.tiler.render_frame_palettes.

Mirrors the production MRMS call (backend/ingest_mrms/activities.py, _render_and_publish):
palettes classic/muted/vivid, reflectivity table, zooms 4-7, overview="max", nodata None,
min_valid_weight 1.0, publication lock root = the output root.

--renderer legacy   what production runs by default (TILE_RENDERER=legacy, RGBA per palette)
--renderer indexed  the algorithm the Rust port implements (indexed PNG, render once)

Each iteration writes a fresh pyramid tree (the marker check would otherwise skip the render).
The last iteration's tree is kept under $BENCH_WORK/out/tile/py-<renderer>/iter<N> for verify.py.
"""

from __future__ import annotations

import argparse
import json
import shutil
import sys
import time
from pathlib import Path

import numpy as np

sys.path.insert(0, str(Path(__file__).resolve().parent))
from common import (INPUTS, OUT, RADAR_NG, count_pngs, load_input_manifest, machine_info,  # noqa: E402
                    measure, proc_status_mb, sha256_file, summarize, write_json, RESULTS_RAW)

PALETTES = ("classic", "muted", "vivid")
ZOOMS = [4, 5, 6, 7]


def main() -> None:
    ap = argparse.ArgumentParser()
    ap.add_argument("--renderer", choices=["legacy", "indexed"], required=True)
    ap.add_argument("--iters", type=int, default=5)
    ap.add_argument("--warmup", type=int, default=1)
    ap.add_argument("--json", type=Path, required=True)
    args = ap.parse_args()

    sys.path.insert(0, str(RADAR_NG))
    from backend.shared import tiler  # production module, imported from the read-only tree

    man = load_input_manifest()
    h, w = man["mrms_grid"]["rows"], man["mrms_grid"]["cols"]
    src = INPUTS / "mrms_3500x7000_t0.f32"
    data = np.fromfile(src, dtype="<f4").reshape(h, w)
    lats = np.fromfile(INPUTS / "lats_3500.f64", dtype="<f8")
    lons = np.fromfile(INPUTS / "lons_7000.f64", dtype="<f8")
    tables = {
        p: json.loads((RADAR_NG / "backend/shared/palettes" / f"{p}.json").read_text())["reflectivity"]
        for p in PALETTES
    }
    after_load = proc_status_mb()

    root = OUT / "tile" / f"py-{args.renderer}"
    shutil.rmtree(root, ignore_errors=True)
    root.mkdir(parents=True)

    def one(i: int) -> dict:
        iter_root = root / f"iter{i}"
        outdirs = {p: str(iter_root / p / "ts") for p in PALETTES}
        res = tiler.render_frame_palettes(
            data, lats, lons, tables, outdirs, ZOOMS,
            nodata_value=None,
            min_valid_weight=1.0,
            renderer=args.renderer,
            overview="max",
            source_id="mrms:bench:t0",
            publication_lock_root=str(iter_root),
        )
        n, nbytes = count_pngs(iter_root)
        return {"tiles": n, "bytes": nbytes, "rendered": sorted(res.rendered_palettes)}

    t0 = time.perf_counter()
    records = measure(one, args.warmup, args.iters)
    peak = proc_status_mb()
    for i in range(args.warmup + args.iters - 1):  # keep only the last tree, outside the timing
        shutil.rmtree(root / f"iter{i}", ignore_errors=True)
    write_json(args.json, {
        "bench": "tile",
        "impl": f"python-{args.renderer}",
        "label": {"legacy": "Python (production default, legacy RGBA)",
                  "indexed": "Python (indexed, same algorithm as Rust)"}[args.renderer],
        "machine": machine_info(),
        "input": {"file": src.name, "sha256": sha256_file(src), "shape": [h, w],
                  "manifest_sha256": man["files"][src.name]["sha256"]},
        "config": {"palettes": list(PALETTES), "zooms": ZOOMS, "overview": "max",
                   "png_compress_level": tiler.PNG_COMPRESS_LEVEL, "renderer": args.renderer,
                   "warmup": args.warmup, "iters": args.iters},
        "records": records,
        "summary": summarize(records),
        "tiles": records[-1]["tiles"],
        "bytes": records[-1]["bytes"],
        "memory": {"after_load_rss_mb": after_load["rss_mb"], "peak_rss_mb": peak["peak_rss_mb"]},
        "output_root": str(root),
        "total_wall_s": time.perf_counter() - t0,
    })
    print(f"python {args.renderer}: median {summarize(records)['wall_s_median']:.2f}s "
          f"tiles={records[-1]['tiles']} peak_rss={peak['peak_rss_mb']:.0f}MB", flush=True)


if __name__ == "__main__":
    main()
