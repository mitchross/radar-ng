#!/usr/bin/env python3
"""HRRR forecast ingest: download GRIB2 from S3, extract variables, render tiles per layer.

Hardened for Phase 1.4:
- Structured JSON logs + exponential-backoff retries on every S3 call
- Byte-range subsetting via .idx sidecar files so we only download the records
  we actually render (5-10x reduction vs the full ~200MB wrfsfc file)
- Processed-runs state persisted to /data/state/ingest-hrrr.json
- Extended runs (00/06/12/18z) pull f01-EXTENDED_FORECAST_HOURS; other runs
  pull f01-FORECAST_HOURS
"""

from __future__ import annotations

import os
import shutil
import sys
import time
from datetime import datetime, timedelta, timezone
from pathlib import Path
from typing import Iterable

import httpx
import numpy as np
import pygrib

sys.path.insert(0, str(Path(__file__).resolve().parent / "shared"))
sys.path.insert(0, "/app/shared")
from logger import get_logger, retry  # type: ignore  # noqa: E402
from state import ProcessedSet  # type: ignore  # noqa: E402
from tiler import apply_categorical_color_table, apply_color_table, render_tiles  # type: ignore  # noqa: E402

HRRR_BASE = "https://noaa-hrrr-bdp-pds.s3.amazonaws.com"
TILE_DIR = os.environ.get("TILE_DIR", "/data/tiles")
STATE_DIR = os.environ.get("STATE_DIR", "/data/state")
COLOR_TABLE_PATH = os.environ.get(
    "COLOR_TABLE_PATH",
    str(Path(__file__).resolve().parent.parent / "shared" / "color_tables.json"),
)
POLL_INTERVAL = int(os.environ.get("POLL_INTERVAL", "3600"))
FORECAST_HOURS = int(os.environ.get("FORECAST_HOURS", "18"))
EXTENDED_FORECAST_HOURS = int(os.environ.get("EXTENDED_FORECAST_HOURS", "48"))
EXTENDED_RUNS = {0, 6, 12, 18}
ZOOM_LEVELS = [4, 5, 6, 7, 8, 9]
RETENTION_HOURS = 12

log = get_logger("ingest-hrrr")

# idx "search strings" — matched against the record's (var_name, level, fcst_time)
# columns in the .idx sidecar. We download only records whose idx line contains
# ALL substrings in the tuple.
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
}

# Variables pygrib will actually look up after we've subsetted the file.
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
}


def load_color_tables() -> dict:
    import json
    with open(COLOR_TABLE_PATH) as f:
        return json.load(f)


@retry(attempts=4, base_delay=2.0, log=log, exceptions=(httpx.HTTPError,))
def _head(client: httpx.Client, url: str) -> int:
    resp = client.head(url, timeout=10)
    return resp.status_code


def find_latest_hrrr_run(client: httpx.Client) -> str | None:
    """Newest run whose f01 file is uploaded."""
    now = datetime.now(timezone.utc)
    for hours_ago in range(0, 12):
        dt = now - timedelta(hours=hours_ago)
        run_dt = dt.replace(minute=0, second=0, microsecond=0)
        date_str = run_dt.strftime("%Y%m%d")
        run_str = f"{run_dt.hour:02d}"
        key = f"hrrr.{date_str}/conus/hrrr.t{run_str}z.wrfsfcf01.grib2"
        try:
            if _head(client, f"{HRRR_BASE}/{key}") == 200:
                return f"{date_str}_{run_str}"
        except httpx.HTTPError:
            continue
    return None


@retry(attempts=4, base_delay=1.0, log=log, exceptions=(httpx.HTTPError,))
def _get_idx(client: httpx.Client, idx_url: str) -> list[dict]:
    """Fetch and parse an HRRR .idx sidecar.

    Each line looks like:
      1:0:d=2026041612:REFC:entire atmosphere:1 hour fcst:
    """
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


def _pick_ranges(records: list[dict], matchers: Iterable[tuple[str, ...]]) -> list[tuple[int, int | None]]:
    """Return (start, end-inclusive-or-None) byte ranges for every matching record."""
    match_list = [tuple(m) for m in matchers]
    ranges: list[tuple[int, int | None]] = []
    for i, rec in enumerate(records):
        if not any(all(term in rec["line"] for term in m) for m in match_list):
            continue
        start = rec["offset"]
        end = records[i + 1]["offset"] - 1 if i + 1 < len(records) else None
        ranges.append((start, end))
    return ranges


@retry(attempts=4, base_delay=1.5, log=log, exceptions=(httpx.HTTPError,))
def _range_get(client: httpx.Client, url: str, start: int, end: int | None) -> bytes:
    header = f"bytes={start}-{end if end is not None else ''}"
    resp = client.get(url, headers={"Range": header}, timeout=120)
    # S3 returns 206 on success; 200 means it ignored the range (large file, full body)
    resp.raise_for_status()
    return resp.content


