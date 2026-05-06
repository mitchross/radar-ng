"""Temporal activities for the HRRR forecast ingest pipeline.

The HRRR run produces ~13 forecast layers per hour for either 18h (regular
runs) or 48h (00z/06z/12z/18z extended runs). We keep one activity per
forecast hour so a transient S3 failure on hour 30 doesn't make us redownload
hours 1-29.
"""

from __future__ import annotations

import asyncio
import os
import shutil
import time
from dataclasses import dataclass, field
from datetime import datetime, timedelta, timezone
from pathlib import Path
from typing import Iterable

import httpx
import numpy as np
import pygrib
from temporalio import activity

from backend.shared.grid_dump import cleanup_old_grids, write_grid
from backend.shared.logger import get_logger
from backend.shared.palettes import get_palette_names, load_palette
from backend.shared.state import ProcessedSet
from backend.shared.tiler import apply_categorical_color_table, apply_color_table, render_tiles


HRRR_BASE = "https://noaa-hrrr-bdp-pds.s3.amazonaws.com"
TILE_DIR = os.environ.get("TILE_DIR", "/data/tiles")
STATE_DIR = os.environ.get("STATE_DIR", "/data/state")
TMP_ROOT = Path(os.environ.get("HRRR_TMP_ROOT", "/tmp/hrrr_work"))
FORECAST_HOURS = int(os.environ.get("FORECAST_HOURS", "18"))
EXTENDED_FORECAST_HOURS = int(os.environ.get("EXTENDED_FORECAST_HOURS", "48"))
EXTENDED_RUNS = {0, 6, 12, 18}
ZOOM_LEVELS = [4, 5, 6, 7, 8, 9]

log = get_logger("ingest-hrrr-activities")


IDX_MATCHERS = {
    "refc": ("REFC", "entire atmosphere"),
    "t2m": ("TMP", "2 m above ground"),
    "dpt2m": ("DPT", "2 m above ground"),
    "cape": ("CAPE", "surface"),
    "u10": ("UGRD", "10 m above ground"),
    "v10": ("VGRD", "10 m above ground"),
    "crain": ("CRAIN", "surface"),
    "csnow": ("CSNOW", "surface"),
    "cfrzr": ("CFRZR", "surface"),
    "cicep": ("CICEP", "surface"),
    "rh2m": ("RH", "2 m above ground"),
    "apcp": ("APCP", "surface"),
    "tcdc": ("TCDC", "entire atmosphere"),
}

VAR_SELECTORS = {
    "refc": {"name": "Composite reflectivity", "typeOfLevel": "atmosphere"},
    "t2m": {"name": "2 metre temperature", "typeOfLevel": "heightAboveGround", "level": 2},
    "dpt2m": {"name": "2 metre dewpoint temperature", "typeOfLevel": "heightAboveGround", "level": 2},
    "cape": {"name": "Convective available potential energy", "typeOfLevel": "surface"},
    "u10": {"name": "10 metre U wind component", "typeOfLevel": "heightAboveGround", "level": 10},
    "v10": {"name": "10 metre V wind component", "typeOfLevel": "heightAboveGround", "level": 10},
    "crain": {"name": "Categorical rain", "typeOfLevel": "surface"},
    "csnow": {"name": "Categorical snow", "typeOfLevel": "surface"},
    "cfrzr": {"name": "Categorical freezing rain", "typeOfLevel": "surface"},
    "cicep": {"name": "Categorical ice pellets", "typeOfLevel": "surface"},
    "rh2m": {"name": "2 metre relative humidity", "typeOfLevel": "heightAboveGround", "level": 2},
    "apcp": {"name": "Total Precipitation", "typeOfLevel": "surface"},
    "tcdc": {"name": "Total Cloud Cover", "typeOfLevel": "atmosphere"},
}

HRRR_TILE_LAYERS = [
    "radar-hrrr", "temperature", "dewpoint", "humidity", "wind", "cape",
    "precip-type", "precip-accum", "cloud",
]


# ---------- serialisable I/O ----------


@dataclass
class FindRunResult:
    run_id: str | None
    already_processed: bool


