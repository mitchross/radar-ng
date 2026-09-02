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

from backend.shared.activity_heartbeat import run_sync_with_heartbeat
from backend.shared.grid_dump import cleanup_old_grids, write_grid
from backend.shared.logger import get_logger
from backend.shared.manifest import (
    read_manifest_file,
    replace_layer_manifest,
    update_manifest_file,
)
from backend.shared.palettes import get_palette_names, load_palette
from backend.shared.state import ProcessedSet
from backend.shared.tiler import (
    MultiPaletteRenderResult,
    is_complete_pyramid,
    render_frame_palettes,
)


HRRR_BASE = "https://noaa-hrrr-bdp-pds.s3.amazonaws.com"
TILE_DIR = os.environ.get("TILE_DIR", "/data/tiles")
STATE_DIR = os.environ.get("STATE_DIR", "/data/state")
TMP_ROOT = Path(os.environ.get("HRRR_TMP_ROOT", "/tmp/hrrr_work"))
FORECAST_HOURS = int(os.environ.get("FORECAST_HOURS", "18"))
EXTENDED_FORECAST_HOURS = int(os.environ.get("EXTENDED_FORECAST_HOURS", "48"))
EXTENDED_RUNS = {0, 6, 12, 18}
# Keep polling a partially published run this long before a newer run may supersede it.
INCOMPLETE_RUN_MAX_AGE = timedelta(hours=3)
# HRRR is a 3 km model. z6 is its honest display ceiling; higher zooms were
# interpolated pixels and dominated the forecast fanout.
ZOOM_LEVELS = [4, 5, 6]
ENABLED_LAYERS = {
    value.strip()
    for value in os.environ.get("HRRR_ENABLED_LAYERS", "radar-hrrr").split(",")
    if value.strip()
}

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
    "refc": {"shortName": "refc", "typeOfLevel": "atmosphere"},
    "t2m": {
        "name": "2 metre temperature",
        "typeOfLevel": "heightAboveGround",
        "level": 2,
    },
    "dpt2m": {
        "name": "2 metre dewpoint temperature",
        "typeOfLevel": "heightAboveGround",
        "level": 2,
    },
    "cape": {"name": "Convective available potential energy", "typeOfLevel": "surface"},
    "u10": {
        "name": "10 metre U wind component",
        "typeOfLevel": "heightAboveGround",
        "level": 10,
    },
    "v10": {
        "name": "10 metre V wind component",
        "typeOfLevel": "heightAboveGround",
        "level": 10,
    },
    "crain": {"name": "Categorical rain", "typeOfLevel": "surface"},
    "csnow": {"name": "Categorical snow", "typeOfLevel": "surface"},
    "cfrzr": {"name": "Categorical freezing rain", "typeOfLevel": "surface"},
    "cicep": {"name": "Categorical ice pellets", "typeOfLevel": "surface"},
    "rh2m": {
        "name": "2 metre relative humidity",
        "typeOfLevel": "heightAboveGround",
        "level": 2,
    },
    "apcp": {"name": "Total Precipitation", "typeOfLevel": "surface"},
    "tcdc": {"name": "Total Cloud Cover", "typeOfLevel": "atmosphere"},
}

HRRR_TILE_LAYERS = [
    "radar-hrrr",
    "temperature",
    "dewpoint",
    "humidity",
    "wind",
    "cape",
    "precip-type",
    "precip-accum",
    "cloud",
]

LAYER_COLOR_KEYS = {
    "radar-hrrr": "reflectivity",
    "temperature": "temperature",
    "dewpoint": "dewpoint",
    "humidity": "humidity",
    "wind": "wind_speed",
    "cape": "cape",
    "precip-type": "precip_type",
    "precip-accum": "precip_accum",
    "cloud": "cloud_cover",
}


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
    valid_timestamp: str = ""


@dataclass
class HrrrCleanupResult:
    tile_dirs_removed: int
    grid_files_removed: int


@dataclass
class ExtractedGrid:
    data: np.ndarray
    lats: np.ndarray
    lons: np.ndarray
    source_crs: str | None = None
    source_x: np.ndarray | None = None
    source_y: np.ndarray | None = None


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


def _horizon_for_run(run_id: str) -> int:
    _, run_hour = run_id.split("_")
    return EXTENDED_FORECAST_HOURS if int(run_hour) in EXTENDED_RUNS else FORECAST_HOURS


