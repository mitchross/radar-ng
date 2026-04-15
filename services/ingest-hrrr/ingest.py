#!/usr/bin/env python3
"""HRRR forecast ingest: download GRIB2 from S3, extract variables, render tiles per layer."""

import gzip
import json
import os
import re
import struct
import sys
import time
from datetime import datetime, timedelta, timezone
from pathlib import Path

import httpx
import numpy as np
import pygrib

sys.path.insert(0, str(Path(__file__).resolve().parent.parent / "shared"))
from tiler import apply_color_table, apply_categorical_color_table, render_tiles

HRRR_BASE = "https://noaa-hrrr-bdp-pds.s3.amazonaws.com"
TILE_DIR = os.environ.get("TILE_DIR", "/data/tiles")
COLOR_TABLE_PATH = os.environ.get(
    "COLOR_TABLE_PATH",
    str(Path(__file__).resolve().parent.parent / "shared" / "color_tables.json"),
)
POLL_INTERVAL = int(os.environ.get("POLL_INTERVAL", "3600"))  # 1 hour
ZOOM_LEVELS = [4, 5, 6, 7, 8, 9]
FORECAST_HOURS = 24
RETENTION_HOURS = 8

# Variables to extract from each forecast hour
VARIABLES = {
    "radar-hrrr": {"name": "Composite reflectivity", "typeOfLevel": "atmosphere", "shortName": "refc"},
    "temperature": {"name": "2 metre temperature", "typeOfLevel": "heightAboveGround", "level": 2},
    "cape": {"name": "Convective available potential energy", "typeOfLevel": "surface"},
}

WIND_U = {"name": "10 metre U wind component", "typeOfLevel": "heightAboveGround", "level": 10}
WIND_V = {"name": "10 metre V wind component", "typeOfLevel": "heightAboveGround", "level": 10}

# HRRR precip type flags
PRECIP_TYPES = {
    "crain": {"name": "Categorical rain", "typeOfLevel": "surface"},
    "csnow": {"name": "Categorical snow", "typeOfLevel": "surface"},
    "cfrzr": {"name": "Categorical freezing rain", "typeOfLevel": "surface"},
    "cicep": {"name": "Categorical ice pellets", "typeOfLevel": "surface"},
}

processed_runs: set[str] = set()


def load_color_tables() -> dict:
    with open(COLOR_TABLE_PATH) as f:
        return json.load(f)


