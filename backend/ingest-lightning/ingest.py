#!/usr/bin/env python3
"""Lightning ingest — streams real-time strikes from Blitzortung's public
WebSocket and maintains a rolling buffer of the last N minutes.

Output: `/data/state/lightning.json` — a compact GeoJSON FeatureCollection the
tile-server's /api/lightning endpoint can serve directly.

Strikes older than RETENTION_MIN are pruned on every message. No fancy
persistence — lightning is inherently ephemeral, and a 15-minute rolling
window is what the app needs for an "active storms" overlay.

Blitzortung protocol notes (reverse-engineered from lightningmaps.org):
- URL: ws://ws{1..8}.blitzortung.org:80{87..90}/
- Must send `{"a": 111}` after connect to request the live feed
- Each message is newline-delimited JSON objects with `time`, `lat`, `lon`,
  `alt`, `pol`, `mds`, `mcg`, `status`, `region`, `delay`, `sig`
- `time` is nanoseconds since epoch
"""

from __future__ import annotations

import asyncio
import json
import os
import random
import sys
import time
from collections import deque
from pathlib import Path

import websockets

sys.path.insert(0, str(Path(__file__).resolve().parent / "shared"))
sys.path.insert(0, "/app/shared")
from logger import get_logger  # type: ignore  # noqa: E402

STATE_DIR = Path(os.environ.get("STATE_DIR", "/data/state"))
OUT_PATH = STATE_DIR / "lightning.json"
RETENTION_MIN = int(os.environ.get("LIGHTNING_RETENTION_MIN", "15"))
FLUSH_EVERY_S = float(os.environ.get("LIGHTNING_FLUSH_S", "2.0"))
MAX_STRIKES = int(os.environ.get("LIGHTNING_MAX_STRIKES", "5000"))

# Regional filter — skip strikes outside this bbox. Default covers CONUS +
# Atlantic basin so hurricane-season activity is captured.
BBOX = (
    float(os.environ.get("LIGHTNING_LAT_MIN", "15")),
    float(os.environ.get("LIGHTNING_LAT_MAX", "55")),
    float(os.environ.get("LIGHTNING_LON_MIN", "-130")),
    float(os.environ.get("LIGHTNING_LON_MAX", "-50")),
)

log = get_logger("ingest-lightning")

_strikes: deque[dict] = deque(maxlen=MAX_STRIKES)


def _prune() -> None:
    cutoff = time.time() - RETENTION_MIN * 60
    while _strikes and _strikes[0]["t"] < cutoff:
        _strikes.popleft()


def _in_bbox(lat: float, lon: float) -> bool:
    return BBOX[0] <= lat <= BBOX[1] and BBOX[2] <= lon <= BBOX[3]


def _write_geojson() -> None:
    STATE_DIR.mkdir(parents=True, exist_ok=True)
    features = [
        {
            "type": "Feature",
            "geometry": {"type": "Point", "coordinates": [s["lon"], s["lat"]]},
            "properties": {
                "time": s["t"],
                "age_s": int(time.time() - s["t"]),
                "polarity": s.get("pol", 0),
                "mds": s.get("mds", 0),
            },
        }
        for s in _strikes
    ]
    body = {
        "type": "FeatureCollection",
        "features": features,
        "generated_at": time.time(),
        "retention_min": RETENTION_MIN,
    }
    tmp = OUT_PATH.with_suffix(".tmp")
    tmp.write_text(json.dumps(body))
    tmp.replace(OUT_PATH)


# Blitzortung encodes the JSON payload with a standard LZW decompression.
# Reference implementation: lightningmaps.org browser JS + bo-python.
def _decode_payload(raw: str) -> str:
    """Decode Blitzortung's LZW string compression into JSON.

    Payloads always start with `{` (the opening brace of the JSON dict) — it
    is the first literal char, NOT a marker that the stream is uncompressed.
    Always run the decoder; if the result already is valid JSON, great.
    """
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


