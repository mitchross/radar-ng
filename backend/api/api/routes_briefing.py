"""Optional AI weather narration through a self-hosted LLM (LiteLLM / vLLM).

- GET /api/briefing?lat=&lon=[&place=]  — a short, friendly summary of what the
  radar, nowcast, forecast, alerts and storm tracks say for one place.
- GET /api/alerts/explain?id=            — a plain-English "what / what to do"
  for one NWS alert.

Decoration only: both always answer 200 and return {"available": false} when
the LLM is unconfigured, slow, failing or tripped open. Nothing else in the API
calls them, so an LLM outage can never break radar, forecast or alerts.
"""

from __future__ import annotations

import asyncio
import json
import math
import os
import re
import time
from collections import OrderedDict
from datetime import datetime, timedelta, timezone
from pathlib import Path

import httpx
from fastapi import APIRouter, Query, Request
from fastapi.responses import JSONResponse

LLM_BASE_URL = os.environ.get("LLM_BASE_URL", "").rstrip("/")
LLM_API_KEY = os.environ.get("LLM_API_KEY", "")
LLM_MODEL = os.environ.get("LLM_MODEL", "qwen3.8-27b")
LLM_TIMEOUT_S = float(os.environ.get("LLM_TIMEOUT_S", "20"))
LLM_MAX_CONCURRENCY = int(os.environ.get("LLM_MAX_CONCURRENCY", "2"))
BRIEFING_TTL_S = int(os.environ.get("BRIEFING_TTL_S", "600"))
BREAKER_FAILURES = int(os.environ.get("LLM_BREAKER_FAILURES", "3"))
BREAKER_COOLDOWN_S = int(os.environ.get("LLM_BREAKER_COOLDOWN_S", "300"))
STATE_DIR = os.environ.get("STATE_DIR", "/data/state")

# Qwen's documented non-thinking sampler; thinking off keeps a briefing to ~1-3 s.
_SAMPLER = {
    "temperature": 0.7, "top_p": 0.8, "top_k": 20, "min_p": 0.0,
    "presence_penalty": 1.5, "repetition_penalty": 1.0,
    "chat_template_kwargs": {"enable_thinking": False, "preserve_thinking": False},
}

router = APIRouter()


class _Breaker:
    """Stop calling a failing LLM for a cooldown after N consecutive failures."""

    def __init__(self) -> None:
        self.failures = 0
        self.open_until = 0.0

    def allow(self) -> bool:
        return time.monotonic() >= self.open_until

    def record(self, ok: bool) -> None:
        if ok:
            self.failures = 0
            return
        self.failures += 1
        if self.failures >= BREAKER_FAILURES:
            self.open_until = time.monotonic() + BREAKER_COOLDOWN_S
            self.failures = 0


_breaker = _Breaker()
_semaphore: asyncio.Semaphore | None = None
_cache: OrderedDict[str, tuple[float, dict]] = OrderedDict()
_inflight: dict[str, asyncio.Future] = {}


def _unavailable(reason: str) -> JSONResponse:
    return JSONResponse({"available": False, "reason": reason}, headers={"Cache-Control": "no-store"})


def _cache_get(key: str) -> dict | None:
    hit = _cache.get(key)
    if hit and time.time() - hit[0] < BRIEFING_TTL_S:
        return hit[1]
    return None


def _cache_put(key: str, value: dict) -> None:
    _cache[key] = (time.time(), value)
    _cache.move_to_end(key)
    while len(_cache) > 512:
        _cache.popitem(last=False)