def _run_init_time(run_id: str) -> datetime:
    return datetime.strptime(run_id, "%Y%m%d_%H").replace(tzinfo=timezone.utc)


def _resume_incomplete_run(manifest: dict, now: datetime) -> str | None:
    """Return the published-but-incomplete radar-hrrr run if it is still worth finishing."""
    layer = manifest.get("layers", {}).get("radar-hrrr") or {}
    run_id = layer.get("run_id")
    if not run_id or layer.get("complete", True):
        return None
    try:
        issued = _run_init_time(str(run_id))
    except ValueError:
        return None
    if now - issued > INCOMPLETE_RUN_MAX_AGE:
        return None
    return str(run_id)


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
        records.append(
            {
                "num": int(parts[0]),
                "offset": int(parts[1]),
                "line": raw,
                "var_name": parts[3],
                "level": parts[4],
                "fcst_time": parts[5] if len(parts) > 5 else "",
            }
        )
    return records


def _pick_ranges(
    records: list[dict], matchers: Iterable[tuple[str, ...]]
) -> list[tuple[int, int | None]]:
    match_list = [tuple(m) for m in matchers]
    ranges: list[tuple[int, int | None]] = []
    for i, rec in enumerate(records):
        if not any(all(term in rec["line"] for term in m) for m in match_list):
            continue
        start = rec["offset"]
        end = records[i + 1]["offset"] - 1 if i + 1 < len(records) else None
        ranges.append((start, end))
    return ranges


def _normalize_lons(lons: np.ndarray) -> np.ndarray:
    lons_arr = np.asarray(lons, dtype=np.float64)
    return np.where(lons_arr > 180.0, lons_arr - 360.0, lons_arr)


def _extract_native_projection(
    grb: object,
    lats: np.ndarray,
    lons: np.ndarray,
) -> tuple[str, np.ndarray, np.ndarray] | None:
    """Return native projected CRS + x/y axes for curvilinear GRIB grids."""
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
        # No .idx yet (or transport error): fall back to the whole file. A 404 there
        # means NOAA hasn't uploaded the hour — pending, not an error, so no retry.
        resp = client.get(base_url, timeout=300)
        if resp.status_code == 404:
            log.info(
                "hour_pending", extra={"run_id": f"{date_str}_{run_hour}", "fhr": fhr}
            )
            return None
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


def _extract_variable(grib_path: Path, match: dict) -> ExtractedGrid | None:
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
                lons = _normalize_lons(lons)
                if hasattr(data, "filled"):
                    data = data.filled(np.nan)
                native = _extract_native_projection(grb, lats, lons)
                grbs.close()
                if native is not None:
                    source_crs, source_x, source_y = native
                    return ExtractedGrid(
                        data=data.astype(np.float32),
                        lats=lats.astype(np.float64),
                        lons=lons.astype(np.float64),
                        source_crs=source_crs,
                        source_x=source_x,
                        source_y=source_y,
                    )
                return ExtractedGrid(
                    data=data.astype(np.float32),
                    lats=lats[:, 0].astype(np.float64),
                    lons=lons[0, :].astype(np.float64),
                )
        grbs.close()
    except Exception as exc:  # noqa: BLE001
        log.warning("extract_failed", extra={"err": str(exc), "match": match})
    return None


def _kelvin_to_f(k: np.ndarray) -> np.ndarray:
    return (k - 273.15) * 9 / 5 + 32


def _ms_to_mph(ms: np.ndarray) -> np.ndarray:
    return ms * 2.237


def _grid_dump_axes(grid: ExtractedGrid) -> tuple[np.ndarray, np.ndarray]:
    if grid.lats.ndim == 1 and grid.lons.ndim == 1:
        return grid.lats, grid.lons
    row = grid.lats.shape[0] // 2
    col = grid.lons.shape[1] // 2
    return grid.lats[:, col].astype(np.float64), grid.lons[row, :].astype(np.float64)


def _safe_grid_dump(
    layer: str, ts: str, data: np.ndarray, grid: ExtractedGrid, unit: str
) -> None:
    try:
        lats, lons = _grid_dump_axes(grid)
        write_grid(layer, ts, data, lats, lons, unit=unit)
    except Exception as exc:  # noqa: BLE001
        log.warning("grid_dump_failed", extra={"layer": layer, "err": str(exc)})


