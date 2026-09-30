import json
from datetime import datetime, timedelta, timezone

import numpy as np

from backend.nowcast import skill
from backend.shared import nowcast_skill as shared


def _write_point_grid(path_base, data: np.ndarray, *, lat_max=45.0, lat_min=40.0, lon_min=-100.0, lon_max=-95.0):
    h, w = data.shape
    bin_path = path_base.parent / f"{path_base.name}.deadbeef.bin"
    path_base.parent.mkdir(parents=True, exist_ok=True)
    np.ascontiguousarray(data.astype(np.float32)).tofile(str(bin_path))
    meta = {
        "height": h, "width": w, "lat_min": lat_min, "lat_max": lat_max,
        "lon_min": lon_min, "lon_max": lon_max, "unit": "dBZ", "fill": -9999.0,
        "stride": 1, "data_file": bin_path.name,
    }
    (path_base.parent / f"{path_base.name}.meta.json").write_text(json.dumps(meta))


def test_downsample_uses_the_point_grid_stride_rule():
    data = np.arange(64, dtype=np.float32).reshape(8, 8)
    assert skill.downsample_like_point_grid(data, 64).shape == (8, 8)
    assert skill.downsample_like_point_grid(data, 20).shape == (4, 4)
    assert skill.downsample_like_point_grid(data, 3).shape == (1, 1)
    assert skill.downsample_like_point_grid(data, 4)[1, 1] == data[4, 4]


def test_contingency_ignores_cells_outside_coverage_and_treats_fill_as_dry():
    observed = np.array([[25.0, 25.0, 10.0, -999.0], [-9999.0, 40.0, 0.0, 30.0]], dtype=np.float32)
    forecast = np.array([[25.0, -9999.0, 30.0, 50.0], [50.0, 40.0, -9999.0, np.nan]], dtype=np.float32)
    counts = skill.contingency(forecast, observed, 20.0)
    # Known observed cells: 6 (two are outside coverage).
    assert counts == {"hits": 2, "misses": 2, "false_alarms": 1, "cells": 6}


def test_resample_nearest_maps_onto_a_coarser_grid():
    src = np.arange(16, dtype=np.float32).reshape(4, 4)
    src_meta = {"lat_max": 44.0, "lat_min": 41.0, "lon_min": -100.0, "lon_max": -97.0}
    dst_meta = {"height": 2, "width": 2, "lat_max": 44.0, "lat_min": 41.0, "lon_min": -100.0, "lon_max": -97.0}
    out = skill.resample_nearest(src, src_meta, dst_meta)
    assert out.shape == (2, 2)
    assert out[0, 0] == 0.0 and out[1, 1] == 15.0


def test_score_observation_matches_runs_by_valid_time_and_logs_persistence(tmp_path):
    grids = tmp_path / "grids"
    state = tmp_path / "state"
    anchor = datetime(2026, 9, 30, 12, 0, tzinfo=timezone.utc)
    valid = anchor + timedelta(minutes=30)
    run_dir = grids / "nowcast" / "runs" / anchor.isoformat()
    # Forecast for +30: rain in the left half; observed at +30: rain in the top half.
    forecast = np.full((4, 4), -9999.0, dtype=np.float32)
    forecast[:, :2] = 30.0
    _write_point_grid(run_dir / valid.isoformat(), forecast)
    _write_point_grid(run_dir / (valid + timedelta(minutes=5)).isoformat(), forecast)  # +35, must not match
    baseline = np.full((4, 4), 0.0, dtype=np.float32)
    baseline[0, :] = 30.0  # persistence: the anchor's own top row
    _write_point_grid(run_dir / skill.OBSERVED_KEY, baseline)
    # A run that starts after the observation must be ignored.
    later = anchor + timedelta(minutes=40)
    _write_point_grid(grids / "nowcast" / "runs" / later.isoformat() / (later + timedelta(minutes=5)).isoformat(), forecast)

    observed = np.full((4, 4), 0.0, dtype=np.float32)
    observed[:2, :] = 30.0
    lats = np.linspace(45.0, 40.0, 4)
    lons = np.linspace(-100.0, -95.0, 4)
    entries = skill.score_observation(
        observed, valid.isoformat(), lats, lons,
        grid_dir=grids, state_dir=state, point_grid_max_cells=16, now=valid,
    )

    assert len(entries) == 1
    entry = entries[0]
    assert entry["anchor"] == anchor.isoformat() and entry["lead_minutes"] == 30 and entry["cells"] == 16
    assert entry["thresholds"]["rain"] == {"hits": 4, "misses": 4, "false_alarms": 4}
    assert entry["persistence"]["rain"] == {"hits": 4, "misses": 4, "false_alarms": 0}
    assert entry["thresholds"]["heavy"] == {"hits": 0, "misses": 0, "false_alarms": 0}

    log = shared.read_log(state)
    assert len(log["entries"]) == 1
    summary = shared.summarize(log, now=valid)
    assert summary["runs_scored"] == 1 and summary["available"] is False


def test_score_observation_downsamples_a_full_science_grid(tmp_path):
    grids = tmp_path / "grids"
    anchor = datetime(2026, 9, 30, 12, 0, tzinfo=timezone.utc)
    valid = anchor + timedelta(minutes=5)
    forecast = np.full((4, 4), 30.0, dtype=np.float32)
    _write_point_grid(grids / "nowcast" / "runs" / anchor.isoformat() / valid.isoformat(), forecast)
    science = np.full((8, 8), 30.0, dtype=np.float32)  # stride 2 -> 4x4
    entries = skill.score_observation(
        science, valid.isoformat(), np.linspace(45, 40, 8), np.linspace(-100, -95, 8),
        grid_dir=grids, state_dir=tmp_path / "state", point_grid_max_cells=16, now=valid,
    )
    assert entries[0]["thresholds"]["rain"] == {"hits": 16, "misses": 0, "false_alarms": 0}
    assert entries[0]["persistence"] is None


def test_score_observation_without_runs_is_a_noop(tmp_path):
    entries = skill.score_observation(
        np.zeros((2, 2), dtype=np.float32), "2026-09-30T12:00:00+00:00",
        np.array([1.0, 0.0]), np.array([0.0, 1.0]),
        grid_dir=tmp_path / "grids", state_dir=tmp_path / "state", point_grid_max_cells=4,
    )
    assert entries == [] and not (tmp_path / "state").exists()
