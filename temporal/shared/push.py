"""APNS / FCM push notification activity.

Phase 0 stub. Real implementation lands in Phase 3 alongside
`WatchStormWorkflow`. Will use:

  - APNS HTTP/2 with token-based auth (apns-key + team-id + key-id from secrets)
  - FCM v1 HTTP API (server key from secrets)
  - `apns-collapse-id` header derived from `(workflowId, frameTs, changeKind)`
    so Temporal's at-least-once retry does not page the user multiple times

Retry policy (per spec): 3 attempts, fail (non-retryable) on 4xx so a bad
token is unregistered immediately rather than retried forever.
"""

from dataclasses import dataclass

from temporalio import activity


@dataclass
class PushToken:
    user_id: str
    token: str
    platform: str  # "ios" | "android"


@dataclass
class PushPayload:
    title: str
    body: str
    collapse_id: str
    extra: dict[str, str]


@activity.defn(name="send_push_notification")
async def send_push_notification(token: PushToken, payload: PushPayload) -> None:
    activity.logger.info(
        "send_push_notification stub: %s/%s -> %s",
        token.platform,
        token.user_id,
        payload.title,
    )
    # Phase 3: real APNS/FCM dispatch with collapse-id
