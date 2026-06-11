"""Temporal activity for tile/grid cleanup.

Walks `/data/tiles/<layer>/[<palette>/]<timestamp>/...` and removes timestamp
subtrees older than the per-layer retention. Replaces the legacy cleanup.sh
shell script.
"""

from __future__ import annotations

import asyncio
import os
import shutil
import time
from dataclasses import dataclass
from datetime import datetime
from pathlib import Path

from temporalio import activity

from backend.shared.grid_dump import cleanup_old_grids
from backend.shared.logger import get_logger
from backend.shared.manifest import update_manifest_file


TILE_DIR = Path(os.environ.get("TILE_DIR", "/data/tiles"))
log = get_logger("tile-cleanup-activities")


# Per-layer retention (minutes). Matches the legacy cleanup.sh policy.
LAYER_RETENTION_MIN: dict[str, int] = {
    "radar": 240,           # MRMS keeps 4 hours
    "radar-composite": 240, # MRMS composite keeps 4 hours
    "nowcast": 60,          # pysteps forecasts keep 1 hour
    # HRRR-derived layers keep 12 hours
    "radar-hrrr": 720,
    "temperature": 720,
    "dewpoint": 720,
    "humidity": 720,
    "wind": 720,
    "cape": 720,
    "precip-type": 720,
    "precip-accum": 720,
    "cloud": 720,
}


@dataclass
class TileCleanupResult:
    layers_swept: int
    tile_dirs_removed: int
    grid_files_removed: int


def _sweep_layer(layer: str, retention_min: int) -> int:
    base = TILE_DIR / layer
    if not base.exists():
        return 0
    cutoff = time.time() - retention_min * 60
    candidates: list[Path] = []
    for entry in sorted(base.iterdir()):
        if not entry.is_dir():
            continue
        # Two layouts: legacy /{layer}/{ts}/ and multi-palette /{layer}/{palette}/{ts}/
        if entry.name[:1].isdigit():
            candidates.append(entry)
        else:
            candidates.extend(p for p in entry.iterdir() if p.is_dir())
    removed = 0
    for ts_dir in candidates:
        try:
            dt = datetime.fromisoformat(ts_dir.name)
        except ValueError:
            continue
        if dt.timestamp() < cutoff:
            # De-list from the manifest BEFORE deleting tiles — the reverse
            # order has a window where the app fetches a manifest that still
            # advertises a timestamp whose tiles are already gone (404s).
            update_manifest_file(layer, ts_dir.name, action="remove")
            shutil.rmtree(ts_dir, ignore_errors=True)
            removed += 1
    return removed


@activity.defn(name="tile_cleanup_sweep")
async def tile_cleanup_sweep() -> TileCleanupResult:
    def _go() -> TileCleanupResult:
        total = 0
        for layer, retention_min in LAYER_RETENTION_MIN.items():
            removed = _sweep_layer(layer, retention_min)
            if removed:
                log.info("layer_swept", extra={"layer": layer, "removed": removed, "retention_min": retention_min})
            total += removed
        grids_removed = cleanup_old_grids()
        return TileCleanupResult(
            layers_swept=len(LAYER_RETENTION_MIN),
            tile_dirs_removed=total,
            grid_files_removed=grids_removed,
        )

    return await asyncio.to_thread(_go)
