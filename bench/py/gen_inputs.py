#!/usr/bin/env python3
"""Generate the deterministic synthetic inputs shared by the Python and Rust benchmarks.

Run inside the bench venv (numpy, eccodes):  bench/.venv/bin/python bench/py/gen_inputs.py

Writes $BENCH_WORK/inputs (default bench/work/inputs):

  mrms_3500x7000_t0.f32, _t1.f32   raw little-endian float32, row 0 = south (lat ascending),
                                   NaN = outside coverage. MRMS geometry (0.01 deg, CONUS).
  lats_3500.f64, lons_7000.f64     the MRMS axes after the south-up normalisation (spec 5.2).
  nowcast_{H}x{W}_t{0..3}.f32      four frames for H x W in {1750x3500, 875x1750}. Missing
                                   cells are -999, which exercises the `< -100 -> NaN` rule.
  mrms_synth_3500x7000.grib2.gz    t0 encoded as an MRMS-style regular_ll grid_png GRIB2
                                   (north-first, 16-bit, no bitmap, missing = -99), gzipped.
  fixture_*                        copies of the radar-grib golden fixtures (small MRMS and a
                                   six-message Lambert HRRR-like file).
  manifest.json                    seed, parameters, shapes and sha256 of every file.

The field is a max over 60 moving Gaussian echo cells (MRMS pixel units, scaled to each grid),
plus N(0, 1.5) noise, rounded to 0.5 dBZ, clipped to [-12, 72]. Every grid shape renders the
same physical scene, so the nowcast frames move coherently (about 6 MRMS px per 5 min east and
3 px north).
"""

from __future__ import annotations

import gzip
import shutil
import sys
from pathlib import Path

import numpy as np

sys.path.insert(0, str(Path(__file__).resolve().parent))
from common import INPUTS, RUST_ROOT, sha256_file, write_json  # noqa: E402

SEED = 20261014
H0, W0 = 3500, 7000  # MRMS CONUS 0.01 degree grid
N_BLOBS = 60
MISSING_NOWCAST = -999.0
MISSING_GRIB = -99.0


def make_blobs(rng: np.random.Generator) -> dict[str, np.ndarray]:
    n = N_BLOBS
    u = rng.uniform(0.15, 0.85, n)  # centre, fraction of columns
    v = rng.uniform(0.2, 0.8, n)  # centre, fraction of rows
    s_px = rng.uniform(10.0, 55.0, n)  # sigma in MRMS pixels
    elong = rng.uniform(1.0, 1.8, n)
    peak = rng.uniform(22.0, 72.0, n)
    du = (6.0 + rng.normal(0.0, 0.5, n)) / W0  # per 5-minute step, fraction of columns
    dv = (3.0 + rng.normal(0.0, 0.3, n)) / H0
    return {
        "u": u, "v": v, "su": s_px * elong / W0, "sv": s_px / H0,
        "peak": peak, "du": du, "dv": dv,
    }


def render_frame(h: int, w: int, t: int, blobs: dict[str, np.ndarray], nodata: float) -> np.ndarray:
    """One frame at step t on an h x w grid. Returns float32 with `nodata` outside coverage."""
    field = np.full((h, w), -12.0, dtype=np.float64)
    for k in range(N_BLOBS):
        cu = blobs["u"][k] + t * blobs["du"][k]
        cv = blobs["v"][k] + t * blobs["dv"][k]
        su, sv = blobs["su"][k], blobs["sv"][k]
        c0 = max(0, int(np.floor((cu - 3.5 * su) * w - 0.5)))
        c1 = min(w, int(np.ceil((cu + 3.5 * su) * w + 0.5)))
        r0 = max(0, int(np.floor((cv - 3.5 * sv) * h - 0.5)))
        r1 = min(h, int(np.ceil((cv + 3.5 * sv) * h + 0.5)))
        if c1 <= c0 or r1 <= r0:
            continue
        x = (np.arange(c0, c1) + 0.5) / w
        y = (np.arange(r0, r1) + 0.5) / h
        d2 = ((y[:, None] - cv) / sv) ** 2 + ((x[None, :] - cu) / su) ** 2
        sub = field[r0:r1, c0:c1]
        np.maximum(sub, blobs["peak"][k] * np.exp(-0.5 * d2), out=sub)
    noise = np.random.default_rng(SEED + 100 + t).normal(0.0, 1.5, (h, w))
    field = np.round(np.clip(field + noise, -12.0, 72.0) * 2.0) / 2.0
    ys = (np.arange(h) + 0.5) / h
    xs = (np.arange(w) + 0.5) / w
    covered = ((xs[None, :] - 0.5) / 0.48) ** 2 + ((ys[:, None] - 0.5) / 0.47) ** 2 <= 1.0
    field[~covered] = nodata
    return field.astype(np.float32)


def write_raw(path: Path, arr: np.ndarray, dtype: str) -> dict:
    arr = np.ascontiguousarray(arr.astype(dtype))
    arr.tofile(path)
    return {"file": path.name, "dtype": dtype, "shape": list(arr.shape),
            "bytes": path.stat().st_size, "sha256": sha256_file(path)}


