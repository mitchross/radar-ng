"""Temporal activity for the pysteps nowcast.

CPU-heavy. Spec retry override: 2 attempts max (deterministic for the same
input; retrying won't change the result if the first try failed for code
reasons). Heartbeats every 15s.
"""

from __future__ import annotations

import asyncio
import json
import os
import shutil
import tempfile
import time
from dataclasses import dataclass, field
from datetime import datetime, timedelta, timezone
from pathlib import Path

import numpy as np
from temporalio import activity

from backend.shared.activity_heartbeat import run_sync_with_heartbeat
from backend.shared.grid_dump import write_grid
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
# S-PROG's default AR order requires at least three precipitation frames.
# Clamp misconfigured deployments instead of invoking pySTEPS with a stack it
# cannot fit (common immediately after an empty-volume/cold start).
N_INPUT_FRAMES = max(3, int(os.environ.get("NOWCAST_INPUT_FRAMES", "4")))
GRID_INPUT_LAYER = os.environ.get("NOWCAST_GRID_INPUT_LAYER", "radar-nowcast-input")
ALLOW_PERSISTENCE_FALLBACK = os.environ.get("NOWCAST_ALLOW_PERSISTENCE_FALLBACK", "0") == "1"
MAX_INPUT_GAP_MIN = float(os.environ.get("NOWCAST_MAX_INPUT_GAP_MIN", "6"))
# The science grid is ~2 km after its bounded downsample. z6 is its honest
# display ceiling; z7 added 4x work while only magnifying interpolated pixels.
ZOOM_LEVELS = [4, 5, 6]
# Compact grids let the API sample the 12 public lead times at an arbitrary
# user location without decoding colorized tiles. Keep these much smaller
# than the seven-million-cell science inputs used by pySTEPS itself.
POINT_GRID_MAX_CELLS = int(os.environ.get("NOWCAST_POINT_GRID_MAX_CELLS", "900000"))

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
        data_file = meta.get("data_file")
        bin_path = (
            meta_path.parent / str(data_file)
            if data_file
            else meta_path.parent / meta_path.name.replace(".meta.json", ".bin")
        )
        arr = np.fromfile(str(bin_path), dtype="<f4").reshape(h, w)
        arr = np.where(np.abs(arr - fill) < 1e-3, -9999.0, arr)
        lats = np.linspace(meta["lat_max"], meta["lat_min"], h, dtype=np.float64)
        lons = np.linspace(meta["lon_min"], meta["lon_max"], w, dtype=np.float64)
        return arr, lats, lons, meta
    except (OSError, KeyError, ValueError) as exc:
        log.warning("grid_load_failed", extra={"path": str(meta_path), "err": str(exc)})
        return None


def _list_recent_grids() -> list[Path]:
    radar_grid = GRID_DIR / GRID_INPUT_LAYER
    if not radar_grid.exists() and GRID_INPUT_LAYER != "radar":
        # Backward-compatible bootstrap while an older MRMS worker has not yet
        # emitted the dedicated science grid.
        radar_grid = GRID_DIR / "radar"
    if not radar_grid.exists():
        return []
    metas = sorted(radar_grid.glob("*.meta.json"), key=lambda p: p.name)
    return metas[-N_INPUT_FRAMES:]


def _persistence_fallback(frames: list[np.ndarray], n_leadtimes: int) -> np.ndarray:
    last = frames[-1]
    return np.repeat(last[np.newaxis, :, :], n_leadtimes, axis=0).astype(np.float32)


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


def _degraded_result(
    frames: list[np.ndarray], lead_steps: list[float], *, reason: str, detail: str
) -> tuple[np.ndarray | None, str]:
    _write_nowcast_status("degraded", reason=reason, detail=detail)
    if ALLOW_PERSISTENCE_FALLBACK:
        return _persistence_fallback(frames, len(lead_steps)), "persistence"
    return None, "unavailable"


