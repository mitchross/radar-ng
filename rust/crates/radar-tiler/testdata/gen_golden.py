#!/usr/bin/env python3
"""Generate golden fixtures for radar-tiler from the Python reference (radar-ng backend/shared).

Run with a venv that has numpy, Pillow, SciPy and pyproj (see README.md):

    PYTHONDONTWRITEBYTECODE=1 python3 gen_golden.py /path/to/radar-ng

Outputs (next to this script):
  reflectivity/{data.f32,lats.f64,lons.f64,meta.json}   synthetic 240x480 fixture (test_tiler_indexed._synthetic_reflectivity)
  golden/reflectivity/{classic,muted,vivid}/z/x/y.png    Python indexed tiles, zooms 4,5,6, 256 px
  golden/categorical/{classic,muted,vivid}/...           Python categorical tiles (precip_type), zoom 5
  golden/projected_3857/p/...                            Python projected EPSG:3857 tiles, tile 64, zooms 4,5
  golden/overview_max/classic/...                        Python max-pooled overview tiles, zooms 4,7
  projected/{data.f32,source_x.f64,source_y.f64,lat_mesh.f64,lon_mesh.f64,meta.json}
  overview/{data.f32,lats.f64,lons.f64}
  categorical/{data.i16,lats.f64,lons.f64}
  palettes/                                              verbatim copies of the shipped palette JSON

The script never writes into the radar-ng tree. Pillow's PNG bytes are not reproduced by
radar-tiler (only decoded pixels are compared), so only the PNGs' decoded content matters.
"""

from __future__ import annotations

import json
import shutil
import sys
import tempfile
from pathlib import Path

import numpy as np

HERE = Path(__file__).resolve().parent


