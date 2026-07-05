"""Temporal activity for the pysteps nowcast.

CPU-heavy. Spec retry override: 2 attempts max (deterministic for the same
input; retrying won't change the result if the first try failed for code
reasons). Heartbeats every 15s.
"""

from __future__ import annotations

import asyncio
import json
import os
import tempfile
import time
from dataclasses import dataclass, field
from datetime import datetime, timedelta, timezone
from pathlib import Path

import numpy as np
from temporalio import activity

from backend.shared.activity_heartbeat import run_sync_with_heartbeat
from backend.shared.logger import get_logger
from backend.shared.manifest import replace_layer_manifest
from backend.shared.palettes import get_palette_names, load_palette
from backend.shared.state import ProcessedSet
from backend.shared.tiler import apply_color_table, render_tiles_atomic


GRID_DIR = Path(os.environ.get("GRID_DIR", "/data/grids"))
TILE_DIR = Path(os.environ.get("TILE_DIR", "/data/tiles"))
STATE_DIR = Path(os.environ.get("STATE_DIR", "/data/state"))
HORIZON_MIN = int(os.environ.get("NOWCAST_HORIZON_MIN", "60"))
STEP_MIN = int(os.environ.get("NOWCAST_STEP_MIN", "5"))
N_INPUT_FRAMES = int(os.environ.get("NOWCAST_INPUT_FRAMES", "4"))
# Zoom 4-7. Dropped z8 (2026-06-18): the nowcast renders a full pyramid per
# leadtime x12 leadtimes; z8 is ~75% of all tiles (each level = 4x the prior)
# and on ~1km MRMS data z8 is pure upsampling — no real detail. Cutting it lets
# the render finish inside the activity timeout so the `nowcast` layer (the
# future-radar frames) actually publishes instead of getting cancelled mid-render.
ZOOM_LEVELS = [4, 5, 6, 7]

log = get_logger("nowcast-activities")


@dataclass
class NowcastResult:
    ran: bool
    anchor_ts: str | None = None
    leadtimes: int = 0
    palettes: list[str] = field(default_factory=list)
    duration_s: float = 0.0


def _load_grid(meta_path: Path) -> tuple[np.ndarray, np.ndarray, np.ndarray, dict] | None:
    try:
        meta = json.loads(meta_path.read_text())
        h = int(meta["height"])
        w = int(meta["width"])
        fill = float(meta.get("fill", -9999.0))
        bin_path = meta_path.parent / meta_path.name.replace(".meta.json", ".bin")
        arr = np.fromfile(str(bin_path), dtype="<f4").reshape(h, w)
        arr = np.where(np.abs(arr - fill) < 1e-3, -9999.0, arr)
        lats = np.linspace(meta["lat_max"], meta["lat_min"], h, dtype=np.float64)
        lons = np.linspace(meta["lon_min"], meta["lon_max"], w, dtype=np.float64)
        return arr, lats, lons, meta
    except (OSError, KeyError, ValueError) as exc:
        log.warning("grid_load_failed", extra={"path": str(meta_path), "err": str(exc)})
        return None


def _list_recent_grids() -> list[Path]:
    radar_grid = GRID_DIR / "radar"
    if not radar_grid.exists():
        return []
    metas = sorted(radar_grid.glob("*.meta.json"), key=lambda p: p.name)
    return metas[-N_INPUT_FRAMES:]


def _persistence_fallback(frames: list[np.ndarray], n_leadtimes: int) -> np.ndarray:
    last = frames[-1]
    out = np.empty((n_leadtimes, *last.shape), dtype=np.float32)
    for i in range(n_leadtimes):
        fade = max(0.0, 1.0 - (i * 0.05))
        out[i] = np.where(last < 0, last, last * fade)
    return out


def _input_interval_min(metas: list[Path]) -> float:
    """Actual cadence of the input grids, from their timestamp filenames.

    pysteps extrapolates in units of the INPUT time step — MRMS grids arrive
    every ~2 min, not every STEP_MIN. Labeling step i as +(i+1)*STEP_MIN while
    extrapolating i input-steps made every published leadtime ~2.5x too far
    out: storms played back at ~40% of their real speed and the "+60 min"
    frame was really a +24 min extrapolation.
    """
    try:
        stamps = [datetime.fromisoformat(p.name.replace(".meta.json", "")) for p in metas[-2:]]
        interval = (stamps[1] - stamps[0]).total_seconds() / 60.0
    except (ValueError, IndexError):
        return 2.0
    # Guard against duplicate/garbled stamps producing zero or absurd steps.
    return interval if 0.5 <= interval <= 15.0 else 2.0


