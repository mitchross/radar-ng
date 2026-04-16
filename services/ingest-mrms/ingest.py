#!/usr/bin/env python3
"""MRMS radar ingest: poll S3 → decode GRIB2 → render PNG tiles.

Hardened for Phase 1.4:
- Structured JSON logs (see services/shared/logger.py)
- Exponential backoff on S3 listing + downloads
- Processed-files state persisted to /data/state/ingest-mrms.json so restarts
  do not re-render tiles we already have.
"""

from __future__ import annotations

import gzip
import os
import shutil
import sys
import time
import xml.etree.ElementTree as ET
from datetime import datetime, timedelta, timezone
from pathlib import Path

import httpx
import numpy as np
import pygrib

sys.path.insert(0, str(Path(__file__).resolve().parent / "shared"))
sys.path.insert(0, "/app/shared")
from logger import get_logger, retry  # type: ignore  # noqa: E402
from state import ProcessedSet  # type: ignore  # noqa: E402
from tiler import apply_color_table, render_tiles  # type: ignore  # noqa: E402

MRMS_BASE = "https://noaa-mrms-pds.s3.amazonaws.com"
MRMS_PREFIX = "CONUS/MergedBaseReflectivity_00.50"
TILE_DIR = os.environ.get("TILE_DIR", "/data/tiles")
STATE_DIR = os.environ.get("STATE_DIR", "/data/state")
COLOR_TABLE_PATH = os.environ.get(
    "COLOR_TABLE_PATH",
    str(Path(__file__).resolve().parent.parent / "shared" / "color_tables.json"),
)
POLL_INTERVAL = int(os.environ.get("POLL_INTERVAL", "120"))
ZOOM_LEVELS = [4, 5, 6, 7, 8, 9]
RETENTION_HOURS = 4

log = get_logger("ingest-mrms")


def load_color_table() -> dict:
    import json
    with open(COLOR_TABLE_PATH) as f:
        return json.load(f)["reflectivity"]


@retry(attempts=4, base_delay=2.0, log=log, exceptions=(httpx.HTTPError,))
def _list_keys(client: httpx.Client, prefix: str) -> list[str]:
    """List all GRIB2 keys under a prefix, paginating past S3's 1000-key cap."""
    ns = {"s3": "http://s3.amazonaws.com/doc/2006-03-01/"}
    keys: list[str] = []
    continuation: str | None = None
    while True:
        url = f"{MRMS_BASE}?prefix={prefix}&list-type=2&max-keys=1000"
        if continuation:
            url += f"&continuation-token={continuation}"
        resp = client.get(url, timeout=30)
        resp.raise_for_status()
        root = ET.fromstring(resp.text)
        for content in root.findall(".//s3:Contents/s3:Key", ns):
            if content.text and content.text.endswith(".grib2.gz"):
                keys.append(content.text)
        is_truncated = root.findtext(".//s3:IsTruncated", default="false", namespaces=ns)
        if is_truncated != "true":
            break
        continuation = root.findtext(".//s3:NextContinuationToken", namespaces=ns)
        if not continuation:
            break
    return keys


def list_recent_files(client: httpx.Client) -> list[str]:
    today = datetime.now(timezone.utc).strftime("%Y%m%d")
    keys = _list_keys(client, f"{MRMS_PREFIX}/{today}")
    if not keys:
        yesterday = (datetime.now(timezone.utc) - timedelta(days=1)).strftime("%Y%m%d")
        keys = _list_keys(client, f"{MRMS_PREFIX}/{yesterday}")
    return sorted(keys)


@retry(attempts=4, base_delay=2.0, log=log, exceptions=(httpx.HTTPError,))
def _fetch(client: httpx.Client, url: str) -> bytes:
    resp = client.get(url, timeout=60)
    resp.raise_for_status()
    return resp.content


