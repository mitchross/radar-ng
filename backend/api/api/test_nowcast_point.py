import json
import os
from datetime import datetime, timedelta

import numpy as np

os.environ.setdefault("DISABLE_WORKFLOW_ROUTES", "1")

from backend.api.api import server
from backend.shared import grid_dump


def _manifest(
    timestamps: list[str],
    *,
    run_id: str = "2026-07-15T16:00:00+00:00",
    grid_keys: list[str] | None = None,
) -> dict:
    return {
        "layers": {
            "nowcast": {
                "run_id": run_id,
                "method": "pysteps-sprog",
                "horizon_minutes": 60,
                "step_minutes": 5,
                "frames": [
                    dict(
                        {
                            "timestamp": timestamp,
                            "lead_minutes": (index + 1) * 5,
                            "spatial_resolution_km": 2.22,
                        },
                        **(
                            {"grid_key": grid_keys[index]}
                            if grid_keys is not None
                            else {}
                        ),
                    )
                    for index, timestamp in enumerate(timestamps)
                ],
            }
        }
    }


def test_nowcast_point_samples_complete_mrms_series(monkeypatch, tmp_path):
    monkeypatch.setattr(server, "GRID_DIR", str(tmp_path))
    monkeypatch.setattr(grid_dump, "GRID_DIR", str(tmp_path))
    timestamps = [
        "2026-07-15T16:05:00+00:00",
        "2026-07-15T16:10:00+00:00",
    ]
    lats = np.array([43.0, 42.0], dtype=np.float64)
    lons = np.array([-86.0, -85.0], dtype=np.float64)
    for timestamp, dbz in zip(timestamps, (30.0, 40.0)):
        grid_dump.write_grid(
            "nowcast",
            timestamp,
            np.full((2, 2), dbz, dtype=np.float32),
            lats,
            lons,
            "dBZ",
            fill=-9999.0,
        )

    monkeypatch.setattr(server, "_build_manifest", lambda: _manifest(timestamps))
    server._nowcast_point_cache.clear()
    response = server.nowcast_point(42.5, -85.5)
    body = json.loads(response.body)

    assert body["status"] == "ok"
    assert body["source"] == "mrms-nowcast"
    assert body["method"] == "pysteps-sprog"
    assert len(body["points"]) == 2
    assert body["points"][0]["dbz"] == 30.0
    assert body["points"][0]["precipitation_mm_h"] > 0
    assert body["spatial_resolution_km"] == 2.22


def test_nowcast_point_reports_warming_grids(monkeypatch):
    timestamp = "2026-07-15T16:05:00+00:00"
    monkeypatch.setattr(server, "_build_manifest", lambda: _manifest([timestamp]))
    monkeypatch.setattr(server, "GRID_DIR", "/missing")
    server._nowcast_point_cache.clear()

    response = server.nowcast_point(42.5, -85.5)
    body = json.loads(response.body)

    assert body == {
        "status": "unavailable",
        "reason": "grids_warming_up",
        "issued_at": "2026-07-15T16:00:00+00:00",
        "points": [],
    }


def test_three_overlapping_runs_keep_old_manifest_readable_until_swap(
    monkeypatch, tmp_path
):
    monkeypatch.setattr(server, "GRID_DIR", str(tmp_path))
    monkeypatch.setattr(grid_dump, "GRID_DIR", str(tmp_path))
    lats = np.array([43.0, 42.0], dtype=np.float64)
    lons = np.array([-86.0, -85.0], dtype=np.float64)
    anchors = [
        "2026-07-15T16:00:00+00:00",
        "2026-07-15T16:05:00+00:00",
        "2026-07-15T16:10:00+00:00",
    ]
    manifests: list[dict] = []
    for run_index, anchor in enumerate(anchors):
        anchor_dt = datetime.fromisoformat(anchor)
        timestamps = [
            (anchor_dt + timedelta(minutes=lead * 5)).isoformat()
            for lead in range(1, 13)
        ]
        grid_keys = [f"runs/{anchor}/{timestamp}" for timestamp in timestamps]
        for timestamp, grid_key in zip(timestamps, grid_keys, strict=True):
            grid_dump.write_grid(
                "nowcast",
                timestamp,
                np.full((2, 2), 10.0 * (run_index + 1), dtype=np.float32),
                lats,
                lons,
                "dBZ",
                fill=-9999.0,
                grid_key=grid_key,
            )
        grid_dump.finalize_grid_generation("nowcast", anchor)
        manifests.append(_manifest(timestamps, run_id=anchor, grid_keys=grid_keys))

    # All three runs can overlap on disk. Until the atomic manifest swap, the
    # original frame keys still resolve its untouched generation.
    visible = {"manifest": manifests[0]}
    monkeypatch.setattr(server, "_build_manifest", lambda: visible["manifest"])
    server._nowcast_point_cache.clear()
    old_body = json.loads(server.nowcast_point(42.5, -85.5).body)
    assert old_body["status"] == "ok"
    assert len(old_body["points"]) == 12
    assert {point["dbz"] for point in old_body["points"]} == {10.0}

    visible["manifest"] = manifests[2]
    grid_dump.prune_grid_generations("nowcast", keep=2, active_generation=anchors[2])
    server._nowcast_point_cache.clear()
    new_body = json.loads(server.nowcast_point(42.5, -85.5).body)
    assert new_body["status"] == "ok"
    assert len(new_body["points"]) == 12
    assert {point["dbz"] for point in new_body["points"]} == {30.0}
    assert not (tmp_path / "nowcast" / "runs" / anchors[0]).exists()
    assert (tmp_path / "nowcast" / "runs" / anchors[2]).is_dir()
