"""Temporal activity for the Blitzortung lightning stream.

Lightning is a streaming source, not a poll source. We wrap the WebSocket
loop in a single long-running activity that:

  - Runs for `duration_s` (default ~50 min) before exiting cleanly
  - Heartbeats every 30s while connected
  - Seeds its buffer from the last published file, then keeps a rolling
    RETENTION_MIN buffer and flushes GeoJSON every 2s
  - Reconnects at once after a server-side close; rotates away from failing
    endpoints with per-failure backoff (see EndpointPicker)

The IngestLightningWorkflow re-launches this activity on a hourly Schedule.
SKIP overlap policy means a still-running activity simply means the next
schedule tick is dropped.
"""

from __future__ import annotations

import asyncio
import json
import os
import random
import tempfile
import time
from collections import deque
from collections.abc import Callable, Iterable
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
# Blitzortung endpoints, verified 2026-09-29: ws1/ws2/ws7/ws8 accept wss on
# 443; ws3-ws6 present a certificate for another hostname; every legacy plain
# ws port (8087-8090) refuses the connection. The old picker chose uniformly
# among 33 variants, so after each server-side close (about every 6 min) it
# burned a median 32 s (p90 2 min) on dead endpoints: ~10% of every hour.
PRIMARY_ENDPOINTS: tuple[str, ...] = tuple(
    f"wss://{host}.blitzortung.org:443/" for host in ("ws1", "ws2", "ws7", "ws8")
)
LEGACY_ENDPOINTS: tuple[str, ...] = tuple(
    f"ws://ws{i}.blitzortung.org:{port}/"
    for i in range(1, 9)
    for port in (8087, 8088, 8089, 8090)
)
# Comma-separated override, e.g. a self-hosted relay. Legacy ports stay a last resort.
_ENDPOINT_OVERRIDE = os.environ.get("LIGHTNING_WS_ENDPOINTS", "")
# A session that delivered a frame counts as healthy however it ends.
RECONNECT_AFTER_CLOSE_S = 1.0
FAILURE_BACKOFF_MAX_S = 30.0
DEMOTE_PRIMARY_S = 60.0
DEMOTE_LEGACY_S = 15 * 60.0
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
    seeded: int = 0
    connect_failures: int = 0


def configured_endpoints() -> tuple[list[str], list[str]]:
    """(primary, legacy) endpoint lists, honouring LIGHTNING_WS_ENDPOINTS."""
    override = [u.strip() for u in _ENDPOINT_OVERRIDE.split(",") if u.strip()]
    if override:
        return override, list(LEGACY_ENDPOINTS)
    return list(PRIMARY_ENDPOINTS), list(LEGACY_ENDPOINTS)


class EndpointPicker:
    """Round-robin over healthy primaries; legacy ports only when none is left.

    A failed endpoint is demoted for a while instead of being retried at random,
    and the reconnect delay backs off per consecutive failure. A delivered frame
    resets both. Pure bookkeeping so it can be unit-tested without sockets.
    """

    def __init__(
        self,
        primary: list[str],
        legacy: list[str] | None = None,
        *,
        clock: Callable[[], float] = time.monotonic,
        rand: Callable[[], float] = random.random,
    ) -> None:
        if not primary:
            raise ValueError("at least one primary endpoint is required")
        self.primary = list(primary)
        self.legacy = list(legacy or [])
        self._clock = clock
        self._rand = rand
        self._demoted_until: dict[str, float] = {}
        self._cursor = 0
        self.consecutive_failures = 0

    def _healthy(self, urls: list[str]) -> list[str]:
        now = self._clock()
        return [u for u in urls if self._demoted_until.get(u, 0.0) <= now]

    def next(self) -> str:
        candidates = self._healthy(self.primary)
        if not candidates:
            candidates = self._healthy(self.legacy)
        if not candidates:
            # Everything is demoted: forget the demotions rather than stall.
            self._demoted_until.clear()
            candidates = self.primary
        url = candidates[self._cursor % len(candidates)]
        self._cursor += 1
        return url

    def succeeded(self, url: str) -> float:
        """Record a session that delivered data; return the delay before reconnecting."""
        self.consecutive_failures = 0
        self._demoted_until.pop(url, None)
        return RECONNECT_AFTER_CLOSE_S

    def failed(self, url: str) -> float:
        """Record a session that delivered nothing; return the backoff before the next try."""
        self.consecutive_failures += 1
        demote = DEMOTE_PRIMARY_S if url in self.primary else DEMOTE_LEGACY_S
        self._demoted_until[url] = self._clock() + demote
        base = min(FAILURE_BACKOFF_MAX_S, 2.0 ** min(self.consecutive_failures, 8))
        return base * (0.5 + self._rand() * 0.5)