def download_forecast_hour_subset(
    client: httpx.Client,
    date_str: str,
    run_hour: str,
    fhr: int,
    needed: Iterable[str],
    tmp_dir: Path,
) -> Path | None:
    """Download ONLY the GRIB records we need, concatenated into a single local file."""
    key = f"hrrr.{date_str}/conus/hrrr.t{run_hour}z.wrfsfcf{fhr:02d}.grib2"
    base_url = f"{HRRR_BASE}/{key}"
    idx_url = f"{base_url}.idx"

    try:
        records = _get_idx(client, idx_url)
    except httpx.HTTPError as exc:
        log.warning("idx_unavailable", extra={"fhr": fhr, "err": str(exc)})
        return _download_full(client, base_url, fhr, tmp_dir)

    matchers = [IDX_MATCHERS[n] for n in needed if n in IDX_MATCHERS]
    ranges = _pick_ranges(records, matchers)
    if not ranges:
        log.warning("no_records_matched", extra={"fhr": fhr, "needed": list(needed)})
        return None

    out_path = tmp_dir / f"hrrr_f{fhr:02d}_subset.grib2"
    total_bytes = 0
    try:
        with out_path.open("wb") as fh:
            for start, end in ranges:
                body = _range_get(client, base_url, start, end)
                fh.write(body)
                total_bytes += len(body)
    except httpx.HTTPError as exc:
        log.warning("range_get_failed_falling_back", extra={"fhr": fhr, "err": str(exc)})
        out_path.unlink(missing_ok=True)
        return _download_full(client, base_url, fhr, tmp_dir)

    log.info(
        "downloaded",
        extra={"fhr": fhr, "records": len(ranges), "bytes": total_bytes, "mode": "byte-range"},
    )
    return out_path


@retry(attempts=3, base_delay=2.0, log=log, exceptions=(httpx.HTTPError,))
def _download_full(client: httpx.Client, url: str, fhr: int, tmp_dir: Path) -> Path | None:
    resp = client.get(url, timeout=300)
    resp.raise_for_status()
    out_path = tmp_dir / f"hrrr_f{fhr:02d}.grib2"
    out_path.write_bytes(resp.content)
    log.info("downloaded", extra={"fhr": fhr, "bytes": len(resp.content), "mode": "full"})
    return out_path


def extract_variable(
    grib_path: Path, match: dict
) -> tuple[np.ndarray, np.ndarray, np.ndarray] | None:
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


def kelvin_to_fahrenheit(k: np.ndarray) -> np.ndarray:
    return (k - 273.15) * 9 / 5 + 32


def ms_to_mph(ms: np.ndarray) -> np.ndarray:
    return ms * 2.237


def _render(
    tile_base: Path, layer: str, timestamp: str, rgba: np.ndarray, lats: np.ndarray, lons: np.ndarray
) -> int:
    if lats[0] > lats[-1]:
        rgba = np.flipud(rgba)
        lats = lats[::-1]
    out_dir = str(tile_base / layer / timestamp)
    count = render_tiles(rgba=rgba, lats=lats, lons=lons, output_dir=out_dir, zoom_levels=ZOOM_LEVELS)
    log.info("rendered", extra={"layer": layer, "timestamp": timestamp, "tiles": count})
    return count


