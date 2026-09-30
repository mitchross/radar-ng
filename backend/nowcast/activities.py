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
from concurrent.futures import ThreadPoolExecutor
from dataclasses import dataclass, field
from datetime import datetime, timedelta, timezone
from pathlib import Path

import numpy as np
from temporalio import activity

from backend.nowcast import skill as nowcast_skill
from backend.shared.activity_heartbeat import run_sync_with_heartbeat
from backend.shared.grid_dump import (
    finalize_grid_generation,
    prune_grid_generations,
    write_grid,
)
from backend.shared.logger import get_logger
from backend.shared.manifest import replace_layer_manifest
from backend.shared.palettes import get_palette_names, load_palette
from backend.shared.state import ProcessedSet
from backend.shared.tiler import render_frame_palettes, tile_renderer_for_role


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
ALLOW_PERSISTENCE_FALLBACK = (
    os.environ.get("NOWCAST_ALLOW_PERSISTENCE_FALLBACK", "0") == "1"
)
MAX_INPUT_GAP_MIN = float(os.environ.get("NOWCAST_MAX_INPUT_GAP_MIN", "6"))
# Inputs are picked ~5 min apart from the 2-min MRMS stream: S-PROG then needs 12
# internal steps for the hour instead of ~30, each a full-CONUS advection.
INPUT_STEP_MIN = float(os.environ.get("NOWCAST_INPUT_STEP_MIN", "5"))
# The science grid is ~2 km after its bounded downsample. z6 is its honest
# display ceiling; z7 added 4x work while only magnifying interpolated pixels.
ZOOM_LEVELS = [4, 5, 6]
# Compact grids let the API sample the 12 public lead times at an arbitrary
# user location without decoding colorized tiles. Keep these much smaller
# than the seven-million-cell science inputs used by pySTEPS itself.
POINT_GRID_MAX_CELLS = int(os.environ.get("NOWCAST_POINT_GRID_MAX_CELLS", "900000"))
RENDER_WORKERS = max(1, int(os.environ.get("NOWCAST_RENDER_WORKERS", "4")))
# Retain enough complete runs to verify every lead time: a 60-min lead is
# scored when the observation arrives ~30 runs later at a 2-min cadence, and
# each run is 13 grids of ~1.5 MB. The shared prune helper also preserves
# recent writer orphans and its lock inode.
POINT_GRID_RETENTION_RUNS = max(2, int(os.environ.get("NOWCAST_POINT_GRID_RETENTION_RUNS", "36")))

log = get_logger("nowcast-activities")


@dataclass
class NowcastResult:
    ran: bool
    anchor_ts: str | None = None
    leadtimes: int = 0
    palettes: list[str] = field(default_factory=list)
    duration_s: float = 0.0
    # Retained earlier runs that predicted this anchor's observation and were scored against it.
    skill_scored: int = 0


def _load_grid(
    meta_path: Path,
) -> tuple[np.ndarray, np.ndarray, np.ndarray, dict] | None:
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
    return select_input_grids(sorted(radar_grid.glob("*.meta.json"), key=lambda p: p.name))


def _grid_time(path: Path) -> datetime | None:
    try:
        return datetime.fromisoformat(path.name.replace(".meta.json", ""))
    except ValueError:
        return None


def select_input_grids(metas: list[Path], step_min: float | None = None, frames: int | None = None) -> list[Path]:
    """Newest grid plus the ones nearest each earlier ``step_min`` mark; newest N if none fit."""
    step = INPUT_STEP_MIN if step_min is None else step_min
    frames = N_INPUT_FRAMES if frames is None else frames
    timed = [(t, p) for p in metas if (t := _grid_time(p)) is not None]
    if not timed:
        return metas[-frames:]
    chosen = [timed[-1]]
    for k in range(1, frames):
        target = timed[-1][0] - timedelta(minutes=k * step)
        earlier = [c for c in timed if (chosen[-1][0] - c[0]).total_seconds() >= 0.6 * step * 60]
        if not earlier:
            break
        best = min(earlier, key=lambda c: abs((c[0] - target).total_seconds()))
        if abs((best[0] - target).total_seconds()) > 0.45 * step * 60:
            break
        chosen.append(best)
    if len(chosen) < frames:
        return metas[-frames:]
    return [p for _, p in reversed(chosen)]


