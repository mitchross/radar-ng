#!/usr/bin/env python3
"""Cross-check the Python and Rust outputs of every benchmark and write work/raw/verify.json.

  tile     Python indexed tree vs Rust tree: same tile set, same decoded palette indices
           (full comparison, every tile of every palette), and same RGBA. Python legacy vs Rust
           is reported as a pixel agreement only, because legacy samples colour first (by design
           it does not match the indexed output).
  storms   per-cell agreement of detect_storms output for the two frames (ids, counts, peaks,
           areas, bboxes, centroids).
  nowcast  per-lead agreement of the forecast fields (max and mean absolute difference, Pearson
           correlation on cells valid on both sides, rain-mask agreement at 5 dBZ).
  grib     bitwise digest comparison of the decoded values (and axes for the MRMS path).
  assets   sha256 of the palette JSON embedded in radar-tiler against the production files.
"""

from __future__ import annotations

import hashlib
import json
import sys
from pathlib import Path

import numpy as np
from PIL import Image

sys.path.insert(0, str(Path(__file__).resolve().parent))
from common import OUT, RADAR_NG, RESULTS_RAW, RUST_ROOT, write_json  # noqa: E402


def load(name: str) -> dict:
    return json.loads((RESULTS_RAW / f"{name}.json").read_text())


def tile_trees(root_py: Path, root_rs: Path, palettes: list[str]) -> dict:
    py_files: dict[str, Path] = {}
    rs_files: dict[str, Path] = {}
    for p in palettes:
        for f in (root_py / p).rglob("*.png"):
            py_files[f"{p}/{f.relative_to(root_py / p)}"] = f
        for f in (root_rs / p).rglob("*.png"):
            rs_files[f"{p}/{f.relative_to(root_rs / p)}"] = f
    common = sorted(set(py_files) & set(rs_files))
    idx_same = rgba_same = 0
    pixel_diff_max = 0
    mismatches: list[str] = []
    for key in common:
        a = Image.open(py_files[key])
        b = Image.open(rs_files[key])
        ia = np.asarray(a) if a.mode == "P" else None
        ib = np.asarray(b) if b.mode == "P" else None
        if ia is not None and ib is not None and ia.shape == ib.shape and np.array_equal(ia, ib):
            idx_same += 1
        else:
            mismatches.append(key)
        ra = np.asarray(a.convert("RGBA"))
        rb = np.asarray(b.convert("RGBA"))
        if ra.shape == rb.shape and np.array_equal(ra, rb):
            rgba_same += 1
        else:
            pixel_diff_max = max(pixel_diff_max, int(np.abs(ra.astype(int) - rb.astype(int)).max()))
    return {
        "python_tiles": len(py_files),
        "rust_tiles": len(rs_files),
        "common_tiles": len(common),
        "only_python": sorted(set(py_files) - set(rs_files))[:20],
        "only_rust": sorted(set(rs_files) - set(py_files))[:20],
        "tile_sets_equal": set(py_files) == set(rs_files),
        "palette_index_identical_tiles": idx_same,
        "rgba_identical_tiles": rgba_same,
        "index_mismatch_tiles": mismatches[:20],
        "max_rgba_abs_diff": pixel_diff_max,
    }


def tile_agreement(root_py: Path, root_rs: Path, palettes: list[str]) -> dict:
    """Pixel agreement between legacy RGBA tiles and indexed Rust tiles (informational)."""
    same = total = 0
    tiles_compared = 0
    for p in palettes:
        for f in (root_py / p).rglob("*.png"):
            g = root_rs / p / f.relative_to(root_py / p)
            if not g.exists():
                continue
            a = np.asarray(Image.open(f).convert("RGBA"))
            b = np.asarray(Image.open(g).convert("RGBA"))
            same += int(np.all(a == b, axis=-1).sum())
            total += a.shape[0] * a.shape[1]
            tiles_compared += 1
    return {"tiles_compared": tiles_compared,
            "pixel_identical_fraction": (same / total) if total else None}


def verify_tile() -> dict:
    py_idx = OUT / "tile" / "py-indexed"
    py_leg = OUT / "tile" / "py-legacy"
    rs = OUT / "tile" / "rust"
    palettes = ["classic", "muted", "vivid"]
    out: dict = {}
    last_py = sorted(py_idx.glob("iter*"), key=lambda p: int(p.name[4:]))[-1]
    last_leg = sorted(py_leg.glob("iter*"), key=lambda p: int(p.name[4:]))[-1] if py_leg.exists() else None
    last_rs = sorted(rs.glob("iter*"), key=lambda p: int(p.name[4:]))[-1]
    out["python_indexed_vs_rust"] = tile_trees(last_py, last_rs, palettes)
    if last_leg is not None:
        out["python_legacy_vs_rust_pixels"] = tile_agreement(last_leg, last_rs, palettes)
    return out