def main() -> None:
    if len(sys.argv) != 2:
        raise SystemExit(__doc__)
    repo = Path(sys.argv[1]).resolve()
    sys.path.insert(0, str(repo))
    from backend.shared import tiler  # noqa: E402
    from backend.shared.test_tiler_indexed import _synthetic_reflectivity  # noqa: E402
    from pyproj import Transformer  # noqa: E402

    palettes_dir = repo / "backend" / "shared" / "palettes"
    names = ("classic", "muted", "vivid")
    palettes = {n: json.loads((palettes_dir / f"{n}.json").read_text()) for n in names}
    golden = HERE / "golden"
    if golden.exists():
        shutil.rmtree(golden)
    work = Path(tempfile.mkdtemp(prefix="gen-golden-"))

    def publish(name, out_names, render_fn):
        out = {n: work / name / n for n in out_names}
        render_fn(out)
        for n in out_names:
            dst = golden / name / n
            dst.mkdir(parents=True, exist_ok=True)
            for png in (out[n]).rglob("*.png"):
                rel = png.relative_to(out[n])
                (dst / rel).parent.mkdir(parents=True, exist_ok=True)
                shutil.copy(png, dst / rel)

    # 1. Reflectivity: indexed continuous, three palettes.
    data, lats, lons = _synthetic_reflectivity()
    ref_dir = HERE / "reflectivity"
    ref_dir.mkdir(exist_ok=True)
    data.astype("<f4").tofile(ref_dir / "data.f32")
    lats.astype("<f8").tofile(ref_dir / "lats.f64")
    lons.astype("<f8").tofile(ref_dir / "lons.f64")
    (ref_dir / "meta.json").write_text(json.dumps({"height": 240, "width": 480}) + "\n")
    tables = {n: palettes[n]["reflectivity"] for n in names}
    publish(
        "reflectivity",
        names,
        lambda out: tiler.render_frame_palettes(
            data, lats, lons, tables, {n: str(p) for n, p in out.items()}, [4, 5, 6],
            nodata_value=None, min_valid_weight=1.0,
        ),
    )

    # 2. Categorical precip_type (nearest class reference).
    cat_map = {1: "rain", 2: "snow", 3: "freezing_rain", 4: "ice_pellets"}
    clats = np.linspace(25.0, 50.0, 120)
    clons = np.linspace(-125.0, -65.0, 240)
    cdata = np.zeros((120, 240), dtype=np.int16)
    cdata[15:105, 20:220] = np.indices((90, 200))[1] % 4 + 1
    cat_dir = HERE / "categorical"
    cat_dir.mkdir(exist_ok=True)
    cdata.astype("<i2").tofile(cat_dir / "data.i16")
    clats.astype("<f8").tofile(cat_dir / "lats.f64")
    clons.astype("<f8").tofile(cat_dir / "lons.f64")
    ctables = {n: palettes[n]["precip_type"] for n in names}
    publish(
        "categorical",
        names,
        lambda out: tiler.render_frame_palettes(
            cdata, clats, clons, ctables, {n: str(p) for n, p in out.items()}, [5],
            category_map=cat_map,
        ),
    )

    # 3. Projected EPSG:3857 continuous grid (single custom palette).
    to_merc = Transformer.from_crs("EPSG:4326", "EPSG:3857", always_xy=True)
    to_geo = Transformer.from_crs("EPSG:3857", "EPSG:4326", always_xy=True)
    x0, y0 = to_merc.transform(-125.0, 25.0)
    x1, y1 = to_merc.transform(-65.0, 50.0)
    source_x = np.linspace(x0, x1, 160)
    source_y = np.linspace(y1, y0, 100)
    xs, ys = np.meshgrid(source_x, source_y, indexing="xy")
    plons, plats = to_geo.transform(xs, ys)
    pdata = np.full((100, 160), np.nan, dtype=np.float32)
    pdata[20:80, 30:130] = 30.0
    ptable = {"ranges": [{"min": 0, "max": 100, "rgba": [0, 128, 255, 200]}], "no_data_below": -1}
    proj_dir = HERE / "projected"
    proj_dir.mkdir(exist_ok=True)
    pdata.astype("<f4").tofile(proj_dir / "data.f32")
    source_x.astype("<f8").tofile(proj_dir / "source_x.f64")
    source_y.astype("<f8").tofile(proj_dir / "source_y.f64")
    np.asarray(plats, dtype="<f8").tofile(proj_dir / "lat_mesh.f64")
    np.asarray(plons, dtype="<f8").tofile(proj_dir / "lon_mesh.f64")
    (proj_dir / "meta.json").write_text(
        json.dumps({"height": 100, "width": 160, "crs": "EPSG:3857", "tile_size": 64, "zooms": [4, 5]}) + "\n"
    )
    (proj_dir / "table.json").write_text(json.dumps(ptable, indent=1) + "\n")
    publish(
        "projected_3857",
        ["p"],
        lambda out: tiler.render_frame_palettes(
            pdata, plats, plons, {"p": ptable}, {"p": str(out["p"])}, [4, 5],
            tile_size=64, source_crs="EPSG:3857", source_x=source_x, source_y=source_y,
        ),
    )

    # 4. Max-pooled overview on a regular 0.01-degree grid with one intense core.
    olats = 30.0 + 0.01 * np.arange(600)
    olons = -110.0 + 0.01 * np.arange(900)
    odata = np.full((600, 900), -999.0, dtype=np.float32)
    odata[250:350, 400:550] = 20.0
    odata[301, 473] = 62.0
    ov_dir = HERE / "overview"
    ov_dir.mkdir(exist_ok=True)
    odata.astype("<f4").tofile(ov_dir / "data.f32")
    olats.astype("<f8").tofile(ov_dir / "lats.f64")
    olons.astype("<f8").tofile(ov_dir / "lons.f64")
    publish(
        "overview_max",
        ["classic"],
        lambda out: tiler.render_frame_palettes(
            odata, olats, olons, {"classic": palettes["classic"]["reflectivity"]},
            {"classic": str(out["classic"])}, [4, 7], overview="max",
        ),
    )

    # 5. Shipped palettes, verbatim, for the Rust crate's include_str! assets.
    pal_dir = HERE / "palettes"
    pal_dir.mkdir(exist_ok=True)
    for n in names:
        shutil.copy(palettes_dir / f"{n}.json", pal_dir / f"{n}.json")

    shutil.rmtree(work, ignore_errors=True)
    print("golden fixtures written under", HERE)


if __name__ == "__main__":
    main()
