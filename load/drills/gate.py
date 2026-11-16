#!/usr/bin/env python3
"""Checks the #293 load and failure pass rules over one time window. Stdlib only.

  python3 load/drills/gate.py --prom http://kube-prometheus-stack-prometheus.prometheus-stack.svc.cluster.local:9090 \
      --mrms https://radar-ng-api.vanillax.me --start 2026-12-01T15:00:00Z --end 2026-12-01T15:20:00Z \
      --level2-cadence-s 268 --k6 k6-pod-*.json

Rules (docs/reference/level2-launch-gate.md):
  1. Observed age never over 600 s: MRMS (`radar_ng_mrms_age_seconds`) and, when k6 summaries
     are given, the Level II age every k6 user saw (`level2_observed_age_s` max).
  2. Temporal schedule-to-start never over one measured source cadence, on every radar-ng queue,
     for activity and workflow tasks. The MRMS cadence is the median gap between MRMS frames
     in the manifest; the Level II cadence comes from the shadow recorder (--level2-cadence-s).
     The SDK histogram is in milliseconds with fixed buckets, so the check uses the largest bucket
     at or below the cadence: zero tasks above it is a pass; any task above it is inconclusive
     (exit 2) until the task's own history shows its real delay.
  3. With k6 summaries, every per-pod threshold passed.

Exit 0 pass, 1 fail, 2 inconclusive.
"""

import argparse
import glob
import json
import statistics
import sys
import urllib.parse
import urllib.request
from datetime import datetime, timezone

HISTOGRAMS = ["temporal_activity_schedule_to_start_latency", "temporal_workflow_task_schedule_to_start_latency"]


def fetch(url):
    req = urllib.request.Request(url, headers={"User-Agent": "radar-ng-level2-gate/1"})
    with urllib.request.urlopen(req, timeout=20) as r:
        return json.loads(r.read())


def ts(text):
    return datetime.fromisoformat(text.replace("Z", "+00:00")).timestamp()


def query(prom, q, at):
    url = f"{prom.rstrip('/')}/api/v1/query?{urllib.parse.urlencode({'query': q, 'time': at})}"
    return fetch(url)["data"]["result"]


def mrms_cadence_s(mrms):
    m = fetch(f"{mrms.rstrip('/')}/api/manifest.json")
    times = sorted(ts(t) for t in m["layers"]["radar"]["timestamps"])
    gaps = [b - a for a, b in zip(times, times[1:]) if b > a]
    return statistics.median(gaps) if gaps else None


def buckets(prom, metric, at):
    res = query(prom, f'count by (le) ({metric}_bucket{{task_queue=~"radar-ng.*"}})', at)
    return sorted((float(r["metric"]["le"]), r["metric"]["le"]) for r in res if r["metric"]["le"] != "+Inf")


def k6_summaries(patterns):
    rows = []
    for pattern in patterns:
        for path in sorted(glob.glob(pattern)):
            with open(path) as fh:
                for line in fh:
                    line = line.strip()
                    if line.startswith("{") and '"metrics"' in line:
                        rows.append((path, json.loads(line)))
    return rows


def main():
    p = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    p.add_argument("--prom", required=True)
    p.add_argument("--mrms", required=True, help="host serving /api/manifest.json, for the MRMS cadence")
    p.add_argument("--start", required=True, help="UTC ISO time the run or drill started")
    p.add_argument("--end", required=True, help="UTC ISO time it ended (after recovery)")
    p.add_argument("--level2-cadence-s", type=float, default=None, help="median low-sweep gap from the recorder")
    p.add_argument("--k6", nargs="*", default=[], help="files or globs holding k6 per-pod summary lines")
    args = p.parse_args()

    start, end = ts(args.start), ts(args.end)
    window = f"{max(60, int(end - start))}s"
    seen = set()
    report = []

    def mark(ok, line, inconclusive=False):
        tag = "PASS" if ok else ("INCONCLUSIVE" if inconclusive else "FAIL")
        seen.add(tag)
        report.append(f"{tag:12} {line}")

    res = query(args.prom, f"max_over_time(radar_ng_mrms_age_seconds[{window}])", end)
    mrms_max = max((float(r["value"][1]) for r in res), default=None)
    if mrms_max is None:
        mark(False, "MRMS age: no radar_ng_mrms_age_seconds samples in the window", inconclusive=True)
    else:
        mark(mrms_max <= 600, f"MRMS age max {mrms_max:.0f} s (limit 600 s)")

    pods = k6_summaries(args.k6)
    if args.k6 and not pods:
        mark(False, "k6: no summary lines found in the given files", inconclusive=True)
    if pods:
        ages = [d["metrics"].get("level2_observed_age_s", {}).get("values", {}).get("max") for _, d in pods]
        ages = [a for a in ages if a is not None]
        if ages:
            mark(max(ages) <= 600, f"Level II observed age max {max(ages):.0f} s across {len(pods)} pods (limit 600 s)")
        else:
            mark(False, "Level II observed age: no ready manifest was read by any pod", inconclusive=True)
        crossed = sorted(
            {f"{name}:{t}" for _, d in pods for name, m in d["metrics"].items() for t, o in (m.get("thresholds") or {}).items() if not o.get("ok")}
        )
        mark(not crossed, f"k6 thresholds: {'all passed' if not crossed else ', '.join(crossed)}")

    cadences = {"mrms": mrms_cadence_s(args.mrms)}
    if args.level2_cadence_s:
        cadences["level2"] = args.level2_cadence_s
    measured = {k: v for k, v in cadences.items() if v}
    if not measured:
        mark(False, "cadence: none measured", inconclusive=True)
    else:
        cadence = min(measured.values())
        report.append(f"{'INFO':12} measured cadence " + ", ".join(f"{k} {v:.0f} s" for k, v in measured.items()) + f"; rule uses {cadence:.0f} s")
        if "level2" not in measured:
            report.append(f"{'INFO':12} no --level2-cadence-s: the radar-ng-level2 queue is held to the MRMS cadence")
        for metric in HISTOGRAMS:
            les = [b for b in buckets(args.prom, metric, end) if b[0] <= cadence * 1000]
            if not les:
                mark(False, f"{metric}: no bucket at or below {cadence:.0f} s", inconclusive=True)
                continue
            le, le_label = les[-1]
            q = (
                f'sum by (task_queue) (increase({metric}_bucket{{task_queue=~"radar-ng.*",le="+Inf"}}[{window}]))'
                f' - sum by (task_queue) (increase({metric}_bucket{{task_queue=~"radar-ng.*",le="{le_label}"}}[{window}]))'
            )
            for r in query(args.prom, q, end):
                over = float(r["value"][1])
                queue = r["metric"].get("task_queue", "?")
                line = f"{metric} {queue}: {over:.1f} tasks over {le / 1000:.0f} s (cadence {cadence:.0f} s)"
                mark(over < 0.5, line, inconclusive=over >= 0.5)

    print(f"window {args.start} .. {args.end} ({window})")
    print("\n".join(report))
    verdict = 1 if "FAIL" in seen else 2 if "INCONCLUSIVE" in seen else 0
    print({0: "RESULT pass", 1: "RESULT fail", 2: "RESULT inconclusive"}[verdict])
    sys.exit(verdict)


if __name__ == "__main__":
    main()
