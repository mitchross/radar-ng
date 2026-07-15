"""HTTP-side checks: API latency, tile sampling, pipeline freshness, client sim.

Everything here talks to the tile-server the exact way the app does — through
Caddy on the public port — so the numbers include the full serving path
(Caddy file_server / reverse_proxy hop / FastAPI threadpool), not just FastAPI.
"""

from __future__ import annotations

import concurrent.futures
import math
from datetime import timedelta
from typing import Any

from .core import (
    FAIL,
    OK,
    WARN,
    Check,
    Fetch,
    age_str,
    fetch,
    fetch_json,
    parse_iso,
    percentiles,
    quote_ts,
    utcnow,
)

# Per-layer freshness expectations, derived from the Temporal schedule cadences
# in temporal/schedules/seed.py and the caching story in ARCHITECTURE.md.
#   kind=observed  → `latest` timestamp is in the past; alarm on its age.
#   kind=forecast  → timestamps are future valid times; alarm when the horizon
#                    has collapsed (max timestamp no longer ahead of now).
# warn_s / fail_s bound the observed age (or the ingest gap for forecasts,
# measured as now - min(future coverage start)).
LAYER_EXPECTATIONS: dict[str, dict[str, Any]] = {
    "radar":           {"kind": "observed", "warn_s": 600, "fail_s": 1800, "cadence_s": 120},
    "radar-composite": {"kind": "observed", "warn_s": 600, "fail_s": 1800, "cadence_s": 120},
    "nowcast":         {"kind": "forecast", "min_horizon_s": 600, "refresh_s": 120},
    "radar-hrrr":      {"kind": "forecast", "min_horizon_s": 3600, "refresh_s": 900},
    "temperature":     {"kind": "forecast", "min_horizon_s": 3600, "refresh_s": 900},
    "dewpoint":        {"kind": "forecast", "min_horizon_s": 3600, "refresh_s": 900},
    "humidity":        {"kind": "forecast", "min_horizon_s": 3600, "refresh_s": 900},
    "wind":            {"kind": "forecast", "min_horizon_s": 3600, "refresh_s": 900},
    "cape":            {"kind": "forecast", "min_horizon_s": 3600, "refresh_s": 900},
    "precip-type":     {"kind": "forecast", "min_horizon_s": 3600, "refresh_s": 900},
    "precip-accum":    {"kind": "forecast", "min_horizon_s": 3600, "refresh_s": 900},
    "cloud":           {"kind": "forecast", "min_horizon_s": 3600, "refresh_s": 900},
}

# Cache-Control the Caddyfile promises per path class. Wrong headers are a
# performance bug: a mutable TTL on observed radar re-downloads the whole
# playback loop; an immutable TTL on forecast layers pins stale predictions.
OBSERVED_LAYERS = ("radar", "radar-composite")
OBSERVED_CACHE = "max-age=86400"
MUTABLE_CACHE = "max-age=120"

# Default probe location: geographic CONUS center — guaranteed inside every
# layer's coverage.
DEFAULT_LAT, DEFAULT_LON = 39.5, -98.35


def _slippy(lat: float, lon: float, z: int) -> tuple[int, int]:
    lat_r = math.radians(lat)
    n = 2 ** z
    x = int((lon + 180.0) / 360.0 * n)
    y = int((1.0 - math.asinh(math.tan(lat_r)) / math.pi) / 2.0 * n)
    return x, y


def _get_manifest(server: str) -> tuple[Fetch, dict | None]:
    f, body = fetch_json(f"{server}/api/manifest.json")
    if body is not None and not isinstance(body.get("layers"), dict):
        f.error = "manifest missing 'layers'"
        body = None
    return f, body


# ---------- api ----------

API_ENDPOINTS = [
    ("manifest", "/api/manifest.json", 1500),
    ("health", "/api/health", 1500),
    ("livez", "/api/livez", 500),
    ("metrics", "/api/metrics", 2000),
    ("lightning", "/api/lightning", 1500),
    ("storms", "/api/storms", 1500),
    ("tropical", "/api/tropical", 1500),
    ("forecast", f"/api/forecast/{DEFAULT_LAT}/{DEFAULT_LON}", 5000),
    ("wind-field", "/api/wind-field/latest", 3000),
]


