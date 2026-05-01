"""Temporal activities for NHC active tropical cyclones.

Single-shot poll: GET CurrentStorms.json, transform to GeoJSON, atomic write.
"""

from __future__ import annotations

import asyncio
import json
import os
import time
from dataclasses import dataclass
from pathlib import Path

import httpx
from temporalio import activity

from backend.shared.logger import get_logger


STATE_DIR = Path(os.environ.get("STATE_DIR", "/data/state"))
OUT_PATH = STATE_DIR / "tropical.json"
FEED_URL = os.environ.get("TROPICAL_FEED_URL", "https://www.nhc.noaa.gov/CurrentStorms.json")

log = get_logger("ingest-tropical-activities")


@dataclass
class TropicalResult:
    storm_count: int
    feature_count: int


def _build_geojson(feed: dict) -> dict:
    features: list[dict] = []
    storms = feed.get("activeStorms", []) or feed.get("storms", []) or []

    for storm in storms:
        sid = storm.get("id") or storm.get("stormId") or ""
        name = storm.get("name") or storm.get("storm_name") or "Unknown"
        classification = storm.get("classification") or storm.get("intensity") or ""
        basin = storm.get("basin") or storm.get("binNumber") or ""
        lat = storm.get("latitudeNumeric") or storm.get("lat") or storm.get("latitude")
        lon = storm.get("longitudeNumeric") or storm.get("lon") or storm.get("longitude")
        wind = storm.get("intensity") or storm.get("windSpeed") or 0
        pressure = storm.get("pressure") or storm.get("minPressure")

        if lat is not None and lon is not None:
            features.append({
                "type": "Feature",
                "geometry": {"type": "Point", "coordinates": [float(lon), float(lat)]},
                "properties": {
                    "kind": "position", "storm_id": sid, "name": name,
                    "classification": classification, "basin": basin,
                    "wind_mph": wind, "pressure_mb": pressure,
                    "updated_at": storm.get("movement", {}).get("datetime") or storm.get("lastUpdate"),
                },
            })

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
                "properties": {"kind": "track", "storm_id": sid, "name": name},
            })

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
                    "properties": {"kind": "cone", "storm_id": sid, "name": name},
                })

    return {
        "type": "FeatureCollection",
        "features": features,
        "generated_at": time.time(),
        "storm_count": len(storms),
    }


@activity.defn(name="tropical_fetch_and_publish")
async def tropical_fetch_and_publish() -> TropicalResult:
    def _go() -> TropicalResult:
        STATE_DIR.mkdir(parents=True, exist_ok=True)
        with httpx.Client() as client:
            resp = client.get(FEED_URL, timeout=20)
            resp.raise_for_status()
            feed = resp.json()
        geo = _build_geojson(feed)
        tmp = OUT_PATH.with_suffix(".tmp")
        tmp.write_text(json.dumps(geo))
        tmp.replace(OUT_PATH)
        log.info("updated", extra={"storm_count": geo["storm_count"], "features": len(geo["features"])})
        return TropicalResult(storm_count=geo["storm_count"], feature_count=len(geo["features"]))

    return await asyncio.to_thread(_go)
