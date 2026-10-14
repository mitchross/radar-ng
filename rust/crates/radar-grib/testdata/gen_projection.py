"""Golden Lambert projection axes and pyproj reference points (spec-ingest-mrms-hrrr §5.3).

Run in its own process (pyproj and eccodes segfault when imported together here), after gen_fixtures.py:
    python3 gen_projection.py

Reads golden/manifest.json and the eccodes lat/lon arrays written by gen_fixtures.py, then writes:
  golden/hrrr_src_x.f64le, golden/hrrr_src_y.f64le   the spec's linspace axes (nanmedian of edge projections)
  golden/hrrr_mesh_x.f64le, golden/hrrr_mesh_y.f64le  pyproj forward of every eccodes lat/lon point
  golden/lcc_points.f64le                               rows of (lon, lat, x, y) from pyproj forward
  golden/manifest_projection.json                       the CRS string used and axis endpoints
"""

import json
import os

import numpy as np
from pyproj import CRS, Transformer

HERE = os.path.dirname(os.path.abspath(__file__))
GOLD = os.path.join(HERE, "golden")


def load(name, dtype, shape):
    return np.fromfile(os.path.join(GOLD, name), dtype="<" + np.dtype(dtype).char).reshape(shape)


def dump(name, arr):
    with open(os.path.join(GOLD, name), "wb") as f:
        f.write(np.ascontiguousarray(arr, dtype="<f8").tobytes())


def main():
    man = json.load(open(os.path.join(GOLD, "manifest.json")))
    msg = man["hrrr"]["messages"][0]  # refc: same grid as every message
    ny, nx = msg["ny"], msg["nx"]
    lat = load(msg["eccodes_lats_f64"]["file"], np.float64, (ny, nx))
    lon = load(msg["eccodes_lons_f64"]["file"], np.float64, (ny, nx))
    proj_string = msg["eccodes_projString"]
    crs = CRS.from_user_input(proj_string)
    fwd = Transformer.from_crs("EPSG:4326", crs, always_xy=True)
    inv = Transformer.from_crs(crs, "EPSG:4326", always_xy=True)

    lon_n = np.where(lon > 180.0, lon - 360.0, lon)
    xs, ys = fwd.transform(lon_n, lat)
    h, w = lat.shape
    x0 = float(np.nanmedian(xs[:, 0]))
    x1 = float(np.nanmedian(xs[:, -1]))
    y0 = float(np.nanmedian(ys[0, :]))
    y1 = float(np.nanmedian(ys[-1, :]))
    src_x = np.linspace(x0, x1, w, dtype=np.float64)
    src_y = np.linspace(y0, y1, h, dtype=np.float64)

    dump("hrrr_src_x.f64le", src_x)
    dump("hrrr_src_y.f64le", src_y)
    dump("hrrr_mesh_x.f64le", xs)
    dump("hrrr_mesh_y.f64le", ys)
    dump("hrrr_lon_norm.f64le", lon_n)

    # A scatter of pyproj forward results for radar-geo::Crs::forward/inverse checks.
    rng = np.random.default_rng(7)
    pts_lon = np.concatenate([[-97.5, -122.7, -60.9, -85.67, -80.0], rng.uniform(-125.0, -65.0, 40)])
    pts_lat = np.concatenate([[38.5, 21.1, 47.8, 42.96, 25.0], rng.uniform(22.0, 50.0, 40)])
    px, py = fwd.transform(pts_lon, pts_lat)
    back_lon, back_lat = inv.transform(px, py)
    table = np.stack([pts_lon, pts_lat, px, py, back_lon, back_lat], axis=1)
    dump("lcc_points.f64le", table)

    with open(os.path.join(GOLD, "manifest_projection.json"), "w") as f:
        json.dump({
            "crs": proj_string,
            "shape": [h, w],
            "src_x_first": float(src_x[0]), "src_x_last": float(src_x[-1]),
            "src_y_first": float(src_y[0]), "src_y_last": float(src_y[-1]),
            "lcc_points_columns": ["lon", "lat", "x", "y", "inv_lon", "inv_lat"],
            "lcc_points_rows": int(table.shape[0]),
        }, f, indent=2, sort_keys=True)
    print("CRS:", proj_string)
    print("src_x", x0, x1, "src_y", y0, y1)


if __name__ == "__main__":
    main()
