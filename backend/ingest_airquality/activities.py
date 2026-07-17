"""Temporal activities for the NOAA air-quality (NAQFC / AQMv7) ingest.

Source: the public `noaa-nws-naqfc-pds` S3 bucket — no API key, no
aggregator. Each AQMv7 CONUS cycle (06z / 12z) ships one GRIB2 file per
pollutant containing 72 hourly-average messages (valid run+0h .. run+71h),
so a single run covers "current" air quality AND a 3-day forecast:

    AQMv7/CS/{YYYYMMDD}/{CC}/aqm.t{CC}z.ave_1hr_pm25_bc.{YYYYMMDD}.227.grib2
    AQMv7/CS/{YYYYMMDD}/{CC}/aqm.t{CC}z.ave_1hr_o3_bc.{YYYYMMDD}.227.grib2

The `_bc` products are NOAA's bias-corrected guidance — the ones AirNow
forecasters use. Grid 227 is a ~5 km Lambert-conformal CONUS grid, handled
by the same curvilinear reprojection path the HRRR ingest uses.

One activity renders a chunk of message indexes so a transient failure on
hour 60 doesn't force redownloading/rerendering hours 0-59.
"""

from __future__ import annotations

import asyncio
import os
import shutil
import time
from dataclasses import dataclass, field
from datetime import datetime, timedelta, timezone
from pathlib import Path

import httpx
import numpy as np
import pygrib
from temporalio import activity

from backend.shared.activity_heartbeat import run_sync_with_heartbeat
from backend.shared.grid_dump import write_grid
from backend.shared.logger import get_logger
from backend.shared.manifest import read_manifest_file, replace_layer_manifest
from backend.shared.palettes import get_palette_names, load_palette
from backend.shared.state import ProcessedSet
from backend.shared.tiler import apply_color_table, render_tiles_atomic


AQM_BASE = "https://noaa-nws-naqfc-pds.s3.amazonaws.com/AQMv7/CS"
TILE_DIR = os.environ.get("TILE_DIR", "/data/tiles")
STATE_DIR = os.environ.get("STATE_DIR", "/data/state")
TMP_ROOT = Path(os.environ.get("AQM_TMP_ROOT", "/tmp/aqm_work"))
# AQM is a ~5 km model; z6 matches the HRRR ceiling and keeps the fanout small.
ZOOM_LEVELS = [4, 5, 6]
FORECAST_MESSAGES = 72
CHUNK_MESSAGES = 24
# Cycles land twice a day (06z/12z) with a few hours of product lag.
AQM_CYCLES = (12, 6)

log = get_logger("ingest-airquality-activities")


# layer name (tile subtree + manifest key) → GRIB product config
AQM_LAYERS: dict[str, dict[str, str]] = {
    "air-quality": {"product": "ave_1hr_pm25_bc", "color_key": "pm25", "unit": "µg/m³"},
    "ozone": {"product": "ave_1hr_o3_bc", "color_key": "ozone", "unit": "ppb"},
}


@dataclass
class AqmFindRunResult:
    run_id: str | None
    already_processed: bool


@dataclass
class AqmChunkResult:
    layer: str
    start_msg: int
    rendered_timestamps: list[str] = field(default_factory=list)


@dataclass
class AqmCleanupResult:
    tile_dirs_removed: int


@dataclass
class _Grid:
    data: np.ndarray
    lats: np.ndarray
    lons: np.ndarray
    source_crs: str | None = None
    source_x: np.ndarray | None = None
    source_y: np.ndarray | None = None


def _state_path() -> Path:
    return Path(STATE_DIR) / "ingest-airquality.json"


def _grib_url(run_id: str, product: str) -> str:
    date_str, cycle = run_id.split("_")
    return (
        f"{AQM_BASE}/{date_str}/{cycle}/"
        f"aqm.t{cycle}z.{product}.{date_str}.227.grib2"
    )


def _find_latest_run_sync(client: httpx.Client) -> str | None:
    now = datetime.now(timezone.utc)
    for days_ago in range(0, 3):
        date_str = (now - timedelta(days=days_ago)).strftime("%Y%m%d")
        for cycle in AQM_CYCLES:
            run_dt = datetime.strptime(f"{date_str}{cycle:02d}", "%Y%m%d%H").replace(
                tzinfo=timezone.utc
            )
            if run_dt > now:
                continue
            run_id = f"{date_str}_{cycle:02d}"
            try:
                r = client.head(_grib_url(run_id, "ave_1hr_pm25_bc"), timeout=10)
                if r.status_code == 200:
                    return run_id
            except httpx.HTTPError:
                continue
    return None


