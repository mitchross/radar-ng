"""Location endpoints that keep the app talking only to this server.

Radar NG's clients (phone, Watch, widget) must never call a third party at
runtime. These routes source public data server-side and hand it back:

- GET /api/alerts?lat=&lon=           — NWS active alerts for a point. NWS does
  the zone/county matching, so we proxy its `point=` query rather than
  re-implement it; the response is the NWS GeoJSON unchanged.
- GET /api/geocode?q=                 — place search via a self-hosted Photon.
- GET /api/reverse-geocode?lat=&lon=  — nearest place name via the same Photon.

Upstream failures are 502 with `no-store`, never a cached or empty body: an
empty alert list must only ever mean "NWS said there are none".
"""

from __future__ import annotations

import os
import time
from collections import OrderedDict

import httpx
from fastapi import APIRouter, HTTPException, Query, Request
from fastapi.responses import JSONResponse

NWS_ALERTS_BASE = os.environ.get("NWS_POINT_ALERTS_URL", "https://api.weather.gov/alerts/active")
NWS_USER_AGENT = os.environ.get("NWS_USER_AGENT", "(radar-ng, admin@example.com)")
ALERTS_TTL_S = int(os.environ.get("ALERTS_TTL_S", "60"))
ALERTS_CACHE_MAX_ENTRIES = int(os.environ.get("ALERTS_CACHE_MAX_ENTRIES", "1024"))

# Photon base URL, e.g. http://photon.photon.svc.cluster.local:2322. Unset means
# this deployment has no geocoder; the routes answer 503 instead of reaching out.
PHOTON_URL = os.environ.get("PHOTON_URL", "").rstrip("/")
GEOCODE_TTL_S = int(os.environ.get("GEOCODE_TTL_S", "86400"))
GEOCODE_CACHE_MAX_ENTRIES = int(os.environ.get("GEOCODE_CACHE_MAX_ENTRIES", "1024"))
UPSTREAM_TIMEOUT_S = float(os.environ.get("PLACES_UPSTREAM_TIMEOUT_S", "6"))

# OSM place=* values that read as a settlement name for the user.
_SETTLEMENTS = {
    "city", "town", "village", "hamlet", "municipality",
    "suburb", "borough", "quarter", "neighbourhood", "locality",
}

router = APIRouter()


class _TTLCache:
    def __init__(self, ttl_s: int, max_entries: int) -> None:
        self.ttl_s = ttl_s
        self.max_entries = max_entries
        self._items: OrderedDict[str, tuple[float, object]] = OrderedDict()

    def get(self, key: str) -> object | None:
        hit = self._items.get(key)
        if hit is None:
            return None
        if time.time() - hit[0] >= self.ttl_s:
            self._items.pop(key, None)
            return None
        self._items.move_to_end(key)
        return hit[1]

    def put(self, key: str, value: object) -> None:
        self._items[key] = (time.time(), value)
        self._items.move_to_end(key)
        while len(self._items) > self.max_entries:
            self._items.popitem(last=False)

    def clear(self) -> None:
        self._items.clear()


_alerts_cache = _TTLCache(ALERTS_TTL_S, ALERTS_CACHE_MAX_ENTRIES)
_geocode_cache = _TTLCache(GEOCODE_TTL_S, GEOCODE_CACHE_MAX_ENTRIES)
_reverse_cache = _TTLCache(GEOCODE_TTL_S, GEOCODE_CACHE_MAX_ENTRIES)


def _client(request: Request) -> httpx.AsyncClient:
    return request.app.state.forecast_http


def _validate_point(lat: float, lon: float) -> None:
    if not (-90.0 <= lat <= 90.0 and -180.0 <= lon <= 180.0):
        raise HTTPException(422, "lat/lon out of range")


def _upstream_error() -> JSONResponse:
    return JSONResponse(
        {"error": "upstream_unavailable"},
        status_code=502,
        headers={"Cache-Control": "no-store"},
    )


@router.get("/api/alerts")
async def get_alerts(request: Request, lat: float, lon: float) -> JSONResponse:
    _validate_point(lat, lon)
    # 3 decimals (~110 m) matches what the app sends; it keeps a point on the
    # right side of a warning polygon while sharing one cache entry per block.
    key = f"{round(lat, 3)},{round(lon, 3)}"
    cached = _alerts_cache.get(key)
    if cached is not None:
        return JSONResponse(cached, headers={"Cache-Control": f"public, max-age={ALERTS_TTL_S}"})

    try:
        resp = await _client(request).get(
            NWS_ALERTS_BASE,
            params={"point": key},
            headers={"User-Agent": NWS_USER_AGENT, "Accept": "application/geo+json"},
            timeout=UPSTREAM_TIMEOUT_S,
        )
        resp.raise_for_status()
        body = resp.json()
    except (httpx.HTTPError, ValueError):
        return _upstream_error()
    if not isinstance(body, dict) or not isinstance(body.get("features"), list):
        return _upstream_error()

    _alerts_cache.put(key, body)
    return JSONResponse(body, headers={"Cache-Control": f"public, max-age={ALERTS_TTL_S}"})


