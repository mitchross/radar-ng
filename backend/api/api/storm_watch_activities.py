"""Activities owned by the API/Watch side.

Covers the storm-watch + alert + push-token persist paths so the API service
deploys these workflows without needing to import ingest-side machinery.

(Could live in temporal/shared/, but keeping it in backend/api keeps the
api-owned domain logic next to the API code.)
"""

from __future__ import annotations

import asyncio
import json
import os
import tempfile
from dataclasses import dataclass, field
from pathlib import Path
from typing import Any

import httpx
from temporalio import activity

from backend.shared.logger import get_logger
from backend.shared.push_tokens import list_for_user, upsert
from backend.shared.storm_watch import (
    FrameDiff,
    FrameSample,
    detect_change,
    latest_radar_meta,
    sample_window,
)


# ---------- shared activity I/O ----------
# Flat fields rather than nested dataclasses — the temporalio Python payload
# converter struggles to deserialise `MyDataclass | None` Union types when
# the inner dataclass lives in another module.


STATE_DIR = Path(os.environ.get("STATE_DIR", "/data/state"))
NWS_ALERTS_URL = os.environ.get("NWS_ALERTS_URL", "https://api.weather.gov/alerts/active?status=actual")
NWS_USER_AGENT = os.environ.get("NWS_USER_AGENT", "(radar-ng, mitch@example.com)")

log = get_logger("storm-watch-activities")


# ---------- push-token persistence ----------


@dataclass
class PushTokenInput:
    user_id: str
    token: str
    platform: str


@activity.defn(name="persist_push_token")
async def persist_push_token(inp: PushTokenInput) -> None:
    await asyncio.to_thread(upsert, inp.user_id, inp.token, inp.platform)


# ---------- storm-watch ----------


@dataclass
class CompareFramesInput:
    lat: float
    lng: float
    prev_timestamp: str | None
    prev_max_dbz: float | None


@dataclass
class CompareFramesResult:
    sampled: bool
    curr_timestamp: str = ""
    curr_max_dbz: float = 0.0
    prev_max_dbz: float = 0.0
    has_prev: bool = False
    max_dbz_delta: float = 0.0


@activity.defn(name="compare_radar_frames")
async def compare_radar_frames(inp: CompareFramesInput) -> CompareFramesResult:
    def _go() -> CompareFramesResult:
        meta = latest_radar_meta()
        if meta is None:
            return CompareFramesResult(sampled=False)
        sample = sample_window(meta, inp.lat, inp.lng, radius_km=20.0)
        if sample is None:
            return CompareFramesResult(sampled=False)
        if inp.prev_timestamp == sample.timestamp:
            # Same frame as last poll — no work to do.
            return CompareFramesResult(sampled=False)
        delta = (sample.max_dbz - inp.prev_max_dbz) if inp.prev_max_dbz is not None else 0.0
        return CompareFramesResult(
            sampled=True,
            curr_timestamp=sample.timestamp,
            curr_max_dbz=float(sample.max_dbz) if sample.max_dbz != float("-inf") else -99.0,
            prev_max_dbz=float(inp.prev_max_dbz) if inp.prev_max_dbz is not None else 0.0,
            has_prev=inp.prev_max_dbz is not None,
            max_dbz_delta=float(delta),
        )

    return await asyncio.to_thread(_go)


@dataclass
class DetectChangeInput:
    has_prev: bool
    curr_max_dbz: float
    prev_max_dbz: float
    max_dbz_delta: float


@dataclass
class DetectChangeResult:
    kind: str
    summary: str


@activity.defn(name="detect_storm_change")
async def detect_storm_change(inp: DetectChangeInput) -> DetectChangeResult:
    if not inp.has_prev:
        return DetectChangeResult(kind="", summary="")
    diff = FrameDiff(
        prev=FrameSample(timestamp="", max_dbz=inp.prev_max_dbz, mean_dbz=float("nan"), above_50_count=0),
        curr=FrameSample(timestamp="", max_dbz=inp.curr_max_dbz, mean_dbz=float("nan"), above_50_count=0),
        max_dbz_delta=inp.max_dbz_delta,
    )
    change = detect_change(diff)
    if change is None:
        return DetectChangeResult(kind="", summary="")
    return DetectChangeResult(kind=change.kind, summary=change.summary)