def _normalize_lons(lons: np.ndarray) -> np.ndarray:
    lons_arr = np.asarray(lons, dtype=np.float64)
    return np.where(lons_arr > 180.0, lons_arr - 360.0, lons_arr)


def _extract_native_projection(
    grb: object, lats: np.ndarray, lons: np.ndarray
) -> tuple[str, np.ndarray, np.ndarray] | None:
    """Return native projected CRS + x/y axes for the curvilinear AQM grid."""
    projparams = getattr(grb, "projparams", None)
    if not projparams:
        return None
    try:
        from pyproj import CRS, Proj, Transformer

        try:
            crs = CRS.from_user_input(projparams)
        except Exception:
            crs = Proj(projparams).crs

        transformer = Transformer.from_crs("EPSG:4326", crs, always_xy=True)
        xs, ys = transformer.transform(
            _normalize_lons(lons), lats.astype(np.float64, copy=False)
        )
        h, w = lats.shape
        x0 = float(np.nanmedian(xs[:, 0]))
        x1 = float(np.nanmedian(xs[:, -1]))
        y0 = float(np.nanmedian(ys[0, :]))
        y1 = float(np.nanmedian(ys[-1, :]))
        if not all(np.isfinite(v) for v in (x0, x1, y0, y1)):
            return None
        if abs(x1 - x0) < 1e-6 or abs(y1 - y0) < 1e-6:
            return None
        source_x = np.linspace(x0, x1, w, dtype=np.float64)
        source_y = np.linspace(y0, y1, h, dtype=np.float64)
        return crs.to_wkt(), source_x, source_y
    except Exception as exc:  # noqa: BLE001
        log.warning("projection_extract_failed", extra={"err": str(exc)})
        return None


def _grid_from_message(grb: object, geometry: _Grid | None = None) -> _Grid:
    """Build a _Grid; pass a prior message's grid to reuse its geometry.

    Every message in an AQM file shares one static grid, and the projection
    extraction transforms all ~1.5M points — recomputing it per message
    roughly doubles render time for zero benefit.
    """
    data = grb.values
    if hasattr(data, "filled"):
        data = data.filled(np.nan)
    data = data.astype(np.float32)
    if geometry is not None:
        return _Grid(
            data=data,
            lats=geometry.lats,
            lons=geometry.lons,
            source_crs=geometry.source_crs,
            source_x=geometry.source_x,
            source_y=geometry.source_y,
        )
    lats, lons = grb.latlons()
    lons = _normalize_lons(lons)
    native = _extract_native_projection(grb, lats, lons)
    if native is not None:
        source_crs, source_x, source_y = native
        return _Grid(
            data=data,
            lats=lats.astype(np.float64),
            lons=lons.astype(np.float64),
            source_crs=source_crs,
            source_x=source_x,
            source_y=source_y,
        )
    return _Grid(
        data=data,
        lats=lats[:, 0].astype(np.float64),
        lons=lons[0, :].astype(np.float64),
    )


def _write_palette_tiles(
    tile_base: Path, layer: str, palette: str, path: str, rgba: np.ndarray, grid: _Grid
) -> int:
    lats = grid.lats
    lons = grid.lons
    source_y = grid.source_y
    if grid.source_crs is None and lats.ndim == 1 and lats[0] > lats[-1]:
        rgba = np.flipud(rgba)
        lats = lats[::-1]
    elif grid.source_crs is not None and source_y is not None and source_y[0] > source_y[-1]:
        rgba = np.flipud(rgba)
        lats = np.flipud(lats)
        lons = np.flipud(lons)
        source_y = source_y[::-1]
    out_dir = str(tile_base / layer / palette / path)
    return render_tiles_atomic(
        rgba=rgba,
        lats=lats,
        lons=lons,
        output_dir=out_dir,
        zoom_levels=ZOOM_LEVELS,
        source_crs=grid.source_crs,
        source_x=grid.source_x,
        source_y=source_y,
    )


def _render_per_palette(
    tile_base: Path,
    layer: str,
    tile_path: str,
    grid: _Grid,
    palette_tables: dict[str, dict],
    color_key: str,
) -> list[str]:
    """All-or-nothing across palettes, mirroring the HRRR frame gate."""
    rendered: list[str] = []
    expected = {name for name, tables in palette_tables.items() if color_key in tables}
    for pname, tables in palette_tables.items():
        entry = tables.get(color_key)
        if not entry:
            continue
        rgba = apply_color_table(grid.data, entry)
        if _write_palette_tiles(tile_base, layer, pname, tile_path, rgba, grid) > 0:
            rendered.append(pname)
    if set(rendered) != expected:
        for pname in rendered:
            shutil.rmtree(tile_base / layer / pname / tile_path, ignore_errors=True)
        return []
    return rendered


