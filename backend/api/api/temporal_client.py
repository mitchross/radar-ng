"""Singleton Temporal client for the API.

The API process holds one connected Client and reuses it across requests.
On reconnect failures (Temporal pod restart), the next request fails with
503 — the client lib does not auto-reconnect at the gRPC channel level
in a way we can rely on, so we re-`Client.connect` on first failure.
"""

from __future__ import annotations

import os
from typing import Optional

from temporalio.client import Client


_client: Optional[Client] = None


def _config() -> tuple[str, str]:
    target = os.environ.get("TEMPORAL_ADDRESS", "localhost:7233")
    namespace = os.environ.get("TEMPORAL_NAMESPACE", "default")
    return target, namespace


async def get_client() -> Client:
    global _client
    if _client is None:
        target, namespace = _config()
        _client = await Client.connect(target, namespace=namespace)
    return _client


async def reset_client() -> None:
    global _client
    _client = None
