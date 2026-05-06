import importlib
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

