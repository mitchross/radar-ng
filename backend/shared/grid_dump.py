"""Downsampled Float32 grid dump + meta sidecar.

Inspector tool ('eyedropper') calls `/api/inspect/{layer}/{timestamp}/{lat}/{lon}`
on tap. That endpoint reads one of these dumps to bilinear-sample a point,
avoiding having to ship the full tile image grid through the API.

Layout:
    /data/grids/{layer}/{timestamp}.{generation}.bin — immutable flat Float32 data
    /data/grids/{layer}/{timestamp}.meta.json        — atomic pointer + grid metadata
    /data/grids/nowcast/runs/{run}/{timestamp}.*     — run-scoped nowcast grids

We downsample to cap dump size at ~3MB (≈900x900 grid). Caller provides the
*raw* grid — this helper handles downsampling + lat/lon axis normalization.
"""

from __future__ import annotations

import errno
import fcntl
import json
import os
import shutil
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
_GENERATION_ROOT = "runs"
_GENERATION_COMPLETE_FILE = ".grid-generation-complete-v1.json"
_DIRECTORY_FSYNC_UNSUPPORTED = {
    errno.EINVAL,
    getattr(errno, "ENOTSUP", errno.EINVAL),
}


def _durable_mkdir(path: Path) -> None:
    """Create a directory tree and persist every new ancestor entry."""
    path = path.absolute()
    ancestor = path
    while not ancestor.exists() and ancestor.parent != ancestor:
        ancestor = ancestor.parent
    path.mkdir(parents=True, exist_ok=True)
    current = path
    while True:
        _fsync_dir(current)
        if current == ancestor or current.parent == current:
            return
        current = current.parent


def _relative_grid_key(value: str) -> Path:
    parts = value.split("/")
    if not value or any(not part or part in {".", ".."} for part in parts):
        raise ValueError(f"invalid grid key {value!r}")
    path = Path(*parts)
    if path.is_absolute():
        raise ValueError(f"invalid grid key {value!r}")
    return path


@contextmanager
def _layer_lock(layer_dir: Path) -> Iterator[None]:
    """Serialize a layer's metadata commits and retention across processes."""
    _durable_mkdir(layer_dir)
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
    except OSError as exc:
        if exc.errno not in _DIRECTORY_FSYNC_UNSUPPORTED:
            raise
        return
    try:
        try:
            os.fsync(fd)
        except OSError as exc:
            if exc.errno not in _DIRECTORY_FSYNC_UNSUPPORTED:
                raise
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
    grid_key: str | None = None,
) -> str | None:
    """Dump `data` to a generation-backed binary + atomic metadata pointer.

    - data: 2D array (height × width), any numeric dtype
    - lats: 1D array len=height (ascending *or* descending)
    - lons: 1D array len=width
    - unit: free-form label ("dBZ", "°F", "mph", "J/kg")
    - fill: sentinel for no-data; stored in meta
    - grid_key: optional relative key below the layer. The legacy default is
      ``timestamp``; run-scoped writers use ``runs/{run}/{timestamp}``.

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

    # Stride-downsample to the caller's cell cap: inspector grids stay compact, nowcast opts into a larger cap.
    cell_limit = max(1, int(max_cells or MAX_CELLS))
    stride = 1
    while (h // stride) * (w // stride) > cell_limit:
        stride *= 2
    if stride > 1:
        data = data[::stride, ::stride]
        lats = lats[::stride]
        lons = lons[::stride]
        h, w = data.shape

    layer_dir = Path(GRID_DIR) / layer
    out_base = layer_dir / _relative_grid_key(grid_key or timestamp)

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
    meta_path = out_base.parent / f"{out_base.name}.meta.json"

    # Retention uses the same per-layer lock. It can never unlink the fsynced
    # generation in the gap before metadata atomically points readers at it.
    with _layer_lock(layer_dir):
        _durable_mkdir(out_base.parent)
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


def finalize_grid_generation(layer: str, generation: str) -> str:
    """Mark a complete run-scoped generation as eligible for whole-run pruning."""
    relative = _relative_grid_key(generation)
    if len(relative.parts) != 1:
        raise ValueError(f"invalid grid generation {generation!r}")
    layer_dir = Path(GRID_DIR) / layer
    generation_dir = layer_dir / _GENERATION_ROOT / relative
    with _layer_lock(layer_dir):
        if not generation_dir.is_dir():
            raise FileNotFoundError(generation_dir)
        meta_names = sorted(path.name for path in generation_dir.glob("*.meta.json"))
        if not meta_names:
            raise ValueError(f"grid generation has no metadata: {generation_dir}")
        marker = generation_dir / _GENERATION_COMPLETE_FILE
        fd, tmp_name = tempfile.mkstemp(
            prefix=f".{marker.name}.", suffix=".tmp", dir=str(generation_dir)
        )
        try:
            with os.fdopen(fd, "w") as handle:
                json.dump(
                    {
                        "schema_version": 1,
                        "generation": generation,
                        "grid_count": len(meta_names),
                        "grid_metadata": meta_names,
                    },
                    handle,
                    separators=(",", ":"),
                    sort_keys=True,
                )
                handle.write("\n")
                handle.flush()
                os.fsync(handle.fileno())
            os.replace(tmp_name, marker)
            _fsync_dir(generation_dir)
        finally:
            try:
                os.unlink(tmp_name)
            except FileNotFoundError:
                pass
    return str(generation_dir)


def _is_complete_grid_generation(path: Path) -> bool:
    marker = path / _GENERATION_COMPLETE_FILE
    try:
        document = json.loads(marker.read_text())
        meta_names = sorted(item.name for item in path.glob("*.meta.json"))
        marked_names = document["grid_metadata"]
        return (
            int(document["schema_version"]) == 1
            and document["generation"] == path.name
            and int(document["grid_count"]) == len(meta_names)
            and marked_names == meta_names
        )
    except (OSError, KeyError, TypeError, ValueError):
        return False


def prune_grid_generations(
    layer: str,
    *,
    keep: int,
    active_generation: str | None = None,
) -> int:
    """Remove complete run directories atomically by generation, never by frame.

    In-progress directories lack the completion marker and are left alone.
    ``active_generation`` is retained even when delayed runs make it older
    than the newest ``keep`` completed generations.
    """
    if active_generation is not None:
        active = _relative_grid_key(active_generation)
        if len(active.parts) != 1:
            raise ValueError(f"invalid active grid generation {active_generation!r}")
    layer_dir = Path(GRID_DIR) / layer
    generation_root = layer_dir / _GENERATION_ROOT
    if not generation_root.is_dir():
        return 0
    with _layer_lock(layer_dir):
        complete = sorted(
            (
                path
                for path in generation_root.iterdir()
                if path.is_dir()
                and not path.is_symlink()
                and _is_complete_grid_generation(path)
            ),
            key=lambda path: path.name,
        )
        retained = set(complete[-max(0, keep) :]) if keep > 0 else set()
        if active_generation is not None:
            retained.add(generation_root / active_generation)

        removed = 0
        for generation_dir in complete:
            if generation_dir in retained:
                continue
            removed += sum(
                1
                for path in generation_dir.rglob("*")
                if path.is_file() or path.is_symlink()
            )
            shutil.rmtree(generation_dir)
        if removed:
            _fsync_dir(generation_root)
        return removed


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
