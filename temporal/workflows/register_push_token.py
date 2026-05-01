from dataclasses import dataclass

from temporalio import workflow


@dataclass
class RegisterPushTokenInput:
    user_id: str
    token: str
    platform: str  # "ios" | "android"


@workflow.defn(name="RegisterPushTokenWorkflow")
class RegisterPushTokenWorkflow:
    """One-shot workflow wrapping push-token persist for observability —
    every registration appears as its own workflow run with a trace.

    Phase 0 stub. Lands in Phase 3.
    """

    @workflow.run
    async def run(self, input: RegisterPushTokenInput) -> None:
        workflow.logger.info(
            "RegisterPushTokenWorkflow stub for user=%s platform=%s",
            input.user_id,
            input.platform,
        )
