"""Workflow-driven API routes — push tokens + storm watches.

These endpoints are the only path mobile uses to talk to Temporal: the
mobile app NEVER opens a gRPC connection to the Temporal frontend
directly (auth, port exposure, RN gRPC pain — see design spec §4).
"""

from __future__ import annotations

import hashlib
import os
from typing import Any, Literal, NoReturn

from fastapi import APIRouter, HTTPException
from pydantic import BaseModel, Field
from temporalio.common import WorkflowIDReusePolicy
from temporalio.exceptions import WorkflowAlreadyStartedError
from temporalio.service import RPCError

from backend.api.api.temporal_client import get_client, reset_client


TASK_QUEUE = os.environ.get("TEMPORAL_TASK_QUEUE", "radar-ng")
router = APIRouter(prefix="/v1")


# ---------- request/response models ----------


# These endpoints are unauthenticated and internet-facing — bound every
# field so a hostile client can't stuff megabyte payloads into workflow
# IDs, the push-token DB, or Temporal history. APNS tokens are 64 hex
# chars; FCM registration tokens run ~140-200 chars; 512 is generous.


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


class WatchState(BaseModel):
    last_frame_ts: str | None
    last_change_kind: str | None
    last_notified_at: float | None
    poll_count: int
    push_count: int


async def _temporal_unreachable(exc: RPCError) -> NoReturn:
    await reset_client()
    raise HTTPException(503, f"temporal unreachable: {exc}")


def _state_value(state: Any, key: str, default: Any = None) -> Any:
    if isinstance(state, dict):
        return state.get(key, default)
    return getattr(state, key, default)


# ---------- push tokens ----------


def _token_wfid(prefix: str, token: str) -> str:
    # Hash rather than truncate: FCM tokens share long structural prefixes
    # (same app instance, rotated tokens), so `token[:32]` collided two
    # distinct tokens onto one workflow ID — and a swallowed
    # WorkflowAlreadyStartedError then silently dropped the second operation.
    return f"{prefix}:{hashlib.sha256(token.encode()).hexdigest()[:32]}"


@router.post("/push-tokens", response_model=WorkflowStartedResponse)
async def register_push_token(body: RegisterPushTokenBody) -> WorkflowStartedResponse:
    client = await get_client()
    wfid = _token_wfid("register-push", body.token)
    try:
        handle = await client.start_workflow(
            "RegisterPushTokenWorkflow",
            {"user_id": body.user_id, "token": body.token, "platform": body.platform},
            id=wfid,
            task_queue=TASK_QUEUE,
            id_reuse_policy=WorkflowIDReusePolicy.ALLOW_DUPLICATE,
        )
    except WorkflowAlreadyStartedError:
        # Same token registering concurrently (app retry) — idempotent.
        return WorkflowStartedResponse(workflow_id=wfid, run_id="")
    except RPCError as e:
        await _temporal_unreachable(e)
    return WorkflowStartedResponse(workflow_id=handle.id, run_id=handle.first_execution_run_id or "")


@router.delete("/push-tokens/{token}", status_code=204)
async def delete_push_token(token: str) -> None:
    # Deletion goes through the worker like registration does: the API pod
    # mounts STATE_DIR read-only, so writing push_tokens.sqlite here always
    # failed with a read-only-database 500 — and the old direct call was
    # sync sqlite on the event loop, which a hung NFS mount could park,
    # freezing /api/livez (the k8s probe) with it. The 204 therefore means
    # "deletion accepted", not "row gone" — a register racing a delete for
    # the same token within the same second is unordered (same as the old
    # code's register-retry race).
    if len(token) > 512:
        raise HTTPException(422, "token too long")
    client = await get_client()
    try:
        await client.start_workflow(
            "DeletePushTokenWorkflow",
            token,
            id=_token_wfid("delete-push", token),
            task_queue=TASK_QUEUE,
            id_reuse_policy=WorkflowIDReusePolicy.ALLOW_DUPLICATE,
        )
    except WorkflowAlreadyStartedError:
        pass  # this exact token already being deleted (hashed id) — idempotent
    except RPCError as e:
        await _temporal_unreachable(e)


# ---------- watches ----------


def _watch_id(user_id: str, storm_cell_id: str) -> str:
    return f"watch:{user_id}:{storm_cell_id}"


@router.post("/watches", response_model=WorkflowStartedResponse, status_code=201)
async def start_watch(body: StartWatchBody) -> WorkflowStartedResponse:
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
async def stop_watch(user_id: str, storm_cell_id: str) -> None:
    client = await get_client()
    handle = client.get_workflow_handle(_watch_id(user_id, storm_cell_id))
    try:
        await handle.signal("unpinSignal")
    except RPCError as e:
        if "not found" in str(e).lower():
            return  # already gone
        await _temporal_unreachable(e)


@router.get("/watches/{user_id}/{storm_cell_id}", response_model=WatchState)
async def get_watch(user_id: str, storm_cell_id: str) -> WatchState:
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
