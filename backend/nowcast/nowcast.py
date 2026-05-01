#!/usr/bin/env python3
"""Nowcast ingestor — pysteps S-PROG extrapolation of the last MRMS frames.

Reads downsampled Float32 reflectivity grids dumped by ingest-mrms at
/data/grids/radar/*.bin, runs pysteps dense optical flow + S-PROG AR(2)
extrapolation for +5..+60 min at 5-min cadence, then renders RGBA tiles into
/data/tiles/nowcast/{palette}/{timestamp}/... (one subtree per active palette).

State (nowcast.json) remembers the latest MRMS timestamp we've already
forecasted off of, so we only re-run when a new MRMS frame lands.
"""

from __future__ import annotations

import json
import os
import struct
import sys
import time
from datetime import datetime, timedelta, timezone
from pathlib import Path

import numpy as np

sys.path.insert(0, str(Path(__file__).resolve().parent / "shared"))
sys.path.insert(0, "/app/shared")
from logger import get_logger  # type: ignore  # noqa: E402
from state import ProcessedSet  # type: ignore  # noqa: E402
from tiler import apply_color_table, render_tiles  # type: ignore  # noqa: E402
from palettes import get_palette_names, load_palette  # type: ignore  # noqa: E402

GRID_DIR = Path(os.environ.get("GRID_DIR", "/data/grids"))
TILE_DIR = Path(os.environ.get("TILE_DIR", "/data/tiles"))
STATE_DIR = Path(os.environ.get("STATE_DIR", "/data/state"))
POLL_INTERVAL = int(os.environ.get("POLL_INTERVAL", "120"))
HORIZON_MIN = int(os.environ.get("NOWCAST_HORIZON_MIN", "60"))
STEP_MIN = int(os.environ.get("NOWCAST_STEP_MIN", "5"))
N_INPUT_FRAMES = int(os.environ.get("NOWCAST_INPUT_FRAMES", "4"))
ZOOM_LEVELS = [4, 5, 6, 7, 8]  # one less than MRMS — nowcasts are coarser by nature

log = get_logger("nowcast")


def load_grid(meta_path: Path) -> tuple[np.ndarray, np.ndarray, np.ndarray, dict] | None:
    try:
        meta = json.loads(meta_path.read_text())
        h = int(meta["height"])
        w = int(meta["width"])
        fill = float(meta.get("fill", -9999.0))
        # meta_path is {ts}.meta.json; bin lives at {ts}.bin (strip both suffixes).
        bin_path = meta_path.parent / meta_path.name.replace(".meta.json", ".bin")
        # Rebuilding from struct is slow; use numpy.fromfile for speed.
        arr = np.fromfile(str(bin_path), dtype="<f4").reshape(h, w)
        # Replace fill with -inf so pysteps masks correctly.
        arr = np.where(np.abs(arr - fill) < 1e-3, -9999.0, arr)
        lats = np.linspace(meta["lat_max"], meta["lat_min"], h, dtype=np.float64)
        lons = np.linspace(meta["lon_min"], meta["lon_max"], w, dtype=np.float64)
        return arr, lats, lons, meta
    except (OSError, KeyError, ValueError) as exc:
        log.warning("grid_load_failed", extra={"path": str(meta_path), "err": str(exc)})
        return None


def list_recent_grids() -> list[Path]:
    """Return the N_INPUT_FRAMES most recent radar grid metas, oldest-first."""
    radar_grid = GRID_DIR / "radar"
    if not radar_grid.exists():
        return []
    metas = sorted(radar_grid.glob("*.meta.json"), key=lambda p: p.name)
    return metas[-N_INPUT_FRAMES:]


def run_nowcast(frames: list[np.ndarray], n_leadtimes: int) -> np.ndarray | None:
    """Return a (n_leadtimes, H, W) Float32 array of forecasted reflectivity.

    Uses pysteps optical flow (DenseLucasKanade) + S-PROG AR(2) extrapolation.
    On import / convergence errors, falls back to pure advection-persistence.
    """
    try:
        import pysteps  # noqa: F401
        from pysteps import motion, nowcasts  # type: ignore
    except ImportError as exc:
        log.warning("pysteps_unavailable", extra={"err": str(exc)})
        return _persistence_fallback(frames, n_leadtimes)

    stack = np.stack(frames, axis=0).astype(np.float32)
    # pysteps expects NaN for missing values when `extrapolation.allow_nonfinite_values=True`.
    stack = np.where(stack < -100, np.nan, stack)

    try:
        oflow_method = motion.get_method("LK")
        uv = oflow_method(stack)
        nowcaster = nowcasts.get_method("sprog")
        # pysteps 1.14 renamed R_thr → precip_thr; older versions still accept R_thr.
        try:
            forecast = nowcaster(
                stack[-3:, :, :],
                uv,
                n_leadtimes,
                n_cascade_levels=6,
                precip_thr=5.0,
            )
        except TypeError:
            forecast = nowcaster(
                stack[-3:, :, :],
                uv,
                n_leadtimes,
                n_cascade_levels=6,
                R_thr=5.0,
            )
    except Exception as exc:  # noqa: BLE001
        log.warning("pysteps_failed", extra={"err": str(exc)})
        return _persistence_fallback(frames, n_leadtimes)

    forecast = np.where(np.isnan(forecast), -9999.0, forecast)
    return forecast


