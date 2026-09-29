"""Max-pooled overviews keep small intense cells visible at low zoom."""

from __future__ import annotations

import json
from pathlib import Path

import numpy as np
import pytest
from PIL import Image

from backend.shared import tiler
from backend.shared.tiler import (
    _footprint_edges,
    _max_block_factors,
    _reduce_ranges,
    render_frame_palettes,
)

PALETTES_DIR = Path(__file__).resolve().parent / "palettes"
CLASSIC = json.loads((PALETTES_DIR / "classic.json").read_text())["reflectivity"]
HEAVY_RGB = next(r["rgba"][:3] for r in CLASSIC["ranges"] if r["min"] == 60)


def _mrms_like_grid(lat0=30.0, lon0=-110.0, n_lat=600, n_lon=900, step=0.01):
    lats = lat0 + step * np.arange(n_lat)
    lons = lon0 + step * np.arange(n_lon)
    data = np.full((n_lat, n_lon), -999.0, dtype=np.float32)
    return data, lats, lons


def _contains_rgb(root: Path, rgb, zoom: int) -> bool:
    for png in (root / str(zoom)).rglob("*.png"):
        arr = np.asarray(Image.open(png).convert("RGBA"))
        hit = (arr[..., 0] == rgb[0]) & (arr[..., 1] == rgb[1]) & (arr[..., 2] == rgb[2])
        if (hit & (arr[..., 3] > 0)).any():
            return True
    return False


def test_block_factors_track_mercator_pixel_size():
    lats = 20.0 + 0.01 * np.arange(3500)
    lons = -130.0 + 0.01 * np.arange(7000)
    assert _max_block_factors(lats, lons, 4, 256) == (5, 8)
    assert _max_block_factors(lats, lons, 5, 256) == (2, 4)
    assert _max_block_factors(lats, lons, 7, 256) == (1, 1)


def test_footprint_edges_cover_every_cell_once_and_flag_off_grid_pixels():
    axis = np.arange(10, dtype=np.float64)  # cell i spans [i-0.5, i+0.5)
    start, stop = _footprint_edges(np.array([-3.0, -1.0, 2.5, 5.5, 9.4, 12.0]), axis)
    assert start.tolist() == [0, 0, 3, 6, 9]
    assert stop.tolist() == [0, 3, 6, 9, 10]
    # descending axes (north-up grids) map to the same cells
    d_start, d_stop = _footprint_edges(np.array([2.5, 5.5]), axis[::-1].copy())
    assert (d_stop - d_start).tolist() == [3]


def test_footprint_edges_keep_the_cell_under_a_sub_cell_pixel():
    start, stop = _footprint_edges(np.array([4.1, 4.2]), np.arange(10, dtype=np.float64))
    assert (start[0], stop[0]) == (4, 5)


def test_reduce_ranges_is_nan_aware_and_empty_ranges_are_nan():
    vals = np.array([[1.0, np.nan, 7.0, 2.0, np.nan]], dtype=np.float32)
    out = _reduce_ranges(vals, np.array([0, 1, 4, 5]), np.array([2, 4, 5, 5]), axis=1)
    assert out[0, 0] == 1.0 and out[0, 1] == 7.0
    assert np.isnan(out[0, 2]) and np.isnan(out[0, 3])


@pytest.mark.parametrize("renderer", ["legacy", "indexed"])
def test_single_cell_core_survives_low_zoom_only_with_overview(tmp_path, renderer):
    data, lats, lons = _mrms_like_grid()
    data[250:350, 400:550] = 20.0
    data[301, 473] = 62.0  # one ~1 km core, far smaller than a z4 pixel
    tables = {"classic": CLASSIC}

    def render(name, overview):
        root = tmp_path / name
        render_frame_palettes(
            data, lats, lons, tables, {"classic": str(root)}, [4, 7],
            renderer=renderer, source_id=f"t:{name}", overview=overview,
        )
        return root

    pooled = render("pooled", "max")
    sampled = render("sampled", None)
    assert _contains_rgb(pooled, HEAVY_RGB, 4)
    assert not _contains_rgb(sampled, HEAVY_RGB, 4)
    assert _contains_rgb(pooled, HEAVY_RGB, 7) == _contains_rgb(sampled, HEAVY_RGB, 7)


def test_overview_is_part_of_the_render_identity_only_when_set():
    base = tiler._frame_render_contract(renderer="legacy", color_tables={"classic": CLASSIC})
    same = tiler._frame_render_contract(renderer="legacy", color_tables={"classic": CLASSIC}, overview=None)
    pooled = tiler._frame_render_contract(renderer="legacy", color_tables={"classic": CLASSIC}, overview="max")
    assert base[1:3] == same[1:3]
    assert pooled[2]["overview"] == "max" and "overview" not in base[2]
    with pytest.raises(ValueError):
        tiler._frame_render_contract(renderer="legacy", color_tables={"classic": CLASSIC}, overview="mean")


def test_overview_rejects_categorical_grids(tmp_path):
    data, lats, lons = _mrms_like_grid()
    with pytest.raises(ValueError):
        render_frame_palettes(
            data.astype(np.int16), lats, lons,
            {"classic": {"categories": {"rain": [0, 255, 0, 255]}}},
            {"classic": str(tmp_path / "c")}, [4],
            renderer="legacy", category_map={1: "rain"}, overview="max",
        )
