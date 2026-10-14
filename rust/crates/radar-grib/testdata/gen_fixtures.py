"""Generate GRIB2 test fixtures and golden decoded arrays for radar-grib.

Run with a Python that has numpy and eccodes (PyPI `eccodes` ships libeccodes):
    python3 gen_fixtures.py

Writes next to this file:
  mrms_regular_ll_png.grib2        one message, regular_ll (3.0), grid_png (5.41), bitmap, 16-bit
  mrms_regular_ll_png.grib2.gz     the same file gzipped (MRMS on-disk form)
  hrrr_lambert_mini.grib2          six messages, Lambert (3.30), templates 5.0 / 5.2 / 5.3, bitmap
  golden/*.f32le, *.f64le          raw little-endian arrays decoded by eccodes (the pygrib backend)
  golden/manifest.json             shapes, dtypes and selector-relevant metadata

The Lambert projection axes (pyproj/spec §5.6) are produced by gen_projection.py, which must run in a
separate process: importing pyproj and eccodes into one interpreter segfaults in this environment.
"""

import gzip
import json
import os

import eccodes as ec
import numpy as np

HERE = os.path.dirname(os.path.abspath(__file__))
GOLD = os.path.join(HERE, "golden")
os.makedirs(GOLD, exist_ok=True)
MISSING = 9999.0
rng = np.random.default_rng(20261013)


def dump_raw(name, arr, dtype):
    arr = np.ascontiguousarray(arr, dtype=dtype)
    path = os.path.join(GOLD, name)
    with open(path, "wb") as f:
        f.write(arr.astype(arr.dtype.newbyteorder("<")).tobytes())
    return {"file": name, "dtype": np.dtype(dtype).name, "shape": list(arr.shape)}


def read_values(h):
    """Decoded values as eccodes returns them, with bitmap-missing points set to NaN (pygrib masked semantics)."""
    vals = np.array(ec.codes_get_double_array(h, "values"), dtype=np.float64)
    if ec.codes_get(h, "bitmapPresent"):
        bm = np.array(ec.codes_get_long_array(h, "bitmap"), dtype=np.int64)
        vals[bm == 0] = np.nan
    return vals


def set_common(h, nj, ni, bitmap_mask, values):
    """values: float array with NaN for missing; bitmap encodes NaN as missing."""
    filled = np.where(np.isnan(values), MISSING, values)
    ec.codes_set(h, "bitmapPresent", 1 if bitmap_mask.any() else 0)
    if bitmap_mask.any():
        ec.codes_set(h, "missingValue", MISSING)
    ec.codes_set_double_array(h, "values", filled.ravel())


def set_lambert(h, nx, ny):
    ec.codes_set(h, "centre", 7)                    # NCEP, so eccodes resolves the NCEP local names
    ec.codes_set(h, "gridType", "lambert")
    ec.codes_set(h, "Nx", nx)
    ec.codes_set(h, "Ny", ny)
    ec.codes_set(h, "shapeOfTheEarth", 6)          # sphere, radius 6371229 m (HRRR)
    ec.codes_set(h, "DxInMetres", 3000)
    ec.codes_set(h, "DyInMetres", 3000)
    ec.codes_set(h, "latitudeOfFirstGridPointInDegrees", 21.138123)
    ec.codes_set(h, "longitudeOfFirstGridPointInDegrees", 237.280472)
    ec.codes_set(h, "LoVInDegrees", 262.5)
    ec.codes_set(h, "LaDInDegrees", 38.5)
    ec.codes_set(h, "Latin1InDegrees", 38.5)
    ec.codes_set(h, "Latin2InDegrees", 38.5)
    ec.codes_set(h, "iScansNegatively", 0)
    ec.codes_set(h, "jScansPositively", 1)
    ec.codes_set(h, "jPointsAreConsecutive", 0)
    ec.codes_set(h, "projectionCentreFlag", 0)


def write_msg(f, h):
    ec.codes_write(h, f)
    ec.codes_release(h)


