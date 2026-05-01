#!/usr/bin/env python3
"""NHC active tropical cyclone ingest.

NHC publishes active storm data as shapefiles at:
    https://www.nhc.noaa.gov/gis/forecast/archive/{stormid}_5day_latest.zip
    https://www.nhc.noaa.gov/gis/best_track/{basin}_all.zip

For simplicity we use the GIS/JSON feed via hurricanes.gov, which NOAA exposes
as `CurrentStorms.json` — a compact index of every active Atlantic + Pacific
cyclone with position, intensity, track forecast points, and cone vertices:

    https://www.nhc.noaa.gov/CurrentStorms.json

We fetch it every POLL_INTERVAL seconds, transform into a single GeoJSON
FeatureCollection (one point per storm + one linestring per forecast track +
one polygon per cone), and write to /data/state/tropical.json for
tile-server's /api/tropical endpoint to serve.
"""

from __future__ import annotations

import json
import os
import sys
import time
from pathlib import Path

import httpx

sys.path.insert(0, str(Path(__file__).resolve().parent / "shared"))
sys.path.insert(0, "/app/shared")
from logger import get_logger, retry  # type: ignore  # noqa: E402

STATE_DIR = Path(os.environ.get("STATE_DIR", "/data/state"))
OUT_PATH = STATE_DIR / "tropical.json"
POLL_INTERVAL = int(os.environ.get("POLL_INTERVAL", "300"))
FEED_URL = os.environ.get("TROPICAL_FEED_URL", "https://www.nhc.noaa.gov/CurrentStorms.json")

log = get_logger("ingest-tropical")


@retry(attempts=4, base_delay=2.0, log=log, exceptions=(httpx.HTTPError,))
def fetch_feed(client: httpx.Client) -> dict:
    resp = client.get(FEED_URL, timeout=20)
    resp.raise_for_status()
    return resp.json()


def build_geojson(feed: dict) -> dict:
    features: list[dict] = []
    storms = feed.get("activeStorms", []) or feed.get("storms", []) or []

    for storm in storms:
        sid = storm.get("id") or storm.get("stormId") or ""
        name = storm.get("name") or storm.get("storm_name") or "Unknown"
        classification = storm.get("classification") or storm.get("intensity") or ""
        basin = storm.get("basin") or storm.get("binNumber") or ""

        # Current position — may live at different keys depending on feed version.
        lat = storm.get("latitudeNumeric") or storm.get("lat") or storm.get("latitude")
        lon = storm.get("longitudeNumeric") or storm.get("lon") or storm.get("longitude")
        wind = storm.get("intensity") or storm.get("windSpeed") or 0
        pressure = storm.get("pressure") or storm.get("minPressure")

        if lat is not None and lon is not None:
            features.append({
                "type": "Feature",
                "geometry": {"type": "Point", "coordinates": [float(lon), float(lat)]},
                "properties": {
                    "kind": "position",
                    "storm_id": sid,
                    "name": name,
                    "classification": classification,
                    "basin": basin,
                    "wind_mph": wind,
                    "pressure_mb": pressure,
                    "updated_at": storm.get("movement", {}).get("datetime")
                        or storm.get("lastUpdate"),
                },
            })

        # Forecast track — typically under `forecastTrack` / `trackForecast`.
        track = storm.get("forecastTrack") or storm.get("trackForecast") or []
        if isinstance(track, dict):
            track = track.get("points") or []
        coords = []
        for p in track:
            plat = p.get("lat") or p.get("latitudeNumeric")
            plon = p.get("lon") or p.get("longitudeNumeric")
            if plat is not None and plon is not None:
                coords.append([float(plon), float(plat)])
        if len(coords) >= 2:
            features.append({
                "type": "Feature",
                "geometry": {"type": "LineString", "coordinates": coords},
                "properties": {
                    "kind": "track",
                    "storm_id": sid,
                    "name": name,
                },
            })

        # Forecast cone — a polygon of uncertainty. Some feeds call it
        # `cone`, some `forecastCone`, some omit it.
        cone = storm.get("cone") or storm.get("forecastCone") or {}
        ring = cone.get("coordinates") or cone.get("points") or []
        if ring and isinstance(ring[0], list):
            coords_ring = []
            for pt in ring:
                if isinstance(pt, (list, tuple)) and len(pt) >= 2:
                    coords_ring.append([float(pt[0]), float(pt[1])])
            if len(coords_ring) >= 3:
                if coords_ring[0] != coords_ring[-1]:
                    coords_ring.append(coords_ring[0])
                features.append({
                    "type": "Feature",
                    "geometry": {"type": "Polygon", "coordinates": [coords_ring]},
                    "properties": {
                        "kind": "cone",
                        "storm_id": sid,
                        "name": name,
                    },
                })

    return {
        "type": "FeatureCollection",
        "features": features,
        "generated_at": time.time(),
        "storm_count": len(storms),
    }


def run() -> None:
    STATE_DIR.mkdir(parents=True, exist_ok=True)
    log.info("startup", extra={"poll_interval_s": POLL_INTERVAL, "feed": FEED_URL})

    # Emit empty collection immediately so the API endpoint never 404s.
    OUT_PATH.write_text(json.dumps({"type": "FeatureCollection", "features": [], "generated_at": 0}))

    with httpx.Client() as client:
        while True:
            started = time.time()
            try:
                feed = fetch_feed(client)
                geo = build_geojson(feed)
                tmp = OUT_PATH.with_suffix(".tmp")
                tmp.write_text(json.dumps(geo))
                tmp.replace(OUT_PATH)
                log.info(
                    "updated",
                    extra={"storm_count": geo["storm_count"], "features": len(geo["features"])},
                )
            except httpx.HTTPError as exc:
                log.warning("fetch_failed", extra={"err": str(exc)})
            except Exception as exc:  # noqa: BLE001
                log.exception("loop_error", extra={"err": str(exc)})

            elapsed = time.time() - started
            time.sleep(max(30.0, POLL_INTERVAL - elapsed))


if __name__ == "__main__":
    run()
