# StormScope Backend Pipeline — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Build a containerized weather tile pipeline that ingests MRMS radar and HRRR forecast data from NOAA, renders PNG map tiles, and serves them via Caddy — replacing the RainViewer free tier with 1km resolution, 3hr history, and multi-layer support (wind, temp, CAPE, precip type).

**Architecture:** Three Python ingest services write tiles to a shared volume. Caddy serves tiles as static files plus a manifest API that lists available timestamps per layer. A cleanup cron expires old tiles. Everything runs via docker-compose locally.

**Tech Stack:** Python 3.12, pygrib, numpy, Pillow, mercantile, httpx, Caddy, Docker

---

## File Structure

```
services/
├── shared/
│   └── color_tables.json              # Shared color lookup tables for all layers
├── ingest-mrms/
│   ├── Dockerfile
│   ├── requirements.txt
│   ├── ingest.py                      # Main loop: poll S3 → decode GRIB2 → render tiles
│   └── tiler.py                       # GRIB2 array → PNG tile generation
├── ingest-hrrr/
│   ├── Dockerfile
│   ├── requirements.txt
│   ├── ingest.py                      # Main loop: poll S3 → decode multi-var → render tiles
│   └── tiler.py                       # Shared tile rendering (same interface as MRMS tiler)
├── tile-server/
│   ├── Dockerfile
│   ├── Caddyfile                      # Static file server + reverse proxy for API
│   └── api/
│       ├── requirements.txt
│       ├── manifest.py                # Scan tile dirs → build manifest.json
│       └── server.py                  # FastAPI: /api/manifest.json, /api/forecast/{lat}/{lon}
├── tile-cleanup/
│   └── cleanup.sh                     # find + delete tiles older than TTL
└── docker-compose.yml                 # Orchestrates all services with shared volume
```

---

### Task 1: Color Tables (shared config)

**Files:**
- Create: `services/shared/color_tables.json`

- [ ] **Step 1: Create color table JSON**

Create `services/shared/color_tables.json`:

```json
{
  "reflectivity": {
    "unit": "dBZ",
    "ranges": [
      { "min": 5,  "max": 10, "rgba": [0, 255, 0, 180] },
      { "min": 10, "max": 20, "rgba": [0, 200, 0, 200] },
      { "min": 20, "max": 30, "rgba": [255, 200, 0, 210] },
      { "min": 30, "max": 40, "rgba": [255, 100, 0, 220] },
      { "min": 40, "max": 50, "rgba": [255, 0, 0, 230] },
      { "min": 50, "max": 60, "rgba": [200, 0, 0, 240] },
      { "min": 60, "max": 65, "rgba": [255, 0, 255, 245] },
      { "min": 65, "max": 75, "rgba": [139, 0, 139, 250] }
    ],
    "no_data_below": 5
  },
  "temperature": {
    "unit": "fahrenheit",
    "ranges": [
      { "min": -40, "max": 0,   "rgba": [128, 0, 255, 160] },
      { "min": 0,   "max": 15,  "rgba": [64, 0, 255, 170] },
      { "min": 15,  "max": 32,  "rgba": [0, 100, 255, 180] },
      { "min": 32,  "max": 45,  "rgba": [0, 200, 255, 180] },
      { "min": 45,  "max": 55,  "rgba": [0, 255, 200, 180] },
      { "min": 55,  "max": 65,  "rgba": [0, 255, 0, 170] },
      { "min": 65,  "max": 75,  "rgba": [200, 255, 0, 170] },
      { "min": 75,  "max": 85,  "rgba": [255, 200, 0, 180] },
      { "min": 85,  "max": 95,  "rgba": [255, 100, 0, 190] },
      { "min": 95,  "max": 110, "rgba": [255, 0, 0, 200] },
      { "min": 110, "max": 130, "rgba": [200, 0, 0, 210] }
    ],
    "no_data_below": -999
  },
  "wind_speed": {
    "unit": "mph",
    "ranges": [
      { "min": 0,  "max": 5,   "rgba": [150, 200, 255, 100] },
      { "min": 5,  "max": 10,  "rgba": [100, 180, 255, 140] },
      { "min": 10, "max": 15,  "rgba": [0, 200, 100, 160] },
      { "min": 15, "max": 25,  "rgba": [0, 255, 0, 170] },
      { "min": 25, "max": 35,  "rgba": [255, 255, 0, 180] },
      { "min": 35, "max": 50,  "rgba": [255, 150, 0, 200] },
      { "min": 50, "max": 75,  "rgba": [255, 0, 0, 220] },
      { "min": 75, "max": 150, "rgba": [200, 0, 200, 240] }
    ],
    "no_data_below": 0
  },
  "cape": {
    "unit": "J/kg",
    "ranges": [
      { "min": 500,  "max": 1000, "rgba": [255, 255, 0, 120] },
      { "min": 1000, "max": 2000, "rgba": [255, 200, 0, 150] },
      { "min": 2000, "max": 3000, "rgba": [255, 100, 0, 180] },
      { "min": 3000, "max": 4000, "rgba": [255, 0, 0, 200] },
      { "min": 4000, "max": 8000, "rgba": [200, 0, 200, 220] }
    ],
    "no_data_below": 500
  },
  "precip_type": {
    "categories": {
      "rain": [0, 200, 0, 180],
      "snow": [100, 150, 255, 200],
      "freezing_rain": [255, 100, 150, 200],
      "ice_pellets": [200, 0, 200, 200],
      "hail": [255, 165, 0, 220]
    }
  }
}
```