async def _chat(client: httpx.AsyncClient, system: str, user: str, max_tokens: int) -> dict | None:
    """One JSON-mode completion, or None on any failure (never raises)."""
    global _semaphore
    if not LLM_BASE_URL or not _breaker.allow():
        return None
    if _semaphore is None:
        _semaphore = asyncio.Semaphore(LLM_MAX_CONCURRENCY)
    if _semaphore.locked():
        return None  # busy: skip rather than queue behind other narrations
    async with _semaphore:
        try:
            resp = await client.post(
                f"{LLM_BASE_URL}/chat/completions",
                headers={"Authorization": f"Bearer {LLM_API_KEY}"} if LLM_API_KEY else {},
                json={
                    "model": LLM_MODEL,
                    "messages": [{"role": "system", "content": system}, {"role": "user", "content": user}],
                    "max_tokens": max_tokens,
                    "response_format": {"type": "json_object"},
                    **_SAMPLER,
                },
                timeout=LLM_TIMEOUT_S,
            )
            resp.raise_for_status()
            content = resp.json()["choices"][0]["message"]["content"] or ""
            parsed = _parse_json(content)
        except (httpx.HTTPError, ValueError, KeyError, IndexError, TypeError):
            _breaker.record(False)
            return None
    _breaker.record(parsed is not None)
    return parsed


def _parse_json(content: str) -> dict | None:
    content = re.sub(r"<think>.*?</think>", "", content, flags=re.S).strip()
    match = re.search(r"\{.*\}", content, flags=re.S)
    if not match:
        return None
    try:
        data = json.loads(match.group(0))
    except json.JSONDecodeError:
        return None
    return data if isinstance(data, dict) else None


def _clean(text: object, limit: int) -> str:
    return re.sub(r"\s+", " ", str(text or "")).strip()[:limit]


# ---------- facts ----------

WMO = {
    0: "clear", 1: "mostly clear", 2: "partly cloudy", 3: "overcast", 45: "fog", 48: "freezing fog",
    51: "light drizzle", 53: "drizzle", 55: "heavy drizzle", 61: "light rain", 63: "rain", 65: "heavy rain",
    66: "freezing rain", 67: "heavy freezing rain", 71: "light snow", 73: "snow", 75: "heavy snow",
    77: "snow grains", 80: "rain showers", 81: "heavy showers", 82: "violent showers", 85: "snow showers",
    86: "heavy snow showers", 95: "thunderstorms", 96: "thunderstorms with hail", 99: "severe thunderstorms with hail",
}


def _local(ts: str, offset_s: int) -> str:
    dt = datetime.fromisoformat(ts.replace("Z", "+00:00"))
    if dt.tzinfo is None:  # Open-Meteo times are already local when a timezone is requested
        return dt.strftime("%-I:%M %p")
    return (dt + timedelta(seconds=offset_s)).strftime("%-I:%M %p")


def summarize_nowcast(points: list[dict], offset_s: int) -> str:
    """'dry' or rain start/peak/end within the next hour from the per-point nowcast."""
    wet = [p for p in points if (p.get("dbz") or -99) >= 15]
    if not wet:
        return "dry for the next hour"
    peak = max(wet, key=lambda p: p["dbz"])
    first, last = wet[0], wet[-1]
    parts = [
        f"rain from about {_local(first['timestamp'], offset_s)}",
        f"peaking near {_local(peak['timestamp'], offset_s)} at {round(peak['dbz'])} dBZ",
    ]
    after = points.index(last) + 1
    if after < len(points):
        parts.append(f"ending around {_local(points[after]['timestamp'], offset_s)}")
    return ", ".join(parts)


def _haversine_km(lon1: float, lat1: float, lon2: float, lat2: float) -> float:
    p1, p2 = math.radians(lat1), math.radians(lat2)
    h = math.sin((p2 - p1) / 2) ** 2 + math.cos(p1) * math.cos(p2) * math.sin(math.radians(lon2 - lon1) / 2) ** 2
    return 2 * 6371.0 * math.asin(min(1.0, math.sqrt(h)))


def incoming_storm(storms: dict, lat: float, lon: float, radius_km: float = 12) -> dict | None:
    """Soonest tracked cell whose next-hour path passes within radius_km (mirrors the web card)."""
    best = None
    for f in storms.get("features", []):
        try:
            clon, clat = f["geometry"]["coordinates"]
            p = f["properties"]
            v = p.get("tracking_vector") or {}
        except (KeyError, TypeError, ValueError):
            continue
        tracked = len(p.get("track_history") or []) >= 3 and float(p.get("tracking_confidence", 0)) >= 0.4
        for minutes in range(0, 65, 5):
            if minutes and not tracked:
                break
            h = minutes / 60
            plon = clon + float(v.get("east_kmh", 0)) * h / max(1e-6, 111.32 * math.cos(math.radians(clat)))
            plat = clat + float(v.get("north_kmh", 0)) * h / 110.574
            if _haversine_km(plon, plat, lon, lat) <= radius_km:
                if best is None or minutes < best["minutes"]:
                    best = {"minutes": minutes, "peak_dbz": round(float(p.get("peak_dbz", 0))),
                            "speed_mph": round(float(v.get("speed_kmh", 0)) * 0.621)}
                break
    return best


