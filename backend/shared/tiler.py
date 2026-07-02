"""Shared tile renderer: numpy array → PNG tiles in XYZ slippy map format."""

import math
import os
import shutil
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


def _axis_to_fractional_indices(values: np.ndarray, axis: np.ndarray) -> np.ndarray:
    """Map coordinates on a monotonic source axis to fractional pixel indices."""
    axis = np.asarray(axis, dtype=np.float64)
    if axis.ndim != 1 or len(axis) < 2:
        raise ValueError("source axes must be 1D arrays with at least two points")

    start = float(axis[0])
    stop = float(axis[-1])
    span = stop - start
    if abs(span) < 1e-12:
        raise ValueError("source axis span is zero")

    return (np.asarray(values, dtype=np.float64) - start) / span * (len(axis) - 1)


def _finite_index_bounds(values: np.ndarray, size: int) -> tuple[int, int] | None:
    finite = np.asarray(values)[np.isfinite(values)]
    if finite.size == 0:
        return None
    if float(finite.max()) < 0 or float(finite.min()) > size - 1:
        return None
    start = max(0, min(int(np.floor(float(finite.min()))), size - 1))
    end = max(0, min(int(np.ceil(float(finite.max()))), size - 1))
    if end <= start:
        return None
    return start, end


def render_tiles(
    rgba: np.ndarray,
    lats: np.ndarray,
    lons: np.ndarray,
    output_dir: str,
    zoom_levels: list[int],
    tile_size: int = 256,
    resample: int = Image.BILINEAR,
    source_crs: object | None = None,
    source_x: np.ndarray | None = None,
    source_y: np.ndarray | None = None,
) -> int:
    """Render RGBA array into XYZ PNG tiles. Returns number of tiles written.

    Tile pixels are generated in exact Web Mercator space, inverse-projected to
    lon/lat, then bilinearly sampled from the source grid.

    Most MRMS grids are regular EPSG:4326 rasters, so 1D `lats`/`lons` are
    enough. Curvilinear model grids such as HRRR's Lambert Conformal Conic
    should also pass `source_crs`, `source_x`, and `source_y`; those axes
    describe the regular native projection coordinates of the source pixels.
    """
    import scipy.ndimage

    lat_min, lat_max = float(lats.min()), float(lats.max())
    lon_min, lon_max = float(lons.min()), float(lons.max())
    h, w = rgba.shape[:2]
    count = 0
    del resample  # Kept for call-site compatibility; sampling order is fixed below.

    transformer = None
    if source_crs is not None or source_x is not None or source_y is not None:
        if source_crs is None or source_x is None or source_y is None:
            raise ValueError("source_crs, source_x, and source_y must be passed together")
        if len(source_x) != w or len(source_y) != h:
            raise ValueError("source_x/source_y lengths must match rgba width/height")
        from pyproj import Transformer

        transformer = Transformer.from_crs("EPSG:4326", source_crs, always_xy=True)
        source_x = np.asarray(source_x, dtype=np.float64)
        source_y = np.asarray(source_y, dtype=np.float64)
    else:
        if lats.ndim != 1 or lons.ndim != 1:
            raise ValueError("2D lats/lons require source_crs, source_x, and source_y")

    for z in zoom_levels:
        tx_min, ty_min = _lat_lon_to_tile(lat_max, lon_min, z)
        tx_max, ty_max = _lat_lon_to_tile(lat_min, lon_max, z)

        n = 2**z
        cols_grid = np.arange(tile_size) + 0.5
        rows_grid = np.arange(tile_size) + 0.5

        for tx in range(tx_min, tx_max + 1):
            xfs = (tx + cols_grid / tile_size) / n
            lons_tile = xfs * 360.0 - 180.0
            cols_1d = None
            if transformer is None:
                cols_1d = _axis_to_fractional_indices(lons_tile, lons)

                # Fast check: are the longitudes outside the source array entirely?
                if cols_1d.max() < 0 or cols_1d.min() > w - 1:
                    continue

            for ty in range(ty_min, ty_max + 1):
                yfs = (ty + rows_grid / tile_size) / n
                lats_tile = np.degrees(np.arctan(np.sinh(np.pi * (1.0 - 2.0 * yfs))))

                if transformer is None:
                    rows_1d = _axis_to_fractional_indices(lats_tile, lats)
                    if rows_1d.max() < 0 or rows_1d.min() > h - 1:
                        continue
                    rows_mapped, cols_mapped = np.meshgrid(rows_1d, cols_1d, indexing="ij")
                else:
                    lon_mesh, lat_mesh = np.meshgrid(lons_tile, lats_tile, indexing="xy")
                    xs, ys = transformer.transform(lon_mesh, lat_mesh)
                    cols_mapped = _axis_to_fractional_indices(xs, source_x)
                    rows_mapped = _axis_to_fractional_indices(ys, source_y)

                    finite = np.isfinite(rows_mapped) & np.isfinite(cols_mapped)
                    rows_mapped = np.where(finite, rows_mapped, -1.0)
                    cols_mapped = np.where(finite, cols_mapped, -1.0)

                row_bounds = _finite_index_bounds(rows_mapped, h)
                col_bounds = _finite_index_bounds(cols_mapped, w)
                if row_bounds is None or col_bounds is None:
                    continue

                # Quick bounding box check for transparency optimization
                row_start, row_end = row_bounds
                col_start, col_end = col_bounds

                # Check if region is completely transparent
                region = rgba[row_start:row_end + 1, col_start:col_end + 1]
                if region.size == 0 or region[:, :, 3].max() == 0:
                    continue

                # Perfect bilinear reprojection using scipy
                tile_rgba = np.zeros((tile_size, tile_size, 4), dtype=np.uint8)
                coords = [rows_mapped, cols_mapped]
                for b in range(4):
                    tile_rgba[:, :, b] = scipy.ndimage.map_coordinates(
                        rgba[:, :, b],
                        coords,
                        order=1,
                        mode="constant",
                        cval=0,
                    )

                # Skip if reprojected tile ended up transparent
                if tile_rgba[:, :, 3].max() == 0:
                    continue

                img = Image.fromarray(tile_rgba, "RGBA")
                tile_path = Path(output_dir) / str(z) / str(tx) / f"{ty}.png"
                tile_path.parent.mkdir(parents=True, exist_ok=True)
                img.save(str(tile_path), "PNG", optimize=False, compress_level=1)
                count += 1

    return count


