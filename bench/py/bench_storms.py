#!/usr/bin/env python3
"""Storm detection benchmark, Python side: backend.shared.storms.detect_storms with tracking.

Each iteration runs two frames, as the production loop does: frame A with no previous
storms.json, then frame B (5 minutes later) with frame A's document as `previous`, which
exercises the tracking, motion fit and assignment. Timing covers both calls.
"""

from __future__ import annotations

import argparse
import sys
import time
from pathlib import Path

import numpy as np

sys.path.insert(0, str(Path(__file__).resolve().parent))
from common import INPUTS, OUT, RADAR_NG, load_input_manifest, machine_info, measure  # noqa: E402
from common import proc_status_mb, sha256_file, summarize, write_json  # noqa: E402

TS_A = "2026-10-14T00:00:00+00:00"
TS_B = "2026-10-14T00:05:00+00:00"


def normalise(doc: dict) -> dict:
    feats = []
    for f in doc["features"]:
        p = f["properties"]
        feats.append({
            "cell_id": int(p["cell_id"]),
            "peak_dbz": float(p["peak_dbz"]),
            "pixel_count": int(p["pixel_count"]),
            "area_km2": float(p["area_km2"]),
            "bbox": [float(v) for v in p["bbox"]],
            "coordinates": [float(v) for v in f["geometry"]["coordinates"]],
        })
    return {"n_features": len(feats), "features": feats}


def main() -> None:
    ap = argparse.ArgumentParser()
    ap.add_argument("--iters", type=int, default=5)
    ap.add_argument("--warmup", type=int, default=1)
    ap.add_argument("--json", type=Path, required=True)
    args = ap.parse_args()

    sys.path.insert(0, str(RADAR_NG))
    from backend.shared import storms  # production module

    man = load_input_manifest()
    h, w = man["mrms_grid"]["rows"], man["mrms_grid"]["cols"]
    a = np.fromfile(INPUTS / "mrms_3500x7000_t0.f32", dtype="<f4").reshape(h, w)
    b = np.fromfile(INPUTS / "mrms_3500x7000_t1.f32", dtype="<f4").reshape(h, w)
    lats = np.fromfile(INPUTS / "lats_3500.f64", dtype="<f8")
    lons = np.fromfile(INPUTS / "lons_7000.f64", dtype="<f8")
    after_load = proc_status_mb()
    last: dict = {}

    def one(i: int) -> dict:
        doc_a = storms.detect_storms(a, lats, lons, timestamp=TS_A, previous=None)
        doc_b = storms.detect_storms(b, lats, lons, timestamp=TS_B, previous=doc_a)
        last["a"], last["b"] = doc_a, doc_b
        return {"features_a": len(doc_a["features"]), "features_b": len(doc_b["features"])}

    t0 = time.perf_counter()
    records = measure(one, args.warmup, args.iters)
    peak = proc_status_mb()
    summary_path = OUT / "storms" / "py_summary.json"
    write_json(summary_path, {"frame_a": normalise(last["a"]), "frame_b": normalise(last["b"])})
    write_json(args.json, {
        "bench": "storms",
        "impl": "python",
        "label": "Python (backend.shared.storms.detect_storms + tracking)",
        "machine": machine_info(),
        "input": {"files": ["mrms_3500x7000_t0.f32", "mrms_3500x7000_t1.f32"], "shape": [h, w],
                  "sha256_t0": sha256_file(INPUTS / "mrms_3500x7000_t0.f32"),
                  "sha256_t1": sha256_file(INPUTS / "mrms_3500x7000_t1.f32")},
        "config": {"threshold_dbz": storms.THRESHOLD_DBZ, "min_pixels": storms.MIN_PIXELS,
                   "max_storms": storms.MAX_STORMS, "warmup": args.warmup, "iters": args.iters},
        "records": records,
        "summary": summarize(records),
        "features_a": records[-1]["features_a"],
        "features_b": records[-1]["features_b"],
        "memory": {"after_load_rss_mb": after_load["rss_mb"], "peak_rss_mb": peak["peak_rss_mb"]},
        "summary_file": str(summary_path),
        "total_wall_s": time.perf_counter() - t0,
    })
    s = summarize(records)
    print(f"python storms: median {s['wall_s_median']:.3f}s features_a={records[-1]['features_a']} "
          f"features_b={records[-1]['features_b']} peak_rss={peak['peak_rss_mb']:.0f}MB", flush=True)


if __name__ == "__main__":
    main()
