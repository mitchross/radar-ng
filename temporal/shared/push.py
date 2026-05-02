"""APNS / FCM push notification activity.

Real implementation. APNS uses HTTP/2 token-based auth (p8 key + team-id +
key-id from secrets). FCM uses HTTP v1 with a service account access token.

Retry policy from the spec: 3 attempts, fail (non-retryable) on 4xx so a
bad token is unregistered immediately rather than retried forever.

Dedupe: every payload carries a `collapse_id` derived from the workflow
context. APNS uses the `apns-collapse-id` header; FCM uses the `collapse_key`
field. Temporal's at-least-once retry → APNS/FCM sees the same id → user's
phone buzzes once.

If APNS returns BadDeviceToken or FCM returns NOT_REGISTERED, we delete the
token from the local store before raising non-retryable so future watches
don't repeatedly try the dead token.
"""

from __future__ import annotations

import json
import os
import time
from dataclasses import dataclass

import httpx
from temporalio import activity
from temporalio.exceptions import ApplicationError

from backend.shared.logger import get_logger
from backend.shared.push_tokens import delete_by_token


APNS_HOST = os.environ.get("APNS_HOST", "https://api.push.apple.com")
APNS_TOPIC = os.environ.get("APNS_TOPIC", "")  # bundle id, e.g. me.vanillax.radar-ng
APNS_KEY_ID = os.environ.get("APNS_KEY_ID", "")
APNS_TEAM_ID = os.environ.get("APNS_TEAM_ID", "")
APNS_KEY_PATH = os.environ.get("APNS_KEY_PATH", "/secrets/apns.p8")

FCM_PROJECT_ID = os.environ.get("FCM_PROJECT_ID", "")
FCM_CREDENTIALS_PATH = os.environ.get("FCM_CREDENTIALS_PATH", "/secrets/fcm.json")

# Kill switch: when set, the activity logs+returns successfully without
# touching APNS/FCM. Lets us run the worker without push secrets and
# without the watch-storm code path failing every push attempt. Re-enable
# by unsetting in the worker manifest (and applying push secrets).
PUSH_DISABLED = os.environ.get("PUSH_DISABLED", "1") == "1"

log = get_logger("push-activity")


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


# ---------- APNS ----------


def _apns_jwt() -> str:
    """Build an ES256 JWT for APNS token-based auth.

    JWTs are valid for up to 60 min; we mint a fresh one per send to keep
    the activity stateless. APNS rate-limits too-frequent JWT issuance,
    but a 60-min cache window across retries is well within limits.
    """
    import jwt  # PyJWT, in temporal/requirements.txt

    with open(APNS_KEY_PATH, "r") as f:
        key = f.read()
    return jwt.encode(
        {"iss": APNS_TEAM_ID, "iat": int(time.time())},
        key,
        algorithm="ES256",
        headers={"kid": APNS_KEY_ID},
    )


async def _send_apns(token: str, payload: PushPayload) -> None:
    if not (APNS_TOPIC and APNS_KEY_ID and APNS_TEAM_ID):
        raise ApplicationError(
            "APNS not configured (APNS_TOPIC/APNS_KEY_ID/APNS_TEAM_ID missing)",
            non_retryable=True,
        )

    body = {
        "aps": {
            "alert": {"title": payload.title, "body": payload.body},
            "sound": "default",
            "mutable-content": 1,
        },
        **payload.extra,
    }
    headers = {
        "authorization": f"bearer {_apns_jwt()}",
        "apns-topic": APNS_TOPIC,
        "apns-push-type": "alert",
        "apns-priority": "10",
        "apns-collapse-id": payload.collapse_id[:64],  # APNS hard caps at 64 bytes
    }

    async with httpx.AsyncClient(http2=True, timeout=10.0) as client:
        r = await client.post(f"{APNS_HOST}/3/device/{token}", headers=headers, json=body)

    if r.status_code == 200:
        return
    err_body = r.text or ""
    if r.status_code == 410 or "BadDeviceToken" in err_body or "Unregistered" in err_body:
        # Token is dead — delete locally and stop retrying.
        delete_by_token(token)
        raise ApplicationError(f"apns dead token: {err_body}", non_retryable=True)
    if 400 <= r.status_code < 500:
        raise ApplicationError(f"apns 4xx: {r.status_code} {err_body}", non_retryable=True)
    raise RuntimeError(f"apns 5xx: {r.status_code} {err_body}")