def _not_configured() -> JSONResponse:
    return JSONResponse(
        {"error": "geocoding_not_configured"},
        status_code=503,
        headers={"Cache-Control": "no-store"},
    )


def _place_from_feature(feature: dict, *, reverse: bool) -> dict | None:
    """Photon GeoJSON feature → the app's SelectedPlace shape."""
    props = feature.get("properties") or {}
    coords = (feature.get("geometry") or {}).get("coordinates") or []
    if len(coords) < 2:
        return None
    lon, lat = coords[0], coords[1]
    is_settlement = props.get("osm_key") == "place" and props.get("osm_value") in _SETTLEMENTS
    if reverse:
        # A reverse hit is usually a street or building; the enclosing
        # settlement fields are what the label wants.
        name = (
            props.get("city")
            or props.get("locality")
            or props.get("district")
            or (props.get("name") if is_settlement else None)
            or props.get("county")
        )
    else:
        if not is_settlement:
            return None
        name = props.get("name")
    if not name or props.get("osm_id") is None:
        return None
    country_code = props.get("countrycode")
    return {
        "id": int(props["osm_id"]),
        "name": name,
        "latitude": lat,
        "longitude": lon,
        "admin1": props.get("state"),
        "country": props.get("country"),
        "countryCode": country_code.upper() if isinstance(country_code, str) else None,
    }


@router.get("/api/geocode")
async def geocode(
    request: Request,
    q: str = Query(..., min_length=2, max_length=100),
    limit: int = Query(8, ge=1, le=20),
) -> JSONResponse:
    if not PHOTON_URL:
        return _not_configured()
    query = " ".join(q.split())
    key = f"{query.lower()}|{limit}"
    cached = _geocode_cache.get(key)
    if cached is not None:
        return JSONResponse(cached, headers={"Cache-Control": "public, max-age=3600"})

    try:
        resp = await _client(request).get(
            f"{PHOTON_URL}/api",
            # Ask for extra rows: non-settlement hits are filtered out below.
            params={"q": query, "limit": str(limit * 3), "osm_tag": "place", "lang": "en"},
            timeout=UPSTREAM_TIMEOUT_S,
        )
        resp.raise_for_status()
        features = resp.json().get("features", [])
    except (httpx.HTTPError, ValueError, AttributeError):
        return _upstream_error()

    results: list[dict] = []
    seen: set[int] = set()
    for feature in features:
        place = _place_from_feature(feature, reverse=False)
        if place and place["id"] not in seen:
            seen.add(place["id"])
            results.append(place)
        if len(results) >= limit:
            break
    body = {"results": results}
    _geocode_cache.put(key, body)
    return JSONResponse(body, headers={"Cache-Control": "public, max-age=3600"})


@router.get("/api/reverse-geocode")
async def reverse_geocode(request: Request, lat: float, lon: float) -> JSONResponse:
    if not PHOTON_URL:
        return _not_configured()
    _validate_point(lat, lon)
    key = f"{round(lat, 3)},{round(lon, 3)}"
    cached = _reverse_cache.get(key)
    if cached is not None:
        return JSONResponse(cached, headers={"Cache-Control": "public, max-age=3600"})

    try:
        resp = await _client(request).get(
            f"{PHOTON_URL}/reverse",
            params={"lat": str(round(lat, 3)), "lon": str(round(lon, 3)), "limit": "1", "lang": "en"},
            timeout=UPSTREAM_TIMEOUT_S,
        )
        resp.raise_for_status()
        features = resp.json().get("features", [])
    except (httpx.HTTPError, ValueError, AttributeError):
        return _upstream_error()

    place = _place_from_feature(features[0], reverse=True) if features else None
    if place:
        # The label belongs to the requested point, not the matched building.
        place["latitude"], place["longitude"] = round(lat, 3), round(lon, 3)
    body = {"place": place}
    _reverse_cache.put(key, body)
    return JSONResponse(body, headers={"Cache-Control": "public, max-age=3600"})
