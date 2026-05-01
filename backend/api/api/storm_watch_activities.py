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
import time
from dataclasses import dataclass, field
from pathlib import Path

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
class FetchAlertsResult:
    alert_count: int
    new_alert_ids: list[str] = field(default_factory=list)


def _load_seen() -> set[str]:
    try:
        return set(json.loads(_ALERT_STATE_PATH.read_text()))
    except (OSError, json.JSONDecodeError):
        return set()


def _save_seen(ids: set[str]) -> None:
    STATE_DIR.mkdir(parents=True, exist_ok=True)
    # cap at 5000 to keep file small
    capped = list(ids)[-5000:]
    _ALERT_STATE_PATH.write_text(json.dumps(capped))


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
        seen_after: set[str] = set()
        for f in features:
            aid = (f.get("properties") or {}).get("id") or f.get("id")
            if not aid:
                continue
            seen_after.add(aid)
            if aid not in seen:
                new_ids.append(aid)
        _save_seen(seen | seen_after)
        return FetchAlertsResult(alert_count=len(features), new_alert_ids=new_ids)

    return await asyncio.to_thread(_go)


@dataclass
class SignalWatchesInput:
    alert_id: str


@dataclass
class SignalWatchesResult:
    matched: int


@activity.defn(name="signal_matching_storm_watches")
async def signal_matching_storm_watches(inp: SignalWatchesInput) -> SignalWatchesResult:
    """Look up every running WatchStormWorkflow whose center sits inside the
    new alert's polygon, signal each with `alertMatchSignal(alert_id)`.

    For simplicity in v1 we signal *every* running watch — the watch
    workflow does the geo check itself when it receives the signal. This
    avoids us re-implementing polygon-in-polygon here. Refined fan-out
    by spatial index lands when the watch count exceeds ~10k.
    """
    from temporalio.client import Client

    target = os.environ.get("TEMPORAL_ADDRESS", "localhost:7233")
    namespace = os.environ.get("TEMPORAL_NAMESPACE", "default")
    client = await Client.connect(target, namespace=namespace)

    matched = 0
    async for w in client.list_workflows("WorkflowType='WatchStormWorkflow' AND ExecutionStatus='Running'"):
        try:
            handle = client.get_workflow_handle(w.id)
            await handle.signal("alertMatchSignal", inp.alert_id)
            matched += 1
        except Exception as exc:  # noqa: BLE001
            log.warning("signal_failed", extra={"workflow_id": w.id, "err": str(exc)})
    return SignalWatchesResult(matched=matched)