def _persistence_fallback(frames: list[np.ndarray], n_leadtimes: int) -> np.ndarray:
    last = frames[-1]
    return np.repeat(last[np.newaxis, :, :], n_leadtimes, axis=0).astype(np.float32)


def _write_nowcast_status(
    status: str, *, reason: str | None = None, detail: str | None = None
) -> None:
    STATE_DIR.mkdir(parents=True, exist_ok=True)
    body = {
        "status": status,
        "reason": reason,
        "detail": detail,
        "updated_at": datetime.now(timezone.utc).isoformat(),
    }
    fd, tmp_name = tempfile.mkstemp(
        prefix=".nowcast-status.", suffix=".tmp", dir=str(STATE_DIR)
    )
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


def _match_empirical_cdf(initial_array, target_array, ignore_indices=None):
    """pysteps' nonparam_match_empirical_cdf with the same output, sorting only wet cells.

    Minimum-valued (dry) cells map to the target minimum, so only the k wet ranks need the
    k largest target values: a partition plus two k-element sorts instead of two full
    argsorts of the ~6M-cell CONUS grid, which were over half of every S-PROG run.
    """
    initial = np.array(initial_array, dtype=float)
    target = np.array(target_array, dtype=float)
    if np.all(np.isnan(initial)):
        raise ValueError("Initial array contains only nans.")
    if initial.size != target.size:
        raise ValueError("dimension mismatch between initial_array and target_array")
    zvalue = np.nanmin(initial)
    if ignore_indices is not None:
        initial[ignore_indices] = zvalue
    if np.any(~np.isfinite(initial)):
        raise ValueError("Initial array contains non-finite values outside ignore_indices mask.")
    zvalue_trg = np.nanmin(target)
    target = np.where(np.isnan(target), zvalue_trg, target).reshape(-1)
    wet = (initial > zvalue).reshape(-1)
    k = int(wet.sum())
    if np.sum(target > zvalue_trg) > k:
        p = np.percentile(target, 100 * (1 - k / initial.size))
        target[target < p] = zvalue_trg
    out = np.full(initial.size, zvalue_trg, dtype=float)
    if k:
        n = target.size
        top = np.sort(np.partition(target, n - k)[n - k:])
        idx = np.flatnonzero(wet)
        out[idx[np.argsort(initial.reshape(-1)[idx])]] = top
    out = out.reshape(initial.shape)
    if ignore_indices is not None:
        out[ignore_indices] = np.asarray(initial_array)[ignore_indices]
    return out


def _install_fast_cdf_match() -> None:
    try:
        from pysteps.postprocessing import probmatching
    except ImportError:
        return
    # S-PROG calls this once per internal step through the module attribute.
    probmatching.nonparam_match_empirical_cdf = _match_empirical_cdf


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
        _install_fast_cdf_match()
        try:
            forecast = nowcaster(
                stack[-3:, :, :],
                uv,
                lead_steps,
                n_cascade_levels=6,
                precip_thr=5.0,
            )
        except TypeError:
            forecast = nowcaster(
                stack[-3:, :, :],
                uv,
                lead_steps,
                n_cascade_levels=6,
                R_thr=5.0,
            )
    except Exception as exc:  # noqa: BLE001
        log.warning("pysteps_failed", extra={"err": str(exc)})
        return _degraded_result(
            frames, lead_steps, reason="pysteps_failed", detail=str(exc)
        )
    forecast = np.asarray(forecast, dtype=np.float32)
    forecast = np.where(np.isnan(forecast), -9999.0, forecast)
    _write_nowcast_status("ok")
    return forecast, "pysteps-sprog"


def _nowcast_tile_path(anchor_ts: str, valid_ts: str) -> str:
    return f"runs/{anchor_ts}/{valid_ts}"


def _nowcast_grid_key(anchor_ts: str, valid_ts: str) -> str:
    return f"runs/{anchor_ts}/{valid_ts}"


def _prune_nowcast_point_grids(active_generation: str) -> int:
    """Retain whole point-grid runs, including the manifest's active run."""
    return prune_grid_generations(
        "nowcast",
        keep=POINT_GRID_RETENTION_RUNS,
        active_generation=active_generation,
    )


