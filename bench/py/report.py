#!/usr/bin/env python3
"""Combine work/raw/*.json into bench/results/<timestamp>.json and <timestamp>.md.

Speedup = Python median wall time / Rust median wall time (higher is better for Rust).
RSS ratio = Python peak RSS / Rust peak RSS (process peak, VmHWM; higher means leaner Rust).
CPU ratio = Python CPU median / Rust CPU median (user + sys of the process).
"""

from __future__ import annotations

import json
import sys
from datetime import datetime, timezone
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
from common import BENCH_DIR, RESULTS_RAW, write_json  # noqa: E402


def maybe(name: str) -> dict | None:
    p = RESULTS_RAW / f"{name}.json"
    return json.loads(p.read_text()) if p.exists() else None


def ratio(a: float | None, b: float | None) -> str:
    if not a or not b:
        return "n/a"
    return f"{a / b:.2f}x"


def fmt(x: float | None, nd: int = 3) -> str:
    return "n/a" if x is None else f"{x:.{nd}f}"


def row_of(rec: dict) -> dict:
    s = rec["summary"]
    return {
        "wall_median_s": s["wall_s_median"], "wall_p90_s": s["wall_s_p90"],
        "cold_wall_s": s.get("cold_wall_s"), "cpu_median_s": s["cpu_s_median"],
        "cpu_p90_s": s["cpu_s_p90"], "n": s["n"],
        "peak_rss_mb": rec["memory"]["peak_rss_mb"],
        "after_load_rss_mb": rec["memory"]["after_load_rss_mb"],
    }


def load_for(prefix: str) -> str:
    """Mean and max 1-minute load sampled while steps whose label starts with prefix ran."""
    steps_p, load_p = RESULTS_RAW / "steps.log", RESULTS_RAW / "load.log"
    if not (steps_p.exists() and load_p.exists()):
        return "n/a"
    steps = []
    for line in steps_p.read_text().splitlines():
        if " " in line:
            ts, label = line.split(" ", 1)
            steps.append((int(ts), label))
    samples = []
    for line in load_p.read_text().splitlines():
        parts = line.split()
        if len(parts) == 2:
            samples.append((int(parts[0]), float(parts[1])))
    vals: list[float] = []
    for i, (t0, label) in enumerate(steps):
        if not label.startswith(prefix):
            continue
        t1 = steps[i + 1][0] if i + 1 < len(steps) else 2**62
        vals += [l for t, l in samples if t0 <= t < t1]
    if not vals:
        return "n/a"
    return f"mean {sum(vals) / len(vals):.2f}, max {max(vals):.2f}"