def _load_existing_strikes(
    path: Path | None = None,
    *,
    retention_min: int | None = None,
    now: float | None = None,
) -> list[dict]:
    """Strikes still inside the retention window from the last published file.

    A new hourly run (or a restarted worker) used to publish an empty
    collection and rebuild it over 15 minutes, blanking the layer every hour.
    """
    path = OUT_PATH if path is None else path
    retention = RETENTION_MIN if retention_min is None else retention_min
    try:
        body = json.loads(path.read_text())
    except (OSError, json.JSONDecodeError):
        return []
    if not isinstance(body, dict):
        return []
    cutoff = (time.time() if now is None else now) - retention * 60
    strikes: list[dict] = []
    for feature in body.get("features") or []:
        try:
            props = feature["properties"]
            lon, lat = feature["geometry"]["coordinates"][:2]
            t = float(props["time"])
            if t < cutoff:
                continue
            strikes.append({
                "t": t,
                "lat": float(lat),
                "lon": float(lon),
                "pol": int(props.get("polarity", 0)),
                "mds": int(props.get("mds", 0)),
            })
        except (KeyError, TypeError, ValueError, IndexError):
            continue
    strikes.sort(key=lambda s: s["t"])
    return strikes[-MAX_STRIKES:]


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


def _write_geojson(strikes: Iterable[dict]) -> None:
    """Blocking; call via to_thread with a snapshot (the live deque mutates)."""
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
    # Unique temp name: a fixed ".tmp" raced a second writer and could publish
    # a half-written file.
    fd, tmp_name = tempfile.mkstemp(prefix=f".{OUT_PATH.name}.", suffix=".tmp", dir=str(STATE_DIR))
    try:
        with os.fdopen(fd, "w") as fh:
            fh.write(json.dumps(body))
        os.replace(tmp_name, OUT_PATH)
    finally:
        try:
            os.unlink(tmp_name)
        except FileNotFoundError:
            pass


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

    async def _flush() -> None:
        # json.dumps of 5000 features on the event loop stalled every other
        # activity's heartbeats in this worker; snapshot, then write off-loop.
        await asyncio.to_thread(_write_geojson, list(strikes))

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
            try:
                await _flush()
            except Exception as exc:  # noqa: BLE001 — a write error must not stop heartbeats
                log.warning("geojson_flush_failed", extra={"err": str(exc)})
            activity.heartbeat(dict(stats))

    primary, legacy = configured_endpoints()
    picker = EndpointPicker(primary, legacy)

    # Carry the previous run's strikes forward so the layer never blanks at the
    # hourly rollover, then publish at once so the API never 404s.
    seeded = await asyncio.to_thread(_load_existing_strikes)
    strikes.extend(seeded)
    _prune()
    stats["seeded"] = len(strikes)
    await _flush()

    beat = asyncio.create_task(_beat())
    connect_failures = 0
    try:
        while time.monotonic() < deadline:
            url = picker.next()
            session_started = time.monotonic()
            session_msgs = 0
            try:
                log.info("ws_connect", extra={"url": url})
                async with websockets.connect(
                    url, ping_interval=30, ping_timeout=30, open_timeout=10
                ) as ws:
                    await ws.send(json.dumps({"a": 111}))
                    async for msg in ws:
                        if time.monotonic() >= deadline:
                            break
                        stats["msgs"] += 1
                        session_msgs += 1
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
                            await _flush()
                            last_flush = now
            except Exception as exc:  # noqa: BLE001
                if session_msgs:
                    delay = picker.succeeded(url)
                    log.info(
                        "ws_closed",
                        extra={
                            "url": url,
                            "err": str(exc),
                            "session_s": round(time.monotonic() - session_started, 1),
                            "session_msgs": session_msgs,
                        },
                    )
                else:
                    delay = picker.failed(url)
                    connect_failures += 1
                    log.warning(
                        "ws_disconnect",
                        extra={
                            "url": url,
                            "err": str(exc),
                            "consecutive_failures": picker.consecutive_failures,
                            "retry_in_s": round(delay, 1),
                        },
                    )
                await _flush()
            else:
                # The server closed a working stream (Blitzortung does this
                # every few minutes); reconnect right away.
                delay = picker.succeeded(url) if session_msgs else picker.failed(url)
                if not session_msgs:
                    connect_failures += 1
                log.info(
                    "ws_closed",
                    extra={
                        "url": url,
                        "session_s": round(time.monotonic() - session_started, 1),
                        "session_msgs": session_msgs,
                    },
                )
            remaining = deadline - time.monotonic()
            if remaining <= 0:
                break
            await asyncio.sleep(min(delay, remaining))
    finally:
        beat.cancel()
        with suppress(asyncio.CancelledError):
            await beat

    await _flush()
    return LightningRunResult(
        duration_s=round(time.time() - started, 1),
        msgs=stats["msgs"], parsed=stats["parsed"], in_bbox=stats["in_bbox"],
        final_buffer=len(strikes),
        seeded=stats.get("seeded", 0),
        connect_failures=connect_failures,
    )
