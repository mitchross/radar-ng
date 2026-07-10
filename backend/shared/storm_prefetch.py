"""Build a location-aware three-bbox storm tile prefetch plan."""

from __future__ import annotations

import math
import os
from datetime import datetime, timedelta
from pathlib import Path
from urllib.parse import quote, urlencode

from backend.shared.manifest import read_manifest_file

MAX_PREFETCH_DISTANCE_KM = float(os.environ.get("STORM_PREFETCH_MAX_DISTANCE_KM", "500"))
MIN_ZOOM = 4
MAX_ZOOM = {"radar": 8, "nowcast": 7}
EARTH_RADIUS_KM = 6371.0088


def _lat_lon_to_tile(lat: float, lon: float, zoom: int) -> tuple[int, int]:
    n = 2**zoom
    x = int((lon + 180.0) / 360.0 * n)
    clipped_lat = max(-85.05112878, min(85.05112878, lat))
    lat_rad = math.radians(clipped_lat)
    y = int((1.0 - math.asinh(math.tan(lat_rad)) / math.pi) / 2.0 * n)
    return max(0, min(x, n - 1)), max(0, min(y, n - 1))


def _parse_timestamp(value: object) -> datetime | None:
    try:
        return datetime.fromisoformat(str(value).replace("Z", "+00:00"))
    except (TypeError, ValueError):
        return None


def _distance_km(lat: float, lon: float, other_lat: float, other_lon: float) -> float:
    phi1 = math.radians(lat)
    phi2 = math.radians(other_lat)
    d_phi = phi2 - phi1
    d_lambda = math.radians(other_lon - lon)
    h = math.sin(d_phi / 2) ** 2 + math.cos(phi1) * math.cos(phi2) * math.sin(d_lambda / 2) ** 2
    return 2 * EARTH_RADIUS_KM * math.asin(min(1.0, math.sqrt(h)))


def _select_storm(storms: dict, lat: float, lon: float) -> dict | None:
    ranked: list[tuple[float, dict]] = []
    for feature in storms.get("features", []):
        try:
            props = feature["properties"]
            bboxes = props["predicted_bboxes"]
            distances = []
            for prediction in bboxes:
                west, south, east, north = prediction["bbox"]
                distances.append(_distance_km(lat, lon, (south + north) / 2, (west + east) / 2))
            distance = min(distances)
            # Distance dominates. Intensity and area only break ties between
            # nearby cells, favoring the storm a user is more likely to inspect.
            severity = max(0.0, float(props.get("peak_dbz", 40.0)) - 40.0)
            area = max(1.0, float(props.get("area_km2", 1.0)))
            score = distance - min(35.0, severity * 0.8 + math.log10(area) * 3.0)
            ranked.append((score, feature))
        except (KeyError, TypeError, ValueError):
            continue
    if not ranked:
        return None
    ranked.sort(key=lambda item: item[0])
    selected = ranked[0][1]
    selected_lon, selected_lat = selected["geometry"]["coordinates"]
    if _distance_km(lat, lon, selected_lat, selected_lon) > MAX_PREFETCH_DISTANCE_KM:
        return None
    return selected


def _nearest_timestamp(timestamps: list[str], target: datetime, *, prefer_past: bool = False) -> str | None:
    candidates: list[tuple[float, str]] = []
    for value in timestamps:
        parsed = _parse_timestamp(value)
        if parsed is None or (prefer_past and parsed > target):
            continue
        candidates.append((abs((parsed - target).total_seconds()), value))
    return min(candidates, default=(0.0, None), key=lambda item: item[0])[1]


