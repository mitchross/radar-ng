"""Workflow-driven API routes — push tokens + storm watches.

These endpoints are the only path mobile uses to talk to Temporal: the
mobile app NEVER opens a gRPC connection to the Temporal frontend
directly (auth, port exposure, RN gRPC pain — see design spec §4).
"""

from __future__ import annotations

import os
from typing import Any, NoReturn

from fastapi import APIRouter, HTTPException
from pydantic import BaseModel
from temporalio.common import WorkflowIDReusePolicy
from temporalio.exceptions import WorkflowAlreadyStartedError
from temporalio.service import RPCError

from backend.api.api.temporal_client import get_client, reset_client
from backend.shared.push_tokens import delete_by_token


TASK_QUEUE = os.environ.get("TEMPORAL_TASK_QUEUE", "radar-ng")
router = APIRouter(prefix="/v1")


# ---------- request/response models ----------


class RegisterPushTokenBody(BaseModel):
    user_id: str
    token: str
    platform: str  # "ios" | "android"


class StartWatchBody(BaseModel):
    storm_cell_id: str
    user_id: str
    lat: float
    lng: float


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


@router.post("/push-tokens", response_model=WorkflowStartedResponse)
async def register_push_token(body: RegisterPushTokenBody) -> WorkflowStartedResponse:
    if body.platform not in ("ios", "android"):
        raise HTTPException(400, f"unknown platform: {body.platform}")
    client = await get_client()
    try:
        handle = await client.start_workflow(
            "RegisterPushTokenWorkflow",
            {"user_id": body.user_id, "token": body.token, "platform": body.platform},
            id=f"register-push:{body.token[:32]}",
            task_queue=TASK_QUEUE,
            id_reuse_policy=WorkflowIDReusePolicy.ALLOW_DUPLICATE,
        )
    except RPCError as e:
        await _temporal_unreachable(e)
    return WorkflowStartedResponse(workflow_id=handle.id, run_id=handle.first_execution_run_id or "")


@router.delete("/push-tokens/{token}", status_code=204)
async def delete_push_token(token: str) -> None:
    delete_by_token(token)


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