def find_latest_hrrr_run(client: httpx.Client) -> str | None:
    """Find the latest available HRRR run by checking S3."""
    now = datetime.now(timezone.utc)
    # Check last 12 hours of runs
    for hours_ago in range(0, 12):
        dt = now - timedelta(hours=hours_ago)
        run_hour = (dt.hour // 1) * 1  # HRRR runs every hour
        run_dt = dt.replace(hour=run_hour, minute=0, second=0, microsecond=0)
        date_str = run_dt.strftime("%Y%m%d")
        run_str = f"{run_dt.hour:02d}"

        # Check if forecast hour 01 exists (indicates run is available)
        key = f"hrrr.{date_str}/conus/hrrr.t{run_str}z.wrfsfcf01.grib2"
        url = f"{HRRR_BASE}/{key}"
        try:
            resp = client.head(url, timeout=10)
            if resp.status_code == 200:
                return f"{date_str}_{run_str}"
        except httpx.HTTPError:
            continue
    return None


def download_forecast_hour(
    client: httpx.Client, date_str: str, run_hour: str, fhr: int, tmp_dir: Path
) -> Path | None:
    """Download a single HRRR forecast hour GRIB2 file."""
    key = f"hrrr.{date_str}/conus/hrrr.t{run_hour}z.wrfsfcf{fhr:02d}.grib2"
    url = f"{HRRR_BASE}/{key}"

    try:
        resp = client.get(url, timeout=120)
        resp.raise_for_status()
        out_path = tmp_dir / f"hrrr_f{fhr:02d}.grib2"
        out_path.write_bytes(resp.content)
        return out_path
    except httpx.HTTPError as e:
        print(f"  Failed to download f{fhr:02d}: {e}")
        return None


def extract_variable(grib_path: Path, match: dict) -> tuple[np.ndarray, np.ndarray, np.ndarray] | None:
    """Extract a specific variable from a GRIB2 file."""
    try:
        grbs = pygrib.open(str(grib_path))
        for grb in grbs:
            if all(
                getattr(grb, k, None) == v or (k == "name" and v.lower() in grb.name.lower())
                for k, v in match.items()
            ):
                data = grb.values
                lats, lons = grb.latlons()
                if hasattr(data, "filled"):
                    data = data.filled(np.nan)
                grbs.close()
                return data.astype(np.float32), lats[:, 0].astype(np.float64), lons[0, :].astype(np.float64)
        grbs.close()
    except Exception as e:
        print(f"  Error extracting from {grib_path}: {e}")
    return None


def kelvin_to_fahrenheit(k: np.ndarray) -> np.ndarray:
    return (k - 273.15) * 9 / 5 + 32


def ms_to_mph(ms: np.ndarray) -> np.ndarray:
    return ms * 2.237


def process_forecast_hour(
    grib_path: Path, run_id: str, fhr: int, color_tables: dict, tile_base: Path
):
    """Extract all variables from one forecast hour and render tiles."""
    # Calculate valid time
    date_str, run_hour = run_id.split("_")
    run_dt = datetime.strptime(f"{date_str}{run_hour}", "%Y%m%d%H").replace(tzinfo=timezone.utc)
    valid_dt = run_dt + timedelta(hours=fhr)
    timestamp = valid_dt.isoformat()

    # Reflectivity
    result = extract_variable(grib_path, VARIABLES["radar-hrrr"])
    if result:
        data, lats, lons = result
        rgba = apply_color_table(data, color_tables["reflectivity"])
        if lats[0] > lats[-1]:
            rgba = np.flipud(rgba)
            lats = lats[::-1]
        out_dir = str(tile_base / "radar-hrrr" / timestamp)
        count = render_tiles(rgba=rgba, lats=lats, lons=lons, output_dir=out_dir, zoom_levels=ZOOM_LEVELS)
        print(f"    radar-hrrr f{fhr:02d}: {count} tiles")

    # Temperature
    result = extract_variable(grib_path, VARIABLES["temperature"])
    if result:
        data, lats, lons = result
        data = kelvin_to_fahrenheit(data)
        rgba = apply_color_table(data, color_tables["temperature"])
        if lats[0] > lats[-1]:
            rgba = np.flipud(rgba)
            lats = lats[::-1]
        out_dir = str(tile_base / "temperature" / timestamp)
        count = render_tiles(rgba=rgba, lats=lats, lons=lons, output_dir=out_dir, zoom_levels=ZOOM_LEVELS)
        print(f"    temperature f{fhr:02d}: {count} tiles")

    # CAPE
    result = extract_variable(grib_path, VARIABLES["cape"])
    if result:
        data, lats, lons = result
        rgba = apply_color_table(data, color_tables["cape"])
        if lats[0] > lats[-1]:
            rgba = np.flipud(rgba)
            lats = lats[::-1]
        out_dir = str(tile_base / "cape" / timestamp)
        count = render_tiles(rgba=rgba, lats=lats, lons=lons, output_dir=out_dir, zoom_levels=ZOOM_LEVELS)
        print(f"    cape f{fhr:02d}: {count} tiles")

    # Wind speed (compute from U + V)
    u_result = extract_variable(grib_path, WIND_U)
    v_result = extract_variable(grib_path, WIND_V)
    if u_result and v_result:
        u_data, lats, lons = u_result
        v_data = v_result[0]
        speed = ms_to_mph(np.sqrt(u_data**2 + v_data**2))
        rgba = apply_color_table(speed, color_tables["wind_speed"])
        if lats[0] > lats[-1]:
            rgba = np.flipud(rgba)
            lats = lats[::-1]
        out_dir = str(tile_base / "wind" / timestamp)
        count = render_tiles(rgba=rgba, lats=lats, lons=lons, output_dir=out_dir, zoom_levels=ZOOM_LEVELS)
        print(f"    wind f{fhr:02d}: {count} tiles")

    # Precip type
    precip_results = {}
    for ptype, match in PRECIP_TYPES.items():
        r = extract_variable(grib_path, match)
        if r:
            precip_results[ptype] = r[0]
            if "lats" not in dir():
                lats, lons = r[1], r[2]

    if precip_results:
        # Combine into a single category array
        # Priority: hail/ice > freezing rain > snow > rain
        h, w = list(precip_results.values())[0].shape
        category = np.zeros((h, w), dtype=np.int32)
        ptype_map = {1: "rain", 2: "snow", 3: "freezing_rain", 4: "ice_pellets"}
        if "crain" in precip_results:
            category[precip_results["crain"] > 0] = 1
        if "csnow" in precip_results:
            category[precip_results["csnow"] > 0] = 2
        if "cfrzr" in precip_results:
            category[precip_results["cfrzr"] > 0] = 3
        if "cicep" in precip_results:
            category[precip_results["cicep"] > 0] = 4

        rgba = apply_categorical_color_table(
            category, color_tables["precip_type"]["categories"], ptype_map
        )
        if lats[0] > lats[-1]:
            rgba = np.flipud(rgba)
            lats = lats[::-1]
        out_dir = str(tile_base / "precip-type" / timestamp)
        count = render_tiles(rgba=rgba, lats=lats, lons=lons, output_dir=out_dir, zoom_levels=ZOOM_LEVELS)
        print(f"    precip-type f{fhr:02d}: {count} tiles")


def cleanup_old_runs(tile_base: Path, layers: list[str], retention_hours: int):
    """Delete tile directories older than retention window."""
    import shutil
    cutoff = time.time() - (retention_hours * 3600)
    for layer in layers:
        layer_dir = tile_base / layer
        if not layer_dir.exists():
            continue
        for ts_dir in sorted(layer_dir.iterdir()):
            if ts_dir.is_dir():
                try:
                    dt = datetime.fromisoformat(ts_dir.name)
                    if dt.timestamp() < cutoff:
                        shutil.rmtree(ts_dir)
                except ValueError:
                    pass


def run():
    color_tables = load_color_tables()
    client = httpx.Client()
    tile_base = Path(TILE_DIR)
    tmp_dir = Path("/tmp/hrrr_work")
    tmp_dir.mkdir(parents=True, exist_ok=True)

    print(f"HRRR ingest starting. Tiles → {tile_base}, poll every {POLL_INTERVAL}s")

    while True:
        try:
            run_id = find_latest_hrrr_run(client)
            if run_id and run_id not in processed_runs:
                date_str, run_hour = run_id.split("_")
                print(f"Processing HRRR run: {date_str} {run_hour}z")

                for fhr in range(1, FORECAST_HOURS + 1):
                    grib_path = download_forecast_hour(client, date_str, run_hour, fhr, tmp_dir)
                    if grib_path:
                        process_forecast_hour(grib_path, run_id, fhr, color_tables, tile_base)
                        grib_path.unlink(missing_ok=True)

                processed_runs.add(run_id)
                print(f"Completed HRRR run {run_id}")

            cleanup_old_runs(
                tile_base,
                ["radar-hrrr", "temperature", "wind", "cape", "precip-type"],
                RETENTION_HOURS,
            )

        except Exception as e:
            print(f"Error in HRRR ingest loop: {e}")

        time.sleep(POLL_INTERVAL)


if __name__ == "__main__":
    run()