def check_api(server: str, *, samples: int = 5) -> list[Check]:
    """Latency percentiles per endpoint. Budgets are generous 'something is
    wrong' thresholds, not SLOs — /api/health does an iterdir over the radar
    tile dir and /api/wind-field decodes two full grids, so they get more room."""
    checks: list[Check] = []
    for name, path, budget_ms in API_ENDPOINTS:
        url = f"{server}{path}"
        fetches = [fetch(url) for _ in range(samples)]
        errors = [f for f in fetches if not f.ok]
        # /api/health 503s by design when radar is stale — degraded, not down.
        degraded = name == "health" and bool(errors) and all(f.status == 503 for f in errors)
        stats = percentiles([f.elapsed_ms for f in fetches if f.ok or degraded])
        if errors and not degraded:
            checks.append(Check(
                f"api.{name}", FAIL,
                f"{len(errors)}/{samples} failed ({errors[0].error}) — {url}",
                {"stats": stats, "errors": [f.error for f in errors]},
            ))
            continue
        p95 = stats.get("p95_ms", 0)
        status = OK if p95 <= budget_ms else WARN
        note = " [503 degraded]" if degraded else ""
        checks.append(Check(
            f"api.{name}", status,
            f"p50 {stats.get('p50_ms')}ms / p95 {p95}ms (budget {budget_ms}ms){note}",
            {"stats": stats, "budget_ms": budget_ms},
        ))
    return checks


# ---------- tiles ----------


def check_tiles(
    server: str,
    *,
    lat: float = DEFAULT_LAT,
    lon: float = DEFAULT_LON,
    per_zoom: int = 9,
) -> list[Check]:
    """Sample real tiles for every manifest layer at every zoom level.

    Validates the three things that make or break map performance:
    latency, error rate, and the Cache-Control split (observed=immutable,
    forecast=120s) that the Caddyfile is supposed to enforce.
    """
    mf, manifest = _get_manifest(server)
    if manifest is None:
        return [Check("tiles.manifest", FAIL, f"cannot load manifest: {mf.error}")]

    template = manifest.get("tile_url_template", "/tiles/{layer}/{palette}/{timestamp}/{z}/{x}/{y}.png")
    checks: list[Check] = []
    for layer_name, layer in sorted(manifest["layers"].items()):
        timestamps = layer.get("timestamps") or []
        if not timestamps:
            checks.append(Check(f"tiles.{layer_name}", WARN, "no timestamps in manifest"))
            continue
        ts = layer.get("latest", timestamps[-1])
        palette = (layer.get("palettes") or ["classic"])[0]

        urls: list[str] = []
        side = max(1, int(math.sqrt(per_zoom)))
        for z in range(4, 9):
            cx, cy = _slippy(lat, lon, z)
            for dy in range(side):
                for dx in range(side):
                    x, y = cx + dx - side // 2, cy + dy - side // 2
                    urls.append(server + template.format(
                        layer=layer_name, palette=palette, timestamp=quote_ts(ts),
                        z=z, x=x, y=y,
                    ))

        with concurrent.futures.ThreadPoolExecutor(max_workers=8) as pool:
            fetches = list(pool.map(fetch, urls))

        hits = [f for f in fetches if f.ok]
        missing = [f for f in fetches if f.status == 404]
        broken = [f for f in fetches if not f.ok and f.status != 404]
        stats = percentiles([f.elapsed_ms for f in hits])

        # Cache-header validation against the Caddyfile contract.
        want = OBSERVED_CACHE if layer_name in OBSERVED_LAYERS else MUTABLE_CACHE
        bad_cache = [f for f in hits if want not in f.headers.get("cache-control", "")]

        if broken:
            status, detail = FAIL, f"{len(broken)}/{len(fetches)} errored ({broken[0].error})"
        elif not hits:
            # An all-404 latest timestamp usually means manifest committed
            # before tiles landed, or cleanup deleted the pyramid under it.
            status, detail = FAIL, f"0/{len(fetches)} tiles found for latest={ts}"
        elif bad_cache:
            status = WARN
            got = bad_cache[0].headers.get("cache-control", "<none>")
            detail = f"cache-control mismatch: want '{want}', got '{got}'"
        else:
            p95 = stats.get("p95_ms", 0)
            status = OK if p95 <= 800 else WARN
            detail = (
                f"{len(hits)} hits / {len(missing)} 404 · p50 {stats.get('p50_ms')}ms "
                f"p95 {p95}ms · avg {int(sum(f.bytes for f in hits) / max(1, len(hits)))}B "
                f"· cache '{want}' ok"
            )
        checks.append(Check(
            f"tiles.{layer_name}", status, detail,
            {"timestamp": ts, "palette": palette, "stats": stats,
             "hits": len(hits), "missing_404": len(missing), "broken": len(broken)},
        ))
    return checks


