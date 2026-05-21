import tempfile
import numpy as np
from pathlib import Path


def test_apply_color_table():
    from backend.shared.tiler import apply_color_table

    color_table = {
        "ranges": [
            {"min": 5, "max": 20, "rgba": [0, 255, 0, 200]},
            {"min": 20, "max": 40, "rgba": [255, 0, 0, 200]},
        ],
        "no_data_below": 5,
    }
    data = np.array([[3.0, 10.0], [25.0, 50.0]])
    rgba = apply_color_table(data, color_table)
    assert rgba.shape == (2, 2, 4)
    # Below threshold → transparent
    assert rgba[0, 0, 3] == 0
    # 10 dBZ → green
    assert rgba[0, 1, 1] == 255
    # 25 dBZ → red
    assert rgba[1, 0, 0] == 255
    # 50 dBZ is above max range → transparent
    assert rgba[1, 1, 3] == 0


def test_render_tiles_creates_files():
    from backend.shared.tiler import apply_color_table, render_tiles

    color_table = {
        "ranges": [{"min": 0, "max": 100, "rgba": [255, 0, 0, 200]}],
        "no_data_below": -1,
    }
    # Create a small CONUS-sized grid
    lats = np.linspace(25.0, 50.0, 100)
    lons = np.linspace(-125.0, -65.0, 200)
    data = np.random.uniform(10, 60, (100, 200)).astype(np.float32)
    rgba = apply_color_table(data, color_table)

    with tempfile.TemporaryDirectory() as tmpdir:
        count = render_tiles(
            rgba=rgba,
            lats=lats,
            lons=lons,
            output_dir=tmpdir,
            zoom_levels=[4, 5],
            tile_size=256,
        )
        assert count > 0
        # Check at least one tile exists
        tiles = list(Path(tmpdir).rglob("*.png"))
        assert len(tiles) > 0
        # Check tile path format: {z}/{x}/{y}.png
        first = tiles[0]
        parts = first.relative_to(tmpdir).parts
        assert len(parts) == 3  # z/x/y.png


def test_render_tiles_accepts_projected_source_grid():
    from pyproj import Transformer

    from backend.shared.tiler import apply_color_table, render_tiles

    color_table = {
        "ranges": [{"min": 0, "max": 100, "rgba": [0, 128, 255, 200]}],
        "no_data_below": -1,
    }
    to_merc = Transformer.from_crs("EPSG:4326", "EPSG:3857", always_xy=True)
    to_geo = Transformer.from_crs("EPSG:3857", "EPSG:4326", always_xy=True)
    x0, y0 = to_merc.transform(-125.0, 25.0)
    x1, y1 = to_merc.transform(-65.0, 50.0)
    source_x = np.linspace(x0, x1, 160)
    source_y = np.linspace(y1, y0, 100)
    xs, ys = np.meshgrid(source_x, source_y, indexing="xy")
    lons, lats = to_geo.transform(xs, ys)
    data = np.full((100, 160), 20.0, dtype=np.float32)
    rgba = apply_color_table(data, color_table)

    with tempfile.TemporaryDirectory() as tmpdir:
        count = render_tiles(
            rgba=rgba,
            lats=lats,
            lons=lons,
            output_dir=tmpdir,
            zoom_levels=[4],
            tile_size=64,
            source_crs="EPSG:3857",
            source_x=source_x,
            source_y=source_y,
        )

        assert count > 0
