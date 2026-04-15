#!/usr/bin/env python3
"""MRMS radar ingest: poll S3 → decode GRIB2 → render PNG tiles."""

import json
import os
import sys
import time
from datetime import datetime, timezone
from pathlib import Path

import httpx
import numpy as np
import pygrib
from PIL import Image

# Add shared module
sys.path.insert(0, str(Path(__file__).resolve().parent / "shared"))
from tiler import apply_color_table, render_tiles

MRMS_BASE = "https://noaa-mrms-pds.s3.amazonaws.com"
MRMS_PREFIX = "CONUS/MergedBaseReflectivity_00.50"
TILE_DIR = os.environ.get("TILE_DIR", "/data/tiles")
COLOR_TABLE_PATH = os.environ.get(
    "COLOR_TABLE_PATH",
    str(Path(__file__).resolve().parent.parent / "shared" / "color_tables.json"),
)
POLL_INTERVAL = int(os.environ.get("POLL_INTERVAL", "120"))  # seconds
ZOOM_LEVELS = [4, 5, 6, 7, 8, 9]
RETENTION_HOURS = 4

processed_files: set[str] = set()


def load_color_table() -> dict:
    with open(COLOR_TABLE_PATH) as f:
        tables = json.load(f)
    return tables["reflectivity"]


def list_recent_files(client: httpx.Client) -> list[str]:
    """List MRMS GRIB2 files from S3 using XML listing."""
    url = f"{MRMS_BASE}?prefix={MRMS_PREFIX}&list-type=2&max-keys=30"
    resp = client.get(url, timeout=30)
    resp.raise_for_status()

    # Parse XML response for Key elements
    import xml.etree.ElementTree as ET
    root = ET.fromstring(resp.text)
    ns = {"s3": "http://s3.amazonaws.com/doc/2006-03-01/"}
    keys = []
    for content in root.findall(".//s3:Contents/s3:Key", ns):
        if content.text and content.text.endswith(".grib2.gz"):
            keys.append(content.text)
    return sorted(keys)


def download_and_decode(client: httpx.Client, key: str, tmp_dir: Path) -> tuple[np.ndarray, np.ndarray, np.ndarray] | None:
    """Download GRIB2 file from S3 and decode to numpy arrays."""
    import gzip

    url = f"{MRMS_BASE}/{key}"
    resp = client.get(url, timeout=60)
    resp.raise_for_status()

    gz_path = tmp_dir / "mrms.grib2.gz"
    grib_path = tmp_dir / "mrms.grib2"
    gz_path.write_bytes(resp.content)

    with gzip.open(gz_path, "rb") as f_in:
        grib_path.write_bytes(f_in.read())

    try:
        grbs = pygrib.open(str(grib_path))
        grb = grbs[1]
        data = grb.values  # 2D numpy array
        lats, lons = grb.latlons()
        lat_col = lats[:, 0]  # 1D latitude array
        lon_row = lons[0, :]  # 1D longitude array

        # Replace masked values with NaN
        if hasattr(data, "filled"):
            data = data.filled(np.nan)

        grbs.close()
        return data.astype(np.float32), lat_col.astype(np.float64), lon_row.astype(np.float64)
    except Exception as e:
        print(f"  Error decoding {key}: {e}")
        return None
    finally:
        gz_path.unlink(missing_ok=True)
        grib_path.unlink(missing_ok=True)


def extract_timestamp(key: str) -> str:
    """Extract ISO timestamp from MRMS filename.
    Example key: CONUS/MergedBaseReflectivity_00.50/MergedBaseReflectivity_00.50_20260414-200200.grib2.gz
    """
    basename = key.split("/")[-1]
    # Extract YYYYMMDD-HHMMSS
    parts = basename.replace(".grib2.gz", "").split("_")
    dt_str = parts[-1]  # 20260414-200200
    dt = datetime.strptime(dt_str, "%Y%m%d-%H%M%S").replace(tzinfo=timezone.utc)
    return dt.isoformat()


def cleanup_old_tiles(base_dir: Path, retention_hours: int):
    """Delete tile directories older than retention window."""
    radar_dir = base_dir / "radar"
    if not radar_dir.exists():
        return
    cutoff = time.time() - (retention_hours * 3600)
    for ts_dir in sorted(radar_dir.iterdir()):
        if ts_dir.is_dir():
            try:
                dt = datetime.fromisoformat(ts_dir.name)
                if dt.timestamp() < cutoff:
                    import shutil
                    shutil.rmtree(ts_dir)
                    print(f"  Cleaned up {ts_dir.name}")
            except ValueError:
                pass


def run():
    color_table = load_color_table()
    client = httpx.Client()
    tile_base = Path(TILE_DIR)
    tmp_dir = Path("/tmp/mrms_work")
    tmp_dir.mkdir(parents=True, exist_ok=True)

    print(f"MRMS ingest starting. Tiles → {tile_base}, poll every {POLL_INTERVAL}s")

    while True:
        try:
            keys = list_recent_files(client)
            new_keys = [k for k in keys if k not in processed_files]

            if new_keys:
                # Process only the latest file
                latest = new_keys[-1]
                print(f"Processing: {latest}")
                result = download_and_decode(client, latest, tmp_dir)
                if result is not None:
                    data, lats, lons = result
                    rgba = apply_color_table(data, color_table)

                    # Flip if lats are descending (common in GRIB2)
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
                    print(f"  Wrote {count} tiles for {timestamp}")
                    processed_files.add(latest)

                    # Mark all older files as processed too
                    for k in new_keys[:-1]:
                        processed_files.add(k)

            cleanup_old_tiles(tile_base, RETENTION_HOURS)

        except Exception as e:
            print(f"Error in ingest loop: {e}")

        time.sleep(POLL_INTERVAL)


if __name__ == "__main__":
    run()
