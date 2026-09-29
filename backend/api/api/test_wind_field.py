import json
import math
import os
import struct

import pytest
from fastapi.testclient import TestClient

os.environ.setdefault("DISABLE_WORKFLOW_ROUTES", "1")

from backend.api.api import server


def _write(dir_, stem, values, w, h, bounds):
    dir_.mkdir(parents=True, exist_ok=True)
    (dir_ / f"{stem}.bin").write_bytes(struct.pack(f"<{len(values)}f", *values))
    (dir_ / f"{stem}.meta.json").write_text(json.dumps({"width": w, "height": h, **bounds}))


@pytest.fixture
def grids(tmp_path, monkeypatch):
    monkeypatch.setattr(server, "GRID_DIR", str(tmp_path))
    server._wind_cache.clear()
    return tmp_path


def test_wind_field_sends_nan_as_fill_and_reports_sampled_bounds(grids):
    w, h = 500, 250  # strides to 250x125 → the far edge row/col are dropped
    bounds = {"lat_min": 20.0, "lat_max": 50.0, "lon_min": -130.0, "lon_max": -60.0}
    u = [float("nan") if i % w == 0 else 10.0 for i in range(w * h)]
    v = [float("nan") if i % w == 0 else -5.0 for i in range(w * h)]
    _write(grids / "wind_u", "2026-09-29T05:00:00+00:00", u, w, h, bounds)
    _write(grids / "wind_v", "2026-09-29T05:00:00+00:00", v, w, h, bounds)
    with TestClient(server.app) as client:
        body = client.get("/api/wind-field/2026-09-29T05:00:00+00:00").json()
    assert body["ok"] and body["u"][0] == -128 and body["v"][0] == -128
    assert all(math.isfinite(body[k]) for k in ("u_min", "u_max", "v_min", "v_max"))
    assert body["lon_min"] == -130.0 and body["lat_max"] == 50.0
    assert body["lon_max"] == pytest.approx(-130.0 + (body["width"] - 1) * 2 * 70.0 / (w - 1))
    assert body["lat_min"] == pytest.approx(50.0 - (body["height"] - 1) * 2 * 30.0 / (h - 1))


def test_all_nan_wind_grid_is_reported_empty(grids):
    bounds = {"lat_min": 20.0, "lat_max": 50.0, "lon_min": -130.0, "lon_max": -60.0}
    nan = [float("nan")] * 4
    _write(grids / "wind_u", "t", nan, 2, 2, bounds)
    _write(grids / "wind_v", "t", nan, 2, 2, bounds)
    with TestClient(server.app) as client:
        assert client.get("/api/wind-field/t").json() == {"ok": False, "reason": "grid_empty"}
