#!/usr/bin/env python3
"""Sample CPU and memory of one process from /proc while a load run is going.

    python3 sample_proc.py --pid 1234 --seconds 32 --out samples.json

Writes one sample per second: utime+stime (seconds, from /proc/<pid>/stat), VmRSS and VmHWM
(MB, from /proc/<pid>/status). The caller computes CPU% from the first and last sample.
"""

from __future__ import annotations

import argparse
import json
import os
import time

CLK_TCK = os.sysconf("SC_CLK_TCK")


def read_cpu_seconds(pid: int) -> float:
    with open(f"/proc/{pid}/stat") as f:
        fields = f.read().rsplit(")", 1)[1].split()
    # fields[11] = utime, fields[12] = stime (fields after the command name, 0-based here)
    return (int(fields[11]) + int(fields[12])) / CLK_TCK


def read_mem_mb(pid: int) -> tuple[float, float]:
    rss = hwm = 0.0
    with open(f"/proc/{pid}/status") as f:
        for line in f:
            if line.startswith("VmRSS:"):
                rss = int(line.split()[1]) / 1024
            elif line.startswith("VmHWM:"):
                hwm = int(line.split()[1]) / 1024
    return rss, hwm


def main() -> None:
    ap = argparse.ArgumentParser()
    ap.add_argument("--pid", type=int, required=True)
    ap.add_argument("--seconds", type=float, required=True)
    ap.add_argument("--out", required=True)
    args = ap.parse_args()
    samples = []
    t0 = time.time()
    while time.time() - t0 < args.seconds:
        try:
            cpu = read_cpu_seconds(args.pid)
            rss, hwm = read_mem_mb(args.pid)
        except (FileNotFoundError, ProcessLookupError):
            break
        samples.append({"t": round(time.time() - t0, 2), "cpu_s": cpu, "rss_mb": rss, "hwm_mb": hwm})
        time.sleep(1.0)
    with open(args.out, "w") as f:
        json.dump({"clk_tck": CLK_TCK, "samples": samples}, f, indent=1)


if __name__ == "__main__":
    main()
