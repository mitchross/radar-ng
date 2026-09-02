"""Guards on the HRRR workflow's tuning constants (they interact with the activity code)."""

import importlib
import sys
import types
from datetime import timedelta


def _import_workflow(monkeypatch):
    monkeypatch.setitem(sys.modules, "pygrib", types.SimpleNamespace())
    sys.modules.pop("backend.ingest_hrrr.activities", None)
    sys.modules.pop("temporal.workflows.ingest_hrrr", None)
    return importlib.import_module("temporal.workflows.ingest_hrrr")


def test_forecast_hour_budget(monkeypatch):
    wf = _import_workflow(monkeypatch)

    assert wf.FORECAST_CONCURRENCY == 4
    assert wf.HOUR_START_TO_CLOSE == timedelta(minutes=8)
    assert wf.HOUR_SCHEDULE_TO_CLOSE == timedelta(minutes=12)
    assert wf.HOUR_HEARTBEAT_TIMEOUT == timedelta(seconds=90)
    assert wf._FORECAST_RETRY.maximum_attempts == 2
    # Both attempts must fit in the schedule_to_close budget or the retry never runs.
    assert wf.HOUR_START_TO_CLOSE < wf.HOUR_SCHEDULE_TO_CLOSE


def test_result_reports_completeness(monkeypatch):
    wf = _import_workflow(monkeypatch)

    result = wf.IngestHrrrResult(run_id="20260715_12", skipped_already_processed=False, forecast_hours_processed=3)
    assert result.complete is False
