"""Storm-cell detection, tracking, and short-horizon prefetch bounds.

Cells are connected regions at or above ``THRESHOLD_DBZ``.  Consecutive
frames are associated by centroid distance, which gives each surviving cell a
stable id and a motion vector.  The vector is extrapolated to 0, 5, and 10
minutes so the API can build a small MapLibre-native prefetch plan instead of
asking a phone to enumerate a regional tile grid.

This remains an at-a-glance product, not an NWS-grade severe-weather tracker.
NEXRAD L3 NSTs would be the right source for rigorous tracking.
"""

from __future__ import annotations

import json
import math
import os
import tempfile
import time
from datetime import datetime
from pathlib import Path

import numpy as np
from scipy import ndimage
from scipy.optimize import linear_sum_assignment

THRESHOLD_DBZ = float(os.environ.get("STORM_THRESHOLD_DBZ", "40"))
MIN_PIXELS = int(os.environ.get("STORM_MIN_PIXELS", "5"))
MAX_STORMS = int(os.environ.get("STORM_MAX_CELLS", "500"))
MAX_TRACK_SPEED_KMH = float(os.environ.get("STORM_MAX_TRACK_SPEED_KMH", "160"))
MIN_TRACK_RADIUS_KM = float(os.environ.get("STORM_MIN_TRACK_RADIUS_KM", "20"))
PREFETCH_PADDING_KM = float(os.environ.get("STORM_PREFETCH_PADDING_KM", "12"))
PREDICTION_MINUTES = (0, 5, 10)
EARTH_RADIUS_KM = 6371.0088


def _parse_timestamp(value: object) -> datetime | None:
    try:
        return datetime.fromisoformat(str(value).replace("Z", "+00:00"))
    except (TypeError, ValueError):
        return None


def _haversine_km(a: tuple[float, float], b: tuple[float, float]) -> float:
    lon1, lat1 = a
    lon2, lat2 = b
    phi1 = math.radians(lat1)
    phi2 = math.radians(lat2)
    d_phi = phi2 - phi1
    d_lambda = math.radians(lon2 - lon1)
    h = math.sin(d_phi / 2) ** 2 + math.cos(phi1) * math.cos(phi2) * math.sin(d_lambda / 2) ** 2
    return 2 * EARTH_RADIUS_KM * math.asin(min(1.0, math.sqrt(h)))


def _component_bbox(
    row_slice: slice,
    col_slice: slice,
    lats: np.ndarray,
    lons: np.ndarray,
) -> list[float]:
    """Return a padded [west, south, east, north] component bbox."""
    row_start = int(row_slice.start or 0)
    row_stop = max(row_start, int(row_slice.stop or row_start + 1) - 1)
    col_start = int(col_slice.start or 0)
    col_stop = max(col_start, int(col_slice.stop or col_start + 1) - 1)

    lat_values = (float(lats[row_start]), float(lats[row_stop]))
    lon_values = (float(lons[col_start]), float(lons[col_stop]))
    south, north = min(lat_values), max(lat_values)
    west, east = min(lon_values), max(lon_values)

    center_lat = (south + north) / 2
    lat_padding = PREFETCH_PADDING_KM / 110.574
    lon_scale = max(1e-6, 111.320 * math.cos(math.radians(center_lat)))
    lon_padding = PREFETCH_PADDING_KM / lon_scale
    return [west - lon_padding, south - lat_padding, east + lon_padding, north + lat_padding]


def _translate_bbox(
    bbox: list[float],
    *,
    center_lat: float,
    east_kmh: float,
    north_kmh: float,
    lead_minutes: int,
) -> list[float]:
    hours = lead_minutes / 60.0
    lat_delta = north_kmh * hours / 110.574
    lon_scale = max(1e-6, 111.320 * math.cos(math.radians(center_lat)))
    lon_delta = east_kmh * hours / lon_scale
    return [round(bbox[0] + lon_delta, 6), round(bbox[1] + lat_delta, 6), round(bbox[2] + lon_delta, 6), round(bbox[3] + lat_delta, 6)]