def _grid_dump_axes(grid: _Grid) -> tuple[np.ndarray, np.ndarray]:
    if grid.lats.ndim == 1 and grid.lons.ndim == 1:
        return grid.lats, grid.lons
    row = grid.lats.shape[0] // 2
    col = grid.lons.shape[1] // 2
    return grid.lats[:, col].astype(np.float64), grid.lons[row, :].astype(np.float64)


def _safe_grid_dump(layer: str, ts: str, grid: _Grid, unit: str) -> None:
    try:
        lats, lons = _grid_dump_axes(grid)
        write_grid(layer, ts, grid.data, lats, lons, unit=unit)
    except Exception as exc:  # noqa: BLE001
        log.warning("grid_dump_failed", extra={"layer": layer, "err": str(exc)})


def _load_palette_tables() -> dict[str, dict]:
    out: dict[str, dict] = {}
    for name in get_palette_names():
        try:
            out[name] = load_palette(name)
        except FileNotFoundError:
            log.warning("palette_missing", extra={"palette": name})
    if not out:
        out["classic"] = load_palette("classic")
    return out


def _safe_path_part(value: object) -> str:
    raw = str(value)
    safe = "".join(ch if ch.isalnum() or ch in ("-", "_") else "_" for ch in raw)
    return safe.strip("_") or "unknown"


def _current_activity_tmp_dir(prefix: str, *parts: object) -> Path:
    info = activity.info()
    name_parts = [
        _safe_path_part(prefix),
        _safe_path_part(info.workflow_id),
        _safe_path_part(info.workflow_run_id),
        _safe_path_part(info.activity_id),
        f"attempt{info.attempt}",
        *(_safe_path_part(p) for p in parts),
    ]
    tmp_dir = TMP_ROOT / "-".join(name_parts)
    tmp_dir.mkdir(parents=True, exist_ok=True)
    return tmp_dir


def _render_chunk_sync(
    run_id: str,
    layer: str,
    start_msg: int,
    end_msg: int,
    grib_path: Path,
    palette_tables: dict[str, dict],
) -> AqmChunkResult:
    config = AQM_LAYERS[layer]
    tile_base = Path(TILE_DIR)
    result = AqmChunkResult(layer=layer, start_msg=start_msg)

    grbs = pygrib.open(str(grib_path))
    geometry: _Grid | None = None
    try:
        for msg_idx in range(start_msg, end_msg):
            grb = grbs.message(msg_idx + 1)  # pygrib messages are 1-based
            valid = grb.validDate.replace(tzinfo=timezone.utc)
            ts = valid.isoformat()
            tile_path = f"runs/{run_id}/{ts}"
            grid = _grid_from_message(grb, geometry)
            geometry = grid
            palettes = _render_per_palette(
                tile_base, layer, tile_path, grid, palette_tables, config["color_key"]
            )
            if not palettes:
                continue
            result.rendered_timestamps.append(ts)
            _safe_grid_dump(layer, ts, grid, config["unit"])
    finally:
        grbs.close()
    return result


# ---------- activities ----------


@activity.defn(name="aqm_find_latest_run")
async def aqm_find_latest_run() -> AqmFindRunResult:
    def _go() -> AqmFindRunResult:
        with httpx.Client() as client:
            run_id = _find_latest_run_sync(client)
        if run_id is None:
            return AqmFindRunResult(run_id=None, already_processed=False)
        state = ProcessedSet(_state_path(), max_entries=100)
        return AqmFindRunResult(run_id=run_id, already_processed=run_id in state)

    return await asyncio.to_thread(_go)


@activity.defn(name="aqm_render_chunk")
async def aqm_render_chunk(run_id: str, layer: str, start_msg: int) -> AqmChunkResult:
    """Download the pollutant file and render one chunk of forecast hours."""
    started = time.time()
    config = AQM_LAYERS[layer]
    end_msg = min(start_msg + CHUNK_MESSAGES, FORECAST_MESSAGES)
    palette_tables = _load_palette_tables()
    tmp_dir = _current_activity_tmp_dir("aqm", run_id, layer, f"m{start_msg:02d}")
    url = _grib_url(run_id, config["product"])

    activity.heartbeat({"phase": "download", "layer": layer, "start_msg": start_msg})

    def _download() -> Path:
        out = tmp_dir / f"{config['product']}.grib2"
        with httpx.Client() as client, client.stream("GET", url, timeout=300) as resp:
            resp.raise_for_status()
            with out.open("wb") as fh:
                for chunk in resp.iter_bytes(chunk_size=1 << 20):
                    fh.write(chunk)
        return out

    try:
        grib_path = await asyncio.to_thread(_download)
        activity.heartbeat({"phase": "render", "layer": layer, "start_msg": start_msg})
        result = await run_sync_with_heartbeat(
            _render_chunk_sync,
            run_id,
            layer,
            start_msg,
            end_msg,
            grib_path,
            palette_tables,
            heartbeat_every=30,
            heartbeat_details=lambda: {
                "phase": "render",
                "run_id": run_id,
                "layer": layer,
                "start_msg": start_msg,
            },
        )
    finally:
        shutil.rmtree(tmp_dir, ignore_errors=True)

    log.info(
        "chunk_done",
        extra={
            "run_id": run_id,
            "layer": layer,
            "start_msg": start_msg,
            "rendered": len(result.rendered_timestamps),
            "duration_s": round(time.time() - started, 1),
        },
    )
    return result