def _r(value: object) -> int | None:
    return round(value) if isinstance(value, (int, float)) else None


def build_facts(place: str, forecast: dict, nowcast: dict, alerts: list[dict], storm: dict | None) -> dict:
    offset = int(forecast.get("utc_offset_seconds") or 0)
    cur = forecast.get("current") or {}
    hourly = forecast.get("hourly") or {}
    daily = forecast.get("daily") or {}
    now_local = datetime.now(timezone.utc) + timedelta(seconds=offset)
    times = hourly.get("time") or []
    start = next((i for i, t in enumerate(times) if t >= now_local.strftime("%Y-%m-%dT%H:00")), 0)
    next_hours = []
    for i in range(start, min(start + 12, len(times)), 3):
        def at(key: str):
            values = hourly.get(key) or []
            return values[i] if i < len(values) else None
        next_hours.append({
            "time": datetime.fromisoformat(times[i]).strftime("%-I %p"),
            "temp_f": _r(at("temperature_2m")),
            "sky": WMO.get(at("weather_code"), "unknown"),
            "rain_in": at("precipitation"),
        })
    first = lambda key: (daily.get(key) or [None])[0]  # noqa: E731
    return {
        "place": place or "this spot",
        "local_time": now_local.strftime("%A %-I:%M %p"),
        "now": {
            "temp_f": _r(cur.get("temperature_2m")),
            "feels_like_f": _r(cur.get("apparent_temperature")),
            "sky": WMO.get(cur.get("weather_code"), "unknown"),
            "wind_mph": _r(cur.get("wind_speed_10m")),
            "gusts_mph": _r(cur.get("wind_gusts_10m")),
        },
        "next_hour_radar": summarize_nowcast(nowcast.get("points") or [], offset)
        if nowcast.get("points") else "radar nowcast unavailable",
        "incoming_storm": storm,
        "next_12_hours": next_hours,
        "today": {"high_f": _r(first("temperature_2m_max")), "low_f": _r(first("temperature_2m_min")),
                  "rain_total_in": first("precipitation_sum")},
        "active_alerts": [{"event": a.get("event"), "until": a.get("ends") or a.get("expires")} for a in alerts[:3]],
    }


BRIEFING_SYSTEM = (
    "You narrate the weather for a personal radar app. Be warm, a little playful, and concise. "
    "Use ONLY the JSON facts given; never invent numbers, alerts or storms. If there is an active alert or an "
    "incoming storm, say that first and plainly. Use local clock times like 2:40 PM. "
    'Reply as JSON: {"headline": "<= 8 words", "body": "1-2 short sentences, <= 45 words"}.'
)

EXPLAIN_SYSTEM = (
    "You explain official NWS weather alerts to a regular person. Use ONLY the alert text given. "
    'Reply as JSON: {"what": "one plain sentence on what is happening and where", '
    '"do": "one plain sentence on what to do"}. No hype, no emojis.'
)


def _json(resp: JSONResponse) -> dict:
    try:
        return json.loads(resp.body)
    except (ValueError, AttributeError):
        return {}


async def _best_effort(coro, default):
    try:
        return await coro
    except Exception:  # noqa: BLE001 — each input is optional; the briefing works with what it has
        return default


async def _gather(request: Request, server, routes_places, lat: float, lon: float) -> tuple[dict, dict, dict, list[dict]]:
    async def forecast():
        return _json(await server.get_forecast(lat, lon))

    async def nowcast():
        return _json(await asyncio.to_thread(server.nowcast_point, lat, lon))

    async def storms():
        path = Path(STATE_DIR) / "storms.json"
        return json.loads(await asyncio.to_thread(path.read_text))

    async def alerts():
        body = _json(await routes_places.get_alerts(request, lat, lon))
        return [f.get("properties", {}) for f in body.get("features", [])]

    return await asyncio.gather(
        _best_effort(forecast(), {}), _best_effort(nowcast(), {}),
        _best_effort(storms(), {}), _best_effort(alerts(), []),
    )


