"""Downsampled Float32 grid dump + meta sidecar.

Inspector tool ('eyedropper') calls `/api/inspect/{layer}/{timestamp}/{lat}/{lon}`
on tap. That endpoint reads one of these dumps to bilinear-sample a point,
avoiding having to ship the full tile image grid through the API.

Layout:
    /data/grids/{layer}/{timestamp}.bin        — flat Float32 (height * width * 4 bytes, row-major)
    /data/grids/{layer}/{timestamp}.meta.json  — { height, width, lat_min, lat_max, lon_min, lon_max, unit, fill }

We downsample to cap dump size at ~3MB (≈900x900 grid). Caller provides the
*raw* grid — this helper handles downsampling + lat/lon axis normalization.
"""

from __future__ import annotations

import json
import os
import tempfile
import time
import uuid
from pathlib import Path

import numpy as np

GRID_DIR = os.environ.get("GRID_DIR", "/data/grids")
GRID_MAX_AGE_S = int(os.environ.get("GRID_MAX_AGE_S", str(12 * 3600)))  # 12h
MAX_CELLS = int(os.environ.get("GRID_MAX_CELLS", str(900 * 900)))


def write_grid(
    layer: str,
    timestamp: str,
    data: np.ndarray,
    lats: np.ndarray,
    lons: np.ndarray,
    unit: str,
    fill: float = float("nan"),
    max_cells: int | None = None,
) -> str | None:
    """Dump `data` to GRID_DIR/{layer}/{timestamp}.bin + meta sidecar.

    - data: 2D array (height × width), any numeric dtype
    - lats: 1D array len=height (ascending *or* descending)
    - lons: 1D array len=width
    - unit: free-form label ("dBZ", "°F", "mph", "J/kg")
    - fill: sentinel for no-data; stored in meta

    Returns the path written, or None if the dump was skipped (bad shape).
    """
    if data.ndim != 2 or lats.ndim != 1 or lons.ndim != 1:
        return None
    h, w = data.shape
    if h != len(lats) or w != len(lons):
        return None

    # Normalize lats ascending (endpoint expects lat_max at row 0 in pixel-space).
    # write_grid stores the array as lats DESCENDING (north first) so pixel y=0 is north.
    if lats[0] < lats[-1]:
        data = np.flipud(data)
        lats = lats[::-1]

    # Normalize lons: MRMS/HRRR often use 0..360, but the app sends -180..180.
    # If any lon > 180, shift into -180..180 and roll the data columns so the
    # antimeridian (lon=180) lands at the array's east edge.
    if float(lons.max()) > 180.0:
        lons = np.where(lons > 180.0, lons - 360.0, lons)
        # Reorder so lons are monotonically ascending left→right.
        order = np.argsort(lons)
        lons = lons[order]
        data = data[:, order]

    # Downsample (stride-based) until total cells fits the caller's purpose.
    # Inspector grids use the compact default; nowcast science inputs opt into
    # a larger cap without forcing every point-inspection request to read them.
    cell_limit = max(1, int(max_cells or MAX_CELLS))
    stride = 1
    while (h // stride) * (w // stride) > cell_limit:
        stride *= 2
    if stride > 1:
        data = data[::stride, ::stride]
        lats = lats[::stride]
        lons = lons[::stride]
        h, w = data.shape

    out_base = Path(GRID_DIR) / layer / timestamp
    out_base.parent.mkdir(parents=True, exist_ok=True)

    arr = np.ascontiguousarray(data.astype(np.float32, copy=False))
    if np.isnan(fill):
        arr = np.where(np.isnan(arr), np.float32(-9999.0), arr)
        fill_val = -9999.0
    else:
        fill_val = float(fill)

    generation = uuid.uuid4().hex
    bin_path = out_base.parent / f"{out_base.name}.{generation}.bin"
    with bin_path.open("wb") as f:
        f.write(arr.tobytes(order="C"))
        f.flush()
        os.fsync(f.fileno())

    meta = {
        "height": int(h),
        "width": int(w),
        "lat_min": float(lats.min()),
        "lat_max": float(lats.max()),
        "lon_min": float(lons.min()),
        "lon_max": float(lons.max()),
        "unit": unit,
        "fill": fill_val,
        "stride": stride,
        "data_file": bin_path.name,
    }
    meta_path = out_base.with_suffix(".meta.json")
    fd, tmp_name = tempfile.mkstemp(
        prefix=f".{meta_path.name}.", suffix=".tmp", dir=str(meta_path.parent)
    )
    try:
        with os.fdopen(fd, "w") as f:
            json.dump(meta, f, separators=(",", ":"), sort_keys=True)
            f.write("\n")
            f.flush()
            os.fsync(f.fileno())
        # The metadata is the commit pointer. Readers either see the complete
        # prior generation or this complete generation, never half a pair.
        os.replace(tmp_name, meta_path)
    finally:
        try:
            os.unlink(tmp_name)
        except FileNotFoundError:
            pass
    return str(bin_path)


def cleanup_old_grids() -> int:
    """Remove grid dumps older than GRID_MAX_AGE_S. Returns files removed."""
    root = Path(GRID_DIR)
    if not root.exists():
        return 0
    cutoff = time.time() - GRID_MAX_AGE_S
    removed = 0
    for layer_dir in root.iterdir():
        if not layer_dir.is_dir():
            continue
        try:
            entries = list(layer_dir.iterdir())
        except OSError:
            # e.g. ext4 lost+found is root-owned and unreadable to the
            # worker's UID; skip anything we can't traverse rather than
            # crashing the whole cleanup activity.
            continue
        for f in entries:
            try:
                if f.stat().st_mtime < cutoff:
                    f.unlink()
                    removed += 1
            except OSError:
                pass
    return removed
