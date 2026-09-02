"""Shared tile renderer: numpy array → PNG tiles in XYZ slippy map format.

Two pipelines share the tile geometry code:

- Render once: continuous products bilinearly sample physical values before
  classification; categorical products classify the source grid and use
  nearest-neighbour sampling. Both encode one mode-"P" PNG per tile and derive
  every other palette by rewriting only its PLTE/tRNS chunks.
- Legacy RGBA (`render_tiles`): bilinear per-band resampling of a colourised
  array. Kept for callers that already hold RGBA and as the golden reference.
"""

from __future__ import annotations

import errno
import hashlib
import io
import json
import math
import os
import shutil
import threading
import uuid
from collections import OrderedDict
from collections.abc import Callable
from dataclasses import dataclass
from enum import Enum
from pathlib import Path

import numpy as np
from PIL import Image

from backend.shared.png_palette import replace_palette


def apply_color_table(data: np.ndarray, color_table: dict) -> np.ndarray:
    """Apply a color table to a 2D data array, returning RGBA uint8 array."""
    model = build_class_model({"_": color_table})
    return model.luts["_"][model.classify(data)]


def apply_categorical_color_table(
    data: np.ndarray, categories: dict[str, list[int]], category_map: dict[int, str]
) -> np.ndarray:
    """Apply categorical colors (e.g., precip type) to a 2D integer array."""
    model = build_categorical_class_model(
        {"_": {"categories": categories}}, category_map
    )
    return model.luts["_"][model.classify(data)]


# ---------- render once: classification model ----------

_CLASSIFY_CHUNK_CELLS = 1 << 21  # bounds the int64 searchsorted temporary to ~16 MB