def _write_palette_tiles(
    tile_base: Path,
    layer: str,
    path: str,
    data: np.ndarray,
    grid: ExtractedGrid,
    color_tables: dict[str, dict],
    category_map: dict[int, str] | None = None,
) -> MultiPaletteRenderResult:
    """Orient once, then sample/classify once per tile for every palette."""
    lats = grid.lats
    lons = grid.lons
    source_y = grid.source_y
    if grid.source_crs is None and lats.ndim == 1 and lats[0] > lats[-1]:
        data = np.flipud(data)
        lats = lats[::-1]
    elif (
        grid.source_crs is not None
        and source_y is not None
        and source_y[0] > source_y[-1]
    ):
        data = np.flipud(data)
        lats = np.flipud(lats)
        lons = np.flipud(lons)
        source_y = source_y[::-1]
    out_dirs = {pname: str(tile_base / layer / pname / path) for pname in color_tables}
    return render_frame_palettes(
        data,
        lats,
        lons,
        color_tables,
        out_dirs,
        ZOOM_LEVELS,
        category_map=category_map,
        source_crs=grid.source_crs,
        source_x=grid.source_x,
        source_y=source_y,
        nodata_value=None,
        min_valid_weight=1.0,
        renderer=os.environ.get("TILE_RENDERER", "legacy"),
    )


def _render_per_palette(
    tile_base: Path,
    layer: str,
    ts: str,
    data: np.ndarray,
    grid: ExtractedGrid,
    palette_tables: dict[str, dict],
    color_key: str,
    *,
    categorical: bool = False,
    categories_map: dict[int, str] | None = None,
) -> list[str]:
    color_tables = {
        name: tables[color_key]
        for name, tables in palette_tables.items()
        if tables.get(color_key)
    }
    if not color_tables or (categorical and categories_map is None):
        return []
    result = _write_palette_tiles(
        tile_base,
        layer,
        ts,
        data,
        grid,
        color_tables,
        category_map=categories_map if categorical else None,
    )
    rendered = result.rendered_palettes
    if set(rendered) != set(color_tables):
        # Empty products are not advertised. A mixed publish cannot be
        # returned: the shared publisher raises so Temporal can retry and
        # converge without deleting immutable winners.
        return []
    return rendered


