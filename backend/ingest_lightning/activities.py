"""Temporal activity for the Blitzortung lightning stream.

Lightning is a streaming source, not a poll source. We wrap the WebSocket
loop in a single long-running activity that:

  - Runs for `duration_s` (default ~50 min) before exiting cleanly
  - Heartbeats every 30s while connected
  - Maintains a rolling RETENTION_MIN buffer and flushes GeoJSON every 2s
  - Reconnects on disconnect with exponential backoff, ws-host rotation

The IngestLightningWorkflow re-launches this activity on a hourly Schedule.
SKIP overlap policy means a still-running activity simply means the next
schedule tick is dropped.
"""

from __future__ import annotations

import asyncio
import json
import os
import random
import time
from collections import deque
from contextlib import suppress
from dataclasses import dataclass
from pathlib import Path

import websockets
from temporalio import activity

from backend.shared.logger import get_logger


STATE_DIR = Path(os.environ.get("STATE_DIR", "/data/state"))
OUT_PATH = STATE_DIR / "lightning.json"
RETENTION_MIN = int(os.environ.get("LIGHTNING_RETENTION_MIN", "15"))
FLUSH_EVERY_S = float(os.environ.get("LIGHTNING_FLUSH_S", "2.0"))
MAX_STRIKES = int(os.environ.get("LIGHTNING_MAX_STRIKES", "5000"))
BBOX = (
    float(os.environ.get("LIGHTNING_LAT_MIN", "15")),
    float(os.environ.get("LIGHTNING_LAT_MAX", "55")),
    float(os.environ.get("LIGHTNING_LON_MIN", "-130")),
    float(os.environ.get("LIGHTNING_LON_MAX", "-50")),
)

log = get_logger("ingest-lightning-activities")


@dataclass
class LightningRunResult:
    duration_s: float
    msgs: int
    parsed: int
    in_bbox: int
    final_buffer: int


def _decode_payload(raw: str) -> str:
    if not raw:
        return raw
    try:
        dict_codes: dict[int, str] = {}
        curr = raw[0]
        result = [curr]
        code = 256
        for i in range(1, len(raw)):
            ch_code = ord(raw[i])
            if ch_code < 256:
                entry = raw[i]
            elif ch_code in dict_codes:
                entry = dict_codes[ch_code]
            else:
                entry = curr + curr[0]
            result.append(entry)
            dict_codes[code] = curr + entry[0]
            code += 1
            curr = entry
        return "".join(result)
    except Exception:  # noqa: BLE001
        return raw


def _in_bbox(lat: float, lon: float) -> bool:
    return BBOX[0] <= lat <= BBOX[1] and BBOX[2] <= lon <= BBOX[3]


def _write_geojson(strikes: deque[dict]) -> None:
    STATE_DIR.mkdir(parents=True, exist_ok=True)
    now = time.time()
    features = [
        {
            "type": "Feature",
            "geometry": {"type": "Point", "coordinates": [s["lon"], s["lat"]]},
            "properties": {
                "time": s["t"],
                "age_s": int(now - s["t"]),
                "polarity": s.get("pol", 0),
                "mds": s.get("mds", 0),
            },
        }
        for s in strikes
    ]
    body = {
        "type": "FeatureCollection",
        "features": features,
        "generated_at": now,
        "retention_min": RETENTION_MIN,
    }
    tmp = OUT_PATH.with_suffix(".tmp")
    tmp.write_text(json.dumps(body))
    tmp.replace(OUT_PATH)


@activity.defn(name="lightning_consume_stream")
async def lightning_consume_stream(duration_s: int) -> LightningRunResult:
    """Run the Blitzortung WS consumer for up to `duration_s` seconds.

    Exits cleanly on deadline so the next schedule fire can pick up.
    """
    deadline = time.monotonic() + duration_s
    started = time.time()
    strikes: deque[dict] = deque(maxlen=MAX_STRIKES)
    stats = {"msgs": 0, "parsed": 0, "in_bbox": 0, "buffer": 0}
    last_flush = 0.0

    def _prune() -> None:
        cutoff = time.time() - RETENTION_MIN * 60
        while strikes and strikes[0]["t"] < cutoff:
            strikes.popleft()

    # Heartbeat from a background task on a fixed 30s cadence, decoupled from
    # message arrival. Blitzortung frames can stop for long stretches (quiet
    # weather, a half-open socket), during which the `async for` below blocks;
    # heartbeating inline would then starve and trip Temporal's heartbeat
    # timeout even though the activity is healthy. The same tick also flushes
    # and prunes so an idle stream still ages old strikes out of the buffer.
    async def _beat() -> None:
        while True:
            await asyncio.sleep(30)
            _prune()
            stats["buffer"] = len(strikes)
            _write_geojson(strikes)
            activity.heartbeat(dict(stats))

    primary_host = "ws2.blitzortung.org"
    fallback_hosts = [f"ws{i}.blitzortung.org" for i in range(1, 9) if i != 2]
    variants = [(primary_host, "wss", 443)] * 5 + [
        (h, "ws", p) for h in fallback_hosts for p in (8087, 8088, 8089, 8090)
    ]

    # Always emit an empty file at start so the API never 404s.
    _write_geojson(strikes)

    beat = asyncio.create_task(_beat())
    try:
        while time.monotonic() < deadline:
            host, scheme, port = random.choice(variants)
            url = f"{scheme}://{host}:{port}/"
            try:
                log.info("ws_connect", extra={"url": url})
                async with websockets.connect(url, ping_interval=30, ping_timeout=30) as ws:
                    await ws.send(json.dumps({"a": 111}))
                    async for msg in ws:
                        if time.monotonic() >= deadline:
                            break
                        stats["msgs"] += 1
                        if isinstance(msg, bytes):
                            try:
                                msg = msg.decode("latin-1")
                            except UnicodeDecodeError:
                                continue
                        elif not isinstance(msg, str):
                            continue
                        data = None
                        for attempt in (msg, _decode_payload(msg)):
                            try:
                                data = json.loads(attempt)
                                break
                            except json.JSONDecodeError:
                                continue
                        if data is None:
                            continue
                        stats["parsed"] += 1
                        lat = float(data.get("lat", 0))
                        lon = float(data.get("lon", 0))
                        t_ns = int(data.get("time", 0))
                        if not _in_bbox(lat, lon) or t_ns == 0:
                            continue
                        stats["in_bbox"] += 1
                        strikes.append({
                            "t": t_ns / 1e9,
                            "lat": lat,
                            "lon": lon,
                            "pol": int(data.get("pol", 0)),
                            "mds": int(data.get("mds", 0)),
                        })
                        _prune()
                        now = time.time()
                        if now - last_flush >= FLUSH_EVERY_S:
                            _write_geojson(strikes)
                            last_flush = now
            except Exception as exc:  # noqa: BLE001
                log.warning("ws_disconnect", extra={"url": url, "err": str(exc)})
                _write_geojson(strikes)
                await asyncio.sleep(5 + random.random() * 5)
    finally:
        beat.cancel()
        with suppress(asyncio.CancelledError):
            await beat

    _write_geojson(strikes)
    return LightningRunResult(
        duration_s=round(time.time() - started, 1),
        msgs=stats["msgs"], parsed=stats["parsed"], in_bbox=stats["in_bbox"],
        final_buffer=len(strikes),
    )