def process_forecast_hour(
    grib_path: Path, run_id: str, fhr: int, color_tables: dict, tile_base: Path
) -> None:
    date_str, run_hour = run_id.split("_")
    run_dt = datetime.strptime(f"{date_str}{run_hour}", "%Y%m%d%H").replace(tzinfo=timezone.utc)
    valid_dt = run_dt + timedelta(hours=fhr)
    timestamp = valid_dt.isoformat()

    # Reflectivity
    r = extract_variable(grib_path, VAR_SELECTORS["refc"])
    if r:
        data, lats, lons = r
        _render(tile_base, "radar-hrrr", timestamp, apply_color_table(data, color_tables["reflectivity"]), lats, lons)

    # Temperature
    r = extract_variable(grib_path, VAR_SELECTORS["t2m"])
    if r:
        data, lats, lons = r
        data = kelvin_to_fahrenheit(data)
        _render(tile_base, "temperature", timestamp, apply_color_table(data, color_tables["temperature"]), lats, lons)

    # Dewpoint (Phase 4 prep — renders if a color table exists)
    if "dewpoint" in color_tables:
        r = extract_variable(grib_path, VAR_SELECTORS["dpt2m"])
        if r:
            data, lats, lons = r
            data = kelvin_to_fahrenheit(data)
            _render(tile_base, "dewpoint", timestamp, apply_color_table(data, color_tables["dewpoint"]), lats, lons)

    # Humidity
    if "humidity" in color_tables:
        r = extract_variable(grib_path, VAR_SELECTORS["rh2m"])
        if r:
            data, lats, lons = r
            _render(tile_base, "humidity", timestamp, apply_color_table(data, color_tables["humidity"]), lats, lons)

    # CAPE
    r = extract_variable(grib_path, VAR_SELECTORS["cape"])
    if r:
        data, lats, lons = r
        _render(tile_base, "cape", timestamp, apply_color_table(data, color_tables["cape"]), lats, lons)

    # Wind speed (from U/V)
    u = extract_variable(grib_path, VAR_SELECTORS["u10"])
    v = extract_variable(grib_path, VAR_SELECTORS["v10"])
    if u and v:
        u_data, lats, lons = u
        v_data = v[0]
        speed = ms_to_mph(np.sqrt(u_data**2 + v_data**2))
        _render(tile_base, "wind", timestamp, apply_color_table(speed, color_tables["wind_speed"]), lats, lons)

    # Precip type
    precip_results: dict[str, np.ndarray] = {}
    lats = lons = None
    for key in ("crain", "csnow", "cfrzr", "cicep"):
        r = extract_variable(grib_path, VAR_SELECTORS[key])
        if r:
            precip_results[key] = r[0]
            lats, lons = r[1], r[2]
    if precip_results and lats is not None and lons is not None:
        h, w = next(iter(precip_results.values())).shape
        category = np.zeros((h, w), dtype=np.int32)
        if "crain" in precip_results:
            category[precip_results["crain"] > 0] = 1
        if "csnow" in precip_results:
            category[precip_results["csnow"] > 0] = 2
        if "cfrzr" in precip_results:
            category[precip_results["cfrzr"] > 0] = 3
        if "cicep" in precip_results:
            category[precip_results["cicep"] > 0] = 4
        ptype_map = {1: "rain", 2: "snow", 3: "freezing_rain", 4: "ice_pellets"}
        rgba = apply_categorical_color_table(category, color_tables["precip_type"]["categories"], ptype_map)
        _render(tile_base, "precip-type", timestamp, rgba, lats, lons)


def cleanup_old_runs(tile_base: Path, layers: list[str], retention_hours: int) -> None:
    cutoff = time.time() - (retention_hours * 3600)
    for layer in layers:
        layer_dir = tile_base / layer
        if not layer_dir.exists():
            continue
        for ts_dir in sorted(layer_dir.iterdir()):
            if not ts_dir.is_dir():
                continue
            try:
                dt = datetime.fromisoformat(ts_dir.name)
            except ValueError:
                continue
            if dt.timestamp() < cutoff:
                shutil.rmtree(ts_dir, ignore_errors=True)


def needed_layers() -> list[str]:
    return list(IDX_MATCHERS.keys())


def fcst_horizon_for_run(run_hour: int) -> int:
    return EXTENDED_FORECAST_HOURS if run_hour in EXTENDED_RUNS else FORECAST_HOURS


def run() -> None:
    color_tables = load_color_tables()
    client = httpx.Client()
    tile_base = Path(TILE_DIR)
    tmp_dir = Path("/tmp/hrrr_work")
    tmp_dir.mkdir(parents=True, exist_ok=True)

    state = ProcessedSet(Path(STATE_DIR) / "ingest-hrrr.json", max_entries=200)
    log.info(
        "startup",
        extra={
            "tile_dir": str(tile_base),
            "poll_interval_s": POLL_INTERVAL,
            "fcst_default": FORECAST_HOURS,
            "fcst_extended": EXTENDED_FORECAST_HOURS,
            "processed_runs": len(state._items),
        },
    )

    while True:
        started = time.time()
        try:
            run_id = find_latest_hrrr_run(client)
            if run_id and run_id not in state:
                date_str, run_hour = run_id.split("_")
                horizon = fcst_horizon_for_run(int(run_hour))
                log.info(
                    "run_start",
                    extra={"run_id": run_id, "horizon": horizon, "extended": int(run_hour) in EXTENDED_RUNS},
                )

                for fhr in range(1, horizon + 1):
                    grib_path = download_forecast_hour_subset(
                        client, date_str, run_hour, fhr, needed_layers(), tmp_dir
                    )
                    if grib_path and grib_path.exists():
                        try:
                            process_forecast_hour(grib_path, run_id, fhr, color_tables, tile_base)
                        finally:
                            grib_path.unlink(missing_ok=True)

                state.add(run_id)
                log.info("run_complete", extra={"run_id": run_id})

            cleanup_old_runs(
                tile_base,
                ["radar-hrrr", "temperature", "dewpoint", "humidity", "wind", "cape", "precip-type"],
                RETENTION_HOURS,
            )

        except Exception as exc:  # noqa: BLE001
            log.exception("loop_error", extra={"err": str(exc)})

        elapsed = time.time() - started
        sleep_for = max(30.0, POLL_INTERVAL - elapsed)
        time.sleep(sleep_for)


if __name__ == "__main__":
    run()
