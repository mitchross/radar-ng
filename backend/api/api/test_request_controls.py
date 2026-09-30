import json
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


def test_manifest_is_read_and_encoded_once_per_ttl(monkeypatch):
    from backend.api.api import server

    reads = []
    body = {"layers": {"nowcast": {"frames": []}}, "schema_version": 1}
    monkeypatch.setattr(server, "_manifest_cache", {"expires_at": 0.0, "body": None, "encoded": None})
    monkeypatch.setattr(server, "_build_manifest", lambda: reads.append(1) or body)

    first = server.get_manifest()
    second = server.get_manifest()
    server.nowcast_point(42.5, -85.5)  # shares the cached manifest

    assert len(reads) == 1
    assert first.body == second.body
    assert json.loads(first.body) == body
    assert first.media_type == "application/json"
    assert first.headers["cache-control"] == "public, max-age=15"


def _fresh_manifest_cache(monkeypatch, body: dict) -> None:
    monkeypatch.setattr(
        server,
        "_manifest_cache",
        {"expires_at": 0.0, "body": None, "encoded": None, "etag": None},
    )
    monkeypatch.setattr(server, "_build_manifest", lambda: body)


def test_manifest_carries_a_strong_etag_and_answers_304_on_match(monkeypatch):
    _fresh_manifest_cache(monkeypatch, {"layers": {"radar": {"frames": [{"timestamp": "t1"}]}}})

    first = server.get_manifest()
    etag = first.headers["etag"]
    assert first.status_code == 200
    assert etag.startswith('"') and etag.endswith('"') and len(etag) > 10
    assert first.headers["cache-control"] == "public, max-age=15"

    revalidated = server.get_manifest(if_none_match=etag)
    assert revalidated.status_code == 304
    assert revalidated.body == b""
    assert revalidated.headers["etag"] == etag
    assert revalidated.headers["cache-control"] == "public, max-age=15"

    # Weak validators and lists from intermediaries still match.
    assert server.get_manifest(if_none_match=f'W/{etag}').status_code == 304
    assert server.get_manifest(if_none_match=f'"stale", {etag}').status_code == 304
    assert server.get_manifest(if_none_match='"stale"').status_code == 200


def test_manifest_etag_changes_with_the_body(monkeypatch):
    _fresh_manifest_cache(monkeypatch, {"layers": {"radar": {"frames": [{"timestamp": "t1"}]}}})
    etag = server.get_manifest().headers["etag"]

    _fresh_manifest_cache(monkeypatch, {"layers": {"radar": {"frames": [{"timestamp": "t2"}]}}})
    assert server.get_manifest(if_none_match=etag).status_code == 200
    assert server.get_manifest().headers["etag"] != etag


def test_cached_manifest_without_etag_backfills_one(monkeypatch):
    body = {"layers": {}}
    encoded = json.dumps(body).encode()
    monkeypatch.setattr(
        server,
        "_manifest_cache",
        {"expires_at": 1e12, "body": body, "encoded": encoded},
    )
    response = server.get_manifest()
    assert response.status_code == 200
    assert response.headers["etag"] == server._manifest_etag(encoded)


def _skill_log(state_dir, runs: int):
    from datetime import datetime, timezone
    from backend.shared import nowcast_skill

    now = datetime.now(timezone.utc)
    entries = [
        {
            "anchor": f"a{i}",
            "valid": "v",
            "lead_minutes": 30,
            "scored_at": now.isoformat(),
            "cells": 10,
            "thresholds": {"rain": {"hits": 8, "misses": 2, "false_alarms": 2}, "heavy": {"hits": 0, "misses": 0, "false_alarms": 0}},
            "persistence": {"rain": {"hits": 5, "misses": 5, "false_alarms": 5}, "heavy": {"hits": 0, "misses": 0, "false_alarms": 0}},
        }
        for i in range(runs)
    ]
    nowcast_skill.append_entries(state_dir, entries, now=now)


def test_nowcast_skill_endpoint_and_health_share_the_headline(monkeypatch, tmp_path):
    monkeypatch.setattr(server, "STATE_DIR", str(tmp_path))
    monkeypatch.setattr(server, "_skill_cache", {"expires_at": 0.0, "summary": None})
    _skill_log(tmp_path, 8)

    summary = json.loads(server.nowcast_skill_summary().body)
    assert summary["available"] is True
    assert summary["headline"]["pod"] == 0.8 and summary["headline"]["far"] == 0.2
    assert summary["headline"]["lead_minutes"] == 30 and summary["headline"]["runs"] == 8

    monkeypatch.setattr(server, "_build_manifest", lambda: {"layers": {}})
    health = json.loads(server.health().body)
    assert health["nowcast_skill"]["available"] is True
    assert health["nowcast_skill"]["headline"] == summary["headline"]

    metrics = server.metrics().body.decode()
    assert "radar_ng_nowcast_skill_runs 8" in metrics
    assert 'radar_ng_nowcast_pod{lead_minutes="30",threshold="rain"} 0.8' in metrics


def test_nowcast_point_carries_the_skill_headline(monkeypatch, tmp_path):
    monkeypatch.setattr(server, "STATE_DIR", str(tmp_path))
    monkeypatch.setattr(server, "_skill_cache", {"expires_at": 0.0, "summary": None})
    monkeypatch.setattr(server, "_manifest_cache", {"expires_at": 0.0, "body": None, "encoded": None, "etag": None})
    monkeypatch.setattr(server, "_nowcast_point_cache", type(server._nowcast_point_cache)())
    _skill_log(tmp_path, 8)
    frames = [{"timestamp": "2026-09-30T12:05:00+00:00", "lead_minutes": 5, "grid_key": "runs/r/2026-09-30T12:05:00+00:00"}]
    monkeypatch.setattr(server, "_build_manifest", lambda: {"layers": {"nowcast": {"frames": frames, "run_id": "r"}}})
    monkeypatch.setattr(
        server,
        "_sample_grid_point",
        lambda *a, **k: {"ok": True, "value": 25.0, "unit": "dBZ"},
    )

    first = json.loads(server.nowcast_point(42.5, -85.5).body)
    second = json.loads(server.nowcast_point(42.5, -85.5).body)  # cache hit path
    assert first["status"] == "ok" and first["skill"]["pod"] == 0.8
    assert second["skill"] == first["skill"]

    # No log yet: the field is present and null, never missing.
    monkeypatch.setattr(server, "_skill_cache", {"expires_at": 0.0, "summary": None})
    monkeypatch.setattr(server, "STATE_DIR", str(tmp_path / "empty"))
    assert json.loads(server.nowcast_point(42.5, -85.5).body)["skill"] is None
