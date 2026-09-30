"""Score retained nowcast runs against the radar that actually arrived.

Every run keeps its lead-time point grids and a copy of the observed grid it
started from under ``grids/nowcast/runs/<anchor>/``. When the next observed
science grid lands, each retained run that predicted this valid time is
compared cell by cell at the rain and heavy thresholds; the counts go to the
rolling log in ``backend/shared/nowcast_skill``. Only the worker needs numpy,
so the grid work lives here and the summary lives in the shared module.
"""

from __future__ import annotations

import json
import os
from datetime import datetime, timezone
from pathlib import Path
from typing import Any

import numpy as np

from backend.shared.nowcast_skill import THRESHOLDS_DBZ, append_entries

# Observed cells at or below this are outside radar coverage or missing (MRMS
# -999/-99, our -9999 fill) and are left out of every count. Forecast cells at
# the fill value simply mean "no rain forecast".
UNKNOWN_BELOW = -100.0
OBSERVED_KEY = "observed"
MATCH_TOLERANCE_S = float(os.environ.get("NOWCAST_SKILL_MATCH_TOLERANCE_S", "150"))


def downsample_like_point_grid(data: np.ndarray, max_cells: int) -> np.ndarray:
    """The stride rule ``grid_dump.write_grid`` applies, so shapes line up exactly."""
    h, w = data.shape
    stride = 1
    while (h // stride) * (w // stride) > max(1, int(max_cells)):
        stride *= 2
    return data[::stride, ::stride] if stride > 1 else data


def contingency(forecast: np.ndarray, observed: np.ndarray, threshold: float) -> dict[str, int]:
    """Hits, misses and false alarms over cells with a known observation."""
    known = np.isfinite(observed) & (observed > UNKNOWN_BELOW)
    obs_event = known & (observed >= threshold)
    fcst_event = known & np.isfinite(forecast) & (forecast >= threshold)
    return {
        "hits": int(np.count_nonzero(fcst_event & obs_event)),
        "misses": int(np.count_nonzero(obs_event & ~fcst_event)),
        "false_alarms": int(np.count_nonzero(fcst_event & ~obs_event)),
        "cells": int(np.count_nonzero(known)),
    }


def _parse_ts(value: str) -> datetime | None:
    try:
        dt = datetime.fromisoformat(value)
    except ValueError:
        return None
    return dt if dt.tzinfo else dt.replace(tzinfo=timezone.utc)


def load_point_grid(meta_path: Path) -> tuple[np.ndarray, dict] | None:
    try:
        meta = json.loads(meta_path.read_text())
        h, w = int(meta["height"]), int(meta["width"])
        data_file = meta.get("data_file")
        bin_path = (
            meta_path.parent / str(data_file)
            if data_file
            else meta_path.parent / meta_path.name.replace(".meta.json", ".bin")
        )
        arr = np.fromfile(str(bin_path), dtype="<f4").reshape(h, w)
    except (OSError, KeyError, TypeError, ValueError):
        return None
    return arr, meta


def resample_nearest(src: np.ndarray, src_meta: dict, dst_meta: dict) -> np.ndarray:
    """Nearest-neighbour ``src`` onto ``dst_meta``'s lat/lon grid (both north-first)."""
    dst_h, dst_w = int(dst_meta["height"]), int(dst_meta["width"])
    src_h, src_w = src.shape
    dst_lats = np.linspace(float(dst_meta["lat_max"]), float(dst_meta["lat_min"]), dst_h)
    dst_lons = np.linspace(float(dst_meta["lon_min"]), float(dst_meta["lon_max"]), dst_w)
    lat_span = float(src_meta["lat_max"]) - float(src_meta["lat_min"])
    lon_span = float(src_meta["lon_max"]) - float(src_meta["lon_min"])
    rows = np.rint((float(src_meta["lat_max"]) - dst_lats) / max(lat_span, 1e-9) * (src_h - 1)).astype(int)
    cols = np.rint((dst_lons - float(src_meta["lon_min"])) / max(lon_span, 1e-9) * (src_w - 1)).astype(int)
    rows = np.clip(rows, 0, src_h - 1)
    cols = np.clip(cols, 0, src_w - 1)
    return src[np.ix_(rows, cols)]


def _observed_meta(observed: np.ndarray, lats: np.ndarray, lons: np.ndarray) -> dict:
    return {
        "height": int(observed.shape[0]),
        "width": int(observed.shape[1]),
        "lat_min": float(np.min(lats)),
        "lat_max": float(np.max(lats)),
        "lon_min": float(np.min(lons)),
        "lon_max": float(np.max(lons)),
    }


def score_observation(
    observed: np.ndarray,
    observed_ts: str,
    lats: np.ndarray,
    lons: np.ndarray,
    *,
    grid_dir: str | Path,
    state_dir: str | Path,
    point_grid_max_cells: int,
    tolerance_s: float = MATCH_TOLERANCE_S,
    now: datetime | None = None,
) -> list[dict[str, Any]]:
    """Score every retained run that predicted ``observed_ts``; append to the log.

    ``observed`` is the full science grid the new nowcast run starts from; it
    is reduced with the point-grid stride so it lines up with the stored frames.
    Returns the entries written (empty when no run predicted this time).
    """
    obs_dt = _parse_ts(observed_ts)
    runs_root = Path(grid_dir) / "nowcast" / "runs"
    if obs_dt is None or not runs_root.is_dir():
        return []
    obs = downsample_like_point_grid(np.asarray(observed, dtype=np.float32), point_grid_max_cells)
    obs_meta = _observed_meta(obs, lats, lons)
    scored_at = (now or datetime.now(timezone.utc)).isoformat()
    entries: list[dict[str, Any]] = []

    for run_dir in sorted(p for p in runs_root.iterdir() if p.is_dir()):
        anchor_dt = _parse_ts(run_dir.name)
        if anchor_dt is None or anchor_dt >= obs_dt:
            continue
        for meta_path in sorted(run_dir.glob("*.meta.json")):
            stem = meta_path.name[: -len(".meta.json")]
            if stem == OBSERVED_KEY:
                continue
            valid_dt = _parse_ts(stem)
            if valid_dt is None or abs((valid_dt - obs_dt).total_seconds()) > tolerance_s:
                continue
            loaded = load_point_grid(meta_path)
            if loaded is None:
                break
            forecast, fmeta = loaded
            if forecast.shape != obs.shape:
                forecast = resample_nearest(forecast, fmeta, obs_meta)
            entry: dict[str, Any] = {
                "anchor": run_dir.name,
                "valid": observed_ts,
                "lead_minutes": int(round((valid_dt - anchor_dt).total_seconds() / 60)),
                "scored_at": scored_at,
                "thresholds": {},
                "persistence": None,
            }
            for label, threshold in THRESHOLDS_DBZ.items():
                counts = contingency(forecast, obs, threshold)
                entry["cells"] = counts.pop("cells")
                entry["thresholds"][label] = counts
            baseline = load_point_grid(run_dir / f"{OBSERVED_KEY}.meta.json")
            if baseline is not None:
                base, bmeta = baseline
                if base.shape != obs.shape:
                    base = resample_nearest(base, bmeta, obs_meta)
                entry["persistence"] = {
                    label: {k: v for k, v in contingency(base, obs, threshold).items() if k != "cells"}
                    for label, threshold in THRESHOLDS_DBZ.items()
                }
            entries.append(entry)
            break  # frames are five minutes apart; at most one matches

    if entries:
        append_entries(state_dir, entries, now=now)
    return entries
