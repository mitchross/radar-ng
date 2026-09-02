"""Downsampled Float32 grid dump + meta sidecar.

Inspector tool ('eyedropper') calls `/api/inspect/{layer}/{timestamp}/{lat}/{lon}`
on tap. That endpoint reads one of these dumps to bilinear-sample a point,
avoiding having to ship the full tile image grid through the API.

Layout:
    /data/grids/{layer}/{timestamp}.{generation}.bin — immutable flat Float32 data
    /data/grids/{layer}/{timestamp}.meta.json        — atomic pointer + grid metadata

We downsample to cap dump size at ~3MB (≈900x900 grid). Caller provides the
*raw* grid — this helper handles downsampling + lat/lon axis normalization.
"""

from __future__ import annotations

import fcntl
import json
import os
import tempfile
import time
import uuid
from contextlib import contextmanager
from pathlib import Path
from typing import Iterator

import numpy as np

GRID_DIR = os.environ.get("GRID_DIR", "/data/grids")
GRID_MAX_AGE_S = int(os.environ.get("GRID_MAX_AGE_S", str(12 * 3600)))  # 12h
MAX_CELLS = int(os.environ.get("GRID_MAX_CELLS", str(900 * 900)))
GRID_ORPHAN_GRACE_S = max(0, int(os.environ.get("GRID_ORPHAN_GRACE_S", "300")))
# Layers that only ever need their newest few generations (keep-N beats 12 h of 24.5 MB dumps).
# Nowcast reads NOWCAST_INPUT_FRAMES; +2 covers a run that starts while MRMS publishes the next.
GRID_KEEP_LAST: dict[str, int] = {
    os.environ.get("NOWCAST_GRID_INPUT_LAYER", "radar-nowcast-input"): max(
        3, int(os.environ.get("NOWCAST_INPUT_FRAMES", "4"))
    )
    + 2,
}

_LAYER_LOCK_FILE = ".grid.lock"


@contextmanager
def _layer_lock(layer_dir: Path) -> Iterator[None]:
    """Serialize a layer's metadata commits and retention across processes."""
    layer_dir.mkdir(parents=True, exist_ok=True)
    with (layer_dir / _LAYER_LOCK_FILE).open("a+b") as lock:
        fcntl.flock(lock.fileno(), fcntl.LOCK_EX)
        try:
            yield
        finally:
            fcntl.flock(lock.fileno(), fcntl.LOCK_UN)


def _fsync_dir(path: Path) -> None:
    """Persist directory entries when the filesystem supports directory fsync."""
    try:
        fd = os.open(path, os.O_RDONLY | getattr(os, "O_DIRECTORY", 0))
    except OSError:
        return
    try:
        os.fsync(fd)
    except OSError:
        pass
    finally:
        os.close(fd)


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
    }
    generation = uuid.uuid4().hex
    bin_path = out_base.parent / f"{out_base.name}.{generation}.bin"
    meta["data_file"] = bin_path.name
    meta_path = out_base.with_suffix(".meta.json")

    # Retention uses the same per-layer lock. It can never unlink the fsynced
    # generation in the gap before metadata atomically points readers at it.
    with _layer_lock(out_base.parent):
        with bin_path.open("wb") as f:
            f.write(arr.tobytes(order="C"))
            f.flush()
            os.fsync(f.fileno())
        _fsync_dir(out_base.parent)

        fd, tmp_name = tempfile.mkstemp(
            prefix=f".{meta_path.name}.", suffix=".tmp", dir=str(meta_path.parent)
        )
        try:
            with os.fdopen(fd, "w") as f:
                json.dump(meta, f, separators=(",", ":"), sort_keys=True)
                f.write("\n")
                f.flush()
                os.fsync(f.fileno())
            # The metadata is the commit pointer. Readers either see the
            # complete prior generation or this complete generation.
            os.replace(tmp_name, meta_path)
            _fsync_dir(out_base.parent)
        finally:
            try:
                os.unlink(tmp_name)
            except FileNotFoundError:
                pass
    return str(bin_path)