@dataclass
class ForecastHourResult:
    fhr: int
    rendered_layers: list[str] = field(default_factory=list)
    duration_s: float = 0.0


@dataclass
class HrrrCleanupResult:
    tile_dirs_removed: int
    grid_files_removed: int


# ---------- helpers ----------


def _state_path() -> Path:
    return Path(STATE_DIR) / "ingest-hrrr.json"


def _safe_path_part(value: object) -> str:
    raw = str(value)
    safe = "".join(ch if ch.isalnum() or ch in ("-", "_") else "_" for ch in raw)
    return safe.strip("_") or "unknown"


def _activity_tmp_dir(
    prefix: str,
    *,
    workflow_id: str,
    run_id: str,
    activity_id: str,
    attempt: int,
    parts: tuple[object, ...] = (),
) -> Path:
    name_parts = [
        _safe_path_part(prefix),
        _safe_path_part(workflow_id),
        _safe_path_part(run_id),
        _safe_path_part(activity_id),
        f"attempt{attempt}",
        *(_safe_path_part(p) for p in parts),
    ]
    return TMP_ROOT / "-".join(name_parts)


def _current_activity_tmp_dir(prefix: str, *parts: object) -> Path:
    info = activity.info()
    tmp_dir = _activity_tmp_dir(
        prefix,
        workflow_id=info.workflow_id,
        run_id=info.workflow_run_id,
        activity_id=info.activity_id,
        attempt=info.attempt,
        parts=parts,
    )
    tmp_dir.mkdir(parents=True, exist_ok=True)
    return tmp_dir


def _head_status(client: httpx.Client, url: str) -> int:
    return client.head(url, timeout=10).raise_for_status().status_code  # type: ignore[union-attr]


def _find_latest_run_sync(client: httpx.Client) -> str | None:
    now = datetime.now(timezone.utc)
    for hours_ago in range(0, 12):
        dt = now - timedelta(hours=hours_ago)
        run_dt = dt.replace(minute=0, second=0, microsecond=0)
        date_str = run_dt.strftime("%Y%m%d")
        run_str = f"{run_dt.hour:02d}"
        key = f"hrrr.{date_str}/conus/hrrr.t{run_str}z.wrfsfcf01.grib2"
        try:
            r = client.head(f"{HRRR_BASE}/{key}", timeout=10)
            if r.status_code == 200:
                return f"{date_str}_{run_str}"
        except httpx.HTTPError:
            continue
    return None


def _get_idx_sync(client: httpx.Client, idx_url: str) -> list[dict]:
    resp = client.get(idx_url, timeout=15)
    resp.raise_for_status()
    records: list[dict] = []
    for raw in resp.text.strip().splitlines():
        parts = raw.split(":")
        if len(parts) < 5:
            continue
        records.append({
            "num": int(parts[0]),
            "offset": int(parts[1]),
            "line": raw,
            "var_name": parts[3],
            "level": parts[4],
            "fcst_time": parts[5] if len(parts) > 5 else "",
        })
    return records


def _pick_ranges(records: list[dict], matchers: Iterable[tuple[str, ...]]) -> list[tuple[int, int | None]]:
    match_list = [tuple(m) for m in matchers]
    ranges: list[tuple[int, int | None]] = []
    for i, rec in enumerate(records):
        if not any(all(term in rec["line"] for term in m) for m in match_list):
            continue
        start = rec["offset"]
        end = records[i + 1]["offset"] - 1 if i + 1 < len(records) else None
        ranges.append((start, end))
    return ranges