async def _ws_loop() -> None:
    """Reconnecting Blitzortung consumer. Never exits — restarts on disconnect.

    Many home/hosting networks block the non-standard 8087-8090 ports
    Blitzortung historically used, so we also try the TLS variants on 443.
    """
    # ws2 is the only host whose TLS cert matches the hostname (as of 2026-04).
    # Others terminate TLS with a wildcard for a different domain, so wss:// to
    # any of ws{1,3..8} fails cert verification. Keeping ws2 + all as fallback.
    primary_host = "ws2.blitzortung.org"
    fallback_hosts = [f"ws{i}.blitzortung.org" for i in range(1, 9) if i != 2]
    variants = [(primary_host, "wss", 443)] * 5 + [
        (h, "ws", p) for h in fallback_hosts for p in (8087, 8088, 8089, 8090)
    ]
    last_flush = 0.0

    while True:
        host, scheme, port = random.choice(variants)
        url = f"{scheme}://{host}:{port}/"
        try:
            log.info("ws_connect", extra={"url": url})
            async with websockets.connect(url, ping_interval=30, ping_timeout=30) as ws:
                await ws.send(json.dumps({"a": 111}))
                msg_count = 0
                parsed_count = 0
                bbox_count = 0
                async for msg in ws:
                    msg_count += 1
                    # Blitzortung sometimes sends bytes, sometimes str. The LZW
                    # decode works on the same codepoint stream either way.
                    if isinstance(msg, bytes):
                        try:
                            msg = msg.decode("latin-1")  # preserve 0..255 codes
                        except UnicodeDecodeError:
                            continue
                    elif not isinstance(msg, str):
                        continue
                    # Try raw-JSON first (some endpoints send uncompressed),
                    # fall back to LZW decode. If BOTH fail, log + drop.
                    data = None
                    for attempt in (msg, _decode_payload(msg)):
                        try:
                            data = json.loads(attempt)
                            break
                        except json.JSONDecodeError:
                            continue
                    if data is None:
                        if msg_count <= 3:
                            log.warning("json_decode_failed", extra={"sample": msg[:120]})
                        continue
                    parsed_count += 1
                    lat = float(data.get("lat", 0))
                    lon = float(data.get("lon", 0))
                    t_ns = int(data.get("time", 0))
                    if not _in_bbox(lat, lon) or t_ns == 0:
                        continue
                    bbox_count += 1
                    _strikes.append({
                        "t": t_ns / 1e9,
                        "lat": lat,
                        "lon": lon,
                        "pol": int(data.get("pol", 0)),
                        "mds": int(data.get("mds", 0)),
                    })
                    _prune()
                    now = time.time()
                    if now - last_flush >= FLUSH_EVERY_S:
                        _write_geojson()
                        last_flush = now
                        if msg_count % 100 == 0:
                            log.info(
                                "stats",
                                extra={
                                    "msgs": msg_count,
                                    "parsed": parsed_count,
                                    "in_bbox": bbox_count,
                                    "buffer": len(_strikes),
                                },
                            )
        except Exception as exc:  # noqa: BLE001
            log.warning("ws_disconnect", extra={"url": url, "err": str(exc)})
            # Even if no strikes land, periodically flush an empty set so the
            # app sees fresh data and doesn't show stale strikes.
            _prune()
            _write_geojson()
            await asyncio.sleep(5 + random.random() * 5)


def main() -> None:
    log.info(
        "startup",
        extra={
            "bbox": BBOX,
            "retention_min": RETENTION_MIN,
            "flush_every_s": FLUSH_EVERY_S,
            "out": str(OUT_PATH),
        },
    )
    _write_geojson()  # emit empty collection immediately so the endpoint never 404s
    try:
        asyncio.run(_ws_loop())
    except KeyboardInterrupt:
        pass


if __name__ == "__main__":
    main()