def _previous_motion(properties: dict) -> tuple[float, float]:
    vector = properties.get("tracking_vector")
    if not isinstance(vector, dict):
        return 0.0, 0.0
    try:
        return float(vector.get("east_kmh", 0.0)), float(vector.get("north_kmh", 0.0))
    except (TypeError, ValueError):
        return 0.0, 0.0


def _attach_tracking(features: list[dict], previous: dict | None, timestamp: str) -> None:
    """Mutate features with stable ids, vectors, and three predicted bboxes."""
    previous_features = previous.get("features", []) if isinstance(previous, dict) else []
    previous_features = [f for f in previous_features if isinstance(f, dict)]
    current_dt = _parse_timestamp(timestamp)
    previous_dt = _parse_timestamp(previous.get("timestamp")) if isinstance(previous, dict) else None
    elapsed_minutes = 0.0
    if current_dt is not None and previous_dt is not None:
        elapsed_minutes = (current_dt - previous_dt).total_seconds() / 60.0

    previous_ids = []
    for feature in previous_features:
        try:
            previous_ids.append(int(feature.get("properties", {}).get("cell_id")))
        except (TypeError, ValueError):
            continue
    next_id = max(previous_ids, default=0) + 1
    matches: dict[int, int] = {}

    # Backfilled/out-of-order frames must not produce backwards vectors.
    if features and previous_features and 0 < elapsed_minutes <= 15:
        distances = np.full((len(features), len(previous_features)), 1e9, dtype=np.float64)
        for current_index, current in enumerate(features):
            current_coords = tuple(current["geometry"]["coordinates"])
            for previous_index, old in enumerate(previous_features):
                try:
                    old_coords = tuple(old["geometry"]["coordinates"])
                    distances[current_index, previous_index] = _haversine_km(current_coords, old_coords)
                except (KeyError, TypeError, ValueError):
                    continue
        rows, cols = linear_sum_assignment(distances)
        max_distance = max(MIN_TRACK_RADIUS_KM, MAX_TRACK_SPEED_KMH * elapsed_minutes / 60.0)
        for row, col in zip(rows.tolist(), cols.tolist()):
            if distances[row, col] <= max_distance:
                matches[row] = col

    for index, feature in enumerate(features):
        props = feature["properties"]
        lon, lat = feature["geometry"]["coordinates"]
        east_kmh = 0.0
        north_kmh = 0.0
        confidence = 0.0

        if index in matches:
            old = previous_features[matches[index]]
            old_props = old.get("properties", {})
            old_lon, old_lat = old["geometry"]["coordinates"]
            hours = elapsed_minutes / 60.0
            measured_north = (lat - old_lat) * 110.574 / hours
            measured_east = (lon - old_lon) * 111.320 * math.cos(math.radians((lat + old_lat) / 2)) / hours
            old_east, old_north = _previous_motion(old_props)
            # Damp cell-centroid wobble while still reacting within one frame.
            east_kmh = 0.7 * measured_east + 0.3 * old_east
            north_kmh = 0.7 * measured_north + 0.3 * old_north
            props["cell_id"] = int(old_props.get("cell_id", next_id))
            confidence = min(1.0, float(old_props.get("tracking_confidence", 0.0)) + 0.35)
        else:
            props["cell_id"] = next_id
            next_id += 1

        speed = math.hypot(east_kmh, north_kmh)
        if speed > MAX_TRACK_SPEED_KMH:
            scale = MAX_TRACK_SPEED_KMH / speed
            east_kmh *= scale
            north_kmh *= scale
            speed = MAX_TRACK_SPEED_KMH
        bearing = (math.degrees(math.atan2(east_kmh, north_kmh)) + 360.0) % 360.0 if speed else 0.0
        props["tracking_vector"] = {
            "east_kmh": round(east_kmh, 3),
            "north_kmh": round(north_kmh, 3),
            "speed_kmh": round(speed, 3),
            "bearing_deg": round(bearing, 2),
        }
        props["tracking_confidence"] = round(confidence, 2)
        props["predicted_bboxes"] = [
            {
                "lead_minutes": lead,
                "bbox": _translate_bbox(
                    props["bbox"],
                    center_lat=lat,
                    east_kmh=east_kmh,
                    north_kmh=north_kmh,
                    lead_minutes=lead,
                ),
            }
            for lead in PREDICTION_MINUTES
        ]


