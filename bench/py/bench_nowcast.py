#!/usr/bin/env python3
"""Nowcast benchmark, Python side: the production _run_nowcast from backend/nowcast/activities.py.

_run_nowcast does the work the nowcast_run activity does for one forecast: stack the frames,
replace values < -100 with NaN, estimate motion with pysteps LK on a stride-2 stack (scipy
upsampling, scaled by the stride), install the fast CDF match, and run pysteps S-PROG with
the last three frames, 6 cascade levels, precip_thr 5 dBZ and 12 leads (60 min / 5 min).

The only stub is _write_nowcast_status, which writes the state file under /data. It is
not part of the forecast computation and is not timed on either side.
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


def main() -> None:
    ap = argparse.ArgumentParser()
    ap.add_argument("--shape", required=True, help="HxW, e.g. 1750x3500")
    ap.add_argument("--iters", type=int, default=2)
    ap.add_argument("--warmup", type=int, default=0)
    ap.add_argument("--json", type=Path, required=True)
    args = ap.parse_args()
    h, w = (int(v) for v in args.shape.split("x"))

    sys.path.insert(0, str(RADAR_NG))
    from backend.nowcast import activities as act  # production module

    act._write_nowcast_status = lambda *a, **k: None  # state file under /data; not measured

    frame_paths = [INPUTS / f"nowcast_{h}x{w}_t{t}.f32" for t in range(4)]
    frames = [np.fromfile(p, dtype="<f4").reshape(h, w) for p in frame_paths]
    lead_steps = [((i + 1) * act.STEP_MIN) / act.INPUT_STEP_MIN
                  for i in range(act.HORIZON_MIN // act.STEP_MIN)]
    after_load = proc_status_mb()
    last: dict = {}

    def one(i: int) -> dict:
        forecast, method = act._run_nowcast(frames, lead_steps)
        if method != "pysteps-sprog" or forecast is None:
            raise RuntimeError(f"expected pysteps-sprog, got {method!r}")
        last["fc"] = forecast
        return {"method": method, "leads": int(forecast.shape[0])}

    t0 = time.perf_counter()
    records = measure(one, args.warmup, args.iters)
    peak = proc_status_mb()
    fc = last["fc"]
    dump = OUT / "nowcast" / f"py_{h}x{w}_fc.f32"
    dump.parent.mkdir(parents=True, exist_ok=True)
    np.ascontiguousarray(fc, dtype="<f4").tofile(dump)
    write_json(args.json, {
        "bench": "nowcast",
        "impl": "python",
        "label": "Python (pysteps 1.21.5 LK stride 2 + S-PROG, production _run_nowcast)",
        "machine": machine_info(),
        "input": {"shape": [h, w], "frames": 4,
                  "sha256": [sha256_file(p) for p in frame_paths]},
        "config": {"stride": act.MOTION_STRIDE, "leads": len(lead_steps), "lead_steps": lead_steps,
                   "n_cascade_levels": 6, "precip_thr": 5.0, "warmup": args.warmup, "iters": args.iters},
        "records": records,
        "summary": summarize(records),
        "forecast_file": str(dump),
        "forecast_shape": list(fc.shape),
        "memory": {"after_load_rss_mb": after_load["rss_mb"], "peak_rss_mb": peak["peak_rss_mb"]},
        "total_wall_s": time.perf_counter() - t0,
    })
    s = summarize(records)
    print(f"python nowcast {h}x{w}: median {s['wall_s_median']:.2f}s cold {s['cold_wall_s']} "
          f"peak_rss={peak['peak_rss_mb']:.0f}MB", flush=True)


if __name__ == "__main__":
    main()
