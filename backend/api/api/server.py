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
import os
import time
from datetime import datetime, timezone
from pathlib import Path

import httpx
from fastapi import FastAPI, HTTPException, Request
from fastapi.responses import JSONResponse, PlainTextResponse

from backend.shared.manifest import read_manifest_file

TILE_DIR = os.environ.get("TILE_DIR", "/data/tiles")
GRID_DIR = os.environ.get("GRID_DIR", "/data/grids")
STATE_DIR = os.environ.get("STATE_DIR", "/data/state")
OPEN_METEO_BASE = os.environ.get("OPEN_METEO_BASE", "https://api.open-meteo.com/v1/forecast")
STYLE_DIR = os.environ.get("STYLE_DIR", "/srv/basemap/styles")
MRMS_MAX_AGE_S = int(os.environ.get("MRMS_MAX_AGE_S", "600"))  # tiles older than this → degraded
FORECAST_TTL = int(os.environ.get("FORECAST_TTL_S", "900"))  # 15min

CURRENT_FIELDS = (
    "temperature_2m,relative_humidity_2m,apparent_temperature,weather_code,"
    "wind_speed_10m,wind_direction_10m,wind_gusts_10m,"
    "dew_point_2m,surface_pressure,precipitation"
)
HOURLY_FIELDS = (
    "temperature_2m,precipitation,precipitation_probability,weather_code,"
    "wind_speed_10m,relative_humidity_2m,apparent_temperature"
)
DAILY_FIELDS = (
    "temperature_2m_max,temperature_2m_min,weather_code,precipitation_sum,"
    "precipitation_probability_max,uv_index_max,sunrise,sunset"
)

app = FastAPI(title="radar-ng Tile API")

# Storm-watch + push-token endpoints are workflow-driven — see
# routes_workflows.py. Register lazily so a deploy that doesn't have
# Temporal in front of it (e.g. a static-tile-only fork) can still boot
# by setting DISABLE_WORKFLOW_ROUTES=1.
if os.environ.get("DISABLE_WORKFLOW_ROUTES") != "1":
    from backend.api.api.routes_workflows import router as workflows_router
    app.include_router(workflows_router)

_forecast_cache: dict[str, tuple[float, dict]] = {}
# Hard cap on cached cells. The key space is every 0.1° cell on Earth
# (~6.5M) x tens-of-kB responses, and this endpoint is unauthenticated —
# without a bound, a coordinate sweep grows the dict until the OOM killer
# takes down /api/* and /v1/* for everyone. 1024 cells ≈ every metro a
# realistic user base queries, at worst a few hundred MB.
_FORECAST_CACHE_MAX = int(os.environ.get("FORECAST_CACHE_MAX_ENTRIES", "1024"))
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
}


def _cached(body: dict | list, max_age: int, status_code: int = 200) -> JSONResponse:
    return JSONResponse(
        body,
        status_code=status_code,
        headers={"Cache-Control": f"public, max-age={max_age}"},
    )


def _newest_mtime(path: Path) -> float | None:
    if not path.exists():
        return None
    try:
        stamps = [p.stat().st_mtime for p in path.iterdir() if p.is_dir()]
        return max(stamps) if stamps else None
    except OSError:
        return None


def _read_nowcast_status() -> dict | None:
    path = Path(STATE_DIR) / "nowcast-status.json"
    try:
        body = json.loads(path.read_text())
    except (OSError, json.JSONDecodeError):
        return None
    return body if isinstance(body, dict) else None


def _is_layer_dirname(name: str) -> bool:
    # Layer names in this app match [a-z][a-z0-9_-]*. Excludes filesystem
    # artifacts like ext4's `lost+found` (root:0 mode 700, unreadable by the
    # non-root container user → would raise PermissionError on iterdir()).
    return bool(name) and name[0].isalpha() and all(
        c.isalnum() or c in ("-", "_") for c in name
    )