def _run_nowcast(
    frames: list[np.ndarray], lead_steps: list[float]
) -> tuple[np.ndarray | None, str]:
    try:
        from pysteps import motion, nowcasts  # type: ignore
    except (ImportError, AttributeError, ModuleNotFoundError) as exc:
        # Catches both "module not found" AND "distutils missing" / "_ARRAY_API"
        # numpy-vs-cv2 incompatibilities on Python 3.12.
        log.warning("pysteps_unavailable", extra={"err": str(exc)})
        return _degraded_result(
            frames, lead_steps, reason="pysteps_unavailable", detail=str(exc)
        )

    stack = np.stack(frames, axis=0).astype(np.float32)
    stack = np.where(stack < -100, np.nan, stack)
    try:
        oflow = motion.get_method("LK")
        uv = oflow(stack)
        nowcaster = nowcasts.get_method("sprog")
        try:
            forecast = nowcaster(
                stack[-3:, :, :], uv, lead_steps,
                n_cascade_levels=6, precip_thr=5.0,
            )
        except TypeError:
            forecast = nowcaster(
                stack[-3:, :, :], uv, lead_steps,
                n_cascade_levels=6, R_thr=5.0,
            )
    except Exception as exc:  # noqa: BLE001
        log.warning("pysteps_failed", extra={"err": str(exc)})
        return _degraded_result(frames, lead_steps, reason="pysteps_failed", detail=str(exc))
    forecast = np.asarray(forecast, dtype=np.float32)
    forecast = np.where(np.isnan(forecast), -9999.0, forecast)
    _write_nowcast_status("ok")
    return forecast, "pysteps-sprog"


def _nowcast_tile_path(anchor_ts: str, valid_ts: str) -> str:
    return f"runs/{anchor_ts}/{valid_ts}"