- [ ] **Step 2: Commit**

```bash
cd /home/vanillax/programming/radar-ng
git add services/shared/color_tables.json
git commit -m "feat: add shared color table definitions for all weather layers"
```

---

### Task 2: Tile Renderer Module

**Files:**
- Create: `services/shared/tiler.py`
- Create: `services/shared/test_tiler.py`

This module is shared between ingest-mrms and ingest-hrrr. It takes a 2D numpy array + lat/lon grid + color table and produces tile PNGs.

- [ ] **Step 1: Write test for tiler**

Create `services/shared/test_tiler.py`:

```python
import json
import os
import tempfile
import numpy as np
from pathlib import Path

# Will import from tiler once created
import pytest


def test_apply_color_table():
    from tiler import apply_color_table

    color_table = {
        "ranges": [
            {"min": 5, "max": 20, "rgba": [0, 255, 0, 200]},
            {"min": 20, "max": 40, "rgba": [255, 0, 0, 200]},
        ],
        "no_data_below": 5,
    }
    data = np.array([[3.0, 10.0], [25.0, 50.0]])
    rgba = apply_color_table(data, color_table)
    assert rgba.shape == (2, 2, 4)
    # Below threshold → transparent
    assert rgba[0, 0, 3] == 0
    # 10 dBZ → green
    assert rgba[0, 1, 1] == 255
    # 25 dBZ → red
    assert rgba[1, 0, 0] == 255
    # 50 dBZ is above max range → transparent
    assert rgba[1, 1, 3] == 0


def test_render_tiles_creates_files():
    from tiler import apply_color_table, render_tiles

    color_table = {
        "ranges": [{"min": 0, "max": 100, "rgba": [255, 0, 0, 200]}],
        "no_data_below": -1,
    }
    # Create a small CONUS-sized grid
    lats = np.linspace(25.0, 50.0, 100)
    lons = np.linspace(-125.0, -65.0, 200)
    data = np.random.uniform(10, 60, (100, 200)).astype(np.float32)
    rgba = apply_color_table(data, color_table)

    with tempfile.TemporaryDirectory() as tmpdir:
        count = render_tiles(
            rgba=rgba,
            lats=lats,
            lons=lons,
            output_dir=tmpdir,
            zoom_levels=[4, 5],
            tile_size=256,
        )
        assert count > 0
        # Check at least one tile exists
        tiles = list(Path(tmpdir).rglob("*.png"))
        assert len(tiles) > 0
        # Check tile path format: {z}/{x}/{y}.png
        first = tiles[0]
        parts = first.relative_to(tmpdir).parts
        assert len(parts) == 3  # z/x/y.png
```

