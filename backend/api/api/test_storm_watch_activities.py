import asyncio
import json
from pathlib import Path

from backend.api.api.storm_watch_activities import (
    MarkAlertsSeenInput,
    _alert_from_feature,
    _point_in_geojson,
    mark_alerts_seen,
)


def test_point_in_geojson_polygon_with_hole() -> None:
    geometry = {
        "type": "Polygon",
        "coordinates": [
            [[-90, 30], [-80, 30], [-80, 40], [-90, 40], [-90, 30]],
            [[-87, 33], [-83, 33], [-83, 37], [-87, 37], [-87, 33]],
        ],
    }

    assert _point_in_geojson(-88, 32, geometry)
    assert not _point_in_geojson(-85, 35, geometry)
    assert not _point_in_geojson(-75, 35, geometry)


def test_point_in_geojson_multipolygon() -> None:
    geometry = {
        "type": "MultiPolygon",
        "coordinates": [
            [[[-100, 30], [-95, 30], [-95, 35], [-100, 35], [-100, 30]]],
            [[[-85, 40], [-80, 40], [-80, 45], [-85, 45], [-85, 40]]],
        ],
    }

    assert _point_in_geojson(-82, 42, geometry)
    assert not _point_in_geojson(-90, 42, geometry)


def test_alert_from_feature_keeps_id_and_geometry() -> None:
    feature = {
        "id": "fallback",
        "properties": {"id": "nws-alert-1"},
        "geometry": {
            "type": "Polygon",
            "coordinates": [[[-90, 30], [-80, 30], [-80, 40], [-90, 40], [-90, 30]]],
        },
    }

    alert = _alert_from_feature(feature)

    assert alert is not None
    assert alert.alert_id == "nws-alert-1"
    assert alert.geometry["type"] == "Polygon"


def _patch_state(monkeypatch, tmp_path: Path) -> tuple[Path, Path]:
    from backend.api.api import storm_watch_activities as mod

    state_path = tmp_path / "alerts_seen.json"
    snapshot_path = tmp_path / "alerts_active_snapshot.json"
    monkeypatch.setattr(mod, "STATE_DIR", tmp_path)
    monkeypatch.setattr(mod, "_ALERT_STATE_PATH", state_path)
    monkeypatch.setattr(mod, "_ACTIVE_SNAPSHOT_PATH", snapshot_path)
    return state_path, snapshot_path


def test_mark_alerts_seen_skips_unhandled_so_next_poll_retries(monkeypatch, tmp_path) -> None:
    """At-least-once: an alert whose signaling failed is NOT committed, so the
    next poll's diff surfaces it as new again."""
    state_path, snapshot_path = _patch_state(monkeypatch, tmp_path)
    state_path.write_text(json.dumps(["old-1"]))
    snapshot_path.write_text(json.dumps(["old-1", "new-ok", "new-failed"]))

    asyncio.run(mark_alerts_seen(MarkAlertsSeenInput(handled_ids=["new-ok"])))

    seen = set(json.loads(state_path.read_text()))
    assert "new-ok" in seen
    assert "old-1" in seen  # still active, keeps its seen bit
    assert "new-failed" not in seen  # left for the next poll to retry


def test_mark_alerts_seen_retains_expired_ids_as_cap_filler(monkeypatch, tmp_path) -> None:
    """Expired alerts stay in the file (within the cap) so a flapping NWS feed
    doesn't re-notify an alert that briefly drops out of /alerts/active."""
    state_path, snapshot_path = _patch_state(monkeypatch, tmp_path)
    state_path.write_text(json.dumps(["expired-1", "active-1"]))
    snapshot_path.write_text(json.dumps(["active-1"]))

    asyncio.run(mark_alerts_seen(MarkAlertsSeenInput(handled_ids=[])))

    seen = set(json.loads(state_path.read_text()))
    assert seen == {"active-1", "expired-1"}


def test_mark_alerts_seen_without_snapshot_never_forgets(monkeypatch, tmp_path) -> None:
    """Missing/corrupt snapshot (fetch write failed, fresh volume): marking
    must fall back to no-prune — forgetting a seen alert would re-push it."""
    state_path, snapshot_path = _patch_state(monkeypatch, tmp_path)
    state_path.write_text(json.dumps(["old-1"]))
    # no snapshot file written

    asyncio.run(mark_alerts_seen(MarkAlertsSeenInput(handled_ids=["new-ok"])))

    seen = set(json.loads(state_path.read_text()))
    assert seen == {"old-1", "new-ok"}
