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

import json
import logging
import os
import time
from collections import OrderedDict, defaultdict
from contextlib import asynccontextmanager
from datetime import datetime, timezone
from pathlib import Path
from threading import Lock

import httpx
from fastapi import FastAPI, HTTPException, Request
from fastapi.responses import JSONResponse, PlainTextResponse

from backend.shared.manifest import read_manifest_file
from backend.shared.storm_prefetch import build_storm_prefetch_plan

TILE_DIR = os.environ.get("TILE_DIR", "/data/tiles")
GRID_DIR = os.environ.get("GRID_DIR", "/data/grids")
STATE_DIR = os.environ.get("STATE_DIR", "/data/state")
OPEN_METEO_BASE = os.environ.get("OPEN_METEO_BASE", "https://api.open-meteo.com/v1/forecast")
STYLE_DIR = os.environ.get("STYLE_DIR", "/srv/basemap/styles")
MRMS_MAX_AGE_S = int(os.environ.get("MRMS_MAX_AGE_S", "600"))  # tiles older than this → degraded
FORECAST_TTL = int(os.environ.get("FORECAST_TTL_S", "900"))  # 15min
FORECAST_CACHE_MAX_ENTRIES = int(os.environ.get("FORECAST_CACHE_MAX_ENTRIES", "512"))
WIND_CACHE_MAX_ENTRIES = int(os.environ.get("WIND_CACHE_MAX_ENTRIES", "48"))
API_RATE_LIMIT_RPS = float(os.environ.get("API_RATE_LIMIT_RPS", "20"))
API_RATE_LIMIT_BURST = float(os.environ.get("API_RATE_LIMIT_BURST", "60"))
log = logging.getLogger(__name__)

CURRENT_FIELDS = (
    "temperature_2m,relative_humidity_2m,apparent_temperature,weather_code,"
    "wind_speed_10m,wind_direction_10m,wind_gusts_10m,"
    "dew_point_2m,surface_pressure,precipitation"
)
HOURLY_FIELDS = (
    "temperature_2m,precipitation,precipitation_probability,weather_code,"
    "wind_speed_10m,relative_humidity_2m,apparent_temperature"
)
MINUTELY_15_FIELDS = (
    "temperature_2m,relative_humidity_2m,apparent_temperature,precipitation,"
    "weather_code,wind_speed_10m,wind_direction_10m,wind_gusts_10m"
)
DAILY_FIELDS = (
    "temperature_2m_max,temperature_2m_min,weather_code,precipitation_sum,"
    "precipitation_probability_max,uv_index_max,sunrise,sunset"
)

@asynccontextmanager
async def _lifespan(app: FastAPI):
    app.state.forecast_http = httpx.AsyncClient(
        limits=httpx.Limits(max_connections=100, max_keepalive_connections=20),
        timeout=httpx.Timeout(15.0),
    )
    try:
        yield
    finally:
        await app.state.forecast_http.aclose()


app = FastAPI(title="radar-ng Tile API", lifespan=_lifespan)

# Storm-watch + push-token endpoints are workflow-driven — see
# routes_workflows.py. Register lazily so a deploy that doesn't have
# Temporal in front of it (e.g. a static-tile-only fork) can still boot
# by setting DISABLE_WORKFLOW_ROUTES=1.
if os.environ.get("DISABLE_WORKFLOW_ROUTES") != "1":
    from backend.api.api.routes_workflows import router as workflows_router
    app.include_router(workflows_router)

_forecast_cache: OrderedDict[str, tuple[float, dict]] = OrderedDict()
_wind_cache: OrderedDict[str, tuple[float, dict]] = OrderedDict()
_wind_cache_lock = Lock()
# Manifest in-memory cache. The source of truth is STATE_DIR/manifest.json,
# maintained by ingest/cleanup activities, so a cold API hit only reads one
# small JSON file instead of crawling the tile PVC.
_MANIFEST_TTL_S = 15.0
_manifest_cache: dict[str, object] = {"expires_at": 0.0, "body": None}
_metrics = {
    "forecast_requests_total": 0,
    "forecast_cache_hits_total": 0,
    "forecast_upstream_errors_total": 0,
    "manifest_requests_total": 0,
    "rate_limited_requests_total": 0,
}
_request_counts: defaultdict[tuple[str, str, int], int] = defaultdict(int)
_request_duration_sums: defaultdict[tuple[str, str], float] = defaultdict(float)
_rate_buckets: OrderedDict[str, tuple[float, float]] = OrderedDict()