- [ ] **Step 2: Run test to verify it fails**

```bash
cd /home/vanillax/programming/radar-ng/services/shared
python -m pytest test_tiler.py -v
```

Expected: FAIL — `ModuleNotFoundError: No module named 'tiler'`

- [ ] **Step 3: Implement tiler module**

Create `services/shared/tiler.py`:

```python
"""Shared tile renderer: numpy array → PNG tiles in XYZ slippy map format."""

import math
from pathlib import Path

import numpy as np
from PIL import Image


def apply_color_table(
    data: np.ndarray, color_table: dict
) -> np.ndarray:
    """Apply a color table to a 2D data array, returning RGBA uint8 array."""
    h, w = data.shape
    rgba = np.zeros((h, w, 4), dtype=np.uint8)

    no_data = color_table.get("no_data_below", -999)

    for rng in color_table["ranges"]:
        mask = (data >= rng["min"]) & (data < rng["max"])
        rgba[mask] = rng["rgba"]

    # Anything below no_data threshold or not in any range → transparent
    below = data < no_data
    rgba[below] = [0, 0, 0, 0]

    return rgba


def apply_categorical_color_table(
    data: np.ndarray, categories: dict[str, list[int]], category_map: dict[int, str]
) -> np.ndarray:
    """Apply categorical colors (e.g., precip type) to a 2D integer array."""
    h, w = data.shape
    rgba = np.zeros((h, w, 4), dtype=np.uint8)

    for value, name in category_map.items():
        if name in categories:
            mask = data == value
            rgba[mask] = categories[name]

    return rgba


def _lat_lon_to_tile(lat: float, lon: float, zoom: int) -> tuple[int, int]:
    """Convert lat/lon to tile x, y at given zoom."""
    n = 2**zoom
    x = int((lon + 180.0) / 360.0 * n)
    lat_rad = math.radians(lat)
    y = int((1.0 - math.log(math.tan(lat_rad) + 1.0 / math.cos(lat_rad)) / math.pi) / 2.0 * n)
    return (max(0, min(x, n - 1)), max(0, min(y, n - 1)))


def _tile_bounds(x: int, y: int, z: int) -> tuple[float, float, float, float]:
    """Return (west, south, east, north) in degrees for a tile."""
    n = 2**z
    west = x / n * 360.0 - 180.0
    east = (x + 1) / n * 360.0 - 180.0
    north = math.degrees(math.atan(math.sinh(math.pi * (1 - 2 * y / n))))
    south = math.degrees(math.atan(math.sinh(math.pi * (1 - 2 * (y + 1) / n))))
    return (west, south, east, north)


def render_tiles(
    rgba: np.ndarray,
    lats: np.ndarray,
    lons: np.ndarray,
    output_dir: str,
    zoom_levels: list[int],
    tile_size: int = 256,
) -> int:
    """Render RGBA array into XYZ PNG tiles. Returns number of tiles written."""
    lat_min, lat_max = float(lats.min()), float(lats.max())
    lon_min, lon_max = float(lons.min()), float(lons.max())
    h, w = rgba.shape[:2]
    count = 0

    for z in zoom_levels:
        tx_min, ty_max = _lat_lon_to_tile(lat_max, lon_min, z)
        tx_max, ty_min = _lat_lon_to_tile(lat_min, lon_max, z)

        for tx in range(tx_min, tx_max + 1):
            for ty in range(ty_min, ty_max + 1):
                west, south, east, north = _tile_bounds(tx, ty, z)

                # Map tile bounds to pixel indices in the source array
                col_start = int((west - lon_min) / (lon_max - lon_min) * w)
                col_end = int((east - lon_min) / (lon_max - lon_min) * w)
                row_start = int((lat_max - north) / (lat_max - lat_min) * h)
                row_end = int((lat_max - south) / (lat_max - lat_min) * h)

                col_start = max(0, min(col_start, w))
                col_end = max(0, min(col_end, w))
                row_start = max(0, min(row_start, h))
                row_end = max(0, min(row_end, h))

                if col_end <= col_start or row_end <= row_start:
                    continue

                region = rgba[row_start:row_end, col_start:col_end]

                # Skip fully transparent tiles
                if region[:, :, 3].max() == 0:
                    continue

                img = Image.fromarray(region, "RGBA")
                img = img.resize((tile_size, tile_size), Image.BILINEAR)

                tile_path = Path(output_dir) / str(z) / str(tx) / f"{ty}.png"
                tile_path.parent.mkdir(parents=True, exist_ok=True)
                img.save(str(tile_path), "PNG", optimize=True)
                count += 1

    return count
```