# ---------- send-push fan-out ----------


@dataclass
class FanOutPushInput:
    user_id: str
    title: str
    body: str
    collapse_id: str
    extra: dict[str, str] = field(default_factory=dict)


@dataclass
class FanOutPushResult:
    sent: int


@activity.defn(name="fan_out_push_to_user")
async def fan_out_push_to_user(inp: FanOutPushInput) -> FanOutPushResult:
    """Look up every device token registered for `user_id` and dispatch a
    push to each. The actual APNS/FCM call is delegated to
    `temporal.shared.push.send_push_notification`, scheduled as a child
    activity by the workflow that calls this — but for simplicity we
    inline the dispatch here so the workflow only needs ONE activity per
    notification.
    """
    from temporal.shared.push import (  # local import — avoids worker startup penalty for api-only deploys
        PushPayload,
        PushToken,
        send_push_notification,
    )

    tokens = await asyncio.to_thread(list_for_user, inp.user_id)
    sent = 0
    for t in tokens:
        try:
            await send_push_notification(
                PushToken(user_id=t.user_id, token=t.token, platform=t.platform),
                PushPayload(title=inp.title, body=inp.body, collapse_id=inp.collapse_id, extra=inp.extra),
            )
            sent += 1
        except Exception as exc:  # noqa: BLE001
            log.warning("push_failed_for_token", extra={"err": str(exc), "platform": t.platform})
    return FanOutPushResult(sent=sent)


# ---------- alert poll ----------


_ALERT_STATE_PATH = STATE_DIR / "alerts_seen.json"


@dataclass
class AlertForSignal:
    alert_id: str
    geometry: dict[str, Any] = field(default_factory=dict)


@dataclass
class FetchAlertsResult:
    alert_count: int
    new_alert_ids: list[str] = field(default_factory=list)
    new_alerts: list[AlertForSignal] = field(default_factory=list)


def _load_seen() -> set[str]:
    try:
        return set(json.loads(_ALERT_STATE_PATH.read_text()))
    except (OSError, json.JSONDecodeError):
        return set()


def _save_seen(active: set[str], expired: set[str]) -> None:
    """Persist seen alert ids: every currently-active id survives, expired ids
    fill the remainder of the 5000 cap. (A plain `list(set)[-5000:]` trims in
    arbitrary order and can forget an ACTIVE alert, which would re-notify it
    as new on the next poll.) Written atomically — this activity can retry,
    and a torn write here would re-fire every active alert at once.
    """
    STATE_DIR.mkdir(parents=True, exist_ok=True)
    capped = sorted(active) + sorted(expired)[: max(0, 5000 - len(active))]
    fd, tmp_name = tempfile.mkstemp(prefix=".alerts-seen.", suffix=".tmp", dir=str(STATE_DIR))
    try:
        with os.fdopen(fd, "w") as fh:
            json.dump(capped, fh)
        os.replace(tmp_name, _ALERT_STATE_PATH)
    finally:
        try:
            os.unlink(tmp_name)
        except FileNotFoundError:
            pass


def _alert_from_feature(feature: dict[str, Any]) -> AlertForSignal | None:
    props = feature.get("properties") or {}
    alert_id = props.get("id") or feature.get("id")
    if not alert_id:
        return None
    geometry = feature.get("geometry") or {}
    if not isinstance(geometry, dict):
        geometry = {}
    return AlertForSignal(alert_id=str(alert_id), geometry=geometry)