@router.get("/api/briefing")
async def get_briefing(
    request: Request,
    lat: float = Query(..., ge=-90, le=90),
    lon: float = Query(..., ge=-180, le=180),
    place: str = Query("", max_length=80),
) -> JSONResponse:
    if not LLM_BASE_URL:
        return _unavailable("not_configured")
    if not _breaker.allow():
        return _unavailable("llm_unavailable")
    from backend.api.api import routes_places, server  # late: server imports this router

    place = _clean(place, 60)
    key = f"{round(lat, 1)},{round(lon, 1)},{place.lower()}"
    cached = _cache_get(key)
    if cached is not None:
        return JSONResponse(cached, headers={"Cache-Control": "public, max-age=120"})
    if key in _inflight:  # one LLM call per area, however many tabs ask at once
        try:
            body = await asyncio.shield(_inflight[key])
        except Exception:  # noqa: BLE001
            body = None
        return JSONResponse(body) if body else _unavailable("llm_unavailable")

    loop = asyncio.get_running_loop()
    future: asyncio.Future = loop.create_future()
    _inflight[key] = future
    body: dict | None = None
    try:
        forecast, nowcast, storms, alerts = await _gather(request, server, routes_places, lat, lon)
        if forecast.get("current"):
            facts = build_facts(place, forecast, nowcast, alerts, incoming_storm(storms, lat, lon))
            reply = await _chat(server.app.state.forecast_http, BRIEFING_SYSTEM, json.dumps(facts), 160)
            if reply and reply.get("body"):
                body = {
                    "available": True,
                    "headline": _clean(reply.get("headline"), 80),
                    "body": _clean(reply.get("body"), 400),
                    "generated_at": datetime.now(timezone.utc).isoformat(),
                    "model": LLM_MODEL,
                }
                _cache_put(key, body)
    finally:
        _inflight.pop(key, None)
        if not future.done():
            future.set_result(body)
    if body is None:
        return _unavailable("llm_unavailable")
    return JSONResponse(body, headers={"Cache-Control": "public, max-age=120"})


@router.get("/api/alerts/explain")
async def explain_alert(request: Request, id: str = Query(..., min_length=8, max_length=200)) -> JSONResponse:
    if not LLM_BASE_URL:
        return _unavailable("not_configured")
    if not id.startswith("urn:oid:") and not id.startswith("https://api.weather.gov/alerts/"):
        return _unavailable("bad_id")
    key = f"alert:{id}"
    cached = _cache_get(key)
    if cached is not None:
        return JSONResponse(cached, headers={"Cache-Control": "public, max-age=600"})
    from backend.api.api import routes_places, server

    alert_id = id.rsplit("/", 1)[-1]
    try:
        resp = await server.app.state.forecast_http.get(
            f"https://api.weather.gov/alerts/{alert_id}",
            headers={"User-Agent": routes_places.NWS_USER_AGENT, "Accept": "application/geo+json"},
            timeout=8,
        )
        resp.raise_for_status()
        props = resp.json().get("properties", {})
    except (httpx.HTTPError, ValueError):
        return _unavailable("alert_unavailable")
    text = {
        "event": props.get("event"), "area": _clean(props.get("areaDesc"), 300),
        "headline": props.get("headline"), "description": _clean(props.get("description"), 1500),
        "instruction": _clean(props.get("instruction"), 600), "ends": props.get("ends") or props.get("expires"),
    }
    reply = await _chat(server.app.state.forecast_http, EXPLAIN_SYSTEM, json.dumps(text), 140)
    if not reply or not reply.get("what"):
        return _unavailable("llm_unavailable")
    body = {"available": True, "what": _clean(reply.get("what"), 300), "do": _clean(reply.get("do"), 300), "model": LLM_MODEL}
    _cache_put(key, body)
    return JSONResponse(body, headers={"Cache-Control": "public, max-age=600"})