- [ ] **Step 4: Run tests**

```bash
cd /home/vanillax/programming/radar-ng/services/shared
pip install numpy Pillow pytest
python -m pytest test_tiler.py -v
```

Expected: 2 tests PASS.

- [ ] **Step 5: Commit**

```bash
cd /home/vanillax/programming/radar-ng
git add services/shared/tiler.py services/shared/test_tiler.py
git commit -m "feat: add shared tile renderer module with color table support"
```

---

### Task 3: MRMS Ingest Service

**Files:**
- Create: `services/ingest-mrms/ingest.py`
- Create: `services/ingest-mrms/requirements.txt`
- Create: `services/ingest-mrms/Dockerfile`

- [ ] **Step 1: Create requirements.txt**

Create `services/ingest-mrms/requirements.txt`:

```
numpy>=1.26
Pillow>=10.0
pygrib>=2.1
httpx>=0.27
```

- [ ] **Step 2: Create ingest.py**

Create `services/ingest-mrms/ingest.py`:

```python
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
sys.path.insert(0, str(Path(__file__).resolve().parent.parent / "shared"))
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
```

- [ ] **Step 3: Create Dockerfile**

Create `services/ingest-mrms/Dockerfile`:

```dockerfile
FROM python:3.12-slim

RUN apt-get update && apt-get install -y --no-install-recommends \
    libeccodes-dev \
    && rm -rf /var/lib/apt/lists/*

WORKDIR /app

COPY requirements.txt .
RUN pip install --no-cache-dir -r requirements.txt

COPY ../shared /app/shared
COPY ingest.py .

ENV TILE_DIR=/data/tiles
ENV COLOR_TABLE_PATH=/app/shared/color_tables.json
ENV POLL_INTERVAL=120

CMD ["python", "-u", "ingest.py"]
```

- [ ] **Step 4: Commit**

```bash
cd /home/vanillax/programming/radar-ng
git add services/ingest-mrms/
git commit -m "feat: add MRMS radar ingest service with GRIB2 decode and tile rendering"
```

---

### Task 4: HRRR Ingest Service

**Files:**
- Create: `services/ingest-hrrr/ingest.py`
- Create: `services/ingest-hrrr/requirements.txt`
- Create: `services/ingest-hrrr/Dockerfile`

- [ ] **Step 1: Create requirements.txt**

Create `services/ingest-hrrr/requirements.txt`:

```
numpy>=1.26
Pillow>=10.0
pygrib>=2.1
httpx>=0.27
```

- [ ] **Step 2: Create ingest.py**

Create `services/ingest-hrrr/ingest.py`:

```python
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
```

- [ ] **Step 3: Create Dockerfile**

Create `services/ingest-hrrr/Dockerfile`:

