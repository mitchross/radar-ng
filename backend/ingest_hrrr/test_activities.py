import importlib
import json
import sys
import types
from pathlib import Path


def import_activities_without_pygrib(monkeypatch):
    monkeypatch.setitem(sys.modules, "pygrib", types.SimpleNamespace())
    sys.modules.pop("backend.ingest_hrrr.activities", None)
    return importlib.import_module("backend.ingest_hrrr.activities")


def test_activity_tmp_dir_is_unique_per_attempt(monkeypatch, tmp_path):
    activities = import_activities_without_pygrib(monkeypatch)

    monkeypatch.setattr(activities, "TMP_ROOT", tmp_path)

    first = activities._activity_tmp_dir(
        "hrrr",
        workflow_id="sched-ingest-hrrr-2026-05-06T02:00:00Z",
        run_id="run-one",
        activity_id="3",
        attempt=1,
        parts=("20260506_02", "f01"),
    )
    retry = activities._activity_tmp_dir(
        "hrrr",
        workflow_id="sched-ingest-hrrr-2026-05-06T02:00:00Z",
        run_id="run-one",
        activity_id="3",
        attempt=2,
        parts=("20260506_02", "f01"),
    )
    other_hour = activities._activity_tmp_dir(
        "hrrr",
        workflow_id="sched-ingest-hrrr-2026-05-06T02:00:00Z",
        run_id="run-one",
        activity_id="4",
        attempt=1,
        parts=("20260506_02", "f02"),
    )

    assert first != retry
    assert first != other_hour
    assert first.parent == tmp_path
    assert retry.parent == tmp_path
    assert other_hour.parent == tmp_path


def test_activity_tmp_dir_sanitizes_temporal_ids(monkeypatch, tmp_path):
    activities = import_activities_without_pygrib(monkeypatch)

    monkeypatch.setattr(activities, "TMP_ROOT", tmp_path)

    tmp_dir = activities._activity_tmp_dir(
        "hrrr",
        workflow_id="sched/ingest:hrrr",
        run_id="run+one",
        activity_id="3",
        attempt=1,
        parts=("20260506_02", "f01"),
    )

    assert tmp_dir == tmp_path / "hrrr-sched_ingest_hrrr-run_one-3-attempt1-20260506_02-f01"
    assert "/" not in tmp_dir.name


def test_publish_run_is_atomic_and_rejects_incomplete_hours(monkeypatch, tmp_path):
    activities = import_activities_without_pygrib(monkeypatch)
    result = activities.ForecastHourResult
    palettes = {"classic": {"reflectivity": {}}}
    complete = [
        result(1, ["radar-hrrr"], valid_timestamp="2026-07-15T13:00:00+00:00"),
        result(2, ["radar-hrrr"], valid_timestamp="2026-07-15T14:00:00+00:00"),
    ]

    assert activities._publish_hrrr_run_sync(
        "20260715_12", complete[1:], palettes, state_dir=tmp_path
    ) == []
    assert not (tmp_path / "manifest.json").exists()

    assert activities._publish_hrrr_run_sync(
        "20260715_12", complete, palettes, state_dir=tmp_path
    ) == ["radar-hrrr"]
    manifest = json.loads((tmp_path / "manifest.json").read_text())
    layer = manifest["layers"]["radar-hrrr"]
    assert layer["complete"] is True
    assert layer["run_id"] == "20260715_12"
    assert [frame["path"] for frame in layer["frames"]] == [
        "runs/20260715_12/2026-07-15T13:00:00+00:00",
        "runs/20260715_12/2026-07-15T14:00:00+00:00",
    ]