def download_and_decode(
    client: httpx.Client, key: str, tmp_dir: Path
) -> tuple[np.ndarray, np.ndarray, np.ndarray] | None:
    url = f"{MRMS_BASE}/{key}"
    try:
        body = _fetch(client, url)
    except httpx.HTTPError as exc:
        log.error("download_failed", extra={"key": key, "err": str(exc)})
        return None

    gz_path = tmp_dir / "mrms.grib2.gz"
    grib_path = tmp_dir / "mrms.grib2"
    gz_path.write_bytes(body)
    with gzip.open(gz_path, "rb") as src:
        grib_path.write_bytes(src.read())

    try:
        grbs = pygrib.open(str(grib_path))
        grb = grbs[1]
        data = grb.values
        lats, lons = grb.latlons()
        lat_col = lats[:, 0]
        lon_row = lons[0, :]
        if hasattr(data, "filled"):
            data = data.filled(np.nan)
        grbs.close()
        return data.astype(np.float32), lat_col.astype(np.float64), lon_row.astype(np.float64)
    except Exception as exc:  # noqa: BLE001
        log.error("decode_failed", extra={"key": key, "err": str(exc)})
        return None
    finally:
        gz_path.unlink(missing_ok=True)
        grib_path.unlink(missing_ok=True)


def extract_timestamp(key: str) -> str:
    """CONUS/MergedBaseReflectivity_00.50/MergedBaseReflectivity_00.50_20260414-200200.grib2.gz"""
    basename = key.split("/")[-1]
    parts = basename.replace(".grib2.gz", "").split("_")
    dt_str = parts[-1]
    dt = datetime.strptime(dt_str, "%Y%m%d-%H%M%S").replace(tzinfo=timezone.utc)
    return dt.isoformat()


def cleanup_old_tiles(base_dir: Path, retention_hours: int) -> None:
    radar_dir = base_dir / "radar"
    if not radar_dir.exists():
        return
    cutoff = time.time() - (retention_hours * 3600)
    for ts_dir in sorted(radar_dir.iterdir()):
        if not ts_dir.is_dir():
            continue
        try:
            dt = datetime.fromisoformat(ts_dir.name)
        except ValueError:
            continue
        if dt.timestamp() < cutoff:
            shutil.rmtree(ts_dir, ignore_errors=True)
            log.info("retention_expired", extra={"layer": "radar", "timestamp": ts_dir.name})


def run() -> None:
    color_table = load_color_table()
    client = httpx.Client()
    tile_base = Path(TILE_DIR)
    tmp_dir = Path("/tmp/mrms_work")
    tmp_dir.mkdir(parents=True, exist_ok=True)

    state = ProcessedSet(Path(STATE_DIR) / "ingest-mrms.json", max_entries=2000)
    log.info(
        "startup",
        extra={"tile_dir": str(tile_base), "poll_interval_s": POLL_INTERVAL, "processed": len(state._items)},
    )

    while True:
        started = time.time()
        try:
            keys = list_recent_files(client)
            new_keys = [k for k in keys if k not in state]

            if new_keys:
                latest = new_keys[-1]
                log.info("processing", extra={"key": latest, "backlog": len(new_keys)})
                result = download_and_decode(client, latest, tmp_dir)
                if result is not None:
                    data, lats, lons = result
                    rgba = apply_color_table(data, color_table)
                    if lats[0] > lats[-1]:
                        rgba = np.flipud(rgba)
                        lats = lats[::-1]
                    timestamp = extract_timestamp(latest)
                    out_dir = str(tile_base / "radar" / timestamp)
                    count = render_tiles(
                        rgba=rgba,
                        lats=lats,
                        lons=lons,
                        output_dir=out_dir,
                        zoom_levels=ZOOM_LEVELS,
                    )
                    log.info(
                        "rendered",
                        extra={"layer": "radar", "timestamp": timestamp, "tiles": count},
                    )
                    state.update(new_keys)  # latest + older keys in one flush

            cleanup_old_tiles(tile_base, RETENTION_HOURS)

        except Exception as exc:  # noqa: BLE001
            log.exception("loop_error", extra={"err": str(exc)})

        elapsed = time.time() - started
        sleep_for = max(5.0, POLL_INTERVAL - elapsed)
        time.sleep(sleep_for)


if __name__ == "__main__":
    run()