```dockerfile
FROM python:3.12-slim

RUN apt-get update && apt-get install -y --no-install-recommends \
    libeccodes-dev \
    && rm -rf /var/lib/apt/lists/*

WORKDIR /app

COPY requirements.txt .
RUN pip install --no-cache-dir -r requirements.txt

COPY ../shared /app/shared
COPY ingest.py .

ENV TILE_DIR=/data/tiles
ENV COLOR_TABLE_PATH=/app/shared/color_tables.json
ENV POLL_INTERVAL=3600

CMD ["python", "-u", "ingest.py"]
```

- [ ] **Step 4: Commit**

```bash
cd /home/vanillax/programming/radar-ng
git add services/ingest-hrrr/
git commit -m "feat: add HRRR forecast ingest service with multi-layer tile rendering"
```

---

### Task 5: Tile Server (Caddy + Manifest API)

**Files:**
- Create: `services/tile-server/Caddyfile`
- Create: `services/tile-server/api/server.py`
- Create: `services/tile-server/api/requirements.txt`
- Create: `services/tile-server/Dockerfile`

- [ ] **Step 1: Create manifest API**

Create `services/tile-server/api/requirements.txt`:

```
fastapi>=0.115
uvicorn>=0.32
httpx>=0.27
```

Create `services/tile-server/api/server.py`:

```python
#!/usr/bin/env python3
"""Tile server API: manifest.json + Open-Meteo forecast proxy."""

import os
import time
from datetime import datetime
from pathlib import Path

import httpx
from fastapi import FastAPI
from fastapi.responses import JSONResponse

TILE_DIR = os.environ.get("TILE_DIR", "/data/tiles")
OPEN_METEO_BASE = "https://api.open-meteo.com/v1/forecast"

app = FastAPI(title="StormScope Tile API")

# Simple in-memory cache for forecast proxy
_forecast_cache: dict[str, tuple[float, dict]] = {}
FORECAST_TTL = 900  # 15 minutes


@app.get("/api/manifest.json")
async def get_manifest():
    """Scan tile directories and return available timestamps per layer."""
    tile_base = Path(TILE_DIR)
    layers: dict[str, dict] = {}

    for layer_dir in sorted(tile_base.iterdir()):
        if not layer_dir.is_dir():
            continue
        layer_name = layer_dir.name
        timestamps = []
        for ts_dir in sorted(layer_dir.iterdir()):
            if ts_dir.is_dir():
                # Verify it has actual tiles (check for zoom level dirs)
                if any(ts_dir.iterdir()):
                    timestamps.append(ts_dir.name)
        if timestamps:
            layers[layer_name] = {"timestamps": timestamps}

    return JSONResponse({
        "layers": layers,
        "tile_url_template": "/tiles/{layer}/{timestamp}/{z}/{x}/{y}.png",
        "updated_at": datetime.utcnow().isoformat() + "Z",
    })


@app.get("/api/forecast/{lat}/{lon}")
async def get_forecast(lat: float, lon: float):
    """Proxy + cache Open-Meteo forecast requests."""
    # Round to 0.1° grid for cache efficiency
    grid_lat = round(lat, 1)
    grid_lon = round(lon, 1)
    cache_key = f"{grid_lat},{grid_lon}"

    # Check cache
    if cache_key in _forecast_cache:
        cached_at, data = _forecast_cache[cache_key]
        if time.time() - cached_at < FORECAST_TTL:
            return JSONResponse(data)

    # Fetch from Open-Meteo
    params = {
        "latitude": str(grid_lat),
        "longitude": str(grid_lon),
        "current": "temperature_2m,relative_humidity_2m,apparent_temperature,weather_code,wind_speed_10m,wind_direction_10m,wind_gusts_10m",
        "hourly": "temperature_2m,precipitation_probability,weather_code,wind_speed_10m",
        "daily": "temperature_2m_max,temperature_2m_min,weather_code,precipitation_sum,sunrise,sunset",
        "temperature_unit": "fahrenheit",
        "wind_speed_unit": "mph",
        "precipitation_unit": "inch",
        "timezone": "auto",
        "forecast_days": "7",
    }

    async with httpx.AsyncClient() as client:
        resp = await client.get(OPEN_METEO_BASE, params=params, timeout=15)
        resp.raise_for_status()
        data = resp.json()

    _forecast_cache[cache_key] = (time.time(), data)
    return JSONResponse(data)


@app.get("/api/health")
async def health():
    return {"status": "ok"}
```

