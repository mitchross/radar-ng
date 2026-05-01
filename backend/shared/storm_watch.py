"""Storm-watch helpers — frame comparison + change detection.

The watch workflow polls the latest MRMS reflectivity grid (already on disk
via `grid_dump.write_grid`), samples a small window around the watched cell,
and decides whether the change since the last poll warrants a push.

Domain logic is intentionally simple in v1:

  - max dBZ in window goes up by ≥ INTENSIFY_DBZ_DELTA  → "intensifying"
  - max dBZ in window goes down by ≥ DISSIPATE_DBZ_DELTA → "dissipating"
  - max dBZ ≥ HAIL_DBZ_THRESHOLD on this poll                → "severe"

Refinements (multi-frame trend, vertically integrated liquid, hail-size
discriminator) are out of scope for v1 — see Phase 5 in the design spec.
"""

from __future__ import annotations

import json
import os
from dataclasses import dataclass
from pathlib import Path

import numpy as np


GRID_DIR = Path(os.environ.get("GRID_DIR", "/data/grids"))
WATCH_RADIUS_KM = float(os.environ.get("WATCH_RADIUS_KM", "20"))
INTENSIFY_DBZ_DELTA = float(os.environ.get("INTENSIFY_DBZ_DELTA", "10"))
DISSIPATE_DBZ_DELTA = float(os.environ.get("DISSIPATE_DBZ_DELTA", "10"))
HAIL_DBZ_THRESHOLD = float(os.environ.get("HAIL_DBZ_THRESHOLD", "60"))


@dataclass
class FrameSample:
    timestamp: str
    max_dbz: float
    mean_dbz: float
    above_50_count: int


@dataclass
class FrameDiff:
    prev: FrameSample | None
    curr: FrameSample
    max_dbz_delta: float


@dataclass
class ChangeKind:
    kind: str  # "intensifying" | "dissipating" | "severe" | "appeared" | "disappeared"
    summary: str


def _km_per_deg_lat() -> float:
    return 111.32


def _km_per_deg_lon(lat: float) -> float:
    return 111.32 * float(np.cos(np.deg2rad(lat)))


def latest_radar_meta() -> Path | None:
    radar_grid = GRID_DIR / "radar"
    if not radar_grid.exists():
        return None
    metas = sorted(radar_grid.glob("*.meta.json"))
    return metas[-1] if metas else None


def sample_window(meta_path: Path, lat: float, lon: float, radius_km: float) -> FrameSample | None:
    """Return statistics over a (radius_km × radius_km) square around (lat, lon)."""
    try:
        meta = json.loads(meta_path.read_text())
        h = int(meta["height"])
        w = int(meta["width"])
        bin_path = meta_path.parent / meta_path.name.replace(".meta.json", ".bin")
        arr = np.fromfile(str(bin_path), dtype="<f4").reshape(h, w)
        lat_max = float(meta["lat_max"])
        lat_min = float(meta["lat_min"])
        lon_min = float(meta["lon_min"])
        lon_max = float(meta["lon_max"])
    except (OSError, KeyError, ValueError):
        return None

    if not (lat_min <= lat <= lat_max and lon_min <= lon <= lon_max):
        return FrameSample(
            timestamp=meta_path.name.replace(".meta.json", ""),
            max_dbz=float("-inf"),
            mean_dbz=float("nan"),
            above_50_count=0,
        )

    # Pixel index for the center cell. Row 0 is lat_max → row index increases southward.
    row_center = int(round((lat_max - lat) / (lat_max - lat_min) * (h - 1)))
    col_center = int(round((lon - lon_min) / (lon_max - lon_min) * (w - 1)))

    half_lat_deg = radius_km / _km_per_deg_lat()
    half_lon_deg = radius_km / _km_per_deg_lon(lat)
    half_rows = max(1, int(round(half_lat_deg / (lat_max - lat_min) * (h - 1))))
    half_cols = max(1, int(round(half_lon_deg / (lon_max - lon_min) * (w - 1))))

    r0 = max(0, row_center - half_rows)
    r1 = min(h, row_center + half_rows + 1)
    c0 = max(0, col_center - half_cols)
    c1 = min(w, col_center + half_cols + 1)
    win = arr[r0:r1, c0:c1]
    win = win[win > -100]  # drop fill values
    if win.size == 0:
        return FrameSample(
            timestamp=meta_path.name.replace(".meta.json", ""),
            max_dbz=float("-inf"),
            mean_dbz=float("nan"),
            above_50_count=0,
        )
    return FrameSample(
        timestamp=meta_path.name.replace(".meta.json", ""),
        max_dbz=float(win.max()),
        mean_dbz=float(win.mean()),
        above_50_count=int((win >= 50).sum()),
    )


def detect_change(diff: FrameDiff) -> ChangeKind | None:
    curr = diff.curr
    if diff.prev is None:
        return None  # nothing to compare yet

    if curr.max_dbz >= HAIL_DBZ_THRESHOLD:
        return ChangeKind("severe", f"reflectivity {curr.max_dbz:.0f} dBZ — possible hail/severe")

    if diff.max_dbz_delta >= INTENSIFY_DBZ_DELTA:
        return ChangeKind(
            "intensifying",
            f"+{diff.max_dbz_delta:.0f} dBZ since last frame ({diff.prev.max_dbz:.0f}→{curr.max_dbz:.0f})",
        )

    if -diff.max_dbz_delta >= DISSIPATE_DBZ_DELTA:
        return ChangeKind(
            "dissipating",
            f"{diff.max_dbz_delta:.0f} dBZ since last frame ({diff.prev.max_dbz:.0f}→{curr.max_dbz:.0f})",
        )

    return None