def _grid_timestamp_of(name: str) -> str:
    """'{ts}.meta.json' / '{ts}.{generation}.bin' / legacy '{ts}.bin' → ts."""
    if name.endswith(".meta.json"):
        return name[: -len(".meta.json")]
    if name.endswith(".bin"):
        stem = name[: -len(".bin")]
        ts, dot, generation = stem.rpartition(".")
        if (
            dot
            and len(generation) == 32
            and all(c in "0123456789abcdef" for c in generation)
        ):
            return ts
        return stem
    return name


def _read_meta_data_file(meta_path: Path) -> str:
    try:
        document = json.loads(meta_path.read_text())
    except (OSError, ValueError, TypeError):
        document = None
    value = document.get("data_file") if isinstance(document, dict) else None
    if isinstance(value, str) and Path(value).name == value:
        return value
    return f"{_grid_timestamp_of(meta_path.name)}.bin"


def _prune_grid_layer_unlocked(layer_dir: Path, keep: int, now: float) -> int:
    try:
        entries = [p for p in layer_dir.iterdir() if p.is_file()]
    except OSError:
        return 0
    metas = sorted(
        (p for p in entries if p.name.endswith(".meta.json")),
        key=lambda p: p.name,
    )
    kept_metas = metas[-keep:] if keep > 0 else []
    keep_names = {p.name for p in kept_metas}
    keep_names.update(_read_meta_data_file(p) for p in kept_metas)
    meta_names = {p.name for p in metas}
    committed_data_names = {_read_meta_data_file(p) for p in metas}

    removed = 0
    for path in entries:
        name = path.name
        if name == _LAYER_LOCK_FILE or name in keep_names:
            continue
        if name.startswith(".") and name.endswith(".tmp"):
            continue  # the age sweep handles a stale temporary
        if not (name.endswith(".bin") or name in meta_names):
            continue
        # Unreferenced data may belong to an older worker that does not take
        # the layer lock. A short grace window makes rolling upgrades safe.
        if name.endswith(".bin") and name not in committed_data_names:
            try:
                if now - path.stat().st_mtime < GRID_ORPHAN_GRACE_S:
                    continue
            except OSError:
                continue
        try:
            path.unlink()
            removed += 1
        except OSError:
            pass
    return removed


def prune_grid_layer(layer: str, keep: int | None = None) -> int:
    """Keep only the newest `keep` timestamps of a layer (meta + data files). Returns files removed.

    Timestamps order lexically (ISO-8601), matching how nowcast picks its inputs.
    Unreferenced data files become removable after the rolling-upgrade grace.
    """
    keep = GRID_KEEP_LAST.get(layer) if keep is None else keep
    if keep is None:
        return 0
    layer_dir = Path(GRID_DIR) / layer
    if not layer_dir.is_dir():
        return 0
    try:
        with _layer_lock(layer_dir):
            return _prune_grid_layer_unlocked(layer_dir, max(0, keep), time.time())
    except OSError:
        return 0


def cleanup_old_grids() -> int:
    """Remove grid dumps older than GRID_MAX_AGE_S (and apply keep-N layers). Returns files removed."""
    root = Path(GRID_DIR)
    if not root.exists():
        return 0
    cutoff = time.time() - GRID_MAX_AGE_S
    removed = 0
    for layer_dir in root.iterdir():
        if not layer_dir.is_dir():
            continue
        try:
            with _layer_lock(layer_dir):
                keep = GRID_KEEP_LAST.get(layer_dir.name)
                if keep is not None:
                    removed += _prune_grid_layer_unlocked(
                        layer_dir,
                        max(0, keep),
                        time.time(),
                    )
                try:
                    entries = list(layer_dir.iterdir())
                except OSError:
                    continue
                for path in entries:
                    if path.name == _LAYER_LOCK_FILE:
                        continue
                    try:
                        if path.stat().st_mtime < cutoff:
                            path.unlink()
                            removed += 1
                    except OSError:
                        pass
        except OSError:
            # e.g. ext4 lost+found is root-owned and cannot host our lock;
            # skip it instead of crashing the whole cleanup activity.
            continue
    return removed
