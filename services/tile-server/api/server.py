#!/usr/bin/env python3
"""Tile server API.

Endpoints:
- GET /api/manifest.json      — tile layers + available timestamps
- GET /api/forecast/{lat}/{lon} — Open-Meteo proxy (public upstream OR self-hosted
                                  if OPEN_METEO_BASE env points at a local instance)
- GET /api/health             — ok / degraded (degrades when MRMS tiles are stale)
- GET /api/metrics            — Prometheus-style counters + gauges
"""

from __future__ import annotations

import os
import time
from datetime import datetime, timezone
from pathlib import Path

import httpx
from fastapi import FastAPI
from fastapi.responses import JSONResponse, PlainTextResponse

TILE_DIR = os.environ.get("TILE_DIR", "/data/tiles")
OPEN_METEO_BASE = os.environ.get("OPEN_METEO_BASE", "https://api.open-meteo.com/v1/forecast")
MRMS_MAX_AGE_S = int(os.environ.get("MRMS_MAX_AGE_S", "600"))  # tiles older than this → degraded
FORECAST_TTL = int(os.environ.get("FORECAST_TTL_S", "900"))  # 15min

CURRENT_FIELDS = (
    "temperature_2m,relative_humidity_2m,apparent_temperature,weather_code,"
    "wind_speed_10m,wind_direction_10m,wind_gusts_10m,"
    "dew_point_2m,surface_pressure"
)
HOURLY_FIELDS = (
    "temperature_2m,precipitation_probability,weather_code,wind_speed_10m,"
    "relative_humidity_2m,apparent_temperature"
)
DAILY_FIELDS = (
    "temperature_2m_max,temperature_2m_min,weather_code,precipitation_sum,"
    "precipitation_probability_max,uv_index_max,sunrise,sunset"
)

app = FastAPI(title="StormScope Tile API")

_forecast_cache: dict[str, tuple[float, dict]] = {}
_metrics = {
    "forecast_requests_total": 0,
    "forecast_cache_hits_total": 0,
    "forecast_upstream_errors_total": 0,
    "manifest_requests_total": 0,
}


def _newest_mtime(path: Path) -> float | None:
    if not path.exists():
        return None
    try:
        stamps = [p.stat().st_mtime for p in path.iterdir() if p.is_dir()]
        return max(stamps) if stamps else None
    except OSError:
        return None


@app.get("/api/manifest.json")
async def get_manifest() -> JSONResponse:
    _metrics["manifest_requests_total"] += 1
    tile_base = Path(TILE_DIR)
    layers: dict[str, dict] = {}

    if tile_base.exists():
        for layer_dir in sorted(tile_base.iterdir()):
            if not layer_dir.is_dir():
                continue
            timestamps: list[str] = []
            for ts_dir in sorted(layer_dir.iterdir()):
                if ts_dir.is_dir() and any(ts_dir.iterdir()):
                    timestamps.append(ts_dir.name)
            if timestamps:
                layers[layer_dir.name] = {
                    "timestamps": timestamps,
                    "latest": timestamps[-1],
                }

    return JSONResponse(
        {
            "layers": layers,
            "tile_url_template": "/tiles/{layer}/{timestamp}/{z}/{x}/{y}.png",
            "updated_at": datetime.now(timezone.utc).isoformat(),
        }
    )


@app.get("/api/forecast/{lat}/{lon}")
async def get_forecast(lat: float, lon: float) -> JSONResponse:
    _metrics["forecast_requests_total"] += 1
    grid_lat = round(lat, 1)
    grid_lon = round(lon, 1)
    cache_key = f"{grid_lat},{grid_lon}"

    cached = _forecast_cache.get(cache_key)
    if cached and time.time() - cached[0] < FORECAST_TTL:
        _metrics["forecast_cache_hits_total"] += 1
        return JSONResponse(cached[1])

    params = {
        "latitude": str(grid_lat),
        "longitude": str(grid_lon),
        "current": CURRENT_FIELDS,
        "hourly": HOURLY_FIELDS,
        "daily": DAILY_FIELDS,
        "temperature_unit": "fahrenheit",
        "wind_speed_unit": "mph",
        "precipitation_unit": "inch",
        "timezone": "auto",
        "forecast_days": "7",
    }

    try:
        async with httpx.AsyncClient() as client:
            resp = await client.get(OPEN_METEO_BASE, params=params, timeout=15)
            resp.raise_for_status()
            data = resp.json()
    except httpx.HTTPError as exc:
        _metrics["forecast_upstream_errors_total"] += 1
        return JSONResponse(
            {"error": "upstream_unavailable", "detail": str(exc), "upstream": OPEN_METEO_BASE},
            status_code=502,
        )

    _forecast_cache[cache_key] = (time.time(), data)
    return JSONResponse(data)


@app.get("/api/health")
async def health() -> JSONResponse:
    """Returns 'ok' when MRMS tiles are fresh, 'degraded' when stale."""
    radar_dir = Path(TILE_DIR) / "radar"
    newest = _newest_mtime(radar_dir)
    now = time.time()

    status = "ok"
    reasons: list[str] = []
    mrms_age = None
    if newest is None:
        status = "degraded"
        reasons.append("no_mrms_tiles")
    else:
        mrms_age = int(now - newest)
        if mrms_age > MRMS_MAX_AGE_S:
            status = "degraded"
            reasons.append(f"mrms_stale_{mrms_age}s")

    body = {
        "status": status,
        "mrms_age_s": mrms_age,
        "mrms_max_age_s": MRMS_MAX_AGE_S,
        "reasons": reasons,
        "upstream_forecast": OPEN_METEO_BASE,
        "checked_at": datetime.now(timezone.utc).isoformat(),
    }
    return JSONResponse(body, status_code=200 if status == "ok" else 503)


@app.get("/api/metrics", response_class=PlainTextResponse)
async def metrics() -> PlainTextResponse:
    """Prometheus text-format metrics."""
    lines: list[str] = []
    tile_base = Path(TILE_DIR)
    layer_counts: dict[str, int] = {}
    if tile_base.exists():
        for layer_dir in tile_base.iterdir():
            if layer_dir.is_dir():
                layer_counts[layer_dir.name] = sum(1 for p in layer_dir.iterdir() if p.is_dir())

    for k, v in _metrics.items():
        lines.append(f"# TYPE stormscope_{k} counter")
        lines.append(f"stormscope_{k} {v}")

    lines.append("# TYPE stormscope_tile_timestamps gauge")
    for layer, count in sorted(layer_counts.items()):
        safe = layer.replace("-", "_")
        lines.append(f'stormscope_tile_timestamps{{layer="{layer}"}} {count}')
        lines.append(f"stormscope_tile_timestamps_{safe} {count}")

    radar_dir = tile_base / "radar"
    newest = _newest_mtime(radar_dir)
    if newest is not None:
        age = int(time.time() - newest)
        lines.append("# TYPE stormscope_mrms_age_seconds gauge")
        lines.append(f"stormscope_mrms_age_seconds {age}")

    return PlainTextResponse("\n".join(lines) + "\n")
