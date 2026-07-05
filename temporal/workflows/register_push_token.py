"""Push-token workflows — register + delete, wrapped for observability.

Every registration/deletion becomes a workflow run with its own trace
span, so we can audit "who registered which token when" via Temporal UI
without adding a separate audit log. Both run on the worker because the
worker is the only pod with a writable STATE_DIR (the tile-server mounts
state read-only) — the API must never touch push_tokens.sqlite directly.
"""

from __future__ import annotations

from dataclasses import dataclass
from datetime import timedelta

from temporalio import workflow
from temporalio.common import RetryPolicy

with workflow.unsafe.imports_passed_through():
    from backend.api.api.storm_watch_activities import (
        PushTokenInput,
        delete_push_token,
        persist_push_token,
    )


@dataclass
class RegisterPushTokenInput:
    user_id: str
    token: str
    platform: str


_RETRY = RetryPolicy(
    initial_interval=timedelta(seconds=1),
    backoff_coefficient=2.0,
    maximum_interval=timedelta(seconds=10),
    maximum_attempts=3,
)


@workflow.defn(name="RegisterPushTokenWorkflow")
class RegisterPushTokenWorkflow:
    @workflow.run
    async def run(self, inp: RegisterPushTokenInput) -> None:
        await workflow.execute_activity(
            persist_push_token,
            PushTokenInput(user_id=inp.user_id, token=inp.token, platform=inp.platform),
            start_to_close_timeout=timedelta(seconds=10),
            retry_policy=_RETRY,
        )


@workflow.defn(name="DeletePushTokenWorkflow")
class DeletePushTokenWorkflow:
    @workflow.run
    async def run(self, token: str) -> int:
        return await workflow.execute_activity(
            delete_push_token,
            token,
            start_to_close_timeout=timedelta(seconds=10),
            retry_policy=_RETRY,
        )