def encode_mrms_grib(values_south_up: np.ndarray, path: Path) -> dict:
    """MRMS-style GRIB2: regular_ll, north-first, grid_png, 16 bit, no bitmap.

    Missing cells carry -99 dBZ in the message, as MRMS does on disk. A bitmap with
    full-size grid_png fails to encode in this eccodes build (checked: "Decoding invalid"),
    so the bitmap is not used.
    """
    import eccodes as ec

    north_first = np.flipud(values_south_up).astype(np.float64)
    nj, ni = north_first.shape
    missing = np.isnan(north_first)
    filled = np.where(missing, MISSING_GRIB, north_first)
    h = ec.codes_grib_new_from_samples("regular_ll_sfc_grib2")
    ec.codes_set(h, "centre", 7)
    ec.codes_set(h, "gridType", "regular_ll")
    ec.codes_set(h, "Ni", ni)
    ec.codes_set(h, "Nj", nj)
    ec.codes_set(h, "latitudeOfFirstGridPointInDegrees", 54.995)
    ec.codes_set(h, "latitudeOfLastGridPointInDegrees", 20.005001)
    ec.codes_set(h, "longitudeOfFirstGridPointInDegrees", 230.005)
    ec.codes_set(h, "longitudeOfLastGridPointInDegrees", 299.994999)
    ec.codes_set(h, "iDirectionIncrementInDegrees", 0.01)
    ec.codes_set(h, "jDirectionIncrementInDegrees", 0.01)
    ec.codes_set(h, "jScansPositively", 0)
    ec.codes_set(h, "packingType", "grid_png")
    ec.codes_set(h, "bitsPerValue", 16)
    ec.codes_set(h, "decimalScaleFactor", 1)
    ec.codes_set(h, "discipline", 0)
    ec.codes_set(h, "parameterCategory", 16)
    ec.codes_set(h, "parameterNumber", 196)
    ec.codes_set(h, "typeOfFirstFixedSurface", 10)
    ec.codes_set(h, "scaledValueOfFirstFixedSurface", 0)
    ec.codes_set_double_array(h, "values", filled.ravel())
    raw = path.with_suffix("")  # .grib2 path without .gz
    with open(raw, "wb") as f:
        ec.codes_write(h, f)
    ec.codes_release(h)
    with open(raw, "rb") as src, gzip.open(path, "wb", compresslevel=6) as dst:
        shutil.copyfileobj(src, dst)
    # Read back with eccodes: the 0.5 dBZ steps must survive the packing exactly.
    with open(raw, "rb") as f:
        h = ec.codes_grib_new_from_file(f)
        back = np.array(ec.codes_get_double_array(h, "values"), dtype=np.float64).reshape(nj, ni)
        ec.codes_release(h)
    raw.unlink()
    same = np.array_equal(back, filled)
    print(f"grib round trip exact={same} missing_cells={int(missing.sum())}", flush=True)
    if not same:
        raise SystemExit("GRIB packing round trip failed")
    return {"file": path.name, "shape": [nj, ni], "missing_cells": int(missing.sum()),
            "missing_value": MISSING_GRIB, "gz_bytes": path.stat().st_size,
            "sha256": sha256_file(path), "round_trip_exact": bool(same)}


def main() -> None:
    INPUTS.mkdir(parents=True, exist_ok=True)
    rng = np.random.default_rng(SEED)
    blobs = make_blobs(rng)
    files: dict[str, dict] = {}

    for t in (0, 1):
        arr = render_frame(H0, W0, t, blobs, nodata=np.nan)
        if t == 0:
            t0_frame = arr
        files[f"mrms_3500x7000_t{t}.f32"] = write_raw(INPUTS / f"mrms_3500x7000_t{t}.f32", arr, "<f4")
        print(f"mrms frame t={t}: covered={np.isfinite(arr).mean():.1%} max={np.nanmax(arr):.1f} dBZ", flush=True)

    lats = np.linspace(20.005001, 54.995, H0)
    lons = np.linspace(-129.995, -60.005, W0)
    files["lats_3500.f64"] = write_raw(INPUTS / "lats_3500.f64", lats, "<f8")
    files["lons_7000.f64"] = write_raw(INPUTS / "lons_7000.f64", lons, "<f8")

    for h, w in ((1750, 3500), (875, 1750)):
        for t in range(4):
            arr = render_frame(h, w, t, blobs, nodata=MISSING_NOWCAST)
            name = f"nowcast_{h}x{w}_t{t}.f32"
            files[name] = write_raw(INPUTS / name, arr, "<f4")
        print(f"nowcast {h}x{w}: 4 frames written", flush=True)

    grib_info = encode_mrms_grib(t0_frame, INPUTS / "mrms_synth_3500x7000.grib2.gz")
    files["mrms_synth_3500x7000.grib2.gz"] = {**grib_info, "file": "mrms_synth_3500x7000.grib2.gz"}
    print("grib encoded", flush=True)

    fixtures = {
        "fixture_mrms_small.grib2.gz": RUST_ROOT / "crates/radar-grib/testdata/mrms_regular_ll_png.grib2.gz",
        "fixture_hrrr_lambert_mini.grib2": RUST_ROOT / "crates/radar-grib/testdata/hrrr_lambert_mini.grib2",
    }
    for name, src in fixtures.items():
        dst = INPUTS / name
        shutil.copyfile(src, dst)
        files[name] = {"file": name, "bytes": dst.stat().st_size, "sha256": sha256_file(dst),
                       "source": str(src)}

    write_json(INPUTS / "manifest.json", {
        "seed": SEED,
        "generator": "bench/py/gen_inputs.py",
        "mrms_grid": {"rows": H0, "cols": W0, "step_deg": 0.01,
                      "lat_range": [20.005001, 54.995], "lon_range": [-129.995, -60.005],
                      "row_order": "south-up (ascending lat)", "missing": "NaN"},
        "nowcast_grids": [[1750, 3500], [875, 1750]],
        "nowcast_missing_value": MISSING_NOWCAST,
        "n_blobs": N_BLOBS,
        "files": files,
    })
    print(f"wrote {len(files)} entries to {INPUTS / 'manifest.json'}", flush=True)


if __name__ == "__main__":
    main()