def _frame_for_lead(manifest: dict, anchor: datetime, lead_minutes: int) -> tuple[str, str] | None:
    if lead_minutes == 0:
        radar = manifest.get("layers", {}).get("radar", {}).get("timestamps", [])
        timestamp = _nearest_timestamp(radar, anchor, prefer_past=True)
        return ("radar", timestamp) if timestamp else None

    nowcast = manifest.get("layers", {}).get("nowcast", {}).get("timestamps", [])
    target = anchor + timedelta(minutes=lead_minutes)
    timestamp = _nearest_timestamp(nowcast, target)
    if timestamp is None:
        return None
    parsed = _parse_timestamp(timestamp)
    if parsed is None or abs((parsed - target).total_seconds()) > 8 * 60:
        return None
    return "nowcast", timestamp


def _tiles_for_bbox(bbox: list[float], zoom: int) -> list[tuple[int, int]]:
    west, south, east, north = bbox
    x_min, y_min = _lat_lon_to_tile(north, west, zoom)
    x_max, y_max = _lat_lon_to_tile(south, east, zoom)
    return [
        (x, y)
        for x in range(min(x_min, x_max), max(x_min, x_max) + 1)
        for y in range(min(y_min, y_max), max(y_min, y_max) + 1)
    ]


def _tile_path(layer: str, palette: str, timestamp: str, zoom: int, x: int, y: int) -> str:
    parts = ("tiles", layer, palette, timestamp, str(zoom), str(x), f"{y}.png")
    return "/" + "/".join(quote(part, safe=":+-.T_") for part in parts)


def build_storm_prefetch_plan(
    *,
    storms: dict,
    state_dir: str | Path,
    tile_dir: str | Path,
    base_url: str,
    lat: float,
    lon: float,
    zoom: int,
    palette: str,
) -> dict:
    """Return exactly three predicted bboxes and their existing tile URLs."""
    selected = _select_storm(storms, lat, lon)
    anchor = _parse_timestamp(storms.get("timestamp"))
    if selected is None or anchor is None:
        return {"plan_id": None, "storm_cell_id": None, "bboxes": [], "tile_urls": []}

    manifest = read_manifest_file(state_dir)
    properties = selected["properties"]
    output_bboxes: list[dict] = []
    all_urls: list[str] = []
    base_url = base_url.rstrip("/")
    requested_zoom = max(MIN_ZOOM, int(zoom))

    for prediction in properties["predicted_bboxes"][:3]:
        lead = int(prediction["lead_minutes"])
        bbox = [float(value) for value in prediction["bbox"]]
        frame = _frame_for_lead(manifest, anchor, lead)
        if frame is None:
            output_bboxes.append({
                "lead_minutes": lead,
                "bbox": bbox,
                "layer": None,
                "timestamp": None,
                "zoom": requested_zoom,
                "style_url": None,
                "tile_urls": [],
            })
            continue

        layer, timestamp = frame
        frame_zoom = min(requested_zoom, MAX_ZOOM[layer])
        urls: list[str] = []
        for x, y in _tiles_for_bbox(bbox, frame_zoom):
            relative = _tile_path(layer, palette, timestamp, frame_zoom, x, y)
            disk_path = Path(tile_dir) / relative.removeprefix("/tiles/")
            if disk_path.is_file():
                urls.append(f"{base_url}{relative}")
        query = urlencode({
            "layer": layer,
            "palette": palette,
            "timestamp": timestamp,
            "zoom": frame_zoom,
        })
        output_bboxes.append({
            "lead_minutes": lead,
            "bbox": bbox,
            "layer": layer,
            "timestamp": timestamp,
            "zoom": frame_zoom,
            "style_url": f"{base_url}/api/storm-prefetch/style.json?{query}",
            "tile_urls": urls,
        })
        all_urls.extend(urls)

    cell_id = properties["cell_id"]
    return {
        "plan_id": f"{storms['timestamp']}:{cell_id}:{palette}:{requested_zoom}",
        "storm_cell_id": cell_id,
        "generated_at": storms.get("generated_at"),
        "tracking_vector": properties.get("tracking_vector"),
        "bboxes": output_bboxes,
        "tile_urls": list(dict.fromkeys(all_urls)),
    }