- [ ] **Step 2: Create Caddyfile**

Create `services/tile-server/Caddyfile`:

```
:8080 {
    # Static tile serving
    handle /tiles/* {
        root * /data
        file_server {
            precompressed gzip
        }
        header Cache-Control "public, max-age=120"
        header Access-Control-Allow-Origin "*"
    }

    # API endpoints (proxied to FastAPI)
    handle /api/* {
        reverse_proxy localhost:8000
        header Access-Control-Allow-Origin "*"
    }

    log {
        output stdout
        format console
    }
}
```

- [ ] **Step 3: Create Dockerfile**

Create `services/tile-server/Dockerfile`:

```dockerfile
FROM python:3.12-slim AS api

WORKDIR /app/api
COPY api/requirements.txt .
RUN pip install --no-cache-dir -r requirements.txt
COPY api/ .

FROM caddy:2-alpine

# Install Python for the API sidecar
RUN apk add --no-cache python3 py3-pip

COPY --from=api /app/api /app/api
COPY --from=api /usr/local/lib/python3.12/site-packages /usr/local/lib/python3.12/site-packages

COPY Caddyfile /etc/caddy/Caddyfile

# Start script that runs both Caddy and the FastAPI server
RUN echo '#!/bin/sh' > /start.sh && \
    echo 'cd /app/api && python -m uvicorn server:app --host 0.0.0.0 --port 8000 &' >> /start.sh && \
    echo 'caddy run --config /etc/caddy/Caddyfile --adapter caddyfile' >> /start.sh && \
    chmod +x /start.sh

ENV TILE_DIR=/data/tiles

CMD ["/start.sh"]
```

- [ ] **Step 4: Commit**

```bash
cd /home/vanillax/programming/radar-ng
git add services/tile-server/
git commit -m "feat: add tile server with Caddy static serving and FastAPI manifest/forecast API"
```

---

### Task 6: Tile Cleanup & Docker Compose

**Files:**
- Create: `services/tile-cleanup/cleanup.sh`
- Create: `deploy/docker-compose.yml`

- [ ] **Step 1: Create cleanup script**

Create `services/tile-cleanup/cleanup.sh`:

```bash
#!/bin/bash
# Tile cleanup: delete expired tile directories
TILE_DIR="${TILE_DIR:-/data/tiles}"

echo "Tile cleanup running at $(date -u)"

# Radar (MRMS): keep 4 hours
find "$TILE_DIR/radar" -mindepth 1 -maxdepth 1 -type d -mmin +240 -exec rm -rf {} + 2>/dev/null

# HRRR layers: keep 8 hours
for layer in radar-hrrr temperature wind cape precip-type; do
    find "$TILE_DIR/$layer" -mindepth 1 -maxdepth 1 -type d -mmin +480 -exec rm -rf {} + 2>/dev/null
done

echo "Cleanup complete"
```

- [ ] **Step 2: Create docker-compose.yml**

Create `deploy/docker-compose.yml`:

