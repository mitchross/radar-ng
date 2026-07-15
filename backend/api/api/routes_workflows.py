"""Workflow-driven API routes — push tokens + storm watches.

These endpoints are the only path mobile uses to talk to Temporal: the
mobile app NEVER opens a gRPC connection to the Temporal frontend
directly (auth, port exposure, RN gRPC pain — see design spec §4).
"""

from __future__ import annotations

import os
import asyncio
import base64
import binascii
import hashlib
import hmac
import json
import logging
import time
from typing import Any, Literal, NoReturn

from fastapi import APIRouter, Depends, Header, HTTPException
from pydantic import BaseModel, Field
from temporalio.common import WorkflowIDReusePolicy
from temporalio.exceptions import WorkflowAlreadyStartedError
from temporalio.service import RPCError

from backend.api.api.temporal_client import get_client, reset_client
from backend.shared.push_tokens import delete_for_user, upsert
from temporal.task_queues import ALERTS_TASK_QUEUE


TASK_QUEUE = os.environ.get("TEMPORAL_ALERTS_TASK_QUEUE", ALERTS_TASK_QUEUE)
SIGNING_KEY = os.environ.get(
    "WORKFLOW_AUTH_SIGNING_KEY", os.environ.get("WORKFLOW_API_SIGNING_KEY", "")
).encode()
router = APIRouter(prefix="/v1")
log = logging.getLogger(__name__)


# ---------- request/response models ----------


# These endpoints require a signed per-user session token. Fields remain
# bounded as defense in depth against oversized workflow histories or storage
# records. APNS tokens are 64 hex chars; FCM tokens can be ~200 chars.


class RegisterPushTokenBody(BaseModel):
    user_id: str = Field(min_length=1, max_length=128)
    token: str = Field(min_length=16, max_length=512)
    platform: Literal["ios", "android"]


class StartWatchBody(BaseModel):
    storm_cell_id: str = Field(min_length=1, max_length=128)
    user_id: str = Field(min_length=1, max_length=128)
    lat: float = Field(ge=-90.0, le=90.0)
    lng: float = Field(ge=-180.0, le=180.0)


class WorkflowStartedResponse(BaseModel):
    workflow_id: str
    run_id: str


class PushTokenRegisteredResponse(BaseModel):
    registered: bool = True


class WatchState(BaseModel):
    last_frame_ts: str | None
    last_change_kind: str | None
    last_notified_at: float | None
    poll_count: int
    push_count: int


async def _temporal_unreachable(exc: RPCError) -> NoReturn:
    await reset_client()
    log.warning("temporal unavailable: %s", exc)
    raise HTTPException(503, "temporal unavailable")


def _state_value(state: Any, key: str, default: Any = None) -> Any:
    if isinstance(state, dict):
        return state.get(key, default)
    return getattr(state, key, default)


def _b64decode(value: str) -> bytes:
    return base64.urlsafe_b64decode(value + "=" * (-len(value) % 4))


async def authenticated_user(authorization: str | None = Header(default=None)) -> str:
    """Validate a compact HMAC session token issued by the trusted auth edge.

    Token payload is base64url JSON with ``sub`` and ``exp`` followed by a
    base64url HMAC-SHA256 signature. The mobile app never receives a shared
    server API key, and every object route is bound to the authenticated sub.
    """
    if not SIGNING_KEY:
        raise HTTPException(503, "workflow API authentication is not configured")
    if not authorization or not authorization.startswith("Bearer "):
        raise HTTPException(401, "missing bearer token")
    token = authorization.removeprefix("Bearer ").strip()
    try:
        payload_part, signature_part = token.split(".", 1)
        expected = hmac.new(SIGNING_KEY, payload_part.encode(), hashlib.sha256).digest()
        if not hmac.compare_digest(expected, _b64decode(signature_part)):
            raise ValueError("signature mismatch")
        payload = json.loads(_b64decode(payload_part))
        subject = str(payload["sub"])
        expires_at = float(payload["exp"])
        if not subject or expires_at <= time.time():
            raise ValueError("expired token")
    except (binascii.Error, KeyError, TypeError, ValueError, json.JSONDecodeError):
        raise HTTPException(401, "invalid bearer token")
    return subject


