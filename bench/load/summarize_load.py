#!/usr/bin/env python3
"""Summarise one bench 5 run directory (output of run_load.sh) into summary.json and summary.md.

For each target (python, rust) and each run (rate200, rate500, rate1000, max):
  - achieved rps, success rate, non-200 status counts (from oha JSON)
  - p50 / p90 / p95 / p99 latency in ms (oha latencyPercentiles, seconds in the file)
  - server CPU as % of its pinned core (delta utime+stime over the sampler window)
  - CPU ms per request, server RSS peak (VmHWM) and final RSS (from the sampler)

    python3 summarize_load.py bench/results/load/<ts>
"""

from __future__ import annotations

import json
import sys
from pathlib import Path

RUNS = [("rate200", "200 rps"), ("rate500", "500 rps"), ("rate1000", "1000 rps"), ("max", "max (-c)")]
TARGETS = [("python", "FastAPI + StaticFiles"), ("rust", "radar-api")]


def load_json(path: Path):
    try:
        return json.loads(path.read_text())
    except (FileNotFoundError, json.JSONDecodeError):
        return None


def proc_stats(path: Path):
    d = load_json(path)
    if not d or len(d.get("samples", [])) < 2:
        return None
    s = d["samples"]
    dt = s[-1]["t"] - s[0]["t"]
    dcpu = s[-1]["cpu_s"] - s[0]["cpu_s"]
    return {
        "window_s": round(dt, 2),
        "cpu_pct_of_core": round(100.0 * dcpu / dt, 1) if dt > 0 else None,
        "cpu_s": round(dcpu, 3),
        "rss_mb_last": round(s[-1]["rss_mb"], 1),
        "hwm_mb_peak": round(max(x["hwm_mb"] for x in s), 1),
    }


def run_row(load_dir: Path, target: str, run: str):
    oha = load_json(load_dir / f"{target}_{run}.oha.json")
    proc = proc_stats(load_dir / f"{target}_{run}.proc.json")
    if oha is None:
        return {"status": "missing"}
    summ = oha["summary"]
    pct = oha.get("latencyPercentiles", {})
    total = int(round(summ["total"] * summ["requestsPerSec"])) if summ.get("requestsPerSec") else 0
    codes = oha.get("statusCodeDistribution", {}) or {}
    row = {
        "requests": sum(codes.values()) if codes else total,
        "rps": round(summ["requestsPerSec"], 1),
        "success_rate": round(summ["successRate"], 5),
        "status": codes,
        # oha counts requests still in flight at the end of the window as "aborted due to deadline".
        # They are not server errors; they are reported separately.
        "in_flight_at_deadline": sum(v for k, v in (oha.get("errorDistribution", {}) or {}).items()
                                     if "deadline" in k),
        "errors": {k: v for k, v in (oha.get("errorDistribution", {}) or {}).items() if "deadline" not in k},
        "p50_ms": round(pct.get("p50", 0) * 1000, 2),
        "p90_ms": round(pct.get("p90", 0) * 1000, 2),
        "p95_ms": round(pct.get("p95", 0) * 1000, 2),
        "p99_ms": round(pct.get("p99", 0) * 1000, 2),
        "avg_ms": round(summ["average"] * 1000, 2),
        "mb_per_s": round(summ["sizePerSec"] / 1e6, 2),
    }
    if proc:
        row["server"] = proc
        n = row["requests"]
        row["cpu_ms_per_req"] = round(1000.0 * proc["cpu_s"] / n, 3) if n else None
    return row


def main() -> None:
    load_dir = Path(sys.argv[1]).resolve()
    out = {"dir": str(load_dir), "targets": {}}
    for target, _ in TARGETS:
        out["targets"][target] = {run: run_row(load_dir, target, run) for run, _ in RUNS}
    (load_dir / "summary.json").write_text(json.dumps(out, indent=2) + "\n")

    lines = ["# API load (bench 5)", "",
             "Same fixture, same request list (local endpoints, tile URLs that exist), server pinned "
             "to one core, oha and the sampler to another. Latency in ms; CPU is % of the server's "
             "pinned core.", ""]
    lines += ["| target | run | achieved rps | success | p50 | p90 | p95 | p99 | server CPU % core | CPU ms/req | RSS peak MB | non-200 |",
              "|---|---|---:|---:|---:|---:|---:|---:|---:|---:|---:|---|"]
    for target, label in TARGETS:
        for run, run_label in RUNS:
            r = out["targets"][target][run]
            if r.get("status") == "missing":
                lines.append(f"| {label} | {run_label} | missing |  |  |  |  |  |  |  |  |  |")
                continue
            srv = r.get("server", {})
            non200 = {k: v for k, v in r["status"].items() if k != "200"}
            non200_s = ", ".join(f"{k}: {v}" for k, v in non200.items()) or "none"
            errs = sum(r["errors"].values()) if r["errors"] else 0
            if errs:
                non200_s += f"; transport errors {errs}"
            if r.get("in_flight_at_deadline"):
                non200_s += f"; {r['in_flight_at_deadline']} in flight at deadline"
            lines.append(
                f"| {label} | {run_label} | {r['rps']} | {r['success_rate']*100:.2f}% | "
                f"{r['p50_ms']} | {r['p90_ms']} | {r['p95_ms']} | {r['p99_ms']} | "
                f"{srv.get('cpu_pct_of_core', '')} | {r.get('cpu_ms_per_req', '')} | "
                f"{srv.get('hwm_mb_peak', '')} | {non200_s} |"
            )
    (load_dir / "summary.md").write_text("\n".join(lines) + "\n")
    print("\n".join(lines))


if __name__ == "__main__":
    main()