```yaml
version: "3.8"

services:
  ingest-mrms:
    build:
      context: ../services
      dockerfile: ingest-mrms/Dockerfile
    volumes:
      - tiles:/data/tiles
    environment:
      - TILE_DIR=/data/tiles
      - POLL_INTERVAL=120
    restart: unless-stopped

  ingest-hrrr:
    build:
      context: ../services
      dockerfile: ingest-hrrr/Dockerfile
    volumes:
      - tiles:/data/tiles
    environment:
      - TILE_DIR=/data/tiles
      - POLL_INTERVAL=3600
    restart: unless-stopped

  tile-server:
    build:
      context: ../services/tile-server
    ports:
      - "8080:8080"
    volumes:
      - tiles:/data/tiles:ro
    environment:
      - TILE_DIR=/data/tiles
    restart: unless-stopped

  tile-cleanup:
    image: alpine:3.19
    volumes:
      - tiles:/data/tiles
      - ../services/tile-cleanup/cleanup.sh:/cleanup.sh:ro
    entrypoint: /bin/sh
    command: ["-c", "while true; do /bin/sh /cleanup.sh; sleep 1800; done"]
    environment:
      - TILE_DIR=/data/tiles
    restart: unless-stopped

volumes:
  tiles:
    driver: local
```

- [ ] **Step 3: Commit**

```bash
cd /home/vanillax/programming/radar-ng
git add services/tile-cleanup/ deploy/
git commit -m "feat: add tile cleanup service and docker-compose for local deployment"
```

---

### Task 7: Dockerfile Build Context Fix

The Dockerfiles reference `../shared` which doesn't work with Docker's build context. Fix by using the `services/` directory as context.

**Files:**
- Modify: `services/ingest-mrms/Dockerfile`
- Modify: `services/ingest-hrrr/Dockerfile`

- [ ] **Step 1: Fix MRMS Dockerfile**

Replace `services/ingest-mrms/Dockerfile`:

```dockerfile
FROM python:3.12-slim

RUN apt-get update && apt-get install -y --no-install-recommends \
    libeccodes-dev \
    && rm -rf /var/lib/apt/lists/*

WORKDIR /app

COPY ingest-mrms/requirements.txt .
RUN pip install --no-cache-dir -r requirements.txt

COPY shared/ /app/shared/
COPY ingest-mrms/ingest.py .

ENV TILE_DIR=/data/tiles
ENV COLOR_TABLE_PATH=/app/shared/color_tables.json
ENV POLL_INTERVAL=120

CMD ["python", "-u", "ingest.py"]
```

- [ ] **Step 2: Fix HRRR Dockerfile**

Replace `services/ingest-hrrr/Dockerfile`:

```dockerfile
FROM python:3.12-slim

RUN apt-get update && apt-get install -y --no-install-recommends \
    libeccodes-dev \
    && rm -rf /var/lib/apt/lists/*

WORKDIR /app

COPY ingest-hrrr/requirements.txt .
RUN pip install --no-cache-dir -r requirements.txt

COPY shared/ /app/shared/
COPY ingest-hrrr/ingest.py .

ENV TILE_DIR=/data/tiles
ENV COLOR_TABLE_PATH=/app/shared/color_tables.json
ENV POLL_INTERVAL=3600

CMD ["python", "-u", "ingest.py"]
```

- [ ] **Step 3: Update docker-compose build contexts**

Update `deploy/docker-compose.yml` ingest services to use `../services` as context:

```yaml
  ingest-mrms:
    build:
      context: ../services
      dockerfile: ingest-mrms/Dockerfile
    # ... rest unchanged

  ingest-hrrr:
    build:
      context: ../services
      dockerfile: ingest-hrrr/Dockerfile
    # ... rest unchanged
```

(Already correct in the compose file from Task 6.)

- [ ] **Step 4: Commit**

```bash
cd /home/vanillax/programming/radar-ng
git add services/ingest-mrms/Dockerfile services/ingest-hrrr/Dockerfile
git commit -m "fix: correct Dockerfile build contexts for shared module"
```

---

## Notes for App Plan

The app plan (separate document) will reference these backend endpoints:

- `GET http://localhost:8080/api/manifest.json` → layer timestamps
- `GET http://localhost:8080/tiles/{layer}/{timestamp}/{z}/{x}/{y}.png` → tile images
- `GET http://localhost:8080/api/forecast/{lat}/{lon}` → cached forecasts
- Layers: `radar`, `radar-hrrr`, `temperature`, `wind`, `cape`, `precip-type`
