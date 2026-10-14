"""Synthetic NAQFC-like GRIB2 fixtures for radar-aux (spec-ingest-aux section 5, test checklist 15.1).

Run with a Python that has eccodes and numpy (PyPI `eccodes` ships libeccodes):
    /tmp/venv-radar-aux/bin/python gen_aqm.py

Writes next to this file:
  aqm_pm25_mini.grib2       72 messages, pm2.5 (discipline 0, category 20, number 62), grid_template 3.30
                            (Lambert, spherical R=6371229, as grid 227 is), product_template 4.0 (instant),
                            hourly forecast time 0..71 h after a 12Z cycle on 2026-07-16. Values in ug/m3,
                            NaN cells encoded as GRIB bitmap-missing.
  aqm_o3_mini.grib2         the same 72-hour layout for ozone (ppb), product_template 4.0.
  aqm_pm25_stat.grib2       3 messages, product_template 4.8 (time-averaged), 1 h range, for the validDate rule.
  aqm_pm25_flipped.grib2    2 messages with iScansNegatively=1 (longitudes decrease along a row), for the
                            ascending-longitude grid dump rule (D-7).
  golden/aqm_manifest.json  per file: message count, eccodes validityDate/validityTime (ISO, UTC), the grid
                            shape, and the first message's values (f32, little endian) as decoded by eccodes.

The grid is small (24 x 16 cells, 5 km) and sits over west Michigan so that the zoom 4-6 tiles it covers are
few. The real grid 227 is about 1473 x 1025 cells. The shape here is a stand-in, not a copy of NOAA's grid.
"""

import json
import os

import eccodes as ec
import numpy as np

HERE = os.path.dirname(os.path.abspath(__file__))
GOLD = os.path.join(HERE, "golden")
os.makedirs(GOLD, exist_ok=True)
NX, NY = 24, 16
RUN = "20260716"
CYCLE = 12
MISSING_OUT = 9999.0
rng = np.random.default_rng(20260716)


def set_lambert(h, flip_i=False):
    ec.codes_set(h, "centre", 7)
    ec.codes_set(h, "gridType", "lambert")
    ec.codes_set(h, "Nx", NX)
    ec.codes_set(h, "Ny", NY)
    ec.codes_set(h, "shapeOfTheEarth", 6)
    ec.codes_set(h, "DxInMetres", 5000)
    ec.codes_set(h, "DyInMetres", 5000)
    ec.codes_set(h, "latitudeOfFirstGridPointInDegrees", 41.2)
    ec.codes_set(h, "longitudeOfFirstGridPointInDegrees", 273.6)  # -86.4
    ec.codes_set(h, "LoVInDegrees", 262.5)
    ec.codes_set(h, "LaDInDegrees", 38.5)
    ec.codes_set(h, "Latin1InDegrees", 38.5)
    ec.codes_set(h, "Latin2InDegrees", 38.5)
    ec.codes_set(h, "iScansNegatively", 1 if flip_i else 0)
    ec.codes_set(h, "jScansPositively", 1)
    ec.codes_set(h, "jPointsAreConsecutive", 0)
    ec.codes_set(h, "projectionCentreFlag", 0)


def values_for(kind, k):
    """pm2.5 ~ 0..60 ug/m3 with a few NaN cells; ozone ~ 20..120 ppb. Message k shifts the field."""
    if kind == "pm25":
        base = rng.uniform(0.0, 60.0, size=(NY, NX))
    else:
        base = rng.uniform(20.0, 120.0, size=(NY, NX))
    base = np.round(base * 10) / 10 + (k % 5) * 0.1
    mask = rng.uniform(size=(NY, NX)) < 0.06
    base[mask] = np.nan
    return base


def write_msg(f, h, vals):
    filled = np.where(np.isnan(vals), MISSING_OUT, vals)
    has_nan = bool(np.isnan(vals).any())
    ec.codes_set(h, "bitmapPresent", 1 if has_nan else 0)
    if has_nan:
        ec.codes_set(h, "missingValue", MISSING_OUT)
    ec.codes_set_double_array(h, "values", filled.ravel())
    ec.codes_write(h, f)
    ec.codes_release(h)