def _build_manifest() -> dict:
    """Read the pre-rendered manifest body from STATE_DIR/manifest.json."""
    return read_manifest_file(STATE_DIR)


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
    grid_lat = round(lat, 1)
    grid_lon = round(lon, 1)
    cache_key = f"{grid_lat},{grid_lon}"

    cached = _forecast_cache.get(cache_key)
    if cached:
        if time.time() - cached[0] < FORECAST_TTL:
            _metrics["forecast_cache_hits_total"] += 1
            return _cached(cached[1], max_age=300)
        # Expired — remove now instead of letting dead entries accumulate.
        _forecast_cache.pop(cache_key, None)

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
    # FIFO eviction (dicts iterate in insertion order) — cheap and good
    # enough at TTL=15min; real hot cells are re-inserted on expiry anyway.
    while len(_forecast_cache) > _FORECAST_CACHE_MAX:
        _forecast_cache.pop(next(iter(_forecast_cache)), None)
    return _cached(data, max_age=300)


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
    bin_path = grid_base.with_suffix(".bin")
    meta_path = grid_base.with_suffix(".meta.json")

    if not (bin_path.exists() and meta_path.exists()):
        return JSONResponse(
            {"ok": False, "reason": "grid_missing", "layer": layer, "timestamp": timestamp},
            status_code=200,
        )

    try:
        meta = json.loads(meta_path.read_text())
        h = int(meta["height"])
        w = int(meta["width"])
        lat_min = float(meta["lat_min"])
        lat_max = float(meta["lat_max"])
        lon_min = float(meta["lon_min"])
        lon_max = float(meta["lon_max"])
        unit = meta.get("unit", "")
        fill = float(meta.get("fill", float("nan")))
    except (OSError, KeyError, ValueError) as exc:
        return JSONResponse({"ok": False, "reason": "meta_invalid", "err": str(exc)}, status_code=200)

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
        return JSONResponse({"ok": False, "reason": "read_error", "err": str(exc)}, status_code=200)

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

    import struct

    try:
        u_m = json.loads(u_meta.read_text())
        v_m = json.loads(v_meta.read_text())
        H = int(u_m["height"])
        W = int(u_m["width"])
        u_bin = (Path(GRID_DIR) / "wind_u" / f"{safe_ts}.bin").read_bytes()
        v_bin = (Path(GRID_DIR) / "wind_v" / f"{safe_ts}.bin").read_bytes()
    except (OSError, KeyError, ValueError) as exc:
        return JSONResponse({"ok": False, "reason": "grid_read_failed", "err": str(exc)}, status_code=200)

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

    return _cached({
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
    }, max_age=300)


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
        return _cached(
            {"type": "FeatureCollection", "features": [], "error": str(exc)},
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
        return _cached({"type": "FeatureCollection", "features": [], "error": str(exc)}, max_age=30)


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
        return _cached({"type": "FeatureCollection", "features": [], "error": str(exc)}, max_age=180)


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
    tile_base = Path(TILE_DIR)
    layer_counts: dict[str, int] = {}
    if tile_base.exists():
        for layer_dir in tile_base.iterdir():
            # Skip filesystem artifacts like ext4's lost+found — the PVC root
            # isn't a pure layer-only directory.
            if not layer_dir.is_dir() or not _is_layer_dirname(layer_dir.name):
                continue
            # Layout is {layer}/{palette}/{timestamp} (legacy: {layer}/{ts}).
            # Counting the layer's immediate children counted PALETTES — a
            # constant — so any "frames dropping" alert on this gauge was
            # blind. Count distinct timestamps across palettes instead.
            stamps: set[str] = set()
            try:
                for child in layer_dir.iterdir():
                    if not child.is_dir() or child.name.endswith(".tmp"):
                        continue
                    if child.name[:1].isdigit():  # legacy timestamp dir
                        stamps.add(child.name)
                    else:  # palette dir
                        stamps.update(
                            p.name for p in child.iterdir()
                            if p.is_dir() and not p.name.endswith(".tmp")
                        )
            except OSError:
                continue
            layer_counts[layer_dir.name] = len(stamps)

    for k, v in _metrics.items():
        lines.append(f"# TYPE radar_ng_{k} counter")
        lines.append(f"radar_ng_{k} {v}")

    lines.append("# TYPE radar_ng_tile_timestamps gauge")
    for layer, count in sorted(layer_counts.items()):
        lines.append(f'radar_ng_tile_timestamps{{layer="{layer}"}} {count}')

    radar_dir = tile_base / "radar"
    newest = _newest_mtime(radar_dir)
    if newest is not None:
        age = int(time.time() - newest)
        lines.append("# TYPE radar_ng_mrms_age_seconds gauge")
        lines.append(f"radar_ng_mrms_age_seconds {age}")

    return PlainTextResponse("\n".join(lines) + "\n")