def _process_forecast_hour_sync(
    grib_path: Path,
    run_id: str,
    fhr: int,
    palette_tables: dict[str, dict],
    tile_base: Path,
) -> list[str]:
    ts = (_run_init_time(run_id) + timedelta(hours=fhr)).isoformat()
    tile_path = f"runs/{run_id}/{ts}"
    rendered: list[str] = []

    r = _extract_variable(grib_path, VAR_SELECTORS["refc"])
    if r is None:
        raise RuntimeError(
            "required HRRR variable refc is missing from downloaded GRIB"
        )
    d = r.data
    palettes = _render_per_palette(
        tile_base, "radar-hrrr", tile_path, d, r, palette_tables, "reflectivity"
    )
    if palettes:
        rendered.append("radar-hrrr")
        _safe_grid_dump("radar-hrrr", ts, d, r, "dBZ")

    # Radar is the latency-critical product. Production defaults to this fast
    # path; secondary layers can be re-enabled explicitly once their separate
    # capacity budget exists.
    if ENABLED_LAYERS == {"radar-hrrr"}:
        return rendered

    r = _extract_variable(grib_path, VAR_SELECTORS["t2m"])
    if r:
        d = r.data
        d = _kelvin_to_f(d)
        palettes = _render_per_palette(
            tile_base, "temperature", tile_path, d, r, palette_tables, "temperature"
        )
        if palettes:
            rendered.append("temperature")
            _safe_grid_dump("temperature", ts, d, r, "°F")

    if any("dewpoint" in t for t in palette_tables.values()):
        r = _extract_variable(grib_path, VAR_SELECTORS["dpt2m"])
        if r:
            d = r.data
            d = _kelvin_to_f(d)
            palettes = _render_per_palette(
                tile_base, "dewpoint", tile_path, d, r, palette_tables, "dewpoint"
            )
            if palettes:
                rendered.append("dewpoint")

    if any("humidity" in t for t in palette_tables.values()):
        r = _extract_variable(grib_path, VAR_SELECTORS["rh2m"])
        if r:
            d = r.data
            palettes = _render_per_palette(
                tile_base, "humidity", tile_path, d, r, palette_tables, "humidity"
            )
            if palettes:
                rendered.append("humidity")

    r = _extract_variable(grib_path, VAR_SELECTORS["cape"])
    if r:
        d = r.data
        palettes = _render_per_palette(
            tile_base, "cape", tile_path, d, r, palette_tables, "cape"
        )
        if palettes:
            rendered.append("cape")
            _safe_grid_dump("cape", ts, d, r, "J/kg")

    u = _extract_variable(grib_path, VAR_SELECTORS["u10"])
    v = _extract_variable(grib_path, VAR_SELECTORS["v10"])
    if u and v:
        u_data = u.data
        v_data = v.data
        u_mph = _ms_to_mph(u_data)
        v_mph = _ms_to_mph(v_data)
        speed = np.sqrt(u_mph**2 + v_mph**2)
        palettes = _render_per_palette(
            tile_base, "wind", tile_path, speed, u, palette_tables, "wind_speed"
        )
        if palettes:
            rendered.append("wind")
            _safe_grid_dump("wind", ts, speed, u, "mph")
            _safe_grid_dump("wind_u", ts, u_mph, u, "mph")
            _safe_grid_dump("wind_v", ts, v_mph, u, "mph")

    r = _extract_variable(grib_path, VAR_SELECTORS["apcp"])
    if r:
        d = r.data
        d_in = d / 25.4
        palettes = _render_per_palette(
            tile_base,
            "precip-accum",
            tile_path,
            d_in,
            r,
            palette_tables,
            "precip_accum",
        )
        if palettes:
            rendered.append("precip-accum")
            _safe_grid_dump("precip-accum", ts, d_in, r, "in")

    r = _extract_variable(grib_path, VAR_SELECTORS["tcdc"])
    if r:
        d = r.data
        palettes = _render_per_palette(
            tile_base, "cloud", tile_path, d, r, palette_tables, "cloud_cover"
        )
        if palettes:
            rendered.append("cloud")
            _safe_grid_dump("cloud", ts, d, r, "%")

    precip: dict[str, np.ndarray] = {}
    pgrid: ExtractedGrid | None = None
    for k in ("crain", "csnow", "cfrzr", "cicep"):
        r = _extract_variable(grib_path, VAR_SELECTORS[k])
        if r:
            precip[k] = r.data
            pgrid = r
    if precip and pgrid is not None:
        h, w = next(iter(precip.values())).shape
        cat = np.zeros((h, w), dtype=np.int32)
        if "crain" in precip:
            cat[precip["crain"] > 0] = 1
        if "csnow" in precip:
            cat[precip["csnow"] > 0] = 2
        if "cfrzr" in precip:
            cat[precip["cfrzr"] > 0] = 3
        if "cicep" in precip:
            cat[precip["cicep"] > 0] = 4
        ptype_map = {1: "rain", 2: "snow", 3: "freezing_rain", 4: "ice_pellets"}
        palettes = _render_per_palette(
            tile_base,
            "precip-type",
            tile_path,
            cat,
            pgrid,
            palette_tables,
            "precip_type",
            categorical=True,
            categories_map=ptype_map,
        )
        if palettes:
            rendered.append("precip-type")

    return rendered


def _required_layers(palette_tables: dict[str, dict]) -> set[str]:
    """Layers an hour must have on disk before it counts as already rendered."""
    if ENABLED_LAYERS == {"radar-hrrr"}:
        return {"radar-hrrr"}
    return {
        layer
        for layer, color_key in LAYER_COLOR_KEYS.items()
        if any(color_key in tables for tables in palette_tables.values())
    }


def _existing_rendered_layers(
    tile_base: Path, tile_path: str, palette_tables: dict[str, dict]
) -> list[str]:
    """Layers with a marker-validated pyramid for every defined palette."""
    existing: list[str] = []
    for layer, color_key in LAYER_COLOR_KEYS.items():
        palettes = [
            name for name, tables in palette_tables.items() if color_key in tables
        ]
        if not palettes:
            continue
        dirs = [tile_base / layer / pname / tile_path for pname in palettes]
        if all(is_complete_pyramid(d) for d in dirs):
            existing.append(layer)
    return existing


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
        # A newer run's f01 must not abandon a run we have half-published; finish it first.
        resume = _resume_incomplete_run(
            read_manifest_file(STATE_DIR), datetime.now(timezone.utc)
        )
        if resume is not None and resume != run_id:
            log.info(
                "resuming_incomplete_run", extra={"run_id": resume, "latest": run_id}
            )
            run_id = resume
        state = ProcessedSet(_state_path(), max_entries=200)
        return FindRunResult(run_id=run_id, already_processed=run_id in state)

    return await asyncio.to_thread(_go)


