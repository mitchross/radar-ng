#!/usr/bin/env python3
"""GRIB decode benchmark, Python side.

--kind mrms-gz     the production MRMS path: backend.ingest_mrms.activities._download_and_decode_sync
                   (gunzip to a temp file, pygrib open, values, 1-D axes), then the caller's
                   south-up flip from process_frame. The HTTP client is a local stub that
                   returns the file bytes, so no network is involved.
--kind grib-multi  pygrib over every message of a multi-message file (Lambert HRRR-like).

Each iteration decodes the whole file. The digest (sha256 of the little-endian float32
values, and of the axes for mrms-gz) is written from the last iteration for verify.py.
"""

from __future__ import annotations

import argparse
import shutil
import sys
import time
from pathlib import Path

import numpy as np

sys.path.insert(0, str(Path(__file__).resolve().parent))
from common import INPUTS, OUT, RADAR_NG, machine_info, measure, proc_status_mb  # noqa: E402
from common import sha256_bytes, sha256_file, summarize, write_json  # noqa: E402


class _Resp:
    def __init__(self, content: bytes) -> None:
        self.content = content

    def raise_for_status(self) -> None:
        return None


class _StubClient:
    """Stands in for httpx.Client: get() returns the local file bytes."""

    def __init__(self, content: bytes) -> None:
        self._content = content

    def get(self, url: str, timeout: float | None = None) -> _Resp:
        return _Resp(self._content)


def decode_mrms_gz(path: Path, tmp_root: Path, i: int, client_content: bytes) -> dict:
    from backend.ingest_mrms import activities as ing  # production module

    tmp = tmp_root / f"it{i}"
    tmp.mkdir(parents=True, exist_ok=True)
    try:
        data, lat, lon = ing._download_and_decode_sync(_StubClient(client_content), path.name, tmp)
    finally:
        shutil.rmtree(tmp, ignore_errors=True)
    # process_frame: flip south-up when the first latitude is north of the last.
    if lat[0] > lat[-1]:
        lat = lat[::-1]
        data = np.flipud(data)
    data = np.ascontiguousarray(data, dtype="<f4")
    return {
        "shape": list(data.shape),
        "nan_cells": int(np.isnan(data).sum()),
        "sha256_values": sha256_bytes(data.tobytes()),
        "sha256_lats": sha256_bytes(np.ascontiguousarray(lat, dtype="<f8").tobytes()),
        "sha256_lons": sha256_bytes(np.ascontiguousarray(lon, dtype="<f8").tobytes()),
        "lat_first": float(lat[0]), "lat_last": float(lat[-1]),
        "lon_first": float(lon[0]), "lon_last": float(lon[-1]),
    }


def decode_grib_multi(path: Path) -> dict:
    import pygrib

    digests = []
    grbs = pygrib.open(str(path))
    try:
        for k, msg in enumerate(grbs.read(), start=1):
            vals = msg.values
            if hasattr(vals, "filled"):
                vals = vals.filled(np.nan)
            arr = np.ascontiguousarray(np.asarray(vals).astype("<f4"))
            digests.append({"message": k, "name": msg.name, "shape": list(arr.shape),
                            "nan_cells": int(np.isnan(arr).sum()),
                            "sha256_values": sha256_bytes(arr.tobytes())})
    finally:
        grbs.close()
    return {"messages": digests}


def main() -> None:
    ap = argparse.ArgumentParser()
    ap.add_argument("--kind", choices=["mrms-gz", "grib-multi"], required=True)
    ap.add_argument("--file", type=Path, required=True)
    ap.add_argument("--label", required=True)
    ap.add_argument("--iters", type=int, default=5)
    ap.add_argument("--warmup", type=int, default=1)
    ap.add_argument("--json", type=Path, required=True)
    args = ap.parse_args()

    sys.path.insert(0, str(RADAR_NG))
    tmp_root = OUT / "grib" / "py-tmp"
    tmp_root.mkdir(parents=True, exist_ok=True)
    content = args.file.read_bytes() if args.kind == "mrms-gz" else b""
    after_load = proc_status_mb()
    last: dict = {}

    def one(i: int) -> dict:
        if args.kind == "mrms-gz":
            d = decode_mrms_gz(args.file, tmp_root, i, content)
        else:
            d = decode_grib_multi(args.file)
        last["d"] = d
        return {"messages": len(d["messages"]) if "messages" in d else 1}

    records = measure(one, args.warmup, args.iters)
    peak = proc_status_mb()
    write_json(args.json, {
        "bench": "grib",
        "case": args.label,
        "impl": "python",
        "label": "Python (pygrib %s + production decode)" % __import__("pygrib").__version__,
        "machine": machine_info(),
        "input": {"file": args.file.name, "bytes": args.file.stat().st_size,
                  "sha256": sha256_file(args.file)},
        "config": {"kind": args.kind, "warmup": args.warmup, "iters": args.iters},
        "records": records,
        "summary": summarize(records),
        "digest": last["d"],
        "memory": {"after_load_rss_mb": after_load["rss_mb"], "peak_rss_mb": peak["peak_rss_mb"]},
    })
    s = summarize(records)
    print(f"python grib {args.label}: median {s['wall_s_median']:.4f}s peak_rss={peak['peak_rss_mb']:.0f}MB",
          flush=True)


if __name__ == "__main__":
    main()
