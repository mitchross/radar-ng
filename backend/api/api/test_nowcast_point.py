import json
import os

import numpy as np

os.environ.setdefault("DISABLE_WORKFLOW_ROUTES", "1")

from backend.api.api import server
from backend.shared import grid_dump


def _manifest(timestamps: list[str]) -> dict:
    return {
        "layers": {
            "nowcast": {
                "run_id": "2026-07-15T16:00:00+00:00",
                "method": "pysteps-sprog",
                "horizon_minutes": 60,
                "step_minutes": 5,
                "frames": [
                    {
                        "timestamp": timestamp,
                        "lead_minutes": (index + 1) * 5,
                        "spatial_resolution_km": 2.22,
                    }
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
