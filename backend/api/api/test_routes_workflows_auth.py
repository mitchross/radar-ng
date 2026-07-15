import asyncio
import base64
import hashlib
import hmac
import json
import time

import pytest
from fastapi import HTTPException

from backend.api.api import routes_workflows


def _encoded(value: bytes) -> str:
    return base64.urlsafe_b64encode(value).decode().rstrip("=")


def _token(key: bytes, payload: dict) -> str:
    payload_part = _encoded(json.dumps(payload, separators=(",", ":")).encode())
    signature = hmac.new(key, payload_part.encode(), hashlib.sha256).digest()
    return f"{payload_part}.{_encoded(signature)}"


def test_authenticated_user_accepts_scoped_unexpired_token(monkeypatch):
    key = b"test-only-signing-key"
    monkeypatch.setattr(routes_workflows, "SIGNING_KEY", key)
    token = _token(key, {"sub": "user-42", "exp": time.time() + 60})

    assert asyncio.run(
        routes_workflows.authenticated_user(authorization=f"Bearer {token}")
    ) == "user-42"


@pytest.mark.parametrize(
    "authorization",
    ["Bearer not-base64.not-base64", "Basic credentials", None],
)
def test_authenticated_user_rejects_malformed_credentials(monkeypatch, authorization):
    monkeypatch.setattr(routes_workflows, "SIGNING_KEY", b"test-only-signing-key")

    with pytest.raises(HTTPException) as raised:
        asyncio.run(routes_workflows.authenticated_user(authorization=authorization))

    assert raised.value.status_code == 401
