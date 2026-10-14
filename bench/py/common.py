"""Shared helpers for the Python side of the radar-ng before/after benchmark.

Measurement rules (identical on the Rust side, see crates/radar-bench/src/lib.rs):
  * wall time: time.perf_counter() around one call.
  * CPU time: getrusage(RUSAGE_SELF) user + system, i.e. all threads of the process.
  * memory: VmHWM (peak resident set) and VmRSS from /proc/self/status, in MB.
  * each process runs `warmup` untimed-for-summary iterations first, then `iters` timed
    iterations. The warmup is reported separately so cold and warm costs are both visible.
"""

from __future__ import annotations

import hashlib
import json
import os
import platform
import resource
import time
from pathlib import Path
from typing import Callable

BENCH_DIR = Path(__file__).resolve().parents[1]
WORK = Path(os.environ.get("BENCH_WORK", BENCH_DIR / "work"))
INPUTS = WORK / "inputs"
OUT = WORK / "out"
RESULTS_RAW = WORK / "raw"
RADAR_NG = Path(os.environ.get("RADAR_NG", "."))
RUST_ROOT = Path(os.environ.get("RUST_ROOT", "rust"))


def sha256_file(path: Path, chunk: int = 1 << 20) -> str:
    h = hashlib.sha256()
    with open(path, "rb") as f:
        while block := f.read(chunk):
            h.update(block)
    return h.hexdigest()


def sha256_bytes(data: bytes) -> str:
    return hashlib.sha256(data).hexdigest()


def cpu_seconds() -> float:
    ru = resource.getrusage(resource.RUSAGE_SELF)
    return ru.ru_utime + ru.ru_stime


def proc_status_mb() -> dict[str, float]:
    """VmRSS and VmHWM (peak) of this process in MB, from /proc/self/status."""
    out: dict[str, float] = {}
    with open("/proc/self/status") as f:
        for line in f:
            if line.startswith(("VmRSS:", "VmHWM:")):
                key, value = line.split(":", 1)
                out[key.lower()] = float(value.split()[0]) / 1024.0
    return {"rss_mb": out.get("vmrss", 0.0), "peak_rss_mb": out.get("vmhwm", 0.0)}


def measure(fn: Callable[[int], dict | None], warmup: int, iters: int) -> list[dict]:
    """Run fn(i) for i in range(warmup + iters); return one record per call."""
    records: list[dict] = []
    for i in range(warmup + iters):
        wall0 = time.perf_counter()
        cpu0 = cpu_seconds()
        extra = fn(i) or {}
        wall = time.perf_counter() - wall0
        cpu = cpu_seconds() - cpu0
        rec = {"phase": "warmup" if i < warmup else "iter", "index": i, "wall_s": wall, "cpu_s": cpu}
        rec.update(extra)
        records.append(rec)
    return records


def percentile(values: list[float], q: float) -> float:
    """Linear-interpolation percentile (numpy default), q in [0, 100]."""
    xs = sorted(values)
    if not xs:
        return float("nan")
    k = (len(xs) - 1) * q / 100.0
    lo = int(k)
    hi = min(lo + 1, len(xs) - 1)
    return xs[lo] + (xs[hi] - xs[lo]) * (k - lo)


def summarize(records: list[dict]) -> dict:
    timed = [r for r in records if r["phase"] == "iter"] or records
    walls = [r["wall_s"] for r in timed]
    cpus = [r["cpu_s"] for r in timed]
    warm = [r for r in records if r["phase"] == "warmup"]
    return {
        "n": len(timed),
        "wall_s_median": percentile(walls, 50),
        "wall_s_p90": percentile(walls, 90),
        "wall_s_min": min(walls),
        "cpu_s_median": percentile(cpus, 50),
        "cpu_s_p90": percentile(cpus, 90),
        "cold_wall_s": warm[0]["wall_s"] if warm else None,
    }


def machine_info() -> dict:
    model = "unknown"
    try:
        with open("/proc/cpuinfo") as f:
            for line in f:
                if line.startswith("model name"):
                    model = line.split(":", 1)[1].strip()
                    break
    except OSError:
        pass
    mem_mb = None
    try:
        with open("/proc/meminfo") as f:
            for line in f:
                if line.startswith("MemTotal:"):
                    mem_mb = int(line.split()[1]) // 1024
                    break
    except OSError:
        pass
    return {
        "cpu_model": model,
        "logical_cpus": os.cpu_count(),
        "mem_total_mb": mem_mb,
        "kernel": platform.release(),
        "python": platform.python_version(),
    }


def write_json(path: Path, obj: object) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    tmp = path.with_suffix(path.suffix + ".tmp")
    tmp.write_text(json.dumps(obj, indent=2, sort_keys=False) + "\n")
    tmp.replace(path)


def load_input_manifest() -> dict:
    return json.loads((INPUTS / "manifest.json").read_text())


def count_pngs(root: Path) -> tuple[int, int]:
    n = 0
    size = 0
    for p in root.rglob("*.png"):
        n += 1
        size += p.stat().st_size
    return n, size