def _download_subset_sync(
    client: httpx.Client,
    date_str: str,
    run_hour: str,
    fhr: int,
    needed: list[str],
    tmp_dir: Path,
) -> Path | None:
    key = f"hrrr.{date_str}/conus/hrrr.t{run_hour}z.wrfsfcf{fhr:02d}.grib2"
    base_url = f"{HRRR_BASE}/{key}"
    idx_url = f"{base_url}.idx"

    try:
        records = _get_idx_sync(client, idx_url)
    except httpx.HTTPError:
        # Full download fallback
        resp = client.get(base_url, timeout=300)
        resp.raise_for_status()
        out = tmp_dir / f"hrrr_f{fhr:02d}.grib2"
        out.write_bytes(resp.content)
        return out

    matchers = [IDX_MATCHERS[n] for n in needed if n in IDX_MATCHERS]
    ranges = _pick_ranges(records, matchers)
    if not ranges:
        return None

    out_path = tmp_dir / f"hrrr_f{fhr:02d}_subset.grib2"
    with out_path.open("wb") as fh:
        for start, end in ranges:
            header = f"bytes={start}-{end if end is not None else ''}"
            r = client.get(base_url, headers={"Range": header}, timeout=120)
            r.raise_for_status()
            fh.write(r.content)
    return out_path


def _extract_variable(grib_path: Path, match: dict) -> tuple[np.ndarray, np.ndarray, np.ndarray] | None:
    try:
        grbs = pygrib.open(str(grib_path))
        for grb in grbs:
            ok = True
            for k, v in match.items():
                if k == "name":
                    if v.lower() not in grb.name.lower():
                        ok = False
                        break
                elif getattr(grb, k, None) != v:
                    ok = False
                    break
            if ok:
                data = grb.values
                lats, lons = grb.latlons()
                if hasattr(data, "filled"):
                    data = data.filled(np.nan)
                grbs.close()
                return (
                    data.astype(np.float32),
                    lats[:, 0].astype(np.float64),
                    lons[0, :].astype(np.float64),
                )
        grbs.close()
    except Exception as exc:  # noqa: BLE001
        log.warning("extract_failed", extra={"err": str(exc), "match": match})
    return None


def _kelvin_to_f(k: np.ndarray) -> np.ndarray:
    return (k - 273.15) * 9 / 5 + 32


def _ms_to_mph(ms: np.ndarray) -> np.ndarray:
    return ms * 2.237


def _safe_grid_dump(layer: str, ts: str, data: np.ndarray, lats: np.ndarray, lons: np.ndarray, unit: str) -> None:
    try:
        write_grid(layer, ts, data, lats, lons, unit=unit)
    except Exception as exc:  # noqa: BLE001
        log.warning("grid_dump_failed", extra={"layer": layer, "err": str(exc)})


def _write_palette_tiles(tile_base: Path, layer: str, palette: str, ts: str, rgba: np.ndarray, lats: np.ndarray, lons: np.ndarray) -> None:
    if lats[0] > lats[-1]:
        rgba = np.flipud(rgba)
        lats = lats[::-1]
    out_dir = str(tile_base / layer / palette / ts)
    render_tiles(rgba=rgba, lats=lats, lons=lons, output_dir=out_dir, zoom_levels=ZOOM_LEVELS)


def _render_per_palette(
    tile_base: Path, layer: str, ts: str, data: np.ndarray, lats: np.ndarray, lons: np.ndarray,
    palette_tables: dict[str, dict], color_key: str, *,
    categorical: bool = False, categories_map: dict[int, str] | None = None,
) -> bool:
    rendered_any = False
    for pname, tables in palette_tables.items():
        entry = tables.get(color_key)
        if not entry:
            continue
        if categorical:
            if categories_map is None:
                continue
            rgba = apply_categorical_color_table(data, entry["categories"], categories_map)
        else:
            rgba = apply_color_table(data, entry)
        _write_palette_tiles(tile_base, layer, pname, ts, rgba, lats, lons)
        rendered_any = True
    return rendered_any


