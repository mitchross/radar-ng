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