def main() -> None:
    stamp = datetime.now(timezone.utc).strftime("%Y%m%dT%H%M%SZ")
    raw = {p.stem: json.loads(p.read_text()) for p in RESULTS_RAW.glob("*.json") if p.stem != "verify"}
    verify = maybe("verify") or {}
    md: list[str] = []
    out: dict = {"timestamp_utc": stamp, "benches": {}, "verification": verify}

    prov_path = RESULTS_RAW / "provenance.json"
    prov = json.loads(prov_path.read_text()) if prov_path.exists() else {}
    out["provenance"] = prov
    any_rec = next(iter(raw.values()), None)
    if any_rec:
        out["machine"] = any_rec["machine"]
        md += [f"# radar-ng before/after benchmark ({stamp})", "",
               f"Machine: {any_rec['machine']['cpu_model']}, {any_rec['machine']['logical_cpus']} logical CPUs, "
               f"{any_rec['machine']['mem_total_mb']} MB RAM, kernel {any_rec['machine']['kernel']}.", ""]
        if prov:
            dirty = [l for l in prov.get("git_status_relevant", []) if l.strip()]
            md += [f"Rust source: git {prov.get('git_head', '?')[:12]}; {len(dirty)} modified or new files in the "
                   f"measured crates at build time (see provenance in the JSON). Python {prov.get('python', '?')}, "
                   f"pysteps {prov.get('python_packages', {}).get('pysteps')}, numpy "
                   f"{prov.get('python_packages', {}).get('numpy')}.", ""]

    # ---- tile ---------------------------------------------------------------
    tile_names = [("python-legacy", "tile_py_legacy"), ("python-indexed", "tile_py_indexed"), ("rust", "tile_rust")]
    tiles = {k: raw[n] for k, n in tile_names if n in raw}
    if tiles:
        out["benches"]["tile"] = {k: row_of(v) | {"tiles": v["tiles"], "bytes": v["bytes"],
                                                  "config": v["config"]} for k, v in tiles.items()}
        md += ["## 1. Tile render (MRMS 3500 x 7000, zooms 4-7, classic + muted + vivid)", "",
               "| Implementation | Wall median (s) | Wall p90 (s) | Cold first call (s) | CPU median (s) | "
               "Peak RSS (MB) | PNG files | PNG bytes (MB) |",
               "|---|---:|---:|---:|---:|---:|---:|---:|"]
        for k, v in tiles.items():
            r = out["benches"]["tile"][k]
            md.append(f"| {v['label']} | {fmt(r['wall_median_s'], 2)} | {fmt(r['wall_p90_s'], 2)} | "
                      f"{fmt(r['cold_wall_s'], 2)} | {fmt(r['cpu_median_s'], 2)} | "
                      f"{fmt(r['peak_rss_mb'], 0)} | {v['tiles']} | {v['bytes'] / 2**20:.1f} |")
        md.append("")
        md.append(f"Load during these runs (1-min loadavg, 2 CPUs): {load_for('Tile render')}.")
        md.append("")
        if "rust" in tiles:
            rr = out["benches"]["tile"]["rust"]
            md += ["| Rust compared with | Wall speedup | CPU speedup | Peak RSS ratio (Python / Rust) |",
                   "|---|---:|---:|---:|"]
            for k in ("python-legacy", "python-indexed"):
                if k in tiles:
                    b = out["benches"]["tile"][k]
                    md.append(f"| {k} | {ratio(b['wall_median_s'], rr['wall_median_s'])} | "
                              f"{ratio(b['cpu_median_s'], rr['cpu_median_s'])} | "
                              f"{ratio(b['peak_rss_mb'], rr['peak_rss_mb'])} |")
            md.append("")
        vt = verify.get("tile") or {}
        if vt:
            vi = vt.get("python_indexed_vs_rust", {})
            md += [f"Verification (indexed Python vs Rust): tile sets equal = **{vi.get('tile_sets_equal')}**, "
                   f"palette indices identical in **{vi.get('palette_index_identical_tiles')} / "
                   f"{vi.get('common_tiles')}** tiles, RGBA identical in {vi.get('rgba_identical_tiles')} tiles.", ""]
            vl = vt.get("python_legacy_vs_rust_pixels")
            if vl and vl.get("pixel_identical_fraction") is not None:
                md += [f"Legacy RGBA vs Rust (informational, different sampling order by design): "
                       f"{vl['pixel_identical_fraction'] * 100:.1f}% of pixels identical over {vl['tiles_compared']} tiles.", ""]

    # ---- grib ---------------------------------------------------------------
    grib_cases = [("grib_mrms_fixture", "MRMS fixture (35 x 70, gzip)"),
                  ("grib_hrrr_lambert_mini", "Lambert HRRR-like mini (6 messages)"),
                  ("grib_mrms_synth", "MRMS synthetic (3500 x 7000, gzip, grid_png)")]
    grib_rows = []
    for key, label in grib_cases:
        p, r = raw.get(f"{key}_py"), raw.get(f"{key}_rust")
        if not (p and r):
            continue
        pr, rr = row_of(p), row_of(r)
        eq = (verify.get("grib") or {}).get(key, {}).get("all_bitwise_equal")
        grib_rows.append((label, pr, rr, eq))
        out["benches"][key] = {"python": pr, "rust": rr, "bitwise_equal": eq}
    if grib_rows:
        md += ["## 2. GRIB2 decode", "",
               "| Case | Python median (s) | Rust median (s) | Speedup | Python peak RSS (MB) | Rust peak RSS (MB) | "
               "CPU speedup | Values bitwise equal |",
               "|---|---:|---:|---:|---:|---:|---:|---|"]
        for label, pr, rr, eq in grib_rows:
            md.append(f"| {label} | {fmt(pr['wall_median_s'], 4)} | {fmt(rr['wall_median_s'], 4)} | "
                      f"{ratio(pr['wall_median_s'], rr['wall_median_s'])} | {fmt(pr['peak_rss_mb'], 0)} | "
                      f"{fmt(rr['peak_rss_mb'], 0)} | {ratio(pr['cpu_median_s'], rr['cpu_median_s'])} | "
                      f"{'yes' if eq else 'NO' if eq is False else 'n/a'} |")
        md.append("")
        md.append(f"Load during these runs (1-min loadavg, 2 CPUs): {load_for('GRIB decode')}.")
        md.append("")

    # ---- storms -------------------------------------------------------------
    if "storms_py" in raw and "storms_rust" in raw:
        p, r = row_of(raw["storms_py"]), row_of(raw["storms_rust"])
        out["benches"]["storms"] = {"python": p, "rust": r, "features": [raw["storms_py"]["features_a"],
                                                                         raw["storms_py"]["features_b"]]}
        vs = verify.get("storms") or {}

        md += ["## 3. Storm detection and tracking (2 frames, 3500 x 7000)", "",
               "| Implementation | Wall median (s) | Wall p90 (s) | CPU median (s) | Peak RSS (MB) | Features A / B |",
               "|---|---:|---:|---:|---:|---:|",
               f"| Python (detect_storms + tracking) | {fmt(p['wall_median_s'], 3)} | {fmt(p['wall_p90_s'], 3)} | "
               f"{fmt(p['cpu_median_s'], 3)} | {fmt(p['peak_rss_mb'], 0)} | {raw['storms_py']['features_a']} / {raw['storms_py']['features_b']} |",
               f"| Rust (radar-storms) | {fmt(r['wall_median_s'], 3)} | {fmt(r['wall_p90_s'], 3)} | "
               f"{fmt(r['cpu_median_s'], 3)} | {fmt(r['peak_rss_mb'], 0)} | {raw['storms_rust']['features_a']} / {raw['storms_rust']['features_b']} |",
               "", f"Speedup {ratio(p['wall_median_s'], r['wall_median_s'])}, CPU {ratio(p['cpu_median_s'], r['cpu_median_s'])}, "
               f"peak RSS ratio (Python / Rust) {ratio(p['peak_rss_mb'], r['peak_rss_mb'])}.", ""]
        md.append(f"Load during these runs (1-min loadavg, 2 CPUs): {load_for('Storms')}.")
        md.append("")
        if vs:
            fb = vs.get("frame_b", {})
            md += [f"Verification: cell ids equal in order = {fb.get('cell_ids_equal_in_order')}, pixel counts equal = "
                   f"{fb.get('pixel_counts_equal')}, max peak diff {fb.get('max_abs_diff', {}).get('peak_dbz')}, "
                   f"max bbox diff {fb.get('max_abs_diff', {}).get('bbox')}, max centroid diff "
                   f"{fb.get('max_abs_diff', {}).get('coordinates')} (degrees).", ""]

    # ---- nowcast ------------------------------------------------------------
    now_rows = []
    for shape in ("875x1750", "1750x3500"):
        p, r = raw.get(f"nowcast_{shape}_py"), raw.get(f"nowcast_{shape}_rust")
        if p and r:
            now_rows.append((shape, p, r))
            out["benches"][f"nowcast_{shape}"] = {"python": row_of(p), "rust": row_of(r)}
    if now_rows:
        md += ["## 4. Nowcast (LK motion stride 2 + S-PROG, 4 input frames, 12 leads)", "",
               "| Science grid | Python median (s) | Python cold (s) | Rust median (s) | Rust cold (s) | Speedup | "
               "Python peak RSS (MB) | Rust peak RSS (MB) | Iterations (warmup + timed) |",
               "|---|---:|---:|---:|---:|---:|---:|---:|---|"]
        for shape, p, r in now_rows:
            pr, rr = row_of(p), row_of(r)
            md.append(f"| {shape} | {fmt(pr['wall_median_s'], 2)} | {fmt(pr['cold_wall_s'], 2)} | "
                      f"{fmt(rr['wall_median_s'], 2)} | {fmt(rr['cold_wall_s'], 2)} | "
                      f"{ratio(pr['wall_median_s'], rr['wall_median_s'])} | {fmt(pr['peak_rss_mb'], 0)} | "
                      f"{fmt(rr['peak_rss_mb'], 0)} | {p['config']['warmup']} + {p['config']['iters']} |")
        md.append("")
        md.append(f"Load during these runs (1-min loadavg, 2 CPUs): {load_for('Nowcast')}.")
        md.append("")
        vn = verify.get("nowcast") or {}
        for key, v in vn.items():
            leads = v["leads"]
            corr = [x["correlation"] for x in leads if x["correlation"] is not None]
            maxd = [x["max_abs_diff_dbz"] for x in leads if x["max_abs_diff_dbz"] is not None]
            rain = [x["rain_mask_agreement"] for x in leads]
            if corr:
                mean_d = [x["mean_abs_diff_dbz"] for x in leads]
                md.append(f"Verification {key} (not bit-exact by design, see radar-nowcast README): correlation "
                          f"min {min(corr):.4f}, mean absolute difference up to {max(mean_d):.3f} dBZ, "
                          f"largest single-cell difference {max(maxd):.1f} dBZ, 5 dBZ rain-mask agreement "
                          f"min {min(rain) * 100:.2f}%.")
        md.append("")

    # ---- API load (bench 5, run by bench/load/run_load.sh) -------------------
    load_dirs = sorted((BENCH_DIR / "results" / "load").glob("*/summary.json"))
    if load_dirs:
        latest_load = load_dirs[-1]
        api = json.loads(latest_load.read_text())
        out["benches"]["api"] = {"dir": latest_load.parent.name, "targets": api["targets"]}
        table = (latest_load.parent / "summary.md").read_text().splitlines()
        table = [l for l in table if l.startswith("|")]
        md += ["## 5. API load (FastAPI + StaticFiles vs radar-api, same fixture and request list)", "",
               f"Run `{latest_load.parent.name}`. One core per side: server on core 0, oha and the /proc "
               "sampler on core 1. Fixed-rate steps are 30 s each; the max run uses 64 connections. "
               "Python serves /tiles with StaticFiles in place of Caddy (no Cache-Control, no gzip), so "
               "its tile numbers are a lower bound for the Caddy setup. Per-IP rate limit lifted for both.", ""]
        md += table + [""]
        md += ["Full caveats and the request mix: bench/README.md, section 'Bench 5'.", ""]

    # ---- ratios table for the summary ----------------------------------------
    out["notes"] = [
        "Wall = median over timed iterations of one call, in-process (warmup excluded).",
        "CPU = user + sys of the process, all threads.",
        "Peak RSS = VmHWM of the process after the run (includes interpreter and numpy for Python).",
        "p90 uses linear interpolation; with 5 samples it is close to the maximum.",
        "GRIB rows on the 35 x 70 fixtures are dominated by per-call overhead; the 3500 x 7000 row is the realistic one.",
        "Python GRIB decode is the production code path: gunzip to a temp file and pygrib open. Rust decodes in memory.",
        "Nowcast verification: the Rust motion field is a re-implementation of OpenCV's Lucas-Kanade and Shi-Tomasi, "
        "so it is close to, not identical with, pysteps' output.",
    ]
    md += ["## Notes", ""] + [f"- {n}" for n in out["notes"]] + [""]

    results_dir = BENCH_DIR / "results"
    results_dir.mkdir(parents=True, exist_ok=True)
    write_json(results_dir / f"{stamp}.json", out)
    (results_dir / f"{stamp}.md").write_text("\n".join(md) + "\n")
    (results_dir / "latest.md").write_text("\n".join(md) + "\n")
    print(results_dir / f"{stamp}.md")


if __name__ == "__main__":
    main()