@activity.defn(name="hrrr_horizon_for_run")
async def hrrr_horizon_for_run(run_id: str) -> int:
    """Return forecast horizon (1..N) for the given run."""
    return _horizon_for_run(run_id)


@activity.defn(name="hrrr_process_forecast_hour")
async def hrrr_process_forecast_hour(run_id: str, fhr: int) -> ForecastHourResult:
    """Download + extract + render every layer for one forecast hour."""
    started = time.time()
    palette_tables = _load_palette_tables()
    tile_base = Path(TILE_DIR)
    date_str, run_hour = run_id.split("_")
    needed = ["refc"] if ENABLED_LAYERS == {"radar-hrrr"} else list(IDX_MATCHERS.keys())
    valid_timestamp = (_run_init_time(run_id) + timedelta(hours=fhr)).isoformat()

    # Idempotent resume: marker-validated run paths are immutable and complete.
    existing = await asyncio.to_thread(
        _existing_rendered_layers,
        tile_base,
        f"runs/{run_id}/{valid_timestamp}",
        palette_tables,
    )
    if _required_layers(palette_tables) <= set(existing):
        log.info(
            "hour_resumed", extra={"run_id": run_id, "fhr": fhr, "rendered": existing}
        )
        return ForecastHourResult(
            fhr=fhr,
            rendered_layers=existing,
            duration_s=round(time.time() - started, 2),
            valid_timestamp=valid_timestamp,
        )

    tmp_dir = _current_activity_tmp_dir("hrrr", run_id, f"f{fhr:02d}")
    activity.heartbeat({"phase": "download", "fhr": fhr})

    def _download() -> Path | None:
        with httpx.Client() as client:
            return _download_subset_sync(
                client, date_str, run_hour, fhr, needed, tmp_dir
            )

    # Heartbeat during the download too: the full-file fallback (~130 MB) outlives heartbeat_timeout.
    grib_path = await run_sync_with_heartbeat(
        _download,
        heartbeat_every=30,
        heartbeat_details=lambda: {"phase": "download", "run_id": run_id, "fhr": fhr},
    )
    if grib_path is None or not grib_path.exists():
        shutil.rmtree(tmp_dir, ignore_errors=True)
        return ForecastHourResult(
            fhr=fhr,
            rendered_layers=[],
            duration_s=round(time.time() - started, 2),
            valid_timestamp=valid_timestamp,
        )

    activity.heartbeat({"phase": "render", "fhr": fhr})
    try:
        rendered = await run_sync_with_heartbeat(
            _process_forecast_hour_sync,
            grib_path,
            run_id,
            fhr,
            palette_tables,
            tile_base,
            heartbeat_every=30,
            heartbeat_details=lambda: {"phase": "render", "run_id": run_id, "fhr": fhr},
        )
    except Exception:
        shutil.rmtree(tmp_dir, ignore_errors=True)
        raise
    else:
        grib_path.unlink(missing_ok=True)
        shutil.rmtree(tmp_dir, ignore_errors=True)

    duration = time.time() - started
    log.info(
        "hour_done",
        extra={
            "run_id": run_id,
            "fhr": fhr,
            "rendered": rendered,
            "duration_s": round(duration, 1),
        },
    )
    return ForecastHourResult(
        fhr=fhr,
        rendered_layers=rendered,
        duration_s=round(duration, 2),
        valid_timestamp=valid_timestamp,
    )


def publishable_prefix(results: list[ForecastHourResult]) -> list[ForecastHourResult]:
    """Longest run of hours f01, f02, ... that all have reflectivity tiles on disk."""
    by_fhr = {result.fhr: result for result in results}
    prefix: list[ForecastHourResult] = []
    fhr = 1
    while fhr in by_fhr and "radar-hrrr" in by_fhr[fhr].rendered_layers:
        prefix.append(by_fhr[fhr])
        fhr += 1
    return prefix


