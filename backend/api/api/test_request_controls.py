import os
from types import SimpleNamespace

import pytest
from fastapi import HTTPException

os.environ.setdefault("DISABLE_WORKFLOW_ROUTES", "1")

from backend.api.api import server


def _request(headers: dict | None = None, route_path: str | None = None, client_host="10.0.0.9"):
    scope = {"route": SimpleNamespace(path=route_path)} if route_path else {}
    return SimpleNamespace(
        headers=headers or {},
        scope=scope,
        client=SimpleNamespace(host=client_host),
        url=SimpleNamespace(path="/api/whatever/123"),
    )


def test_client_key_uses_last_forwarded_hop():
    # Caddy appends; the first hop is attacker-chosen.
    request = _request({"x-forwarded-for": "1.2.3.4, 203.0.113.7"})
    assert server._client_key(request) == "203.0.113.7"


def test_client_key_falls_back_to_peer():
    assert server._client_key(_request()) == "10.0.0.9"


def test_route_label_uses_template_not_raw_path():
    assert server._route_label(_request(route_path="/api/forecast/{lat}/{lon}")) == "/api/forecast/{lat}/{lon}"


def test_route_label_buckets_unmatched():
    assert server._route_label(_request()) == "unmatched"


def test_build_manifest_serves_last_good_copy_then_503(monkeypatch):
    good = {"layers": {"radar": {}}}
    monkeypatch.setattr(server, "_manifest_last_good", {"body": None})
    monkeypatch.setattr(server, "read_manifest_file", lambda _dir: good)
    assert server._build_manifest() == good

    def _boom(_dir):
        raise OSError("EIO")

    monkeypatch.setattr(server, "read_manifest_file", _boom)
    assert server._build_manifest() == good

    monkeypatch.setattr(server, "_manifest_last_good", {"body": None})
    with pytest.raises(HTTPException) as excinfo:
        server._build_manifest()
    assert excinfo.value.status_code == 503