def _write_nowcast_status(status: str, *, reason: str | None = None, detail: str | None = None) -> None:
    STATE_DIR.mkdir(parents=True, exist_ok=True)
    body = {
        "status": status,
        "reason": reason,
        "detail": detail,
        "updated_at": datetime.now(timezone.utc).isoformat(),
    }
    fd, tmp_name = tempfile.mkstemp(prefix=".nowcast-status.", suffix=".tmp", dir=str(STATE_DIR))
    try:
        with os.fdopen(fd, "w") as fh:
            json.dump(body, fh, separators=(",", ":"), sort_keys=True)
            fh.write("\n")
        os.replace(tmp_name, STATE_DIR / "nowcast-status.json")
    finally:
        try:
            os.unlink(tmp_name)
        except FileNotFoundError:
            pass


def _run_nowcast(frames: list[np.ndarray], timesteps: list[float]) -> np.ndarray | None:
    """`timesteps` are forecast times in units of the INPUT time step (pysteps
    convention), e.g. 2-min inputs and 5-min output spacing → [2.5, 5.0, …].
    Returns len(timesteps) frames."""
    try:
        from pysteps import motion, nowcasts  # type: ignore
    except (ImportError, AttributeError, ModuleNotFoundError) as exc:
        # Catches both "module not found" AND "distutils missing" / "_ARRAY_API"
        # numpy-vs-cv2 incompatibilities on Python 3.12.
        log.warning("pysteps_unavailable", extra={"err": str(exc)})
        _write_nowcast_status("degraded", reason="pysteps_unavailable", detail=str(exc))
        return _persistence_fallback(frames, len(timesteps))

    stack = np.stack(frames, axis=0).astype(np.float32)
    stack = np.where(stack < -100, np.nan, stack)
    try:
        oflow = motion.get_method("LK")
        uv = oflow(stack)
        nowcaster = nowcasts.get_method("sprog")
        try:
            forecast = nowcaster(stack[-3:, :, :], uv, timesteps, n_cascade_levels=6, precip_thr=5.0)
        except TypeError:
            forecast = nowcaster(stack[-3:, :, :], uv, timesteps, n_cascade_levels=6, R_thr=5.0)
    except Exception as exc:  # noqa: BLE001
        log.warning("pysteps_failed", extra={"err": str(exc)})
        _write_nowcast_status("degraded", reason="pysteps_failed", detail=str(exc))
        return _persistence_fallback(frames, len(timesteps))
    forecast = np.where(np.isnan(forecast), -9999.0, forecast)
    _write_nowcast_status("ok")
    return forecast


def _render_frame(tile_base: Path, palette_tables: dict[str, dict], ts: str, data: np.ndarray, lats: np.ndarray, lons: np.ndarray) -> list[str]:
    """Render one leadtime's tile pyramid. Manifest publishing happens once
    per RUN (replace_layer_manifest in nowcast_run), not per frame — so a
    half-finished run is never visible to the app, and frames from previous
    anchor runs don't pile up in the manifest.
    """
    rendered: list[str] = []
    for pname, tables in palette_tables.items():
        entry = tables.get("reflectivity")
        if not entry:
            continue
        rgba = apply_color_table(data, entry)
        out_dir = str(tile_base / "nowcast" / pname / ts)
        # Zero tiles = fully-transparent leadtime; render_tiles_atomic wrote
        # no dir, so advertising it in the manifest would 404 every fetch.
        if render_tiles_atomic(rgba=rgba, lats=lats, lons=lons, output_dir=out_dir, zoom_levels=ZOOM_LEVELS) > 0:
            rendered.append(pname)
    return rendered