def _render_frame(
    tile_base: Path,
    palette_tables: dict[str, dict],
    tile_path: str,
    data: np.ndarray,
    lats: np.ndarray,
    lons: np.ndarray,
) -> list[str]:
    """Render one leadtime's tile pyramid. Manifest publishing happens once
    per RUN (replace_layer_manifest in nowcast_run), not per frame — so a
    half-finished run is never visible to the app, and frames from previous
    anchor runs don't pile up in the manifest.
    """
    color_tables = {
        pname: tables["reflectivity"]
        for pname, tables in palette_tables.items()
        if tables.get("reflectivity")
    }
    if not color_tables:
        return []
    out_dirs = {
        pname: str(tile_base / "nowcast" / pname / tile_path) for pname in color_tables
    }
    result = render_frame_palettes(
        data,
        lats,
        lons,
        color_tables,
        out_dirs,
        ZOOM_LEVELS,
        nodata_value=-9999.0,
        min_valid_weight=1.0,
        renderer=tile_renderer_for_role("nowcast"),
        overview="max",
        source_id=f"nowcast:{tile_path}",
        publication_lock_root=tile_base / "nowcast",
    )
    return result.rendered_palettes


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
        # Kicked and buffered runs often arrive before a new grid exists;
        # answer them before reading ~100 MB of inputs.
        if latest_iso in ProcessedSet(STATE_DIR / "nowcast.json", max_entries=100):
            return (False, latest_iso, [], {}, 0.0)
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
        if not intervals or min(intervals) <= 0 or max(intervals) > max(MAX_INPUT_GAP_MIN, 1.5 * INPUT_STEP_MIN):
            _write_nowcast_status(
                "degraded",
                reason="invalid_input_cadence",
                detail=f"intervals_minutes={intervals}",
            )
            return (False, latest_iso, [], {}, 0.0)
        input_interval_min = float(np.median(np.asarray(intervals)))
        return (True, latest_iso, grids, meta_used, input_interval_min)

    ok, latest_iso, grids, meta_used, input_interval_min = await asyncio.to_thread(
        _setup
    )
    if not ok:
        return NowcastResult(ran=False, anchor_ts=latest_iso)

    n_lead = HORIZON_MIN // STEP_MIN
    # pySTEPS lead times are measured in input timesteps, not minutes. MRMS is
    # commonly ~2 minutes; passing the integer count mislabeled a 2-minute
    # forecast step as 5 minutes. Fractional requested timesteps preserve the
    # public 5-minute timeline against the measured input cadence.
    lead_steps = [
        ((index + 1) * STEP_MIN) / input_interval_min for index in range(n_lead)
    ]
    activity.heartbeat(
        {"phase": "pysteps", "input_frames": len(grids), "leadtimes": n_lead}
    )
    forecast = await run_sync_with_heartbeat(
        _run_nowcast,
        grids,
        lead_steps,
        heartbeat_every=30,
        heartbeat_details=lambda: {
            "phase": "pysteps",
            "input_frames": len(grids),
            "leadtimes": n_lead,
        },
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
    lats_arr = np.linspace(
        meta_used["lat_max"], meta_used["lat_min"], h, dtype=np.float64
    )
    lons_arr = np.linspace(
        meta_used["lon_min"], meta_used["lon_max"], w, dtype=np.float64
    )

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

    def _render_lead(i: int) -> tuple[set[str], dict] | None:
        valid = latest_dt + timedelta(minutes=(i + 1) * STEP_MIN)
        ts = valid.isoformat()
        tile_path = _nowcast_tile_path(latest_iso, ts)
        frame = np.where(forecast[i] < 5, -9999.0, forecast[i])
        palettes = _render_frame(TILE_DIR, palette_tables, tile_path, frame, lats_arr, lons_arr)
        if set(palettes) == expected_palettes:
            grid_key = _nowcast_grid_key(latest_iso, ts)
            grid_file = write_grid(
                "nowcast", ts, frame, lats_arr, lons_arr, "dBZ", -9999.0,
                POINT_GRID_MAX_CELLS, grid_key=grid_key,
            )
            if grid_file:
                return set(palettes), {
                    "timestamp": ts,
                    "path": tile_path,
                    "grid_key": grid_key,
                    "source": "mrms-nowcast",
                    "kind": "nowcast",
                    "issued_at": latest_dt.isoformat(),
                    "lead_minutes": (i + 1) * STEP_MIN,
                    "input_interval_minutes": round(input_interval_min, 3),
                    "method": method,
                    "spatial_resolution_km": resolution_km,
                    "max_zoom": max(ZOOM_LEVELS),
                }
        for palette in palettes:
            shutil.rmtree(TILE_DIR / "nowcast" / palette / tile_path, ignore_errors=True)
        return None

    def _render_all() -> list[tuple[set[str], dict] | None]:
        # Tile encoding releases the GIL; leads are independent pyramids under one publish lock root.
        with ThreadPoolExecutor(max_workers=RENDER_WORKERS) as pool:
            return list(pool.map(_render_lead, range(n_lead)))

    for rendered in await run_sync_with_heartbeat(
        _render_all,
        heartbeat_every=30,
        heartbeat_details={"phase": "render", "leadtimes": n_lead},
    ):
        if rendered is None:
            continue
        palettes, frame_meta = rendered
        rendered_palettes.update(palettes)
        rendered_timestamps.append(frame_meta["timestamp"])
        manifest_frames.append(frame_meta)

    def _store_observed_anchor() -> None:
        # The observation this run started from, at point-grid resolution, so
        # the verifier can score a "nothing moves" baseline next to the forecast.
        # Written before the generation marker so retention sees a complete run.
        write_grid(
            "nowcast", latest_iso, grids[-1], lats_arr, lons_arr, "dBZ", -9999.0,
            POINT_GRID_MAX_CELLS, grid_key=_nowcast_grid_key(latest_iso, nowcast_skill.OBSERVED_KEY),
        )

    def _commit() -> None:
        if len(rendered_timestamps) != n_lead:
            raise RuntimeError(
                f"nowcast incomplete: rendered {len(rendered_timestamps)}/{n_lead} leadtimes"
            )
        try:
            _store_observed_anchor()
        except Exception as exc:  # noqa: BLE001 - verification is optional; publication is not
            log.warning("nowcast_observed_anchor_failed", extra={"anchor": latest_iso, "err": str(exc)})
        # Mark complete before the manifest swap so a crash here leaves the old manifest fully readable.
        finalize_grid_generation("nowcast", latest_iso)
        # One atomic swap replaces ALL previous nowcast frames; old tile dirs linger until the cleanup sweep.
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
        # Prune under the shared layer lock only (an ad-hoc sweep could unlink .grid.lock or a committed generation);
        # keep two full runs so a newer writer cannot evict grids this manifest references.
        _prune_nowcast_point_grids(latest_iso)
        state = ProcessedSet(STATE_DIR / "nowcast.json", max_entries=100)
        state.add(latest_iso)

    await asyncio.to_thread(_commit)
    log.info("nowcast_complete", extra={"anchor": latest_iso, "leadtimes": n_lead})

    # Verification: this anchor is the observation earlier runs forecast. Score
    # them now, after publication, so a scoring fault can never delay a frame.
    skill_scored = 0
    try:
        scored = await asyncio.to_thread(
            nowcast_skill.score_observation,
            grids[-1],
            latest_iso,
            lats_arr,
            lons_arr,
            grid_dir=GRID_DIR,
            state_dir=STATE_DIR,
            point_grid_max_cells=POINT_GRID_MAX_CELLS,
        )
        skill_scored = len(scored)
        if scored:
            log.info(
                "nowcast_scored",
                extra={
                    "valid": latest_iso,
                    "runs": skill_scored,
                    "leads": sorted(int(e["lead_minutes"]) for e in scored),
                },
            )
    except Exception as exc:  # noqa: BLE001
        log.warning("nowcast_skill_failed", extra={"valid": latest_iso, "err": str(exc)})

    return NowcastResult(
        ran=True,
        anchor_ts=latest_iso,
        leadtimes=n_lead,
        palettes=sorted(rendered_palettes),
        duration_s=round(time.time() - started, 2),
        skill_scored=skill_scored,
    )
