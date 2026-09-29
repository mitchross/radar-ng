"""Temporal activities for NHC active tropical cyclones.

Single-shot poll: GET CurrentStorms.json, transform to GeoJSON, atomic write.
"""

from __future__ import annotations

import asyncio
import json
import os
import time
from concurrent.futures import ThreadPoolExecutor
from dataclasses import dataclass
from pathlib import Path

import httpx
from temporalio import activity

from backend.shared.logger import get_logger


STATE_DIR = Path(os.environ.get("STATE_DIR", "/data/state"))
OUT_PATH = STATE_DIR / "tropical.json"
FEED_URL = os.environ.get("TROPICAL_FEED_URL", "https://www.nhc.noaa.gov/CurrentStorms.json")
# CurrentStorms.json only links KMZ/zip files; this service serves the same cone/track as GeoJSON.
GIS_URL = os.environ.get(
    "TROPICAL_GIS_URL",
    "https://mapservices.weather.noaa.gov/tropical/rest/services/tropical/NHC_tropical_weather/MapServer",
)
GIS_LAYERS = {"cone": "Forecast Cone", "track": "Forecast Track", "points": "Forecast Points"}
KT_TO_MPH = 1.15078

log = get_logger("ingest-tropical-activities")


@dataclass
class TropicalResult:
    storm_count: int
    feature_count: int


def _num(value: object) -> float | None:
    try:
        return float(value)  # NHC sends numbers as strings ("100")
    except (TypeError, ValueError):
        return None


def _saffir_simpson(wind_kt: float | None) -> int | None:
    if wind_kt is None or wind_kt < 64:
        return None
    return 1 + sum(wind_kt >= t for t in (83, 96, 113, 137))


def _gis_features(fc: dict | None) -> list[dict]:
    return [f for f in (fc or {}).get("features", []) if f.get("geometry")]


def _build_geojson(feed: dict, gis: dict[str, dict[str, dict]] | None = None) -> dict:
    """NHC feed + optional per-bin GIS layers ({"EP2": {"cone": fc, ...}}) → one FeatureCollection."""
    gis = gis or {}
    features: list[dict] = []
    storms = feed.get("activeStorms", []) or feed.get("storms", []) or []

    for storm in storms:
        sid = storm.get("id") or storm.get("stormId") or ""
        name = storm.get("name") or storm.get("storm_name") or "Unknown"
        classification = storm.get("classification") or ""
        basin = storm.get("binNumber") or storm.get("basin") or ""
        lat = storm.get("latitudeNumeric") or storm.get("lat")
        lon = storm.get("longitudeNumeric") or storm.get("lon")
        wind_kt = _num(storm.get("intensity") or storm.get("windSpeed"))
        pressure = _num(storm.get("pressure") or storm.get("minPressure"))
        base = {"storm_id": sid, "name": name}

        if lat is not None and lon is not None:
            features.append({
                "type": "Feature",
                "geometry": {"type": "Point", "coordinates": [float(lon), float(lat)]},
                "properties": {
                    "kind": "position", **base,
                    "classification": classification, "basin": basin,
                    "wind_kt": wind_kt,
                    "wind_mph": round(wind_kt * KT_TO_MPH) if wind_kt is not None else None,
                    "category": _saffir_simpson(wind_kt),
                    "pressure_mb": pressure,
                    "movement_dir_deg": _num(storm.get("movementDir")),
                    "movement_mph": _num(storm.get("movementSpeed")),
                    "updated_at": storm.get("lastUpdate"),
                    "advisory_url": (storm.get("publicAdvisory") or {}).get("url"),
                },
            })

        layers = gis.get(basin, {})
        for f in _gis_features(layers.get("cone")):
            features.append({"type": "Feature", "geometry": f["geometry"],
                             "properties": {"kind": "cone", **base}})
        for f in _gis_features(layers.get("track")):
            features.append({"type": "Feature", "geometry": f["geometry"],
                             "properties": {"kind": "track", **base}})
        for f in _gis_features(layers.get("points")):
            props = f.get("properties") or {}
            kt = _num(props.get("maxwind"))
            features.append({
                "type": "Feature", "geometry": f["geometry"],
                "properties": {
                    "kind": "forecast_point", **base,
                    "tau_h": _num(props.get("tau")),
                    "label": props.get("datelbl") or props.get("fldatelbl"),
                    "storm_type": props.get("dvlbl"),
                    "wind_kt": kt,
                    "wind_mph": round(kt * KT_TO_MPH) if kt is not None else None,
                    "category": _saffir_simpson(kt),
                },
            })

    return {
        "type": "FeatureCollection",
        "features": features,
        "generated_at": time.time(),
        "storm_count": len(storms),
    }


def _fetch_gis(client: httpx.Client, bins: list[str]) -> dict[str, dict[str, dict]]:
    """Best effort: a GIS outage still publishes storm positions from the primary feed."""
    if not bins:
        return {}
    try:
        index = client.get(f"{GIS_URL}?f=json", timeout=15)
        index.raise_for_status()
        ids = {layer["name"]: layer["id"] for layer in index.json().get("layers", [])}
    except (httpx.HTTPError, ValueError, KeyError) as exc:
        log.warning("gis_index_failed", extra={"err": str(exc)})
        return {}

    def _query(layer_id: int) -> dict | None:
        try:
            resp = client.get(
                f"{GIS_URL}/{layer_id}/query",
                params={"where": "1=1", "outFields": "*", "f": "geojson"},
                timeout=15,
            )
            resp.raise_for_status()
            return resp.json()
        except (httpx.HTTPError, ValueError) as exc:
            log.warning("gis_layer_failed", extra={"layer": layer_id, "err": str(exc)})
            return None

    jobs = {
        (b, kind): ids[f"{b} {suffix}"]
        for b in bins
        for kind, suffix in GIS_LAYERS.items()
        if f"{b} {suffix}" in ids
    }
    with ThreadPoolExecutor(max_workers=8) as pool:
        results = dict(zip(jobs, pool.map(_query, jobs.values())))
    out: dict[str, dict[str, dict]] = {}
    for (b, kind), fc in results.items():
        if fc:
            out.setdefault(b, {})[kind] = fc
    return out


@activity.defn(name="tropical_fetch_and_publish")
async def tropical_fetch_and_publish() -> TropicalResult:
    def _go() -> TropicalResult:
        STATE_DIR.mkdir(parents=True, exist_ok=True)
        with httpx.Client() as client:
            resp = client.get(FEED_URL, timeout=20)
            resp.raise_for_status()
            feed = resp.json()
            bins = [s.get("binNumber") for s in feed.get("activeStorms", []) if s.get("binNumber")]
            gis = _fetch_gis(client, bins)
        geo = _build_geojson(feed, gis)
        tmp = OUT_PATH.with_suffix(".tmp")
        tmp.write_text(json.dumps(geo))
        tmp.replace(OUT_PATH)
        log.info("updated", extra={"storm_count": geo["storm_count"], "features": len(geo["features"])})
        return TropicalResult(storm_count=geo["storm_count"], feature_count=len(geo["features"]))

    return await asyncio.to_thread(_go)