def verify_storms() -> dict:
    py = json.loads((OUT / "storms" / "py_summary.json").read_text())
    rs = json.loads((OUT / "storms" / "rust_summary.json").read_text())
    out = {}
    for frame in ("frame_a", "frame_b"):
        a, b = py[frame], rs[frame]
        res = {"n_features_python": a["n_features"], "n_features_rust": b["n_features"]}
        fa, fb = a["features"], b["features"]
        res["features_equal_count"] = a["n_features"] == b["n_features"]
        ids_equal = [x["cell_id"] for x in fa] == [x["cell_id"] for x in fb]
        res["cell_ids_equal_in_order"] = ids_equal
        dmax = {"peak_dbz": 0.0, "area_km2_rel": 0.0, "bbox": 0.0, "coordinates": 0.0}
        pix_equal = True
        for x, y in zip(fa, fb):
            pix_equal &= x["pixel_count"] == y["pixel_count"]
            dmax["peak_dbz"] = max(dmax["peak_dbz"], abs(x["peak_dbz"] - y["peak_dbz"]))
            dmax["area_km2_rel"] = max(dmax["area_km2_rel"],
                                       abs(x["area_km2"] - y["area_km2"]) / max(abs(x["area_km2"]), 1e-12))
            dmax["bbox"] = max(dmax["bbox"], max(abs(p - q) for p, q in zip(x["bbox"], y["bbox"])))
            dmax["coordinates"] = max(dmax["coordinates"],
                                      max(abs(p - q) for p, q in zip(x["coordinates"], y["coordinates"])))
        res["pixel_counts_equal"] = pix_equal
        res["max_abs_diff"] = dmax
        out[frame] = res
    return out


def verify_nowcast() -> dict:
    out = {}
    for h, w in ((875, 1750), (1750, 3500)):
        p = OUT / "nowcast" / f"py_{h}x{w}_fc.f32"
        r = OUT / "nowcast" / f"rust_{h}x{w}_fc.f32"
        if not (p.exists() and r.exists()):
            continue
        a = np.fromfile(p, dtype="<f4").reshape(-1, h, w)
        b = np.fromfile(r, dtype="<f4").reshape(-1, h, w)
        leads = []
        for k in range(a.shape[0]):
            x, y = a[k].astype(np.float64), b[k].astype(np.float64)
            valid = (x > -9000) & (y > -9000)
            xs, ys = x[valid], y[valid]
            corr = float(np.corrcoef(xs, ys)[0, 1]) if xs.size > 1 and xs.std() > 0 and ys.std() > 0 else None
            rain_a, rain_b = x >= 5.0, y >= 5.0
            leads.append({
                "lead": k + 1,
                "max_abs_diff_dbz": float(np.abs(xs - ys).max()) if xs.size else None,
                "mean_abs_diff_dbz": float(np.abs(xs - ys).mean()) if xs.size else None,
                "correlation": corr,
                "rain_mask_agreement": float((rain_a == rain_b).mean()),
                "python_rain_fraction": float(rain_a.mean()),
                "rust_rain_fraction": float(rain_b.mean()),
            })
        out[f"{h}x{w}"] = {"leads": leads, "shape": [int(v) for v in a.shape]}
    return out


def verify_grib() -> dict:
    out = {}
    for name in ("grib_mrms_fixture", "grib_hrrr_lambert_mini", "grib_mrms_synth"):
        try:
            py = load(f"{name}_py")
            rs = load(f"{name}_rust")
        except FileNotFoundError:
            continue
        if "messages" in py["digest"]:
            pm, rm = py["digest"]["messages"], rs["digest"]["messages"]
            out[name] = {
                "messages": len(pm),
                "messages_equal_count": len(pm) == len(rm),
                "bitwise_equal_per_message": [a["sha256_values"] == b["sha256_values"]
                                              and a["shape"] == b["shape"] for a, b in zip(pm, rm)],
            }
            out[name]["all_bitwise_equal"] = all(out[name]["bitwise_equal_per_message"])
        else:
            pd, rd = py["digest"], rs["digest"]
            out[name] = {
                "shape_python": pd["shape"], "shape_rust": rd["shape"],
                "values_bitwise_equal": pd["sha256_values"] == rd["sha256_values"],
                "lats_bitwise_equal": pd["sha256_lats"] == rd["sha256_lats"],
                "lons_bitwise_equal": pd["sha256_lons"] == rd["sha256_lons"],
                "nan_cells": [pd["nan_cells"], rd["nan_cells"]],
            }
            out[name]["all_bitwise_equal"] = (out[name]["values_bitwise_equal"]
                                              and out[name]["lats_bitwise_equal"]
                                              and out[name]["lons_bitwise_equal"])
    return out


def verify_assets() -> dict:
    out = {}
    for p in ("classic", "muted", "vivid"):
        prod = RADAR_NG / "backend/shared/palettes" / f"{p}.json"
        emb = RUST_ROOT / "crates/radar-tiler/assets" / f"{p}.json"
        if prod.exists() and emb.exists():
            out[p] = hashlib.sha256(prod.read_bytes()).hexdigest() == hashlib.sha256(emb.read_bytes()).hexdigest()
    return {"palette_json_identical_to_production": out}


def main() -> None:
    result = {
        "tile": verify_tile() if (OUT / "tile").exists() else None,
        "storms": verify_storms() if (OUT / "storms" / "rust_summary.json").exists() else None,
        "nowcast": verify_nowcast(),
        "grib": verify_grib(),
        "assets": verify_assets(),
    }
    write_json(RESULTS_RAW / "verify.json", result)
    print(json.dumps(result, indent=1)[:4000])


if __name__ == "__main__":
    main()