def _client_key(request: Request) -> str:
    forwarded = request.headers.get("x-forwarded-for", "")
    if forwarded:
        return forwarded.split(",", 1)[0].strip()
    return request.client.host if request.client else "unknown"


@app.middleware("http")
async def request_controls(request: Request, call_next):
    started = time.perf_counter()
    now = time.monotonic()
    client = _client_key(request)
    tokens, updated = _rate_buckets.pop(client, (API_RATE_LIMIT_BURST, now))
    tokens = min(API_RATE_LIMIT_BURST, tokens + max(0.0, now - updated) * API_RATE_LIMIT_RPS)
    rate_limited = request.url.path != "/api/livez" and tokens < 1.0
    if request.url.path != "/api/livez" and not rate_limited:
        tokens -= 1.0
    # Store the debit before yielding to downstream request handling so
    # concurrent requests cannot observe a missing bucket and reset to burst.
    _rate_buckets[client] = (tokens, now)
    while len(_rate_buckets) > 4096:
        _rate_buckets.popitem(last=False)

    if rate_limited:
        _metrics["rate_limited_requests_total"] += 1
        response = JSONResponse(
            {"error": "rate_limited"}, status_code=429, headers={"Retry-After": "1"}
        )
    else:
        response = await call_next(request)

    route = request.scope.get("route")
    path = getattr(route, "path", request.url.path)
    key = (request.method, str(path), response.status_code)
    _request_counts[key] += 1
    duration_key = (request.method, str(path))
    _request_duration_sums[duration_key] += time.perf_counter() - started
    return response


def _cached(body: dict | list, max_age: int, status_code: int = 200) -> JSONResponse:
    return JSONResponse(
        body,
        status_code=status_code,
        headers={"Cache-Control": f"public, max-age={max_age}"},
    )


def _read_nowcast_status() -> dict | None:
    path = Path(STATE_DIR) / "nowcast-status.json"
    try:
        body = json.loads(path.read_text())
    except (OSError, json.JSONDecodeError):
        return None
    return body if isinstance(body, dict) else None


def _grid_binary_path(meta_path: Path, meta: dict) -> Path:
    data_file = meta.get("data_file")
    if data_file:
        candidate = meta_path.parent / str(data_file)
        if candidate.parent != meta_path.parent:
            raise ValueError("invalid grid data_file")
        return candidate
    return meta_path.parent / meta_path.name.replace(".meta.json", ".bin")


def _build_manifest() -> dict:
    """Read the pre-rendered manifest body from STATE_DIR/manifest.json."""
    return read_manifest_file(STATE_DIR)


def _layer_age_seconds(manifest: dict, layer_name: str, now: float | None = None) -> int | None:
    layer = manifest.get("layers", {}).get(layer_name, {})
    latest = layer.get("latest")
    if not latest:
        return None
    try:
        observed = datetime.fromisoformat(str(latest)).timestamp()
    except ValueError:
        return None
    return max(0, int((now or time.time()) - observed))


# Endpoint concurrency rule: everything that touches the NFS-backed PVCs is a
# plain `def` so FastAPI runs it in the threadpool — a hung NFS mount (the
# documented TrueNAS failure mode) then parks worker threads instead of
# freezing the event loop. Only /api/forecast (pure httpx) and /api/livez
# stay `async def`; livez in particular MUST remain on the event loop so the
# k8s probe keeps answering while disk-bound requests are stuck.
@app.get("/api/manifest.json")
def get_manifest() -> JSONResponse:
    _metrics["manifest_requests_total"] += 1
    now = time.time()
    cached_body = _manifest_cache.get("body")
    if cached_body is not None and float(_manifest_cache.get("expires_at", 0)) > now:
        return _cached(cached_body, max_age=15)
    body = _build_manifest()
    _manifest_cache["body"] = body
    _manifest_cache["expires_at"] = now + _MANIFEST_TTL_S
    return _cached(body, max_age=15)


