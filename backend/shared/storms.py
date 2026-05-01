"""Storm cell detection on MRMS reflectivity grids.

Simple connected-component approach: threshold ≥ THRESHOLD_DBZ, label regions
via scipy.ndimage, compute centroid + peak + area for each. Output as a
GeoJSON Point FeatureCollection suitable for an app overlay.

A "storm cell" here is intentionally coarse — this is for at-a-glance
annotation, not NWS-grade severe-weather algorithms. NEXRAD L3 NSTs would be
the right source for rigorous tracking.
"""

from __future__ import annotations

import json
import os
import time
from pathlib import Path

import numpy as np
from scipy import ndimage

THRESHOLD_DBZ = float(os.environ.get("STORM_THRESHOLD_DBZ", "40"))
MIN_PIXELS = int(os.environ.get("STORM_MIN_PIXELS", "5"))
MAX_STORMS = int(os.environ.get("STORM_MAX_CELLS", "500"))


def detect_storms(
    data: np.ndarray,
    lats: np.ndarray,
    lons: np.ndarray,
    *,
    timestamp: str,
) -> dict:
    """Return a GeoJSON FeatureCollection of storm cells above THRESHOLD_DBZ.

    `data` is a 2D reflectivity grid (dBZ). `lats` (len=H, descending or
    ascending) and `lons` (len=W) give the coord axes. Output lats/lons are
    interpolated from the pixel indices of each cell's centroid.
    """
    mask = data >= THRESHOLD_DBZ
    labels, n = ndimage.label(mask)
    if n == 0:
        return {
            "type": "FeatureCollection",
            "features": [],
            "generated_at": time.time(),
            "timestamp": timestamp,
        }

    # Compute per-label stats in bulk.
    indices = np.arange(1, n + 1)
    sizes = ndimage.sum(mask, labels, indices).astype(np.int64)
    peaks = ndimage.maximum(data, labels, indices)
    centroids = ndimage.center_of_mass(mask, labels, indices)

    H, W = data.shape
    # Normalize longitudes — MRMS native grid is 0..360; the app wants -180..180.
    lons_norm = np.where(lons > 180.0, lons - 360.0, lons)

    lat_top = float(lats[0])
    lat_bottom = float(lats[-1])
    lon_left = float(lons_norm[0])
    lon_right = float(lons_norm[-1])

    features: list[dict] = []
    for i in range(n):
        if sizes[i] < MIN_PIXELS:
            continue
        row, col = centroids[i]
        lat = lat_top + (lat_bottom - lat_top) * (row / max(1, H - 1))
        lon = lon_left + (lon_right - lon_left) * (col / max(1, W - 1))
        # Rough pixel → km² using ~2.5 km grid (MRMS at CONUS).
        area_km2 = float(sizes[i]) * 2.5 * 2.5
        features.append({
            "type": "Feature",
            "geometry": {"type": "Point", "coordinates": [lon, lat]},
            "properties": {
                "cell_id": int(i + 1),
                "peak_dbz": float(peaks[i]),
                "area_km2": area_km2,
                "pixel_count": int(sizes[i]),
            },
        })

    # Cap to the strongest N cells (protect against wide stratiform systems
    # that label into hundreds of small regions).
    features.sort(key=lambda f: f["properties"]["peak_dbz"], reverse=True)
    features = features[:MAX_STORMS]

    return {
        "type": "FeatureCollection",
        "features": features,
        "generated_at": time.time(),
        "timestamp": timestamp,
        "threshold_dbz": THRESHOLD_DBZ,
        "cell_count": len(features),
    }


def write_storms_json(state_dir: Path, data: np.ndarray, lats: np.ndarray, lons: np.ndarray, timestamp: str) -> None:
    state_dir.mkdir(parents=True, exist_ok=True)
    payload = detect_storms(data, lats, lons, timestamp=timestamp)
    tmp = state_dir / "storms.json.tmp"
    tmp.write_text(json.dumps(payload))
    tmp.replace(state_dir / "storms.json")