def detect_storms(
    data: np.ndarray,
    lats: np.ndarray,
    lons: np.ndarray,
    *,
    timestamp: str,
    previous: dict | None = None,
) -> dict:
    """Return tracked storm cells as a GeoJSON FeatureCollection."""
    mask = data >= THRESHOLD_DBZ
    labels, n = ndimage.label(mask)
    if n == 0:
        return {
            "type": "FeatureCollection",
            "features": [],
            "generated_at": time.time(),
            "timestamp": timestamp,
            "prediction_minutes": list(PREDICTION_MINUTES),
        }

    indices = np.arange(1, n + 1)
    sizes = ndimage.sum(mask, labels, indices).astype(np.int64)
    peaks = ndimage.maximum(data, labels, indices)
    centroids = ndimage.center_of_mass(mask, labels, indices)
    slices = ndimage.find_objects(labels)

    height, width = data.shape
    lons_norm = np.where(lons > 180.0, lons - 360.0, lons)
    lat_top = float(lats[0])
    lat_bottom = float(lats[-1])
    lon_left = float(lons_norm[0])
    lon_right = float(lons_norm[-1])

    features: list[dict] = []
    for i in range(n):
        component_slice = slices[i]
        if sizes[i] < MIN_PIXELS or component_slice is None:
            continue
        row, col = centroids[i]
        lat = lat_top + (lat_bottom - lat_top) * (row / max(1, height - 1))
        lon = lon_left + (lon_right - lon_left) * (col / max(1, width - 1))
        area_km2 = float(sizes[i]) * 2.5 * 2.5
        features.append({
            "type": "Feature",
            "geometry": {"type": "Point", "coordinates": [lon, lat]},
            "properties": {
                "peak_dbz": float(peaks[i]),
                "area_km2": area_km2,
                "pixel_count": int(sizes[i]),
                "bbox": _component_bbox(component_slice[0], component_slice[1], lats, lons_norm),
            },
        })

    features.sort(key=lambda feature: feature["properties"]["peak_dbz"], reverse=True)
    features = features[:MAX_STORMS]
    _attach_tracking(features, previous, timestamp)

    return {
        "type": "FeatureCollection",
        "features": features,
        "generated_at": time.time(),
        "timestamp": timestamp,
        "threshold_dbz": THRESHOLD_DBZ,
        "cell_count": len(features),
        "prediction_minutes": list(PREDICTION_MINUTES),
        "tracking_vectors": [
            {
                "cell_id": feature["properties"]["cell_id"],
                **feature["properties"]["tracking_vector"],
            }
            for feature in features
        ],
    }


def write_storms_json(
    state_dir: Path,
    data: np.ndarray,
    lats: np.ndarray,
    lons: np.ndarray,
    timestamp: str,
) -> None:
    """Atomically advance storms.json, ignoring stale/backfilled frames."""
    import fcntl

    state_dir.mkdir(parents=True, exist_ok=True)
    path = state_dir / "storms.json"
    lock_path = state_dir / "storms.lock"
    with lock_path.open("w") as lock:
        fcntl.flock(lock.fileno(), fcntl.LOCK_EX)
        try:
            previous = json.loads(path.read_text())
        except (OSError, json.JSONDecodeError):
            previous = None

        current_dt = _parse_timestamp(timestamp)
        previous_dt = _parse_timestamp(previous.get("timestamp")) if isinstance(previous, dict) else None
        if current_dt is not None and previous_dt is not None and current_dt <= previous_dt:
            return

        payload = detect_storms(data, lats, lons, timestamp=timestamp, previous=previous)
        fd, tmp_name = tempfile.mkstemp(prefix=".storms.", suffix=".tmp", dir=str(state_dir))
        try:
            with os.fdopen(fd, "w") as fh:
                json.dump(payload, fh, separators=(",", ":"))
                fh.write("\n")
            os.replace(tmp_name, path)
        finally:
            try:
                os.unlink(tmp_name)
            except FileNotFoundError:
                pass
        fcntl.flock(lock.fileno(), fcntl.LOCK_UN)