def make_msg(kind, k, template, flip_i=False):
    h = ec.codes_grib_new_from_samples("GRIB2")
    set_lambert(h, flip_i)
    ec.codes_set(h, "packingType", "grid_simple")
    ec.codes_set(h, "bitsPerValue", 16)
    ec.codes_set(h, "dataDate", int(RUN))
    ec.codes_set(h, "dataTime", CYCLE * 100)
    ec.codes_set(h, "discipline", 0)
    ec.codes_set(h, "parameterCategory", 20)
    ec.codes_set(h, "parameterNumber", 62 if kind == "pm25" else 1)
    ec.codes_set(h, "typeOfFirstFixedSurface", 1)
    ec.codes_set(h, "scaledValueOfFirstFixedSurface", 0)
    ec.codes_set(h, "productDefinitionTemplateNumber", template)
    if template == 0:
        ec.codes_set(h, "indicatorOfUnitOfTimeRange", 1)
        ec.codes_set(h, "forecastTime", k)
    else:
        # time-averaged over [k, k+1) hours; eccodes derives startStep/endStep from stepRange.
        ec.codes_set(h, "indicatorOfUnitOfTimeRange", 1)
        ec.codes_set(h, "stepRange", f"{k}-{k + 1}")
        ec.codes_set(h, "typeOfStatisticalProcessing", 0)
        ec.codes_set(h, "typeOfTimeIncrement", 2)
        ec.codes_set(h, "indicatorOfUnitForTimeRange", 1)
        ec.codes_set(h, "lengthOfTimeRange", 1)
    return h


def read_back(path, n):
    out = []
    with open(path, "rb") as f:
        for _ in range(n):
            h = ec.codes_grib_new_from_file(f)
            vd = ec.codes_get(h, "validityDate")
            vt = ec.codes_get(h, "validityTime")
            rd = ec.codes_get(h, "dataDate")
            rt = ec.codes_get(h, "dataTime")
            nj = ec.codes_get(h, "Ny")
            ni = ec.codes_get(h, "Nx")
            vals = np.array(ec.codes_get_double_array(h, "values"), dtype=np.float64)
            if ec.codes_get(h, "bitmapPresent"):
                bm = np.array(ec.codes_get_long_array(h, "bitmap"), dtype=np.int64)
                vals[bm == 0] = np.nan
            out.append({
                "valid_date": int(vd), "valid_time": int(vt),
                "data_date": int(rd), "data_time": int(rt),
                "ny": int(nj), "nx": int(ni),
                "values_f32_first": None,
                "_vals": vals.astype(np.float32),
            })
            ec.codes_release(h)
    return out


def iso_valid(vd, vt):
    s = f"{vd:08d}"
    hh, mm = vt // 100, vt % 100
    return f"{s[0:4]}-{s[4:6]}-{s[6:8]}T{hh:02d}:{mm:02d}:00+00:00"


def write_file(name, kind, count, template, flip_i=False):
    path = os.path.join(HERE, name)
    with open(path, "wb") as f:
        for k in range(count):
            h = make_msg(kind, k, template, flip_i)
            write_msg(f, h, values_for(kind, k))
    recs = read_back(path, count)
    first = recs[0]["_vals"]
    entries = []
    for i, r in enumerate(recs):
        entries.append({
            "message": i,
            "valid": iso_valid(r["valid_date"], r["valid_time"]),
            "data_date": r["data_date"], "data_time": r["data_time"],
            "ny": r["ny"], "nx": r["nx"],
        })
    raw_name = name.replace(".grib2", "_msg0.f32le")
    with open(os.path.join(GOLD, raw_name), "wb") as f:
        f.write(first.astype("<f4").tobytes())
    return {"file": name, "messages": count, "template": template,
            "flip_i": flip_i, "entries": entries, "msg0_values_f32le": raw_name,
            "msg0_nan_count": int(np.isnan(first).sum())}


def write_blank_frame_file(name="aqm_pm25_blank24.grib2", count=24, blank=5):
    """24 messages; message `blank` has every value missing (all-NaN frame, must not be published)."""
    path = os.path.join(HERE, name)
    with open(path, "wb") as f:
        for k in range(count):
            h = make_msg("pm25", k, 0, False)
            vals = np.full((NY, NX), np.nan) if k == blank else values_for("pm25", k)
            write_msg(f, h, vals)
    return name


def main():
    manifest = {
        "pm25": write_file("aqm_pm25_mini.grib2", "pm25", 72, 0),
        "ozone": write_file("aqm_o3_mini.grib2", "o3", 72, 0),
        "stat": write_file("aqm_pm25_stat.grib2", "pm25", 3, 8),
        "flipped": write_file("aqm_pm25_flipped.grib2", "pm25", 2, 0, flip_i=True),
    }
    with open(os.path.join(GOLD, "aqm_manifest.json"), "w") as f:
        json.dump(manifest, f, indent=1, sort_keys=True)
        f.write("\n")
    write_blank_frame_file()
    print("ok", {k: v["messages"] for k, v in manifest.items()})


if __name__ == "__main__":
    main()