@app.get("/api/forecast/{lat}/{lon}")
async def get_forecast(lat: float, lon: float) -> JSONResponse:
    _metrics["forecast_requests_total"] += 1
    if not (-90.0 <= lat <= 90.0 and -180.0 <= lon <= 180.0):
        raise HTTPException(422, "lat/lon out of range")
    grid_lat = round(lat, 1)
    grid_lon = round(lon, 1)
    cache_key = f"{grid_lat},{grid_lon}"

    cached = _forecast_cache.get(cache_key)
    if cached and time.time() - cached[0] < FORECAST_TTL:
        _metrics["forecast_cache_hits_total"] += 1
        _forecast_cache.move_to_end(cache_key)
        return _cached(cached[1], max_age=FORECAST_TTL)
    if cached:
        _forecast_cache.pop(cache_key, None)

    params = {
        "latitude": str(grid_lat),
        "longitude": str(grid_lon),
        "current": CURRENT_FIELDS,
        "hourly": HOURLY_FIELDS,
        "minutely_15": MINUTELY_15_FIELDS,
        "daily": DAILY_FIELDS,
        "temperature_unit": "fahrenheit",
        "wind_speed_unit": "mph",
        "precipitation_unit": "inch",
        "timezone": "auto",
        "forecast_days": "7",
    }

    try:
        client: httpx.AsyncClient = app.state.forecast_http
        resp = await client.get(OPEN_METEO_BASE, params=params)
        resp.raise_for_status()
        data = resp.json()
    except httpx.HTTPError as exc:
        _metrics["forecast_upstream_errors_total"] += 1
        return JSONResponse(
            {"error": "upstream_unavailable"},
            status_code=502,
        )

    _forecast_cache[cache_key] = (time.time(), data)
    while len(_forecast_cache) > FORECAST_CACHE_MAX_ENTRIES:
        _forecast_cache.popitem(last=False)
    return _cached(data, max_age=FORECAST_TTL)


@app.get("/api/inspect/{layer}/{timestamp}/{lat}/{lon}")
def inspect_point(layer: str, timestamp: str, lat: float, lon: float) -> JSONResponse:
    """Bilinear-interpolate a single point from a stored Float32 grid.

    Ingestors dump downsampled grids to GRID_DIR/{layer}/{timestamp}.bin with
    a sidecar .meta.json describing shape + lat/lon bounds. Returns a 404 body
    (status 200 — so the app can handle gracefully) when the grid isn't there.
    """
    safe_layer = "".join(ch for ch in layer if ch.isalnum() or ch in "-_")
    safe_ts = "".join(ch for ch in timestamp if ch.isalnum() or ch in ":-_+.T")
    grid_base = Path(GRID_DIR) / safe_layer / safe_ts
    meta_path = grid_base.with_suffix(".meta.json")

    if not meta_path.exists():
        return JSONResponse(
            {"ok": False, "reason": "grid_missing", "layer": layer, "timestamp": timestamp},
            status_code=200,
        )

    try:
        meta = json.loads(meta_path.read_text())
        bin_path = _grid_binary_path(meta_path, meta)
        if not bin_path.exists():
            raise OSError("grid generation missing")
        h = int(meta["height"])
        w = int(meta["width"])
        lat_min = float(meta["lat_min"])
        lat_max = float(meta["lat_max"])
        lon_min = float(meta["lon_min"])
        lon_max = float(meta["lon_max"])
        unit = meta.get("unit", "")
        fill = float(meta.get("fill", float("nan")))
    except (OSError, KeyError, ValueError) as exc:
        log.warning("inspect metadata read failed: %s", exc)
        return JSONResponse({"ok": False, "reason": "meta_invalid"}, status_code=200)

    if lat < lat_min or lat > lat_max or lon < lon_min or lon > lon_max:
        return JSONResponse(
            {"ok": False, "reason": "out_of_bounds", "lat": lat, "lon": lon},
            status_code=200,
        )

    # Bilinear interpolation — read only the 4 values we need instead of the full grid.
    fx = (lon - lon_min) / (lon_max - lon_min) * (w - 1)
    fy = (lat_max - lat) / (lat_max - lat_min) * (h - 1)
    x0 = max(0, min(int(fx), w - 2))
    y0 = max(0, min(int(fy), h - 2))
    dx = fx - x0
    dy = fy - y0

    import struct

    def read_cell(ix: int, iy: int) -> float:
        offset = (iy * w + ix) * 4
        with open(bin_path, "rb") as f:
            f.seek(offset)
            return struct.unpack("<f", f.read(4))[0]

    try:
        v00 = read_cell(x0, y0)
        v10 = read_cell(x0 + 1, y0)
        v01 = read_cell(x0, y0 + 1)
        v11 = read_cell(x0 + 1, y0 + 1)
    except OSError as exc:
        log.warning("inspect grid read failed: %s", exc)
        return JSONResponse({"ok": False, "reason": "read_error"}, status_code=200)

    def is_fill(v: float) -> bool:
        return v != v or abs(v - fill) < 1e-6

    valid = [v for v in (v00, v10, v01, v11) if not is_fill(v)]
    if not valid:
        return JSONResponse(
            {"ok": True, "value": None, "unit": unit, "reason": "no_data"},
            status_code=200,
        )

    v0 = v00 * (1 - dx) + v10 * dx if not (is_fill(v00) or is_fill(v10)) else (valid[0])
    v1 = v01 * (1 - dx) + v11 * dx if not (is_fill(v01) or is_fill(v11)) else (valid[-1])
    value = v0 * (1 - dy) + v1 * dy

    return JSONResponse(
        {
            "ok": True,
            "value": float(value),
            "unit": unit,
            "layer": layer,
            "timestamp": timestamp,
            "lat": lat,
            "lon": lon,
        }
    )