def render_tiles_atomic(
    rgba: np.ndarray,
    lats: np.ndarray,
    lons: np.ndarray,
    output_dir: str,
    zoom_levels: list[int],
    **kwargs,
) -> int:
    """render_tiles, but the pyramid appears atomically at `output_dir`.

    Renders into a sibling `<name>.tmp` directory and renames it into place
    once complete. A crash mid-render leaves only a `.tmp` dir (the cleanup
    sweep removes stale ones) — a reader can never observe a partial
    pyramid, and a manifest entry never points at a half-written frame.

    If `output_dir` already exists (forecast layers re-render the same
    valid-time path on every model run) it is replaced.
    """
    final = Path(output_dir)
    tmp = final.parent / f"{final.name}.tmp"
    if tmp.exists():
        shutil.rmtree(tmp, ignore_errors=True)
    try:
        count = render_tiles(
            rgba=rgba, lats=lats, lons=lons,
            output_dir=str(tmp), zoom_levels=zoom_levels, **kwargs,
        )
    except BaseException:
        shutil.rmtree(tmp, ignore_errors=True)
        raise
    if count == 0:
        # Fully transparent frame → nothing was written, no dir to publish.
        shutil.rmtree(tmp, ignore_errors=True)
        return 0
    if final.exists():
        shutil.rmtree(final, ignore_errors=True)
    os.rename(tmp, final)
    return count