@activity.defn(name="fetch_nws_active_alerts")
async def fetch_nws_active_alerts() -> FetchAlertsResult:
    def _go() -> FetchAlertsResult:
        with httpx.Client(headers={"User-Agent": NWS_USER_AGENT}) as client:
            r = client.get(NWS_ALERTS_URL, timeout=20)
            r.raise_for_status()
            payload = r.json()
        features = payload.get("features", []) or []
        seen = _load_seen()
        new_ids: list[str] = []
        new_alerts: list[AlertForSignal] = []
        seen_after: set[str] = set()
        for f in features:
            alert = _alert_from_feature(f)
            if alert is None:
                continue
            aid = alert.alert_id
            seen_after.add(aid)
            if aid not in seen:
                new_ids.append(aid)
                new_alerts.append(alert)
        _save_seen(seen_after, seen - seen_after)
        return FetchAlertsResult(alert_count=len(features), new_alert_ids=new_ids, new_alerts=new_alerts)

    return await asyncio.to_thread(_go)


@dataclass
class SignalWatchesInput:
    alert_id: str
    geometry: dict[str, Any] = field(default_factory=dict)


@dataclass
class SignalWatchesResult:
    matched: int


def _coord(point: Any) -> tuple[float, float] | None:
    if not isinstance(point, (list, tuple)) or len(point) < 2:
        return None
    try:
        return float(point[0]), float(point[1])
    except (TypeError, ValueError):
        return None


def _point_in_ring(lng: float, lat: float, ring: Any) -> bool:
    if not isinstance(ring, list) or len(ring) < 4:
        return False

    inside = False
    prev = _coord(ring[-1])
    if prev is None:
        return False

    for point in ring:
        curr = _coord(point)
        if curr is None:
            return False

        curr_lng, curr_lat = curr
        prev_lng, prev_lat = prev
        crosses = (curr_lat > lat) != (prev_lat > lat)
        if crosses:
            intersect_lng = (prev_lng - curr_lng) * (lat - curr_lat) / (prev_lat - curr_lat) + curr_lng
            if lng < intersect_lng:
                inside = not inside
        prev = curr

    return inside


def _point_in_polygon(lng: float, lat: float, polygon: Any) -> bool:
    if not isinstance(polygon, list) or not polygon:
        return False
    if not _point_in_ring(lng, lat, polygon[0]):
        return False
    return not any(_point_in_ring(lng, lat, hole) for hole in polygon[1:])


def _point_in_geojson(lng: float, lat: float, geometry: dict[str, Any]) -> bool:
    if not geometry:
        return False
    geo_type = geometry.get("type")
    coordinates = geometry.get("coordinates")
    if geo_type == "Polygon":
        return _point_in_polygon(lng, lat, coordinates)
    if geo_type == "MultiPolygon" and isinstance(coordinates, list):
        return any(_point_in_polygon(lng, lat, polygon) for polygon in coordinates)
    return False


def _state_value(state: Any, key: str, default: Any = None) -> Any:
    if isinstance(state, dict):
        return state.get(key, default)
    return getattr(state, key, default)


@activity.defn(name="signal_matching_storm_watches")
async def signal_matching_storm_watches(inp: SignalWatchesInput) -> SignalWatchesResult:
    """Signal running WatchStormWorkflow instances whose center is inside the
    new alert polygon.

    Reuses the process-wide singleton client — this activity fires once per
    new NWS alert, and a fresh `Client.connect` per invocation leaks a gRPC
    channel each time (the Python SDK has no Client.close; channels die only
    with the process).
    """
    from backend.api.api.temporal_client import get_client

    if not inp.geometry:
        return SignalWatchesResult(matched=0)

    client = await get_client()

    matched = 0
    async for w in client.list_workflows("WorkflowType='WatchStormWorkflow' AND ExecutionStatus='Running'"):
        try:
            handle = client.get_workflow_handle(w.id)
            state = await handle.query("getCurrentState")
            lat = _state_value(state, "lat")
            lng = _state_value(state, "lng")
            if lat is None or lng is None:
                continue
            if not _point_in_geojson(float(lng), float(lat), inp.geometry):
                continue
            await handle.signal("alertMatchSignal", inp.alert_id)
            matched += 1
        except Exception as exc:  # noqa: BLE001
            log.warning("signal_failed", extra={"workflow_id": w.id, "err": str(exc)})
    return SignalWatchesResult(matched=matched)