def main():
    manifest = {"mrms": {}, "hrrr": {"messages": []}}

    # ---------------- MRMS: regular_ll, grid_png 16-bit, bitmap ----------------
    nj, ni = 35, 70                      # 54.0 .. 20.0 N (descending), 230.0 .. 299.0 E
    lat_first, lat_last = 54.0, 20.0
    lon_first, lon_last = 230.0, 299.0
    vals = np.round(rng.uniform(-10.0, 60.0, size=(nj, ni)) * 10) / 10     # 0.1 dBZ steps
    vals[:6, :9] = np.nan                # NW corner outside coverage
    vals[rng.uniform(size=(nj, ni)) < 0.05] = np.nan
    mask = np.isnan(vals)

    h = ec.codes_grib_new_from_samples("regular_ll_sfc_grib2")
    ec.codes_set(h, "centre", 7)
    ec.codes_set(h, "gridType", "regular_ll")
    ec.codes_set(h, "Ni", ni)
    ec.codes_set(h, "Nj", nj)
    ec.codes_set(h, "latitudeOfFirstGridPointInDegrees", lat_first)
    ec.codes_set(h, "latitudeOfLastGridPointInDegrees", lat_last)
    ec.codes_set(h, "longitudeOfFirstGridPointInDegrees", lon_first)
    ec.codes_set(h, "longitudeOfLastGridPointInDegrees", lon_last)
    ec.codes_set(h, "iDirectionIncrementInDegrees", (lon_last - lon_first) / (ni - 1))
    ec.codes_set(h, "jDirectionIncrementInDegrees", (lat_first - lat_last) / (nj - 1))
    ec.codes_set(h, "jScansPositively", 0)          # north first, as MRMS stores it
    ec.codes_set(h, "packingType", "grid_png")
    ec.codes_set(h, "bitsPerValue", 16)
    ec.codes_set(h, "decimalScaleFactor", 1)
    ec.codes_set(h, "discipline", 0)
    ec.codes_set(h, "parameterCategory", 16)
    ec.codes_set(h, "parameterNumber", 196)
    ec.codes_set(h, "typeOfFirstFixedSurface", 10)  # entire atmosphere
    ec.codes_set(h, "scaledValueOfFirstFixedSurface", 0)
    set_common(h, nj, ni, mask, vals)
    grib_path = os.path.join(HERE, "mrms_regular_ll_png.grib2")
    with open(grib_path, "wb") as f:
        write_msg(f, h)
    with open(grib_path, "rb") as f:
        raw = f.read()
    with gzip.open(grib_path + ".gz", "wb", compresslevel=9) as f:
        f.write(raw)

    h = ec.codes_grib_new_from_file(open(grib_path, "rb"))
    decoded = read_values(h)
    lats = np.array(ec.codes_get_double_array(h, "latitudes"), dtype=np.float64).reshape(nj, ni)
    lons = np.array(ec.codes_get_double_array(h, "longitudes"), dtype=np.float64).reshape(nj, ni)
    ec.codes_release(h)
    # Python _grid_axes (regular_ll branch): linspace from header keys, not latlons().
    lat_axis = np.linspace(lat_first, lat_last, nj)
    lon_axis = np.linspace(lon_first, lon_last, ni)
    lon_axis = np.where(lon_axis > 180.0, lon_axis - 360.0, lon_axis)
    manifest["mrms"] = {
        "file": "mrms_regular_ll_png.grib2",
        "values_f32": dump_raw("mrms_values.f32le", decoded.astype(np.float32), np.float32),
        "values_f64": dump_raw("mrms_values.f64le", decoded, np.float64),
        "lat_axis_f64": dump_raw("mrms_lat_axis.f64le", lat_axis, np.float64),
        "lon_axis_f64": dump_raw("mrms_lon_axis.f64le", lon_axis, np.float64),
        "eccodes_lats_f64": dump_raw("mrms_eccodes_lats.f64le", lats, np.float64),
        "eccodes_lons_f64": dump_raw("mrms_eccodes_lons.f64le", lons, np.float64),
        "nj": nj, "ni": ni,
        "missing_count": int(np.isnan(decoded).sum()),
        "template": "5.41",
    }

    # ---------------- HRRR-like: Lambert 3.30, mixed 5.0 / 5.2 / 5.3 ----------------
    nx, ny = 60, 40
    yy, xx = np.mgrid[0:ny, 0:nx]
    cover = ((xx - 30) ** 2 / 900.0 + (yy - 20) ** 2 / 400.0) <= 1.0    # elliptical echo, NaN outside

    specs = [
        # name, (discipline, cat, num), typeOfFirstFixedSurface, level, packing, field, flags, bitmap
        ("refc", (0, 16, 196), 10, 0, "grid_complex_spatial_differencing", "refc", 0),
        ("t2m", (0, 0, 0), 103, 2, "grid_complex_spatial_differencing", "t2m", 0),
        ("u10", (0, 2, 2), 103, 10, "grid_complex_spatial_differencing", "u10", 8),
        ("v10", (0, 2, 3), 103, 10, "grid_simple", "v10", 8),
        ("cape", (0, 7, 6), 1, 0, "grid_complex", "cape", 0),
        ("apcp", (0, 1, 8), 1, 0, "grid_complex", "apcp", 0),
    ]
    field_gen = {
        "refc": lambda: np.round(rng.uniform(-10.0, 60.0, size=(ny, nx)) * 4) / 4,
        "t2m": lambda: np.round(rng.uniform(250.0, 305.0, size=(ny, nx)) * 8) / 8,
        "u10": lambda: np.round(rng.uniform(-25.0, 25.0, size=(ny, nx)) * 16) / 16,
        "v10": lambda: np.round(rng.uniform(-25.0, 25.0, size=(ny, nx)) * 16) / 16,
        "cape": lambda: np.round(rng.uniform(0.0, 4000.0, size=(ny, nx))),
        "apcp": lambda: np.round(rng.uniform(0.0, 30.0, size=(ny, nx)) * 100) / 100,
    }
    hrrr_path = os.path.join(HERE, "hrrr_lambert_mini.grib2")
    with open(hrrr_path, "wb") as f:
        for name, (d, c, n), tl, lev, packing, field, flags in specs:
            h = ec.codes_grib_new_from_samples("GRIB2")
            set_lambert(h, nx, ny)
            ec.codes_set(h, "discipline", d)
            ec.codes_set(h, "parameterCategory", c)
            ec.codes_set(h, "parameterNumber", n)
            ec.codes_set(h, "typeOfFirstFixedSurface", tl)
            ec.codes_set(h, "scaledValueOfFirstFixedSurface", lev)
            ec.codes_set(h, "resolutionAndComponentFlags", flags)
            ec.codes_set(h, "packingType", packing)
            if packing == "grid_complex_spatial_differencing":
                ec.codes_set(h, "orderOfSpatialDifferencing", 2)
            vals = field_gen[field]()
            # HRRR-style: missing outside the echo for refc, everywhere else all valid.
            if name == "refc":
                vals = np.where(cover, vals, np.nan)
            set_common(h, ny, nx, np.isnan(vals), vals)
            write_msg(f, h)
            # golden: re-read from the file we just wrote, so the decoder sees exactly these bytes
            manifest["hrrr"]["messages"].append({"name": name, "discipline": d, "category": c, "number": n,
                                                 "typeOfLevel": tl, "level": lev, "packing": packing,
                                                 "flags": flags, "grid_relative": bool(flags & 8)})
    idx = 0
    h_all = []
    with open(hrrr_path, "rb") as f:
        while True:
            h = ec.codes_new_from_file(f, ec.CODES_PRODUCT_GRIB)
            if h is None:
                break
            h_all.append(h)
    for h in h_all:
        m = manifest["hrrr"]["messages"][idx]
        name = m["name"]
        decoded = read_values(h)
        lat = np.array(ec.codes_get_double_array(h, "latitudes"), dtype=np.float64).reshape(ny, nx)
        lon = np.array(ec.codes_get_double_array(h, "longitudes"), dtype=np.float64).reshape(ny, nx)
        m["eccodes_name"] = ec.codes_get(h, "name")
        m["eccodes_shortName"] = ec.codes_get(h, "shortName")
        m["eccodes_typeOfLevel"] = ec.codes_get(h, "typeOfLevel")
        m["eccodes_packingType"] = ec.codes_get(h, "packingType")
        m["eccodes_projString"] = ec.codes_get(h, "projString")
        m["values_f32"] = dump_raw(f"hrrr_{name}_values.f32le", decoded.astype(np.float32), np.float32)
        m["values_f64"] = dump_raw(f"hrrr_{name}_values.f64le", decoded, np.float64)
        m["eccodes_lats_f64"] = dump_raw(f"hrrr_{name}_lats.f64le", lat, np.float64)
        m["eccodes_lons_f64"] = dump_raw(f"hrrr_{name}_lons.f64le", lon, np.float64)
        m["missing_count"] = int(np.isnan(decoded).sum())
        m["nx"], m["ny"] = nx, ny
        ec.codes_release(h)
        idx += 1
    # The Lambert projection axes are computed in gen_projection.py.
    with open(os.path.join(GOLD, "manifest.json"), "w") as f:
        json.dump(manifest, f, indent=2, sort_keys=True)
    print("wrote", grib_path, hrrr_path, "and golden manifest")


if __name__ == "__main__":
    main()