# ---------- FCM ----------


_FCM_TOKEN: tuple[str, float] | None = None


async def _fcm_access_token() -> str:
    """Mint (or reuse cached) Google OAuth2 access token for FCM v1.

    The token is valid for ~60 min; we cache in-process for 50 min.
    """
    global _FCM_TOKEN
    if _FCM_TOKEN and _FCM_TOKEN[1] > time.time() + 60:
        return _FCM_TOKEN[0]

    import jwt
    with open(FCM_CREDENTIALS_PATH, "r") as f:
        creds = json.load(f)
    now = int(time.time())
    assertion = jwt.encode(
        {
            "iss": creds["client_email"],
            "scope": "https://www.googleapis.com/auth/firebase.messaging",
            "aud": "https://oauth2.googleapis.com/token",
            "iat": now,
            "exp": now + 3600,
        },
        creds["private_key"],
        algorithm="RS256",
    )
    async with httpx.AsyncClient(timeout=10.0) as client:
        r = await client.post(
            "https://oauth2.googleapis.com/token",
            data={
                "grant_type": "urn:ietf:params:oauth:grant-type:jwt-bearer",
                "assertion": assertion,
            },
        )
    r.raise_for_status()
    tok = r.json()["access_token"]
    _FCM_TOKEN = (tok, time.time() + 3000)
    return tok


async def _send_fcm(token: str, payload: PushPayload) -> None:
    if not FCM_PROJECT_ID:
        raise ApplicationError("FCM not configured (FCM_PROJECT_ID missing)", non_retryable=True)

    access = await _fcm_access_token()
    body = {
        "message": {
            "token": token,
            "notification": {"title": payload.title, "body": payload.body},
            "data": payload.extra,
            "android": {
                "collapse_key": payload.collapse_id,
                "priority": "high",
                "notification": {"sound": "default"},
            },
        }
    }
    async with httpx.AsyncClient(timeout=10.0) as client:
        r = await client.post(
            f"https://fcm.googleapis.com/v1/projects/{FCM_PROJECT_ID}/messages:send",
            headers={"authorization": f"Bearer {access}", "content-type": "application/json"},
            json=body,
        )

    if r.status_code == 200:
        return
    err_body = r.text or ""
    if "NOT_REGISTERED" in err_body or "INVALID_ARGUMENT" in err_body or r.status_code == 404:
        delete_by_token(token)
        raise ApplicationError(f"fcm dead token: {err_body}", non_retryable=True)
    if 400 <= r.status_code < 500:
        raise ApplicationError(f"fcm 4xx: {r.status_code} {err_body}", non_retryable=True)
    raise RuntimeError(f"fcm 5xx: {r.status_code} {err_body}")


# ---------- activity ----------


@activity.defn(name="send_push_notification")
async def send_push_notification(token: PushToken, payload: PushPayload) -> None:
    if PUSH_DISABLED:
        log.info(
            "push_skipped (PUSH_DISABLED=1)",
            extra={"platform": token.platform, "user_id": token.user_id, "title": payload.title},
        )
        return
    if token.platform == "ios":
        await _send_apns(token.token, payload)
    elif token.platform == "android":
        await _send_fcm(token.token, payload)
    else:
        raise ApplicationError(f"unknown platform: {token.platform}", non_retryable=True)
    log.info("push_sent", extra={"platform": token.platform, "user_id": token.user_id})