def _publish_run_sync(
    run_id: str,
    results: list[AqmChunkResult],
    palette_tables: dict[str, dict],
    *,
    state_dir: str | Path | None = None,
) -> list[str]:
    """Commit each pollutant layer to the manifest only when all 72 hours exist."""
    run_dt = datetime.strptime(run_id, "%Y%m%d_%H").replace(tzinfo=timezone.utc)
    run_issued_at = run_dt.isoformat()
    published: list[str] = []

    for layer, config in AQM_LAYERS.items():
        timestamps = sorted(
            {
                ts
                for result in results
                if result.layer == layer
                for ts in result.rendered_timestamps
            }
        )
        if len(timestamps) != FORECAST_MESSAGES:
            continue
        palettes = sorted(
            name
            for name, tables in palette_tables.items()
            if config["color_key"] in tables
        )
        if not palettes:
            continue
        frames = [
            {
                "timestamp": ts,
                "path": f"runs/{run_id}/{ts}",
                "source": "aqm",
                "kind": "model_guidance",
                "issued_at": run_issued_at,
                "run_id": run_id,
                "lead_minutes": int(
                    (datetime.fromisoformat(ts) - run_dt).total_seconds() // 60
                ),
                "spatial_resolution_km": 5.0,
                "max_zoom": max(ZOOM_LEVELS),
            }
            for ts in timestamps
        ]
        replace_layer_manifest(
            layer,
            timestamps,
            palettes=palettes,
            state_dir=state_dir or STATE_DIR,
            frames=frames,
            layer_metadata={
                "title": "AQM PM2.5 (US AQI)" if layer == "air-quality" else "AQM ozone",
                "kind": "model_guidance",
                "run_id": run_id,
                "complete": True,
            },
        )
        published.append(layer)
    return published


@activity.defn(name="aqm_publish_run")
async def aqm_publish_run(run_id: str, results: list[AqmChunkResult]) -> list[str]:
    palette_tables = await asyncio.to_thread(_load_palette_tables)
    return await asyncio.to_thread(_publish_run_sync, run_id, results, palette_tables)


@activity.defn(name="aqm_mark_processed")
async def aqm_mark_processed(run_id: str) -> None:
    def _go() -> None:
        state = ProcessedSet(_state_path(), max_entries=100)
        state.add(run_id)

    await asyncio.to_thread(_go)


@activity.defn(name="aqm_cleanup")
async def aqm_cleanup(retention_hours: int) -> AqmCleanupResult:
    """Sweep superseded AQM run subtrees (same layout as HRRR versioned runs)."""

    def _go() -> AqmCleanupResult:
        tile_base = Path(TILE_DIR)
        manifest = read_manifest_file(STATE_DIR)
        cutoff = time.time() - retention_hours * 3600
        removed = 0
        for layer in AQM_LAYERS:
            current_run = manifest.get("layers", {}).get(layer, {}).get("run_id")
            layer_dir = tile_base / layer
            if not layer_dir.exists():
                continue
            for palette_dir in sorted(layer_dir.iterdir()):
                runs_root = palette_dir / "runs"
                if not runs_root.is_dir():
                    continue
                for run_dir in runs_root.iterdir():
                    if not run_dir.is_dir() or run_dir.name == current_run:
                        continue
                    try:
                        run_dt = datetime.strptime(run_dir.name, "%Y%m%d_%H").replace(
                            tzinfo=timezone.utc
                        )
                    except ValueError:
                        continue
                    if run_dt.timestamp() < cutoff:
                        shutil.rmtree(run_dir, ignore_errors=True)
                        removed += 1
        return AqmCleanupResult(tile_dirs_removed=removed)

    return await asyncio.to_thread(_go)