def _process_forecast_hour_sync(grib_path: Path, run_id: str, fhr: int, palette_tables: dict[str, dict], tile_base: Path) -> list[str]:
    date_str, run_hour = run_id.split("_")
    run_dt = datetime.strptime(f"{date_str}{run_hour}", "%Y%m%d%H").replace(tzinfo=timezone.utc)
    ts = (run_dt + timedelta(hours=fhr)).isoformat()
    rendered: list[str] = []

    r = _extract_variable(grib_path, VAR_SELECTORS["refc"])
    if r:
        d, lats, lons = r
        if _render_per_palette(tile_base, "radar-hrrr", ts, d, lats, lons, palette_tables, "reflectivity"):
            rendered.append("radar-hrrr")
            _safe_grid_dump("radar-hrrr", ts, d, lats, lons, "dBZ")

    r = _extract_variable(grib_path, VAR_SELECTORS["t2m"])
    if r:
        d, lats, lons = r
        d = _kelvin_to_f(d)
        if _render_per_palette(tile_base, "temperature", ts, d, lats, lons, palette_tables, "temperature"):
            rendered.append("temperature")
            _safe_grid_dump("temperature", ts, d, lats, lons, "°F")

    if any("dewpoint" in t for t in palette_tables.values()):
        r = _extract_variable(grib_path, VAR_SELECTORS["dpt2m"])
        if r:
            d, lats, lons = r
            d = _kelvin_to_f(d)
            if _render_per_palette(tile_base, "dewpoint", ts, d, lats, lons, palette_tables, "dewpoint"):
                rendered.append("dewpoint")

    if any("humidity" in t for t in palette_tables.values()):
        r = _extract_variable(grib_path, VAR_SELECTORS["rh2m"])
        if r:
            d, lats, lons = r
            if _render_per_palette(tile_base, "humidity", ts, d, lats, lons, palette_tables, "humidity"):
                rendered.append("humidity")

    r = _extract_variable(grib_path, VAR_SELECTORS["cape"])
    if r:
        d, lats, lons = r
        if _render_per_palette(tile_base, "cape", ts, d, lats, lons, palette_tables, "cape"):
            rendered.append("cape")
            _safe_grid_dump("cape", ts, d, lats, lons, "J/kg")

    u = _extract_variable(grib_path, VAR_SELECTORS["u10"])
    v = _extract_variable(grib_path, VAR_SELECTORS["v10"])
    if u and v:
        u_data, lats, lons = u
        v_data = v[0]
        u_mph = _ms_to_mph(u_data)
        v_mph = _ms_to_mph(v_data)
        speed = np.sqrt(u_mph ** 2 + v_mph ** 2)
        if _render_per_palette(tile_base, "wind", ts, speed, lats, lons, palette_tables, "wind_speed"):
            rendered.append("wind")
            _safe_grid_dump("wind", ts, speed, lats, lons, "mph")
            _safe_grid_dump("wind_u", ts, u_mph, lats, lons, "mph")
            _safe_grid_dump("wind_v", ts, v_mph, lats, lons, "mph")

    r = _extract_variable(grib_path, VAR_SELECTORS["apcp"])
    if r:
        d, lats, lons = r
        d_in = d / 25.4
        if _render_per_palette(tile_base, "precip-accum", ts, d_in, lats, lons, palette_tables, "precip_accum"):
            rendered.append("precip-accum")
            _safe_grid_dump("precip-accum", ts, d_in, lats, lons, "in")

    r = _extract_variable(grib_path, VAR_SELECTORS["tcdc"])
    if r:
        d, lats, lons = r
        if _render_per_palette(tile_base, "cloud", ts, d, lats, lons, palette_tables, "cloud_cover"):
            rendered.append("cloud")
            _safe_grid_dump("cloud", ts, d, lats, lons, "%")

    precip: dict[str, np.ndarray] = {}
    plats = plons = None
    for k in ("crain", "csnow", "cfrzr", "cicep"):
        r = _extract_variable(grib_path, VAR_SELECTORS[k])
        if r:
            precip[k] = r[0]
            plats, plons = r[1], r[2]
    if precip and plats is not None and plons is not None:
        h, w = next(iter(precip.values())).shape
        cat = np.zeros((h, w), dtype=np.int32)
        if "crain" in precip: cat[precip["crain"] > 0] = 1
        if "csnow" in precip: cat[precip["csnow"] > 0] = 2
        if "cfrzr" in precip: cat[precip["cfrzr"] > 0] = 3
        if "cicep" in precip: cat[precip["cicep"] > 0] = 4
        ptype_map = {1: "rain", 2: "snow", 3: "freezing_rain", 4: "ice_pellets"}
        if _render_per_palette(tile_base, "precip-type", ts, cat, plats, plons, palette_tables, "precip_type",
                               categorical=True, categories_map=ptype_map):
            rendered.append("precip-type")

    return rendered


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