@dataclass(frozen=True)
class ClassModel:
    """Value → class index shared by every palette, plus one RGBA LUT per palette.

    Class 0 is transparent in all palettes, so `idx.any()` is a transparency
    test. Classes are the union of every palette's bin edges (and
    `no_data_below`), so palettes with different edges still share one index.
    """

    edges: np.ndarray | None  # strictly increasing bin edges; None → categorical
    remap: np.ndarray  # raw searchsorted index (or category value) → class
    luts: dict[str, np.ndarray]  # palette → (n_classes, 4) uint8

    @property
    def n_classes(self) -> int:
        return int(next(iter(self.luts.values())).shape[0])

    def plte(self, palette: str) -> bytes:
        return np.ascontiguousarray(self.luts[palette][:, :3]).tobytes()

    def trns(self, palette: str) -> bytes:
        return np.ascontiguousarray(self.luts[palette][:, 3]).tobytes()

    def classify(self, data: np.ndarray) -> np.ndarray:
        """Return the uint8 class index of `data` (same shape). NaN/inf → class 0."""
        data = np.asarray(data)
        out = np.empty(data.shape, dtype=np.uint8)
        if data.size == 0:
            return out
        if data.ndim == 2:
            rows = max(1, _CLASSIFY_CHUNK_CELLS // max(1, data.shape[1]))
            chunks = (
                (data[r : r + rows], out[r : r + rows])
                for r in range(0, data.shape[0], rows)
            )
        else:
            flat_in, flat_out = data.reshape(-1), out.reshape(-1)
            chunks = (
                (
                    flat_in[i : i + _CLASSIFY_CHUNK_CELLS],
                    flat_out[i : i + _CLASSIFY_CHUNK_CELLS],
                )
                for i in range(0, flat_in.size, _CLASSIFY_CHUNK_CELLS)
            )
        if self.edges is None:
            top = self.remap.shape[0] - 1
            for src, dst in chunks:
                v = src.astype(np.int64, copy=False)
                v = np.where((v < 0) | (v > top), 0, v)
                dst[...] = self.remap[v]
            return out
        edges = self.edges
        if data.dtype.kind == "f" and data.dtype != edges.dtype:
            # Compare in the data's own precision so a float32 value that equals
            # a rounded edge lands on the same side as `data >= edge` did.
            cast = edges.astype(data.dtype)
            if np.all(np.diff(cast) > 0):
                edges = cast
        for src, dst in chunks:
            dst[...] = self.remap[np.searchsorted(edges, src, side="right")]
        return out


def _collapse_transparent(
    raw_luts: dict[str, np.ndarray],
) -> tuple[np.ndarray, dict[str, np.ndarray]]:
    """Renumber raw classes so 0 is the only class transparent in every palette."""
    raw_n = next(iter(raw_luts.values())).shape[0]
    visible = np.zeros(raw_n, dtype=bool)
    for lut in raw_luts.values():
        visible |= lut[:, 3] > 0
    remap = np.zeros(raw_n, dtype=np.uint8)
    keep = np.flatnonzero(visible)
    if keep.size > 255:
        raise ValueError(f"{keep.size} visible classes exceed the uint8 palette budget")
    remap[keep] = np.arange(1, keep.size + 1, dtype=np.uint8)
    luts = {}
    for name, lut in raw_luts.items():
        out = np.zeros((keep.size + 1, 4), dtype=np.uint8)
        out[1:] = lut[keep]
        luts[name] = out
    return remap, luts


def build_class_model(color_tables: dict[str, dict]) -> ClassModel:
    """Build one shared class index for several `{"ranges": [...], "no_data_below": x}` tables.

    Per palette the LUT reproduces `apply_color_table` exactly: later ranges
    win where ranges overlap, and anything below `no_data_below` is transparent.
    """
    if not color_tables:
        raise ValueError("at least one color table is required")
    edge_set: set[float] = set()
    for table in color_tables.values():
        for rng in table["ranges"]:
            edge_set.add(float(rng["min"]))
            edge_set.add(float(rng["max"]))
        edge_set.add(float(table.get("no_data_below", -999)))
    edges = np.array(sorted(edge_set), dtype=np.float64)
    raw_n = len(edges) + 1  # 0 = below all edges, len(edges) = at/above the last edge

    raw_luts: dict[str, np.ndarray] = {}
    for name, table in color_tables.items():
        lut = np.zeros((raw_n, 4), dtype=np.uint8)
        no_data = float(table.get("no_data_below", -999))
        for i in range(1, len(edges)):
            lo, hi = edges[i - 1], edges[i]
            if lo < no_data:
                continue
            for rng in table["ranges"]:
                if float(rng["min"]) <= lo and hi <= float(rng["max"]):
                    lut[i] = rng["rgba"]
        raw_luts[name] = lut
    remap, luts = _collapse_transparent(raw_luts)
    return ClassModel(edges=edges, remap=remap, luts=luts)


def build_categorical_class_model(
    color_tables: dict[str, dict], category_map: dict[int, str]
) -> ClassModel:
    """Class model for integer category grids; each table is `{"categories": {name: rgba}}`."""
    if not color_tables:
        raise ValueError("at least one color table is required")
    values = [int(v) for v in category_map if int(v) >= 0]
    raw_n = (max(values) if values else 0) + 1
    raw_luts: dict[str, np.ndarray] = {}
    for name, table in color_tables.items():
        lut = np.zeros((raw_n, 4), dtype=np.uint8)
        categories = table["categories"]
        for value, cat_name in category_map.items():
            if int(value) >= 0 and cat_name in categories:
                lut[int(value)] = categories[cat_name]
        raw_luts[name] = lut
    remap, luts = _collapse_transparent(raw_luts)
    return ClassModel(edges=None, remap=remap, luts=luts)


def _lat_lon_to_tile(lat: float, lon: float, zoom: int) -> tuple[int, int]:
    """Convert lat/lon to tile x, y at given zoom."""
    n = 2**zoom
    x = int((lon + 180.0) / 360.0 * n)
    lat_rad = math.radians(lat)
    y = int(
        (1.0 - math.log(math.tan(lat_rad) + 1.0 / math.cos(lat_rad)) / math.pi)
        / 2.0
        * n
    )
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
    steps = np.diff(axis)
    if np.all(steps > 0):
        ordered = axis
        positions = np.arange(len(axis), dtype=np.float64)
    elif np.all(steps < 0):
        ordered = axis[::-1]
        positions = np.arange(len(axis) - 1, -1, -1, dtype=np.float64)
    else:
        raise ValueError("source axes must be strictly monotonic")

    values = np.asarray(values, dtype=np.float64)
    # NOAA grids are normally affine. Keep that path allocation-light while
    # correctly supporting a monotonic non-uniform axis when one is supplied.
    if np.allclose(steps, steps[0], rtol=1e-7, atol=abs(float(steps[0])) * 1e-10):
        return (values - axis[0]) / steps[0]

    segment = np.searchsorted(ordered, values, side="right") - 1
    segment = np.clip(segment, 0, len(ordered) - 2)
    fraction = (values - ordered[segment]) / (ordered[segment + 1] - ordered[segment])
    return positions[segment] + fraction * (positions[segment + 1] - positions[segment])


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
            raise ValueError(
                "source_crs, source_x, and source_y must be passed together"
            )
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
                    rows_mapped, cols_mapped = np.meshgrid(
                        rows_1d, cols_1d, indexing="ij"
                    )
                else:
                    lon_mesh, lat_mesh = np.meshgrid(
                        lons_tile, lats_tile, indexing="xy"
                    )
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
                region = rgba[row_start : row_end + 1, col_start : col_end + 1]
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

    Renders into a unique sibling staging directory and renames it into place
    once complete. A crash mid-render leaves only a `.tmp` dir (the cleanup
    sweep removes stale ones) — a reader can never observe a partial
    pyramid, and a manifest entry never points at a half-written frame.

    Published paths are immutable. If a retry finds an existing complete
    directory, the new staging tree is discarded. Forecast runs therefore use
    run-versioned paths instead of rewriting a valid-time directory in place.
    """
    final = Path(output_dir)
    tmp = final.parent / f".{final.name}.tmp-{uuid.uuid4().hex}"
    try:
        count = render_tiles(
            rgba=rgba,
            lats=lats,
            lons=lons,
            output_dir=str(tmp),
            zoom_levels=zoom_levels,
            **kwargs,
        )
    except BaseException:
        shutil.rmtree(tmp, ignore_errors=True)
        raise
    if count == 0:
        # Fully transparent frame → nothing was written, no dir to publish.
        shutil.rmtree(tmp, ignore_errors=True)
        return 0
    try:
        os.rename(tmp, final)
    except OSError as exc:
        if exc.errno not in (errno.EEXIST, errno.ENOTEMPTY):
            shutil.rmtree(tmp, ignore_errors=True)
            raise
        # Another retry/activity won the publish race. The winner is already a
        # complete immutable pyramid, so never create a delete/rename 404 gap.
        shutil.rmtree(tmp, ignore_errors=True)
    return count


# ---------- render once: tile geometry cache ----------


def _bounded_int_env(name: str, default: int, minimum: int, maximum: int) -> int:
    try:
        value = int(os.environ.get(name, str(default)))
    except ValueError:
        return default
    return min(maximum, max(minimum, value))


TILE_GEOMETRY_CACHE_MB = _bounded_int_env("TILE_GEOMETRY_CACHE_MB", 256, 0, 4096)
TILE_GEOMETRY_CACHE_ENTRIES = _bounded_int_env(
    "TILE_GEOMETRY_CACHE_ENTRIES",
    4096,
    1,
    1_000_000,
)
PNG_COMPRESS_LEVEL = _bounded_int_env("TILE_PNG_COMPRESS_LEVEL", 6, 0, 9)


def _array_fingerprint(values: np.ndarray) -> tuple[tuple[int, ...], str, bytes]:
    """Stable cache identity for the coordinate arrays that define a grid."""
    arr = np.ascontiguousarray(np.asarray(values))
    digest = hashlib.blake2b(arr.view(np.uint8), digest_size=16).digest()
    return arr.shape, arr.dtype.str, digest


def _finite_bounds(values: np.ndarray, name: str) -> tuple[float, float]:
    arr = np.asarray(values)
    if arr.size == 0:
        raise ValueError(f"{name} has no finite coordinates")
    # np.nanmin/max avoid copying the multi-million-cell HRRR coordinate
    # meshes on every frame. Infinity is exceptional, so pay for a filtered
    # fallback only when it is actually present at an extremum.
    with np.errstate(all="ignore"):
        low = float(np.nanmin(arr))
        high = float(np.nanmax(arr))
    if math.isfinite(low) and math.isfinite(high):
        return low, high
    finite = arr[np.isfinite(arr)]
    if finite.size == 0:
        raise ValueError(f"{name} has no finite coordinates")
    return float(finite.min()), float(finite.max())


@dataclass(frozen=True)
class GridSpec:
    """Immutable identity for every input that affects tile coordinate mapping.

    Projected grids are mapped from their native x/y axes, so hashing multi-
    million-cell latitude/longitude meshes every frame would be pure overhead;
    their finite bbox is sufficient for candidate rejection and the full native
    axes are fingerprinted for the actual mapping. Regular axes are small and
    fingerprinted in full, preventing same-endpoint/non-linear grids colliding.
    """

    kind: str
    height: int
    width: int
    lat_bounds: tuple[float, float]
    lon_bounds: tuple[float, float]
    coordinate_identity: tuple[object, ...]
    sampling: str


def _grid_spec(
    h: int,
    w: int,
    lats: np.ndarray,
    lons: np.ndarray,
    source_crs: object | None,
    source_x: np.ndarray | None,
    source_y: np.ndarray | None,
    sampling: str,
) -> GridSpec:
    if sampling not in {"bilinear", "nearest"}:
        raise ValueError("sampling must be 'bilinear' or 'nearest'")
    lat_bounds = _finite_bounds(lats, "lats")
    lon_bounds = _finite_bounds(lons, "lons")
    if source_crs is None:
        if source_x is not None or source_y is not None:
            raise ValueError(
                "source_crs, source_x, and source_y must be passed together"
            )
        if lats.ndim != 1 or lons.ndim != 1:
            raise ValueError("2D lats/lons require source_crs, source_x, and source_y")
        if len(lats) != h or len(lons) != w:
            raise ValueError("lats/lons lengths must match the grid height/width")
        identity: tuple[object, ...] = (
            _array_fingerprint(lats),
            _array_fingerprint(lons),
        )
        kind = "regular"
    else:
        if source_x is None or source_y is None:
            raise ValueError(
                "source_crs, source_x, and source_y must be passed together"
            )
        if len(source_x) != w or len(source_y) != h:
            raise ValueError(
                "source_x/source_y lengths must match the grid width/height"
            )
        identity = (
            str(source_crs),
            _array_fingerprint(source_x),
            _array_fingerprint(source_y),
        )
        kind = "projected"
    return GridSpec(
        kind=kind,
        height=h,
        width=w,
        lat_bounds=lat_bounds,
        lon_bounds=lon_bounds,
        coordinate_identity=identity,
        sampling=sampling,
    )


@dataclass(frozen=True)
class _TileGeometry:
    """Cached source coordinates for one tile and one reviewed sampling mode."""

    sampling: str
    rows: np.ndarray | None  # regular: [tile]; projected bilinear: [tile,tile]
    cols: np.ndarray | None  # regular: [tile]; projected bilinear: [tile,tile]
    flat: np.ndarray | None  # projected nearest: row*w + col, -1 outside
    row_bounds: tuple[int, int]
    col_bounds: tuple[int, int]

    @property
    def nbytes(self) -> int:
        return sum(
            int(a.nbytes) for a in (self.rows, self.cols, self.flat) if a is not None
        )

    def sample_nearest(self, idx: np.ndarray) -> np.ndarray:
        """Nearest-neighbour gather from a C-contiguous 2D class index."""
        if self.sampling != "nearest":
            raise ValueError("nearest sampling requires nearest tile geometry")
        if self.flat is None:
            assert self.rows is not None and self.cols is not None
            tile = idx[np.ix_(np.maximum(self.rows, 0), np.maximum(self.cols, 0))]
            tile[self.rows < 0, :] = 0
            tile[:, self.cols < 0] = 0
            return tile
        tile = idx.reshape(-1)[np.maximum(self.flat, 0)]
        tile[self.flat < 0] = 0
        return tile

    def sample_bilinear(
        self,
        data: np.ndarray,
        *,
        nodata_value: float | None,
        min_valid_weight: float,
    ) -> np.ndarray:
        """Bilinearly sample physical values without ever interpolating classes.

        Invalid contributors are excluded from the weighted sum. By default a
        pixel is emitted only when all non-zero bilinear support is valid
        (`min_valid_weight=1`), which prevents NaN/fill values from bleeding or
        expanding a field at its coverage boundary.
        """
        if self.sampling != "bilinear" or self.rows is None or self.cols is None:
            raise ValueError("bilinear sampling requires bilinear tile geometry")
        if not 0 < min_valid_weight <= 1:
            raise ValueError("min_valid_weight must be in (0, 1]")
        values = np.asarray(data).astype(np.float32, copy=False)
        h, w = values.shape

        if self.rows.ndim == self.cols.ndim == 1:
            row_floor = np.floor(self.rows).astype(np.int32)
            col_floor = np.floor(self.cols).astype(np.int32)
            row_frac = (self.rows - row_floor)[:, None]
            col_frac = (self.cols - col_floor)[None, :]
            coord_valid = ((self.rows >= 0) & (self.rows <= h - 1))[:, None] & (
                (self.cols >= 0) & (self.cols <= w - 1)
            )[None, :]
            r0 = np.clip(row_floor, 0, h - 1)
            r1 = np.clip(row_floor + 1, 0, h - 1)
            c0 = np.clip(col_floor, 0, w - 1)
            c1 = np.clip(col_floor + 1, 0, w - 1)
            samples = (
                values[np.ix_(r0, c0)],
                values[np.ix_(r0, c1)],
                values[np.ix_(r1, c0)],
                values[np.ix_(r1, c1)],
            )
        else:
            if self.rows.shape != self.cols.shape:
                raise ValueError("projected row/column coordinate shapes differ")
            row_floor = np.floor(self.rows).astype(np.int32)
            col_floor = np.floor(self.cols).astype(np.int32)
            row_frac = self.rows - row_floor
            col_frac = self.cols - col_floor
            coord_valid = (
                (self.rows >= 0)
                & (self.rows <= h - 1)
                & (self.cols >= 0)
                & (self.cols <= w - 1)
            )
            r0 = np.clip(row_floor, 0, h - 1)
            r1 = np.clip(row_floor + 1, 0, h - 1)
            c0 = np.clip(col_floor, 0, w - 1)
            c1 = np.clip(col_floor + 1, 0, w - 1)
            samples = (
                values[r0, c0],
                values[r0, c1],
                values[r1, c0],
                values[r1, c1],
            )

        weights = (
            (1 - row_frac) * (1 - col_frac),
            (1 - row_frac) * col_frac,
            row_frac * (1 - col_frac),
            row_frac * col_frac,
        )
        numerator = np.zeros(coord_valid.shape, dtype=np.float32)
        valid_weight = np.zeros(coord_valid.shape, dtype=np.float32)
        for sample, weight in zip(samples, weights):
            valid = np.isfinite(sample)
            if nodata_value is not None and math.isfinite(nodata_value):
                valid &= sample != np.float32(nodata_value)
            numerator += np.where(valid, sample, 0.0) * weight
            valid_weight += valid * weight

        good = coord_valid & (valid_weight >= np.float32(min_valid_weight - 1e-6))
        out = np.full(coord_valid.shape, np.nan, dtype=np.float32)
        out[good] = numerator[good] / valid_weight[good]
        return out


_geometry_cache: "OrderedDict[tuple, _TileGeometry | None]" = OrderedDict()
_geometry_cache_bytes = 0
_geometry_cache_lock = threading.Lock()
_geometry_cache_inflight: dict[tuple, threading.Event] = {}
_geometry_cache_stats = {"hits": 0, "misses": 0, "waits": 0, "evictions": 0}


def tile_geometry_cache_stats() -> dict[str, int]:
    with _geometry_cache_lock:
        return {
            "hits": _geometry_cache_stats["hits"],
            "misses": _geometry_cache_stats["misses"],
            "waits": _geometry_cache_stats["waits"],
            "evictions": _geometry_cache_stats["evictions"],
            "entries": len(_geometry_cache),
            "bytes": _geometry_cache_bytes,
        }


def clear_tile_geometry_cache() -> None:
    global _geometry_cache_bytes
    with _geometry_cache_lock:
        _geometry_cache.clear()
        _geometry_cache_bytes = 0
        for name in _geometry_cache_stats:
            _geometry_cache_stats[name] = 0


def _geometry_cache_put(key: tuple, geom: _TileGeometry | None) -> None:
    global _geometry_cache_bytes
    size = geom.nbytes if geom is not None else 0
    limit = TILE_GEOMETRY_CACHE_MB * 1024 * 1024
    with _geometry_cache_lock:
        if key in _geometry_cache:
            return
        _geometry_cache[key] = geom
        _geometry_cache_bytes += size
        while (
            _geometry_cache_bytes > limit
            or len(_geometry_cache) > TILE_GEOMETRY_CACHE_ENTRIES
        ) and len(_geometry_cache) > 1:
            _, old = _geometry_cache.popitem(last=False)
            _geometry_cache_bytes -= old.nbytes if old is not None else 0
            _geometry_cache_stats["evictions"] += 1


def _geometry_cache_get_or_compute(
    key: tuple,
    compute: Callable[[], _TileGeometry | None],
) -> _TileGeometry | None:
    """Single-flight cache fill: one projection per key under concurrent frames."""
    while True:
        with _geometry_cache_lock:
            if key in _geometry_cache:
                _geometry_cache.move_to_end(key)
                _geometry_cache_stats["hits"] += 1
                return _geometry_cache[key]
            event = _geometry_cache_inflight.get(key)
            if event is None:
                event = threading.Event()
                _geometry_cache_inflight[key] = event
                _geometry_cache_stats["misses"] += 1
                break
            _geometry_cache_stats["waits"] += 1
        event.wait()

    try:
        geom = compute()
        _geometry_cache_put(key, geom)
    except BaseException:
        with _geometry_cache_lock:
            _geometry_cache_inflight.pop(key, None)
            event.set()
        raise
    else:
        with _geometry_cache_lock:
            _geometry_cache_inflight.pop(key, None)
            event.set()
    return geom


def _nearest_index(coord: np.ndarray, n: int) -> np.ndarray:
    """scipy order=0/mode=constant semantics: valid on [0, n-1], round half up."""
    valid = (coord >= 0) & (coord <= n - 1)
    out = np.floor(np.where(valid, coord, 0.0) + 0.5).astype(np.int32)
    np.minimum(out, n - 1, out=out)
    out[~valid] = -1
    return out


def _compute_tile_geometry(
    z: int,
    tx: int,
    ty: int,
    tile_size: int,
    h: int,
    w: int,
    lats: np.ndarray,
    lons: np.ndarray,
    bbox: tuple[float, float, float, float],
    transformer_factory: Callable[[], object] | None,
    source_x: np.ndarray | None,
    source_y: np.ndarray | None,
    sampling: str,
) -> _TileGeometry | None:
    n = 2**z
    px = (np.arange(tile_size) + 0.5) / tile_size
    lons_tile = (tx + px) / n * 360.0 - 180.0
    lats_tile = np.degrees(np.arctan(np.sinh(np.pi * (1.0 - 2.0 * (ty + px) / n))))

    if transformer_factory is None:
        cols_1d = _axis_to_fractional_indices(lons_tile, lons)
        if cols_1d.max() < 0 or cols_1d.min() > w - 1:
            return None
        rows_1d = _axis_to_fractional_indices(lats_tile, lats)
        if rows_1d.max() < 0 or rows_1d.min() > h - 1:
            return None
        row_bounds = _finite_index_bounds(rows_1d, h)
        col_bounds = _finite_index_bounds(cols_1d, w)
        if row_bounds is None or col_bounds is None:
            return None
        if sampling == "nearest":
            rows = _nearest_index(rows_1d, h)
            cols = _nearest_index(cols_1d, w)
            if not (rows >= 0).any() or not (cols >= 0).any():
                return None
        else:
            rows = rows_1d.astype(np.float32)
            cols = cols_1d.astype(np.float32)
        return _TileGeometry(
            sampling=sampling,
            rows=rows,
            cols=cols,
            flat=None,
            row_bounds=row_bounds,
            col_bounds=col_bounds,
        )

    # Lat/lon bbox rejection first: it is conservative and skips the pyproj call entirely.
    lat_min, lat_max, lon_min, lon_max = bbox
    west, south, east, north = _tile_bounds(tx, ty, z)
    if east < lon_min or west > lon_max or north < lat_min or south > lat_max:
        return None
    lon_mesh, lat_mesh = np.meshgrid(lons_tile, lats_tile, indexing="xy")
    transformer = transformer_factory()
    xs, ys = transformer.transform(lon_mesh, lat_mesh)  # type: ignore[attr-defined]
    cols_mapped = _axis_to_fractional_indices(xs, source_x)
    rows_mapped = _axis_to_fractional_indices(ys, source_y)
    finite = np.isfinite(rows_mapped) & np.isfinite(cols_mapped)
    rows_mapped = np.where(finite, rows_mapped, -1.0)
    cols_mapped = np.where(finite, cols_mapped, -1.0)
    row_bounds = _finite_index_bounds(rows_mapped, h)
    col_bounds = _finite_index_bounds(cols_mapped, w)
    if row_bounds is None or col_bounds is None:
        return None
    if sampling == "nearest":
        ri = _nearest_index(rows_mapped, h)
        ci = _nearest_index(cols_mapped, w)
        valid = (ri >= 0) & (ci >= 0)
        if not valid.any():
            return None
        flat = np.where(valid, ri.astype(np.int64) * w + ci, -1).astype(np.int32)
        rows_out = cols_out = None
    else:
        flat = None
        rows_out = rows_mapped.astype(np.float32)
        cols_out = cols_mapped.astype(np.float32)
    return _TileGeometry(
        sampling=sampling,
        rows=rows_out,
        cols=cols_out,
        flat=flat,
        row_bounds=row_bounds,
        col_bounds=col_bounds,
    )


def _iter_tile_geometry(
    lats: np.ndarray,
    lons: np.ndarray,
    h: int,
    w: int,
    zoom_levels: list[int],
    tile_size: int,
    source_crs: object | None,
    source_x: np.ndarray | None,
    source_y: np.ndarray | None,
    sampling: str = "nearest",
):
    """Yield (z, tx, ty, geometry) for every tile that overlaps the grid, via the process cache."""
    if source_crs is not None or source_x is not None or source_y is not None:
        if source_crs is None or source_x is None or source_y is None:
            raise ValueError(
                "source_crs, source_x, and source_y must be passed together"
            )
        if len(source_x) != w or len(source_y) != h:
            raise ValueError(
                "source_x/source_y lengths must match the grid width/height"
            )
        source_x = np.asarray(source_x, dtype=np.float64)
        source_y = np.asarray(source_y, dtype=np.float64)

    spec = _grid_spec(h, w, lats, lons, source_crs, source_x, source_y, sampling)
    lat_min, lat_max = spec.lat_bounds
    lon_min, lon_max = spec.lon_bounds
    bbox = (lat_min, lat_max, lon_min, lon_max)
    transformer: object | None = None

    def _transformer() -> object:
        nonlocal transformer
        if transformer is None:
            from pyproj import Transformer

            transformer = Transformer.from_crs("EPSG:4326", source_crs, always_xy=True)
        return transformer

    transformer_factory = _transformer if source_crs is not None else None

    for z in zoom_levels:
        tx_min, ty_min = _lat_lon_to_tile(lat_max, lon_min, z)
        tx_max, ty_max = _lat_lon_to_tile(lat_min, lon_max, z)
        for tx in range(tx_min, tx_max + 1):
            for ty in range(ty_min, ty_max + 1):
                key = (spec, tile_size, z, tx, ty)
                geom = _geometry_cache_get_or_compute(
                    key,
                    lambda z=z, tx=tx, ty=ty: _compute_tile_geometry(
                        z,
                        tx,
                        ty,
                        tile_size,
                        h,
                        w,
                        lats,
                        lons,
                        bbox,
                        transformer_factory,
                        source_x,
                        source_y,
                        sampling,
                    ),
                )
                if geom is None:
                    continue
                yield z, tx, ty, geom


# ---------- render once: indexed tiles ----------


def _region_transparent(region: np.ndarray) -> bool:
    if region.size == 0:
        return True
    # Large (low-zoom) regions: a strided probe usually proves "not empty" cheaply.
    if region.size > (1 << 22) and region[::8, ::8].any():
        return False
    return not region.any()


def encode_indexed_png(tile: np.ndarray, plte: bytes, trns: bytes) -> bytes:
    """Encode a uint8 class-index tile as a mode-"P" PNG with the given PLTE/tRNS."""
    tile = np.ascontiguousarray(tile, dtype=np.uint8)
    img = Image.frombuffer(
        "P", (tile.shape[1], tile.shape[0]), tile.tobytes(), "raw", "P", 0, 1
    )
    img.putpalette(plte)
    buf = io.BytesIO()
    img.save(
        buf, "PNG", transparency=trns, optimize=False, compress_level=PNG_COMPRESS_LEVEL
    )
    return buf.getvalue()


def _palette_chunks(
    output_dirs: dict[str, str],
    luts: dict[str, np.ndarray],
) -> tuple[list[str], dict[str, bytes], dict[str, bytes]]:
    palettes = list(output_dirs)
    if set(palettes) - set(luts):
        missing = sorted(set(palettes) - set(luts))
        raise ValueError(f"missing lookup tables for palettes: {missing}")
    plte = {p: np.ascontiguousarray(luts[p][:, :3]).tobytes() for p in palettes}
    trns = {p: np.ascontiguousarray(luts[p][:, 3]).tobytes() for p in palettes}
    return palettes, plte, trns


def _write_index_tile(
    tile: np.ndarray,
    z: int,
    tx: int,
    ty: int,
    output_dirs: dict[str, str],
    palettes: list[str],
    plte: dict[str, bytes],
    trns: dict[str, bytes],
    made_dirs: set[Path],
) -> None:
    first = palettes[0]
    png = encode_indexed_png(tile, plte[first], trns[first])
    for palette in palettes:
        body = (
            png
            if palette == first
            else replace_palette(png, plte[palette], trns[palette])
        )
        tile_dir = Path(output_dirs[palette]) / str(z) / str(tx)
        if tile_dir not in made_dirs:
            tile_dir.mkdir(parents=True, exist_ok=True)
            made_dirs.add(tile_dir)
        (tile_dir / f"{ty}.png").write_bytes(body)


def render_indexed_tiles(
    idx: np.ndarray,
    lats: np.ndarray,
    lons: np.ndarray,
    output_dirs: dict[str, str],
    luts: dict[str, np.ndarray],
    zoom_levels: list[int],
    tile_size: int = 256,
    source_crs: object | None = None,
    source_x: np.ndarray | None = None,
    source_y: np.ndarray | None = None,
) -> int:
    """Write one pyramid per palette from a single uint8 class index.

    `output_dirs` maps palette → pyramid root; `luts[palette]` is the model's
    (n_classes, 4) RGBA table, class 0 transparent. Each tile is sampled and
    encoded once; the other palettes get the same IDAT with a rewritten
    PLTE/tRNS. Returns the tile count (identical for every palette).
    """
    if not output_dirs:
        return 0
    idx = np.ascontiguousarray(idx, dtype=np.uint8)
    if idx.ndim != 2:
        raise ValueError("class index must be 2D")
    h, w = idx.shape
    palettes, plte, trns = _palette_chunks(output_dirs, luts)
    made_dirs: set[Path] = set()
    count = 0

    for z, tx, ty, geom in _iter_tile_geometry(
        lats,
        lons,
        h,
        w,
        zoom_levels,
        tile_size,
        source_crs,
        source_x,
        source_y,
        sampling="nearest",
    ):
        (r0, r1), (c0, c1) = geom.row_bounds, geom.col_bounds
        if _region_transparent(idx[r0 : r1 + 1, c0 : c1 + 1]):
            continue
        tile = geom.sample_nearest(idx)
        if not tile.any():
            continue
        _write_index_tile(
            tile,
            z,
            tx,
            ty,
            output_dirs,
            palettes,
            plte,
            trns,
            made_dirs,
        )
        count += 1
    return count


def render_continuous_tiles(
    data: np.ndarray,
    lats: np.ndarray,
    lons: np.ndarray,
    output_dirs: dict[str, str],
    model: ClassModel,
    zoom_levels: list[int],
    tile_size: int = 256,
    source_crs: object | None = None,
    source_x: np.ndarray | None = None,
    source_y: np.ndarray | None = None,
    nodata_value: float | None = None,
    min_valid_weight: float = 1.0,
) -> int:
    """Bilinearly sample physical values once, then classify/encode every palette.

    Palette indices are categorical and are never interpolated. Missing source
    contributors are governed explicitly by `nodata_value` and
    `min_valid_weight`; NaN and infinity are always missing.
    """
    if not output_dirs:
        return 0
    values = np.ascontiguousarray(data)
    if values.ndim != 2:
        raise ValueError("continuous source grid must be 2D")
    if model.edges is None:
        raise ValueError("continuous rendering requires a range class model")
    h, w = values.shape
    palettes, plte, trns = _palette_chunks(output_dirs, model.luts)
    made_dirs: set[Path] = set()
    count = 0

    for z, tx, ty, geom in _iter_tile_geometry(
        lats,
        lons,
        h,
        w,
        zoom_levels,
        tile_size,
        source_crs,
        source_x,
        source_y,
        sampling="bilinear",
    ):
        sampled = geom.sample_bilinear(
            values,
            nodata_value=nodata_value,
            min_valid_weight=min_valid_weight,
        )
        tile = model.classify(sampled)
        if not tile.any():
            continue
        _write_index_tile(
            tile,
            z,
            tx,
            ty,
            output_dirs,
            palettes,
            plte,
            trns,
            made_dirs,
        )
        count += 1
    return count


class PublishStatus(str, Enum):
    CREATED = "created"
    EXISTING_VALID = "existing_valid"
    EMPTY = "empty"


@dataclass(frozen=True)
class PalettePublishOutcome:
    status: PublishStatus
    tile_count: int


@dataclass(frozen=True)
class MultiPaletteRenderResult:
    outcomes: dict[str, PalettePublishOutcome]

    @property
    def counts(self) -> dict[str, int]:
        return {name: outcome.tile_count for name, outcome in self.outcomes.items()}

    @property
    def rendered_palettes(self) -> list[str]:
        return [
            name for name, outcome in self.outcomes.items() if outcome.tile_count > 0
        ]


_PYRAMID_COMPLETE_FILE = ".render-complete.json"


def _tile_paths(path: Path) -> set[str]:
    return {str(tile.relative_to(path)) for tile in path.rglob("*.png")}


def _tile_path_digest(paths: set[str]) -> str:
    encoded = "\0".join(sorted(paths)).encode()
    return hashlib.sha256(encoded).hexdigest()


def _write_completion_marker(path: Path, paths: set[str]) -> None:
    marker = path / _PYRAMID_COMPLETE_FILE
    marker.write_text(
        json.dumps(
            {
                "schema_version": 1,
                "tile_count": len(paths),
                "tile_path_digest": _tile_path_digest(paths),
            },
            sort_keys=True,
        )
        + "\n"
    )


def _inspect_complete_pyramid(path: Path) -> tuple[int, set[str]]:
    """Validate a pyramid published by the render-once protocol."""
    if not path.is_dir():
        raise RuntimeError(f"published pyramid is not a directory: {path}")
    paths = _tile_paths(path)
    if not paths:
        raise RuntimeError(f"published pyramid has no PNG tiles: {path}")
    marker = path / _PYRAMID_COMPLETE_FILE
    if not marker.is_file():
        raise RuntimeError(f"published pyramid has no completion marker: {path}")
    try:
        document = json.loads(marker.read_text())
        if document["schema_version"] != 1:
            raise ValueError("unsupported schema")
        marked = int(document["tile_count"])
        marked_digest = str(document["tile_path_digest"])
    except (OSError, KeyError, TypeError, ValueError, json.JSONDecodeError) as exc:
        raise RuntimeError(f"invalid pyramid completion marker: {marker}") from exc
    if marked != len(paths):
        raise RuntimeError(
            f"pyramid completion count mismatch at {path}: marker={marked}, actual={len(paths)}"
        )
    if marked_digest != _tile_path_digest(paths):
        raise RuntimeError(f"pyramid completion path mismatch at {path}")
    return len(paths), paths


def is_complete_pyramid(path: str | Path) -> bool:
    """Return whether `path` is a marker-validated immutable pyramid."""
    try:
        _inspect_complete_pyramid(Path(path))
    except (OSError, RuntimeError):
        return False
    return True


def _render_and_publish_atomic(
    output_dirs: dict[str, str],
    render: Callable[[dict[str, str]], int],
) -> MultiPaletteRenderResult:
    """Stage missing palettes, then publish each immutable pyramid safely.

    POSIX cannot atomically rename directories under several palette parents;
    the manifest remains the group visibility gate. Each individual pyramid is
    atomic, EEXIST is accepted only after complete-content validation, and a
    partial rename failure leaves only complete but unadvertised directories
    that the next retry can converge on.
    """
    if not output_dirs:
        return MultiPaletteRenderResult(outcomes={})
    finals = {name: Path(path) for name, path in output_dirs.items()}
    outcomes: dict[str, PalettePublishOutcome] = {}
    reference_paths: set[str] | None = None

    for name, final in finals.items():
        if not final.exists():
            continue
        count, paths = _inspect_complete_pyramid(final)
        if reference_paths is not None and paths != reference_paths:
            raise RuntimeError("existing palette pyramids have different tile paths")
        reference_paths = paths
        outcomes[name] = PalettePublishOutcome(PublishStatus.EXISTING_VALID, count)

    missing = {name: final for name, final in finals.items() if name not in outcomes}
    if not missing:
        return MultiPaletteRenderResult(outcomes=outcomes)
    staging = {
        name: final.parent / f".{final.name}.tmp-{uuid.uuid4().hex}"
        for name, final in missing.items()
    }

    def _discard_staging() -> None:
        for tmp in staging.values():
            shutil.rmtree(tmp, ignore_errors=True)

    try:
        count = render({name: str(path) for name, path in staging.items()})
        if count == 0:
            if outcomes:
                raise RuntimeError("existing pyramid conflicts with newly empty render")
            _discard_staging()
            return MultiPaletteRenderResult(
                outcomes={
                    name: PalettePublishOutcome(PublishStatus.EMPTY, 0)
                    for name in output_dirs
                },
            )

        staged_paths: set[str] | None = None
        for tmp in staging.values():
            paths = _tile_paths(tmp)
            if len(paths) != count:
                raise RuntimeError(
                    f"staged pyramid tile count mismatch at {tmp}: expected={count}, actual={len(paths)}"
                )
            if staged_paths is not None and paths != staged_paths:
                raise RuntimeError("staged palette pyramids have different tile paths")
            staged_paths = paths
            _write_completion_marker(tmp, paths)
        assert staged_paths is not None
        if reference_paths is not None and staged_paths != reference_paths:
            raise RuntimeError(
                "existing and newly rendered palette pyramids have different tile paths"
            )
        reference_paths = staged_paths

        for name, final in missing.items():
            tmp = staging[name]
            try:
                os.rename(tmp, final)
            except OSError as exc:
                if exc.errno not in (errno.EEXIST, errno.ENOTEMPTY):
                    raise
                # A concurrent winner using this publication protocol writes
                # the marker before its atomic rename. Do not mistake an
                # unrelated or half-built pre-existing directory for success.
                existing_count, existing_paths = _inspect_complete_pyramid(final)
                if existing_paths != reference_paths:
                    raise RuntimeError(
                        f"concurrent pyramid has different tile paths: {final}"
                    ) from exc
                shutil.rmtree(tmp, ignore_errors=True)
                outcomes[name] = PalettePublishOutcome(
                    PublishStatus.EXISTING_VALID,
                    existing_count,
                )
            else:
                outcomes[name] = PalettePublishOutcome(PublishStatus.CREATED, count)
    except BaseException:
        _discard_staging()
        raise

    return MultiPaletteRenderResult(
        outcomes={name: outcomes[name] for name in output_dirs}
    )


def render_indexed_tiles_atomic(
    idx: np.ndarray,
    lats: np.ndarray,
    lons: np.ndarray,
    output_dirs: dict[str, str],
    luts: dict[str, np.ndarray],
    zoom_levels: list[int],
    **kwargs,
) -> MultiPaletteRenderResult:
    """render_indexed_tiles with render_tiles_atomic publish semantics per palette.

    Every palette renders into its own unique `.tmp-` staging sibling and is
    renamed into place; EEXIST/ENOTEMPTY means another writer already
    published that immutable path and the staging tree is discarded. A crash
    or a fully transparent frame publishes nothing for any palette.
    """
    return _render_and_publish_atomic(
        output_dirs,
        lambda staging: render_indexed_tiles(
            idx,
            lats,
            lons,
            staging,
            luts,
            zoom_levels,
            **kwargs,
        ),
    )


def render_continuous_tiles_atomic(
    data: np.ndarray,
    lats: np.ndarray,
    lons: np.ndarray,
    output_dirs: dict[str, str],
    model: ClassModel,
    zoom_levels: list[int],
    **kwargs,
) -> MultiPaletteRenderResult:
    return _render_and_publish_atomic(
        output_dirs,
        lambda staging: render_continuous_tiles(
            data,
            lats,
            lons,
            staging,
            model,
            zoom_levels,
            **kwargs,
        ),
    )


def render_frame_palettes(
    data: np.ndarray,
    lats: np.ndarray,
    lons: np.ndarray,
    color_tables: dict[str, dict],
    output_dirs: dict[str, str],
    zoom_levels: list[int],
    *,
    category_map: dict[int, str] | None = None,
    nodata_value: float | None = None,
    min_valid_weight: float = 1.0,
    **kwargs,
) -> MultiPaletteRenderResult:
    """Sample/classify once per tile and publish every palette's pyramid.

    `color_tables[palette]` is a `ranges` table (or a `categories` table when
    `category_map` is given); only palettes present in `output_dirs` render.
    Range products use physical-value bilinear resampling with explicit nodata
    policy. Category products use nearest sampling and never interpolate IDs.
    """
    if not output_dirs:
        return MultiPaletteRenderResult(outcomes={})
    tables = {p: color_tables[p] for p in output_dirs}
    if category_map is not None:
        model = build_categorical_class_model(tables, category_map)
        idx = model.classify(data)
        return render_indexed_tiles_atomic(
            idx,
            lats,
            lons,
            output_dirs,
            model.luts,
            zoom_levels,
            **kwargs,
        )
    model = build_class_model(tables)
    return render_continuous_tiles_atomic(
        data,
        lats,
        lons,
        output_dirs,
        model,
        zoom_levels,
        nodata_value=nodata_value,
        min_valid_weight=min_valid_weight,
        **kwargs,
    )
