"""Shared tile renderer: numpy array → PNG tiles in XYZ slippy map format."""

import math
from pathlib import Path

import numpy as np
from PIL import Image


def apply_color_table(
    data: np.ndarray, color_table: dict
) -> np.ndarray:
    """Apply a color table to a 2D data array, returning RGBA uint8 array."""
    h, w = data.shape
    rgba = np.zeros((h, w, 4), dtype=np.uint8)

    no_data = color_table.get("no_data_below", -999)

    for rng in color_table["ranges"]:
        mask = (data >= rng["min"]) & (data < rng["max"])
        rgba[mask] = rng["rgba"]

    # Anything below no_data threshold or not in any range → transparent
    below = data < no_data
    rgba[below] = [0, 0, 0, 0]

    return rgba


def apply_categorical_color_table(
    data: np.ndarray, categories: dict[str, list[int]], category_map: dict[int, str]
) -> np.ndarray:
    """Apply categorical colors (e.g., precip type) to a 2D integer array."""
    h, w = data.shape
    rgba = np.zeros((h, w, 4), dtype=np.uint8)

    for value, name in category_map.items():
        if name in categories:
            mask = data == value
            rgba[mask] = categories[name]

    return rgba


def _lat_lon_to_tile(lat: float, lon: float, zoom: int) -> tuple[int, int]:
    """Convert lat/lon to tile x, y at given zoom."""
    n = 2**zoom
    x = int((lon + 180.0) / 360.0 * n)
    lat_rad = math.radians(lat)
    y = int((1.0 - math.log(math.tan(lat_rad) + 1.0 / math.cos(lat_rad)) / math.pi) / 2.0 * n)
    return (max(0, min(x, n - 1)), max(0, min(y, n - 1)))


def _tile_bounds(x: int, y: int, z: int) -> tuple[float, float, float, float]:
    """Return (west, south, east, north) in degrees for a tile."""
    n = 2**z
    west = x / n * 360.0 - 180.0
    east = (x + 1) / n * 360.0 - 180.0
    north = math.degrees(math.atan(math.sinh(math.pi * (1 - 2 * y / n))))
    south = math.degrees(math.atan(math.sinh(math.pi * (1 - 2 * (y + 1) / n))))
    return (west, south, east, north)


def render_tiles(
    rgba: np.ndarray,
    lats: np.ndarray,
    lons: np.ndarray,
    output_dir: str,
    zoom_levels: list[int],
    tile_size: int = 256,
    resample: int = Image.BILINEAR,
) -> int:
    """Render RGBA array into XYZ PNG tiles. Returns number of tiles written.

    BILINEAR resampling smooths the dBZ→pixel transition so tile edges look
    crisp on the client (matches AccuWeather/RadarScope). Earlier we used
    NEAREST for a small render-time win, but the visual cost ("blocky")
    was real and the speedup wasn't worth it. PNG `optimize` stays off:
    tiles are short-lived (4–8h retention) and Caddy gzips on the wire,
    so the extra zlib pass halves throughput for ~5% size win.
    """
    lat_min, lat_max = float(lats.min()), float(lats.max())
    lon_min, lon_max = float(lons.min()), float(lons.max())
    h, w = rgba.shape[:2]
    count = 0

    for z in zoom_levels:
        tx_min, ty_min = _lat_lon_to_tile(lat_max, lon_min, z)
        tx_max, ty_max = _lat_lon_to_tile(lat_min, lon_max, z)

        for tx in range(tx_min, tx_max + 1):
            for ty in range(ty_min, ty_max + 1):
                west, south, east, north = _tile_bounds(tx, ty, z)

                # Map tile bounds to pixel indices in the source array
                col_start = int((west - lon_min) / (lon_max - lon_min) * w)
                col_end = int((east - lon_min) / (lon_max - lon_min) * w)
                row_start = int((lat_max - north) / (lat_max - lat_min) * h)
                row_end = int((lat_max - south) / (lat_max - lat_min) * h)

                col_start = max(0, min(col_start, w))
                col_end = max(0, min(col_end, w))
                row_start = max(0, min(row_start, h))
                row_end = max(0, min(row_end, h))

                if col_end <= col_start or row_end <= row_start:
                    continue

                region = rgba[row_start:row_end, col_start:col_end]

                # Skip fully transparent tiles
                if region[:, :, 3].max() == 0:
                    continue

                img = Image.fromarray(region, "RGBA")
                img = img.resize((tile_size, tile_size), resample)

                tile_path = Path(output_dir) / str(z) / str(tx) / f"{ty}.png"
                tile_path.parent.mkdir(parents=True, exist_ok=True)
                img.save(str(tile_path), "PNG", optimize=False, compress_level=1)
                count += 1

    return count