# ---------- pipeline ----------


def check_pipeline(server: str) -> list[Check]:
    """Data freshness per layer, straight from the manifest — the same signal
    the app uses to draw frames, so this is client-perceived staleness."""
    checks: list[Check] = []
    now = utcnow()

    hf, health = fetch_json(f"{server}/api/health")
    if health is not None:
        status = OK if health.get("status") == "ok" else FAIL
        checks.append(Check(
            "pipeline.health", status,
            f"status={health.get('status')} mrms_age={health.get('mrms_age_s')}s "
            f"reasons={health.get('reasons') or '[]'} nowcast={health.get('nowcast', {}).get('status')}",
            {"health": health},
        ))
    else:
        checks.append(Check("pipeline.health", FAIL, f"/api/health unreachable: {hf.error}"))

    mf, manifest = _get_manifest(server)
    if manifest is None:
        checks.append(Check("pipeline.manifest", FAIL, f"cannot load manifest: {mf.error}"))
        return checks

    for layer_name, exp in LAYER_EXPECTATIONS.items():
        layer = manifest["layers"].get(layer_name)
        if layer is None:
            checks.append(Check(f"pipeline.{layer_name}", WARN, "layer absent from manifest"))
            continue
        stamps = sorted(filter(None, (parse_iso(t) for t in layer.get("timestamps", []))))
        if not stamps:
            checks.append(Check(f"pipeline.{layer_name}", FAIL, "no parseable timestamps"))
            continue

        if exp["kind"] == "observed":
            age = (now - stamps[-1]).total_seconds()
            status = OK if age <= exp["warn_s"] else WARN if age <= exp["fail_s"] else FAIL
            # Completeness: frames present in the trailing hour vs cadence.
            window_start = now - timedelta(hours=1)
            got = sum(1 for t in stamps if t >= window_start)
            want = int(3600 / exp["cadence_s"])
            comp = got / want
            if status == OK and comp < 0.8:
                status = WARN
            checks.append(Check(
                f"pipeline.{layer_name}", status,
                f"latest {age_str(age)} old · {got}/{want} frames in trailing hour "
                f"({comp:.0%}) · {len(stamps)} total",
                {"age_s": int(age), "frames_hour": got, "expected_hour": want},
            ))
        else:
            horizon = (stamps[-1] - now).total_seconds()
            status = OK if horizon >= exp["min_horizon_s"] else WARN if horizon > 0 else FAIL
            checks.append(Check(
                f"pipeline.{layer_name}", status,
                f"forecast horizon +{age_str(max(0, horizon))} "
                f"(want ≥ +{age_str(exp['min_horizon_s'])}) · {len(stamps)} valid times",
                {"horizon_s": int(horizon), "timestamps": len(stamps)},
            ))

    unknown = set(manifest["layers"]) - set(LAYER_EXPECTATIONS)
    if unknown:
        checks.append(Check(
            "pipeline.unknown-layers", WARN,
            f"layers with no freshness expectation: {sorted(unknown)} "
            "(add them to LAYER_EXPECTATIONS in tools/debug_harness/checks.py)",
        ))

    # Point-data feeds: generated_at / feature counts.
    lf, lightning = fetch_json(f"{server}/api/lightning")
    if lightning is not None:
        gen = lightning.get("generated_at") or 0
        n = len(lightning.get("features", []))
        age = now.timestamp() - float(gen) if gen else None
        stale = age is not None and age > 120
        detail = f"{n} strikes, written {age_str(age)} ago" if age is not None else f"{n} strikes, no generated_at (service down or quiet)"
        checks.append(Check("pipeline.lightning", WARN if (stale or age is None) else OK, detail))
    else:
        checks.append(Check("pipeline.lightning", FAIL, f"unreachable: {lf.error}"))

    return checks