def _require_owner(authenticated: str, requested: str) -> None:
    if not hmac.compare_digest(authenticated, requested):
        # Do not reveal whether another user's object exists.
        raise HTTPException(404, "not found")


# ---------- push tokens ----------


@router.post("/push-tokens", response_model=PushTokenRegisteredResponse)
async def register_push_token(
    body: RegisterPushTokenBody,
    user_id: str = Depends(authenticated_user),
) -> PushTokenRegisteredResponse:
    _require_owner(user_id, body.user_id)
    # Push tokens are credentials. Persist them directly instead of placing
    # the plaintext token in an immutable Temporal workflow history.
    await asyncio.to_thread(upsert, body.user_id, body.token, body.platform)
    return PushTokenRegisteredResponse()


@router.delete("/push-tokens/{token}", status_code=204)
async def delete_push_token(
    token: str,
    user_id: str = Depends(authenticated_user),
) -> None:
    await asyncio.to_thread(delete_for_user, user_id, token)


# ---------- watches ----------


def _watch_id(user_id: str, storm_cell_id: str) -> str:
    digest = hashlib.sha256(f"{user_id}\0{storm_cell_id}".encode()).hexdigest()
    return f"watch:{digest}"


@router.post("/watches", response_model=WorkflowStartedResponse, status_code=201)
async def start_watch(
    body: StartWatchBody,
    user_id: str = Depends(authenticated_user),
) -> WorkflowStartedResponse:
    _require_owner(user_id, body.user_id)
    client = await get_client()
    wfid = _watch_id(body.user_id, body.storm_cell_id)
    try:
        handle = await client.start_workflow(
            "WatchStormWorkflow",
            {
                "user_id": body.user_id,
                "storm_cell_id": body.storm_cell_id,
                "lat": body.lat,
                "lng": body.lng,
            },
            id=wfid,
            task_queue=TASK_QUEUE,
            id_reuse_policy=WorkflowIDReusePolicy.ALLOW_DUPLICATE_FAILED_ONLY,
        )
    except WorkflowAlreadyStartedError:
        raise HTTPException(409, "already watching this storm")
    except RPCError as e:
        await _temporal_unreachable(e)
    return WorkflowStartedResponse(workflow_id=handle.id, run_id=handle.first_execution_run_id or "")


@router.delete("/watches/{user_id}/{storm_cell_id}", status_code=204)
async def stop_watch(
    user_id: str,
    storm_cell_id: str,
    authenticated: str = Depends(authenticated_user),
) -> None:
    _require_owner(authenticated, user_id)
    client = await get_client()
    handle = client.get_workflow_handle(_watch_id(user_id, storm_cell_id))
    try:
        await handle.signal("unpinSignal")
    except RPCError as e:
        if "not found" in str(e).lower():
            return  # already gone
        await _temporal_unreachable(e)


@router.get("/watches/{user_id}/{storm_cell_id}", response_model=WatchState)
async def get_watch(
    user_id: str,
    storm_cell_id: str,
    authenticated: str = Depends(authenticated_user),
) -> WatchState:
    _require_owner(authenticated, user_id)
    client = await get_client()
    handle = client.get_workflow_handle(_watch_id(user_id, storm_cell_id))
    try:
        state = await handle.query("getCurrentState")
    except RPCError as e:
        if "not found" in str(e).lower():
            raise HTTPException(410, "watch no longer exists")
        await _temporal_unreachable(e)
    return WatchState(
        last_frame_ts=_state_value(state, "last_frame_ts"),
        last_change_kind=_state_value(state, "last_change_kind"),
        last_notified_at=_state_value(state, "last_notified_at"),
        poll_count=int(_state_value(state, "poll_count", 0)),
        push_count=int(_state_value(state, "push_count", 0)),
    )