@activity.defn(name="nowcast_run")
async def nowcast_run() -> NowcastResult:
    """Heartbeats live in the async wrapper because temporalio.activity.heartbeat
    requires a running asyncio loop, which is not the case from threads."""
    started = time.time()

    def _setup() -> tuple[bool, str | None, list[np.ndarray], dict, float]:
        TILE_DIR.mkdir(parents=True, exist_ok=True)
        STATE_DIR.mkdir(parents=True, exist_ok=True)
        metas = _list_recent_grids()
        if len(metas) < 2:
            log.info("waiting_for_grids", extra={"count": len(metas)})
            return (False, None, [], {}, 2.0)
        latest_iso = metas[-1].name.replace(".meta.json", "")
        interval_min = _input_interval_min(metas)
        state = ProcessedSet(STATE_DIR / "nowcast.json", max_entries=100)
        if latest_iso in state:
            return (False, latest_iso, [], {}, interval_min)
        grids: list[np.ndarray] = []
        meta_used: dict = {}
        for p in metas:
            loaded = _load_grid(p)
            if loaded is None:
                continue
            arr, _, _, meta = loaded
            grids.append(arr)
            meta_used = meta
        if len(grids) < 2:
            return (False, latest_iso, [], {}, interval_min)
        target_shape = grids[-1].shape
        grids = [g for g in grids if g.shape == target_shape]
        if len(grids) < 2:
            return (False, latest_iso, [], {}, interval_min)
        return (True, latest_iso, grids, meta_used, interval_min)

    ok, latest_iso, grids, meta_used, interval_min = await asyncio.to_thread(_setup)
    if not ok:
        return NowcastResult(ran=False, anchor_ts=latest_iso)

    # Published leadtimes stay on the STEP_MIN grid (12 frames for a 60-min
    # horizon); pysteps is asked for exactly those instants in units of the
    # real input cadence, so a +5-min label is a +5-min extrapolation.
    lead_minutes = [STEP_MIN * (i + 1) for i in range(HORIZON_MIN // STEP_MIN)]
    timesteps = [m / interval_min for m in lead_minutes]
    n_lead = len(lead_minutes)
    activity.heartbeat({"phase": "pysteps", "input_frames": len(grids), "leadtimes": n_lead})
    forecast = await run_sync_with_heartbeat(
        _run_nowcast,
        grids,
        timesteps,
        heartbeat_every=30,
        heartbeat_details=lambda: {"phase": "pysteps", "input_frames": len(grids), "leadtimes": n_lead},
    )
    if forecast is None:
        return NowcastResult(ran=False, anchor_ts=latest_iso)

    try:
        latest_dt = datetime.fromisoformat(latest_iso)
    except ValueError:
        return NowcastResult(ran=False, anchor_ts=latest_iso)

    h = meta_used["height"]
    w = meta_used["width"]
    lats_arr = np.linspace(meta_used["lat_max"], meta_used["lat_min"], h, dtype=np.float64)
    lons_arr = np.linspace(meta_used["lon_min"], meta_used["lon_max"], w, dtype=np.float64)
    def _load_palette_tables() -> dict[str, dict]:
        tables: dict[str, dict] = {}
        for name in get_palette_names():
            try:
                tables[name] = load_palette(name)
            except (FileNotFoundError, KeyError):
                log.warning("palette_missing", extra={"palette": name})
        if not tables:
            tables["classic"] = load_palette("classic")
        return tables

    palette_tables = await asyncio.to_thread(_load_palette_tables)
    rendered_palettes: set[str] = set()
    rendered_timestamps: list[str] = []

    for i in range(n_lead):
        valid = latest_dt + timedelta(minutes=lead_minutes[i])
        ts = valid.isoformat()
        frame = forecast[i]
        frame = np.where(frame < 5, -9999.0, frame)
        palettes = await run_sync_with_heartbeat(
            _render_frame,
            TILE_DIR,
            palette_tables,
            ts,
            frame,
            lats_arr,
            lons_arr,
            heartbeat_every=30,
            heartbeat_details=lambda i=i: {"phase": "render", "leadtime": i},
        )
        if palettes:
            rendered_palettes.update(palettes)
            rendered_timestamps.append(ts)
        if i % 4 == 0:
            activity.heartbeat({"phase": "render", "leadtime": i})

    def _commit() -> None:
        # One atomic swap: this run's frames replace ALL previous nowcast
        # frames in the manifest. Old tile dirs stay on disk until the
        # cleanup sweep removes them, but the app never sees them again.
        replace_layer_manifest("nowcast", rendered_timestamps, palettes=rendered_palettes)
        state = ProcessedSet(STATE_DIR / "nowcast.json", max_entries=100)
        state.add(latest_iso)

    await asyncio.to_thread(_commit)
    log.info("nowcast_complete", extra={"anchor": latest_iso, "leadtimes": n_lead})
    return NowcastResult(
        ran=True,
        anchor_ts=latest_iso,
        leadtimes=n_lead,
        palettes=sorted(rendered_palettes),
        duration_s=round(time.time() - started, 2),
    )