def _publish_hrrr_run_sync(
    run_id: str,
    results: list[ForecastHourResult],
    palette_tables: dict[str, dict],
    *,
    state_dir: str | Path | None = None,
    horizon: int | None = None,
) -> list[str]:
    """Publish the consecutive f01.. prefix of a run; `complete` flips once it spans the horizon.

    Frames are advertised only for hours whose every palette rendered, so a partial
    manifest is still never a mixed or half-written one.
    """
    ordered = publishable_prefix(results)
    if not ordered:
        return []
    horizon = horizon if horizon is not None else _horizon_for_run(run_id)
    complete = len(ordered) >= horizon

    complete_layers = set(ordered[0].rendered_layers)
    for result in ordered[1:]:
        complete_layers.intersection_update(result.rendered_layers)

    published: list[str] = []
    run_issued_at = _run_init_time(run_id).isoformat()
    for layer in sorted(complete_layers):
        color_key = LAYER_COLOR_KEYS.get(layer)
        if color_key is None:
            continue
        palettes = sorted(
            name for name, tables in palette_tables.items() if color_key in tables
        )
        if not palettes:
            continue
        frames = [
            {
                "timestamp": result.valid_timestamp,
                "path": f"runs/{run_id}/{result.valid_timestamp}",
                "source": "hrrr",
                "kind": "model_guidance",
                "issued_at": run_issued_at,
                "run_id": run_id,
                "lead_minutes": result.fhr * 60,
                "spatial_resolution_km": 3.0,
                "max_zoom": max(ZOOM_LEVELS),
            }
            for result in ordered
        ]
        replace_layer_manifest(
            layer,
            [frame["timestamp"] for frame in frames],
            palettes=palettes,
            state_dir=state_dir or STATE_DIR,
            frames=frames,
            layer_metadata={
                "title": "HRRR simulated reflectivity"
                if layer == "radar-hrrr"
                else f"HRRR {layer}",
                "kind": "model_guidance",
                "run_id": run_id,
                "complete": complete,
            },
        )
        published.append(layer)
    return published


@activity.defn(name="hrrr_publish_run")
async def hrrr_publish_run(run_id: str, results: list[ForecastHourResult]) -> list[str]:
    """Publish whatever consecutive prefix of the run exists; the workflow marks it processed only when complete."""
    palette_tables = await asyncio.to_thread(_load_palette_tables)
    return await asyncio.to_thread(
        _publish_hrrr_run_sync, run_id, results, palette_tables
    )


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
        manifest = read_manifest_file(STATE_DIR)
        cutoff = time.time() - (retention_hours * 3600)
        removed = 0
        for layer in HRRR_TILE_LAYERS:
            current_run = manifest.get("layers", {}).get(layer, {}).get("run_id")
            layer_dir = tile_base / layer
            if not layer_dir.exists():
                continue
            legacy_candidates: list[Path] = []
            for palette_dir in sorted(layer_dir.iterdir()):
                if not palette_dir.is_dir():
                    continue
                if palette_dir.name[:1].isdigit():
                    legacy_candidates.append(palette_dir)
                    continue
                runs_root = palette_dir / "runs"
                if runs_root.is_dir():
                    for run_dir in runs_root.iterdir():
                        if not run_dir.is_dir():
                            continue
                        if run_dir.name == current_run:
                            continue
                        try:
                            run_dt = datetime.strptime(
                                run_dir.name, "%Y%m%d_%H"
                            ).replace(tzinfo=timezone.utc)
                        except ValueError:
                            continue
                        if run_dt.timestamp() < cutoff:
                            shutil.rmtree(run_dir, ignore_errors=True)
                            removed += 1
                legacy_candidates.extend(
                    path
                    for path in palette_dir.iterdir()
                    if path.is_dir() and path.name != "runs"
                )
            for ts_dir in legacy_candidates:
                try:
                    dt = datetime.fromisoformat(ts_dir.name)
                except ValueError:
                    continue
                if dt.timestamp() < cutoff:
                    # Manifest first, then tiles — never advertise deleted tiles.
                    update_manifest_file(layer, ts_dir.name, action="remove")
                    shutil.rmtree(ts_dir, ignore_errors=True)
                    removed += 1
        grids_removed = cleanup_old_grids()
        return HrrrCleanupResult(
            tile_dirs_removed=removed, grid_files_removed=grids_removed
        )

    return await asyncio.to_thread(_go)