# ---------- client simulation ----------


def check_client_sim(
    server: str,
    *,
    lat: float = DEFAULT_LAT,
    lon: float = DEFAULT_LON,
    zoom: int = 7,
    frames: int = 5,
    layer: str = "radar",
) -> list[Check]:
    """Replay the app's radar playback access pattern from the network side.

    Playback (frontend/src/components/timeline/TimelineBar.tsx) advances a
    frame every PLAYBACK_MS=420ms, and RadarOverlay remounts its RasterSource
    per frame — so a cold client must fetch each frame's viewport tiles within
    one tick or playback shows blank frames. This fetches the same viewport
    tile set for the last N frames and reports whether the server could keep up.
    """
    TICK_BUDGET_MS = 420.0

    mf, manifest = _get_manifest(server)
    if manifest is None:
        return [Check("client.manifest", FAIL, f"cannot load manifest: {mf.error}")]
    lyr = manifest["layers"].get(layer)
    if not lyr or not lyr.get("timestamps"):
        return [Check("client.frames", FAIL, f"layer '{layer}' has no frames")]

    template = manifest.get("tile_url_template", "/tiles/{layer}/{palette}/{timestamp}/{z}/{x}/{y}.png")
    palette = (lyr.get("palettes") or ["classic"])[0]
    window = lyr["timestamps"][-frames:]

    # A phone viewport at a given zoom is roughly 3x2 tiles + a border row the
    # renderer prefetches — call it 3x3.
    cx, cy = _slippy(lat, lon, zoom)
    coords = [(cx + dx, cy + dy) for dy in (-1, 0, 1) for dx in (-1, 0, 1)]

    checks: list[Check] = []
    total_bytes = 0
    frame_times: list[float] = []
    for ts in window:
        urls = [
            server + template.format(layer=layer, palette=palette,
                                     timestamp=quote_ts(ts), z=zoom, x=x, y=y)
            for x, y in coords
        ]
        with concurrent.futures.ThreadPoolExecutor(max_workers=6) as pool:
            fetches = list(pool.map(fetch, urls))
        frame_ms = max(f.elapsed_ms for f in fetches)  # parallel fetch → slowest tile gates the frame
        misses = [f for f in fetches if not f.ok]
        total_bytes += sum(f.bytes for f in fetches)
        frame_times.append(frame_ms)
        status = OK if frame_ms <= TICK_BUDGET_MS and not misses else WARN
        checks.append(Check(
            f"client.frame[{ts}]", status,
            f"viewport z{zoom} loaded in {frame_ms:.0f}ms "
            f"({len(fetches) - len(misses)}/{len(fetches)} tiles)",
            {"frame_ms": round(frame_ms, 1), "misses": len(misses)},
        ))

    worst_ms = max(frame_times)
    verdict = OK if worst_ms <= TICK_BUDGET_MS else WARN
    checks.append(Check(
        "client.playback", verdict,
        f"worst frame {worst_ms:.0f}ms vs {TICK_BUDGET_MS:.0f}ms playback tick "
        f"· {total_bytes // 1024}KB for {len(window)} frames — "
        + ("cold playback should be smooth" if verdict == OK
           else "cold clients will see blank frames on the first loop (server-cached replays are fine)"),
        {"worst_frame_ms": round(worst_ms, 1), "budget_ms": TICK_BUDGET_MS,
         "total_kb": total_bytes // 1024},
    ))
    return checks
