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
