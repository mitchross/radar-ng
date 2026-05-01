"""RegisterPushTokenWorkflow — wraps push-token persist for observability.

Every registration becomes a workflow run with its own trace span, so we
can audit "who registered which token when" via Temporal UI without
adding a separate audit log.
"""

from __future__ import annotations

from dataclasses import dataclass
from datetime import timedelta

from temporalio import workflow
from temporalio.common import RetryPolicy

with workflow.unsafe.imports_passed_through():
    from backend.api.api.storm_watch_activities import (
        PushTokenInput,
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