# ---------- activities ----------


@activity.defn(name="hrrr_find_latest_run")
async def hrrr_find_latest_run() -> FindRunResult:
    def _go() -> FindRunResult:
        with httpx.Client() as client:
            run_id = _find_latest_run_sync(client)
        if run_id is None:
            return FindRunResult(run_id=None, already_processed=False)
        state = ProcessedSet(_state_path(), max_entries=200)
        return FindRunResult(run_id=run_id, already_processed=run_id in state)

    return await asyncio.to_thread(_go)


@activity.defn(name="hrrr_horizon_for_run")
async def hrrr_horizon_for_run(run_id: str) -> int:
    """Return forecast horizon (1..N) for the given run."""
    _, run_hour = run_id.split("_")
    return EXTENDED_FORECAST_HOURS if int(run_hour) in EXTENDED_RUNS else FORECAST_HOURS


@activity.defn(name="hrrr_process_forecast_hour")
async def hrrr_process_forecast_hour(run_id: str, fhr: int) -> ForecastHourResult:
    """Download + extract + render every layer for one forecast hour."""
    started = time.time()
    palette_tables = _load_palette_tables()
    tile_base = Path(TILE_DIR)
    date_str, run_hour = run_id.split("_")
    tmp_dir = _current_activity_tmp_dir("hrrr", run_id, f"f{fhr:02d}")
    needed = list(IDX_MATCHERS.keys())

    activity.heartbeat({"phase": "download", "fhr": fhr})

    def _download() -> Path | None:
        with httpx.Client() as client:
            return _download_subset_sync(client, date_str, run_hour, fhr, needed, tmp_dir)

    grib_path = await asyncio.to_thread(_download)
    if grib_path is None or not grib_path.exists():
        shutil.rmtree(tmp_dir, ignore_errors=True)
        return ForecastHourResult(fhr=fhr, rendered_layers=[], duration_s=round(time.time() - started, 2))

    activity.heartbeat({"phase": "render", "fhr": fhr})
    try:
        rendered = await asyncio.to_thread(
            _process_forecast_hour_sync, grib_path, run_id, fhr, palette_tables, tile_base
        )
    except Exception:
        shutil.rmtree(tmp_dir, ignore_errors=True)
        raise
    else:
        grib_path.unlink(missing_ok=True)
        shutil.rmtree(tmp_dir, ignore_errors=True)

    duration = time.time() - started
    log.info("hour_done", extra={"run_id": run_id, "fhr": fhr, "rendered": rendered, "duration_s": round(duration, 1)})
    return ForecastHourResult(fhr=fhr, rendered_layers=rendered, duration_s=round(duration, 2))


@activity.defn(name="hrrr_mark_processed")
async def hrrr_mark_processed(run_id: str) -> None:
    def _go() -> None:
        state = ProcessedSet(_state_path(), max_entries=200)
        state.add(run_id)

    await asyncio.to_thread(_go)


@activity.defn(name="hrrr_cleanup")
async def hrrr_cleanup(retention_hours: int) -> HrrrCleanupResult:
    def _go() -> HrrrCleanupResult:
        tile_base = Path(TILE_DIR)
        cutoff = time.time() - (retention_hours * 3600)
        removed = 0
        for layer in HRRR_TILE_LAYERS:
            layer_dir = tile_base / layer
            if not layer_dir.exists():
                continue
            candidates: list[Path] = []
            for entry in sorted(layer_dir.iterdir()):
                if not entry.is_dir():
                    continue
                if entry.name[:1].isdigit():
                    candidates.append(entry)
                else:
                    candidates.extend(p for p in entry.iterdir() if p.is_dir())
            for ts_dir in candidates:
                try:
                    dt = datetime.fromisoformat(ts_dir.name)
                except ValueError:
                    continue
                if dt.timestamp() < cutoff:
                    shutil.rmtree(ts_dir, ignore_errors=True)
                    removed += 1
        grids_removed = cleanup_old_grids()
        return HrrrCleanupResult(tile_dirs_removed=removed, grid_files_removed=grids_removed)

    return await asyncio.to_thread(_go)