@app.get("/api/wind-field/{timestamp}")
def wind_field(timestamp: str) -> JSONResponse:
    """Return packed U/V wind components for a timestamp, decimated + scaled.

    Reads the paired Float32 grids (wind_u + wind_v) dumped by ingest-hrrr,
    downsamples to a ~120x60 grid (small payload <70 kB), and returns:

        {
          "width": W, "height": H,
          "lat_min": ..., "lat_max": ..., "lon_min": ..., "lon_max": ...,
          "umin": ..., "umax": ..., "u": [int8 * N],
          "vmin": ..., "vmax": ..., "v": [int8 * N]
        }

    The app maps int8 [-127..127] back to the real mph range via umin/umax +
    vmin/vmax so the payload stays JSON-friendly (no binary endpoint plumbing).
    """
    safe_ts = "".join(ch for ch in timestamp if ch.isalnum() or ch in ":-_+.T")
    u_dir = Path(GRID_DIR) / "wind_u"
    v_dir = Path(GRID_DIR) / "wind_v"

    # Resolve "latest" or any timestamp without a matching grid to the most
    # recent wind dump that has both U and V on disk. The MRMS frame timeline
    # spans further back than HRRR wind grids, so falling back keeps the
    # particle overlay alive instead of returning grid_missing.
    u_meta = u_dir / f"{safe_ts}.meta.json"
    v_meta = v_dir / f"{safe_ts}.meta.json"
    if safe_ts.lower() == "latest" or not (u_meta.exists() and v_meta.exists()):
        try:
            u_stems = {p.stem.replace(".meta", "") for p in u_dir.glob("*.meta.json")}
            v_stems = {p.stem.replace(".meta", "") for p in v_dir.glob("*.meta.json")}
            common = sorted(u_stems & v_stems)
        except OSError:
            common = []
        if not common:
            return JSONResponse(
                {"ok": False, "reason": "grid_missing", "timestamp": timestamp},
                status_code=200,
            )
        safe_ts = common[-1]
        u_meta = u_dir / f"{safe_ts}.meta.json"
        v_meta = v_dir / f"{safe_ts}.meta.json"

    with _wind_cache_lock:
        cached_wind = _wind_cache.get(safe_ts)
        if cached_wind and time.time() - cached_wind[0] < 300:
            _wind_cache.move_to_end(safe_ts)
    if cached_wind and time.time() - cached_wind[0] < 300:
        return _cached(cached_wind[1], max_age=300)

    import struct

    try:
        u_m = json.loads(u_meta.read_text())
        v_m = json.loads(v_meta.read_text())
        H = int(u_m["height"])
        W = int(u_m["width"])
        u_bin = _grid_binary_path(u_meta, u_m).read_bytes()
        v_bin = _grid_binary_path(v_meta, v_m).read_bytes()
    except (OSError, KeyError, ValueError) as exc:
        log.warning("wind grid read failed: %s", exc)
        return JSONResponse({"ok": False, "reason": "grid_read_failed"}, status_code=200)

    # Target downsampled grid — 240x120 is enough detail for a continent-scale
    # particle field and keeps the payload under ~60 kB.
    target_w = 240
    target_h = 120
    sx = max(1, W // target_w)
    sy = max(1, H // target_h)
    out_w = W // sx
    out_h = H // sy

    # Walk the source grids, decimating + finding min/max of each channel.
    u_vals: list[float] = []
    v_vals: list[float] = []
    for ry in range(out_h):
        for rx in range(out_w):
            src_i = (ry * sy) * W + (rx * sx)
            u_vals.append(struct.unpack_from("<f", u_bin, src_i * 4)[0])
            v_vals.append(struct.unpack_from("<f", v_bin, src_i * 4)[0])

    u_min = min(u_vals)
    u_max = max(u_vals)
    v_min = min(v_vals)
    v_max = max(v_vals)
    u_span = max(1e-6, u_max - u_min)
    v_span = max(1e-6, v_max - v_min)

    # Scale to int8 [-127..127]. -128 is reserved as a fill sentinel (unused now).
    u_scaled = [int(round((u - u_min) / u_span * 254 - 127)) for u in u_vals]
    v_scaled = [int(round((v - v_min) / v_span * 254 - 127)) for v in v_vals]

    body = {
        "ok": True,
        "timestamp": timestamp,
        "width": out_w,
        "height": out_h,
        "lat_min": u_m["lat_min"],
        "lat_max": u_m["lat_max"],
        "lon_min": u_m["lon_min"],
        "lon_max": u_m["lon_max"],
        "u_min": u_min,
        "u_max": u_max,
        "v_min": v_min,
        "v_max": v_max,
        "u": u_scaled,
        "v": v_scaled,
    }
    with _wind_cache_lock:
        _wind_cache[safe_ts] = (time.time(), body)
        while len(_wind_cache) > WIND_CACHE_MAX_ENTRIES:
            _wind_cache.popitem(last=False)
    return _cached(body, max_age=300)


@app.get("/api/lightning")
def lightning() -> JSONResponse:
    """Return the rolling 15-min GeoJSON FeatureCollection of lightning strikes.

    The ingest-lightning container streams from Blitzortung and writes this
    file every couple of seconds. If the file is missing (service down), we
    return an empty collection so the app's overlay fails gracefully.
    """
    path = Path(STATE_DIR) / "lightning.json"
    if not path.exists():
        return _cached(
            {"type": "FeatureCollection", "features": [], "generated_at": 0, "reason": "no_data"},
            max_age=10,
        )
    try:
        body = json.loads(path.read_text())
        return _cached(body, max_age=10)
    except (OSError, json.JSONDecodeError) as exc:
        log.warning("lightning state read failed: %s", exc)
        return _cached(
            {"type": "FeatureCollection", "features": [], "reason": "state_read_failed"},
            max_age=10,
        )


@app.get("/api/storms")
def storms() -> JSONResponse:
    """Return the latest storm-cell GeoJSON from ingest-mrms.

    Each feature is a point at a storm cell's centroid with properties:
    peak_dbz, area_km2, pixel_count, cell_id. Missing file → empty collection.
    """
    path = Path(STATE_DIR) / "storms.json"
    if not path.exists():
        return _cached({"type": "FeatureCollection", "features": [], "reason": "no_data"}, max_age=30)
    try:
        return _cached(json.loads(path.read_text()), max_age=30)
    except (OSError, json.JSONDecodeError) as exc:
        log.warning("storm state read failed: %s", exc)
        return _cached({"type": "FeatureCollection", "features": [], "reason": "state_read_failed"}, max_age=30)


@app.get("/api/storm-prefetch")
def storm_prefetch(
    request: Request,
    lat: float,
    lon: float,
    zoom: int = 6,
    palette: str = "classic",
) -> JSONResponse:
    """Return a location-aware, three-bbox MapLibre tile prefetch plan."""
    if not (-90 <= lat <= 90 and -180 <= lon <= 180):
        raise HTTPException(422, "lat/lon out of range")
    if not (4 <= zoom <= 8):
        raise HTTPException(422, "zoom must be between 4 and 8")
    if palette not in {"classic", "vivid", "muted"}:
        raise HTTPException(422, "unsupported palette")

    path = Path(STATE_DIR) / "storms.json"
    try:
        body = json.loads(path.read_text())
    except (OSError, json.JSONDecodeError):
        body = {"type": "FeatureCollection", "features": []}
    plan = build_storm_prefetch_plan(
        storms=body,
        state_dir=STATE_DIR,
        tile_dir=TILE_DIR,
        base_url=str(request.base_url).rstrip("/"),
        lat=lat,
        lon=lon,
        zoom=zoom,
        palette=palette,
    )
    return _cached(plan, max_age=30)


@app.get("/api/storm-prefetch/style.json")
def storm_prefetch_style(
    request: Request,
    layer: str,
    palette: str,
    timestamp: str,
    zoom: int,
) -> JSONResponse:
    """Minimal raster style consumed by MapLibre's native offline loader."""
    if layer not in {"radar", "nowcast"} or palette not in {"classic", "vivid", "muted"}:
        raise HTTPException(422, "unsupported layer or palette")
    max_zoom = 7 if layer == "radar" else 6
    if not (4 <= zoom <= max_zoom):
        raise HTTPException(422, "zoom outside layer coverage")
    safe_timestamp = "".join(ch for ch in timestamp if ch.isalnum() or ch in ":-_+.T")
    if safe_timestamp != timestamp:
        raise HTTPException(422, "invalid timestamp")
    base_url = str(request.base_url).rstrip("/")
    tile_template = f"{base_url}/tiles/{layer}/{palette}/{timestamp}/{{z}}/{{x}}/{{y}}.png"
    return _cached({
        "version": 8,
        "name": "radar-ng storm prefetch",
        "sources": {
            "storm-prefetch": {
                "type": "raster",
                "tiles": [tile_template],
                "tileSize": 256,
                "minzoom": zoom,
                "maxzoom": zoom,
            }
        },
        "layers": [{
            "id": "storm-prefetch",
            "type": "raster",
            "source": "storm-prefetch",
            "paint": {"raster-fade-duration": 0},
        }],
    }, max_age=30)


@app.get("/api/tropical")
def tropical() -> JSONResponse:
    """Return the active tropical cyclone GeoJSON from ingest-tropical.

    Expect a merged FeatureCollection with one feature per storm (current
    position point + forecast track line + cone polygon). Missing file → empty.
    """
    path = Path(STATE_DIR) / "tropical.json"
    if not path.exists():
        return _cached({"type": "FeatureCollection", "features": [], "reason": "no_data"}, max_age=180)
    try:
        return _cached(json.loads(path.read_text()), max_age=180)
    except (OSError, json.JSONDecodeError) as exc:
        log.warning("tropical state read failed: %s", exc)
        return _cached({"type": "FeatureCollection", "features": [], "reason": "state_read_failed"}, max_age=180)


@app.get("/api/livez")
async def livez() -> JSONResponse:
    """Process-liveness only — 200 iff this FastAPI process answers.

    This is the k8s probe target (hit via Caddy on :8080 so the probe
    proves BOTH processes in the pod work — /start.sh backgrounds uvicorn,
    and without a probe a dead uvicorn leaves Caddy 502ing /api/* forever).
    /api/health is deliberately NOT a probe: it reports degraded on stale
    radar DATA, a condition shared by every replica — wiring it to
    liveness/readiness would restart or drain the whole fleet at once the
    moment NOAA has a slow day.

    Must stay `async def` and must never touch the filesystem: the
    disk-bound endpoints run in the threadpool, so this stays responsive
    on the event loop even while every worker thread is parked on a hung
    NFS mount.
    """
    return JSONResponse({"status": "ok"})


@app.get("/api/health")
def health() -> JSONResponse:
    """Returns 'ok' when MRMS tiles are fresh, 'degraded' when stale."""
    now = time.time()
    manifest = _build_manifest()

    status = "ok"
    reasons: list[str] = []
    mrms_age = _layer_age_seconds(manifest, "radar", now)
    if mrms_age is None:
        status = "degraded"
        reasons.append("no_mrms_tiles")
    else:
        if mrms_age > MRMS_MAX_AGE_S:
            status = "degraded"
            reasons.append(f"mrms_stale_{mrms_age}s")

    nowcast_status = _read_nowcast_status()
    if nowcast_status and nowcast_status.get("status") == "degraded":
        status = "degraded"
        reason = nowcast_status.get("reason") or "nowcast_degraded"
        reasons.append(str(reason))

    body = {
        "status": status,
        "mrms_age_s": mrms_age,
        "mrms_max_age_s": MRMS_MAX_AGE_S,
        "nowcast": nowcast_status or {"status": "unknown", "reason": "no_status"},
        "reasons": reasons,
        "upstream_forecast": OPEN_METEO_BASE,
        "checked_at": datetime.now(timezone.utc).isoformat(),
    }
    return JSONResponse(body, status_code=200 if status == "ok" else 503)


@app.get("/api/basemap/style/{name}")
def get_basemap_style(name: str, request: Request) -> JSONResponse:
    """Return a MapLibre style JSON with absolute tile URLs baked in.

    MapLibre Native requires absolute URLs in `sources.*.tiles`. The shipped
    style files use a relative `/basemap/tiles/{z}/{x}/{y}.mvt` so they stay
    portable; we rewrite that to `{request_origin}/basemap/tiles/...` here.
    """
    if not name.isalnum() and name not in ("positron", "dark-matter"):
        # Defence in depth against path traversal — reject anything non-alphanumeric
        # other than our two known style names.
        if "/" in name or ".." in name or "\\" in name:
            raise HTTPException(status_code=400, detail="invalid style name")

    style_path = Path(STYLE_DIR) / f"{name}.json"
    if not style_path.exists():
        raise HTTPException(status_code=404, detail="style not found")

    style = json.loads(style_path.read_text())

    # Reconstruct the client-facing origin. x-forwarded-* takes precedence so
    # the app sees the same origin it used to fetch this style (important when
    # serverUrl differs from the container-internal address).
    proto = request.headers.get("x-forwarded-proto") or request.url.scheme
    host = request.headers.get("x-forwarded-host") or request.headers.get("host") or request.url.netloc
    origin = f"{proto}://{host}"

    for src in style.get("sources", {}).values():
        tiles = src.get("tiles")
        if not tiles:
            continue
        src["tiles"] = [
            (origin + t) if isinstance(t, str) and t.startswith("/") else t
            for t in tiles
        ]

    return JSONResponse(style)


@app.get("/api/metrics", response_class=PlainTextResponse)
def metrics() -> PlainTextResponse:
    """Prometheus text-format metrics."""
    lines: list[str] = []
    manifest = _build_manifest()
    layer_counts = {
        layer: len(entry.get("frames", entry.get("timestamps", [])))
        for layer, entry in manifest.get("layers", {}).items()
        if isinstance(entry, dict)
    }

    for k, v in _metrics.items():
        lines.append(f"# TYPE radar_ng_{k} counter")
        lines.append(f"radar_ng_{k} {v}")

    lines.append("# TYPE radar_ng_tile_timestamps gauge")
    for layer, count in sorted(layer_counts.items()):
        lines.append(f'radar_ng_tile_timestamps{{layer="{layer}"}} {count}')

    age = _layer_age_seconds(manifest, "radar")
    if age is not None:
        lines.append("# TYPE radar_ng_mrms_age_seconds gauge")
        lines.append(f"radar_ng_mrms_age_seconds {age}")

    lines.append("# TYPE radar_ng_http_requests_total counter")
    for (method, path, status), count in sorted(_request_counts.items()):
        lines.append(
            f'radar_ng_http_requests_total{{method="{method}",path="{path}",status="{status}"}} {count}'
        )
    lines.append("# TYPE radar_ng_http_request_duration_seconds_sum counter")
    for (method, path), duration in sorted(_request_duration_sums.items()):
        lines.append(
            f'radar_ng_http_request_duration_seconds_sum{{method="{method}",path="{path}"}} {duration:.6f}'
        )

    return PlainTextResponse("\n".join(lines) + "\n")