def _render_frame(tile_base: Path, palette_tables: dict[str, dict], tile_path: str, data: np.ndarray, lats: np.ndarray, lons: np.ndarray) -> list[str]:
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
        out_dir = str(tile_base / "nowcast" / pname / tile_path)
        count = render_tiles_atomic(
            rgba=rgba, lats=lats, lons=lons,
            output_dir=out_dir, zoom_levels=ZOOM_LEVELS,
        )
        if count > 0:
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
        if len(metas) < 3:
            log.info("waiting_for_grids", extra={"count": len(metas)})
            _write_nowcast_status(
                "warming_up",
                reason="insufficient_inputs",
                detail=f"count={len(metas)},required=3",
            )
            return (False, None, [], {}, 0.0)
        latest_iso = metas[-1].name.replace(".meta.json", "")
        records: list[tuple[np.ndarray, datetime, dict, str]] = []
        for p in metas:
            loaded = _load_grid(p)
            if loaded is None:
                continue
            arr, _, _, meta = loaded
            timestamp = p.name.replace(".meta.json", "")
            try:
                observed_at = datetime.fromisoformat(timestamp)
            except ValueError:
                continue
            records.append((arr, observed_at, meta, timestamp))
        if len(records) < 3:
            _write_nowcast_status(
                "warming_up",
                reason="insufficient_valid_inputs",
                detail=f"count={len(records)},required=3",
            )
            return (False, latest_iso, [], {}, 0.0)
        target_shape = records[-1][0].shape
        records = [record for record in records if record[0].shape == target_shape]
        if len(records) < 3:
            _write_nowcast_status(
                "warming_up",
                reason="inconsistent_input_shapes",
                detail=f"count={len(records)},required=3",
            )
            return (False, latest_iso, [], {}, 0.0)
        grids = [record[0] for record in records]
        grid_times = [record[1] for record in records]
        meta_used = records[-1][2]
        latest_iso = records[-1][3]
        state = ProcessedSet(STATE_DIR / "nowcast.json", max_entries=100)
        if latest_iso in state:
            return (False, latest_iso, [], {}, 0.0)
        intervals = [
            (current - previous).total_seconds() / 60.0
            for previous, current in zip(grid_times, grid_times[1:])
        ]
        if not intervals or min(intervals) <= 0 or max(intervals) > MAX_INPUT_GAP_MIN:
            _write_nowcast_status(
                "degraded",
                reason="invalid_input_cadence",
                detail=f"intervals_minutes={intervals}",
            )
            return (False, latest_iso, [], {}, 0.0)
        input_interval_min = float(np.median(np.asarray(intervals)))
        return (True, latest_iso, grids, meta_used, input_interval_min)

    ok, latest_iso, grids, meta_used, input_interval_min = await asyncio.to_thread(_setup)
    if not ok:
        return NowcastResult(ran=False, anchor_ts=latest_iso)

    n_lead = HORIZON_MIN // STEP_MIN
    # pySTEPS lead times are measured in input timesteps, not minutes. MRMS is
    # commonly ~2 minutes; passing the integer count mislabeled a 2-minute
    # forecast step as 5 minutes. Fractional requested timesteps preserve the
    # public 5-minute timeline against the measured input cadence.
    lead_steps = [((index + 1) * STEP_MIN) / input_interval_min for index in range(n_lead)]
    activity.heartbeat({"phase": "pysteps", "input_frames": len(grids), "leadtimes": n_lead})
    forecast = await run_sync_with_heartbeat(
        _run_nowcast,
        grids,
        lead_steps,
        heartbeat_every=30,
        heartbeat_details=lambda: {"phase": "pysteps", "input_frames": len(grids), "leadtimes": n_lead},
    )
    forecast, method = forecast
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
    point_grid_files: list[str] = []
    manifest_frames: list[dict] = []
    expected_palettes = {
        name for name, tables in palette_tables.items() if tables.get("reflectivity")
    }
    resolution_km = round(
        abs(float(meta_used["lat_max"]) - float(meta_used["lat_min"]))
        / max(1, int(meta_used["height"]) - 1)
        * 111.0,
        2,
    )

    for i in range(n_lead):
        valid = latest_dt + timedelta(minutes=(i + 1) * STEP_MIN)
        ts = valid.isoformat()
        tile_path = _nowcast_tile_path(latest_iso, ts)
        frame = forecast[i]
        frame = np.where(frame < 5, -9999.0, frame)
        palettes = await run_sync_with_heartbeat(
            _render_frame,
            TILE_DIR,
            palette_tables,
            tile_path,
            frame,
            lats_arr,
            lons_arr,
            heartbeat_every=30,
            heartbeat_details=lambda i=i: {"phase": "render", "leadtime": i},
        )
        if set(palettes) == expected_palettes:
            grid_file = await asyncio.to_thread(
                write_grid,
                "nowcast",
                ts,
                frame,
                lats_arr,
                lons_arr,
                "dBZ",
                -9999.0,
                POINT_GRID_MAX_CELLS,
            )
            if not grid_file:
                for palette in palettes:
                    shutil.rmtree(
                        TILE_DIR / "nowcast" / palette / tile_path,
                        ignore_errors=True,
                    )
                continue
            rendered_palettes.update(palettes)
            rendered_timestamps.append(ts)
            point_grid_files.append(grid_file)
            manifest_frames.append({
                "timestamp": ts,
                "path": tile_path,
                "source": "mrms-nowcast",
                "kind": "nowcast",
                "issued_at": latest_dt.isoformat(),
                "lead_minutes": (i + 1) * STEP_MIN,
                "input_interval_minutes": round(input_interval_min, 3),
                "method": method,
                "spatial_resolution_km": resolution_km,
                "max_zoom": max(ZOOM_LEVELS),
            })
        else:
            for palette in palettes:
                shutil.rmtree(
                    TILE_DIR / "nowcast" / palette / tile_path, ignore_errors=True
                )
        if i % 4 == 0:
            activity.heartbeat({"phase": "render", "leadtime": i})

    def _commit() -> None:
        if len(rendered_timestamps) != n_lead:
            raise RuntimeError(
                f"nowcast incomplete: rendered {len(rendered_timestamps)}/{n_lead} leadtimes"
            )
        # One atomic swap: this run's frames replace ALL previous nowcast
        # frames in the manifest. Old tile dirs stay on disk until the
        # cleanup sweep removes them, but the app never sees them again.
        replace_layer_manifest(
            "nowcast",
            rendered_timestamps,
            palettes=rendered_palettes,
            frames=manifest_frames,
            layer_metadata={
                "title": "MRMS motion nowcast",
                "kind": "nowcast",
                "horizon_minutes": HORIZON_MIN,
                "step_minutes": STEP_MIN,
                "method": method,
                "run_id": latest_iso,
            },
        )
        # The manifest swap is the publication boundary. The endpoint samples
        # that complete run in one request, so older point grids can go after
        # the swap without exposing a partially written new series.
        point_dir = GRID_DIR / "nowcast"
        keep = {f"{timestamp}.meta.json" for timestamp in rendered_timestamps}
        keep.update(Path(path).name for path in point_grid_files)
        if point_dir.exists():
            for path in point_dir.iterdir():
                if path.is_file() and path.name not in keep:
                    path.unlink(missing_ok=True)
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