def _persistence_fallback(frames: list[np.ndarray], n_leadtimes: int) -> np.ndarray:
    """Dumb advection-persistence — repeat the last frame, slowly fading."""
    last = frames[-1]
    out = np.empty((n_leadtimes, *last.shape), dtype=np.float32)
    for i in range(n_leadtimes):
        fade = max(0.0, 1.0 - (i * 0.05))
        out[i] = np.where(last < 0, last, last * fade)
    return out


def render_nowcast_frame(
    tile_base: Path,
    palette_tables: dict[str, dict],
    timestamp: str,
    data: np.ndarray,
    lats: np.ndarray,
    lons: np.ndarray,
) -> None:
    for pname, tables in palette_tables.items():
        entry = tables.get("reflectivity")
        if not entry:
            continue
        rgba = apply_color_table(data, entry)
        out_dir = str(tile_base / "nowcast" / pname / timestamp)
        count = render_tiles(
            rgba=rgba, lats=lats, lons=lons, output_dir=out_dir, zoom_levels=ZOOM_LEVELS
        )
        log.info(
            "rendered",
            extra={"layer": "nowcast", "palette": pname, "timestamp": timestamp, "tiles": count},
        )


def process() -> bool:
    metas = list_recent_grids()
    if len(metas) < 2:
        log.info("waiting_for_grids", extra={"count": len(metas)})
        return False

    # Filename shape: {iso_ts}.meta.json → strip ".meta.json" to recover ts.
    latest_ts_iso = metas[-1].name.replace(".meta.json", "")

    state_path = STATE_DIR / "nowcast.json"
    state_path.parent.mkdir(parents=True, exist_ok=True)
    state = ProcessedSet(state_path, max_entries=100)
    if latest_ts_iso in state:
        return False

    grids: list[np.ndarray] = []
    meta_used: dict = {}
    for p in metas:
        loaded = load_grid(p)
        if loaded is None:
            continue
        arr, lats, lons, meta = loaded
        grids.append(arr)
        meta_used = meta
    if len(grids) < 2:
        log.warning("too_few_grids_loaded", extra={"count": len(grids)})
        return False

    # Shape check: pysteps needs uniform shape. Drop any frames that differ.
    target_shape = grids[-1].shape
    grids = [g for g in grids if g.shape == target_shape]
    if len(grids) < 2:
        return False

    n_leadtimes = HORIZON_MIN // STEP_MIN
    log.info("running_nowcast", extra={"input_frames": len(grids), "leadtimes": n_leadtimes})

    forecast = run_nowcast(grids, n_leadtimes)
    if forecast is None:
        return False

    try:
        latest_dt = datetime.fromisoformat(latest_ts_iso)
    except ValueError:
        log.warning("bad_timestamp", extra={"ts": latest_ts_iso})
        return False

    # Rebuild lat/lon axes from meta
    h = meta_used["height"]
    w = meta_used["width"]
    lats_arr = np.linspace(meta_used["lat_max"], meta_used["lat_min"], h, dtype=np.float64)
    lons_arr = np.linspace(meta_used["lon_min"], meta_used["lon_max"], w, dtype=np.float64)

    palette_tables = {name: load_palette(name) for name in get_palette_names()}

    for i in range(n_leadtimes):
        valid = latest_dt + timedelta(minutes=(i + 1) * STEP_MIN)
        ts = valid.isoformat()
        frame = forecast[i]
        # Filter dBZ < 5 (transparent) — same rule as MRMS.
        frame = np.where(frame < 5, -9999.0, frame)
        render_nowcast_frame(TILE_DIR, palette_tables, ts, frame, lats_arr, lons_arr)

    state.add(latest_ts_iso)
    log.info("nowcast_complete", extra={"anchor": latest_ts_iso, "leadtimes": n_leadtimes})
    return True


def run() -> None:
    TILE_DIR.mkdir(parents=True, exist_ok=True)
    GRID_DIR.mkdir(parents=True, exist_ok=True)
    STATE_DIR.mkdir(parents=True, exist_ok=True)

    log.info(
        "startup",
        extra={
            "grid_dir": str(GRID_DIR),
            "tile_dir": str(TILE_DIR),
            "poll_interval_s": POLL_INTERVAL,
            "horizon_min": HORIZON_MIN,
            "step_min": STEP_MIN,
            "palettes": get_palette_names(),
        },
    )

    while True:
        started = time.time()
        try:
            process()
        except Exception as exc:  # noqa: BLE001
            log.exception("loop_error", extra={"err": str(exc)})
        elapsed = time.time() - started
        time.sleep(max(15.0, POLL_INTERVAL - elapsed))


if __name__ == "__main__":
    run()
