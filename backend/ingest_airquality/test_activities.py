import importlib
import json
import sys
import types
from datetime import datetime, timedelta, timezone


def import_activities_without_pygrib(monkeypatch):
    monkeypatch.setitem(sys.modules, "pygrib", types.SimpleNamespace())
    sys.modules.pop("backend.ingest_airquality.activities", None)
    return importlib.import_module("backend.ingest_airquality.activities")


def _run_timestamps(activities, run_id: str) -> list[str]:
    run_dt = datetime.strptime(run_id, "%Y%m%d_%H").replace(tzinfo=timezone.utc)
    return [
        (run_dt + timedelta(hours=h)).isoformat()
        for h in range(activities.FORECAST_MESSAGES)
    ]


def test_grib_url_layout(monkeypatch):
    activities = import_activities_without_pygrib(monkeypatch)
    url = activities._grib_url("20260716_12", "ave_1hr_pm25_bc")
    assert url == (
        "https://noaa-nws-naqfc-pds.s3.amazonaws.com/AQMv7/CS/20260716/12/"
        "aqm.t12z.ave_1hr_pm25_bc.20260716.227.grib2"
    )


def test_publish_run_rejects_incomplete_layers(monkeypatch, tmp_path):
    activities = import_activities_without_pygrib(monkeypatch)
    palettes = {"classic": {"pm25": {}, "ozone": {}}}
    timestamps = _run_timestamps(activities, "20260716_12")

    partial = [
        activities.AqmChunkResult(
            layer="air-quality", start_msg=0, rendered_timestamps=timestamps[:50]
        )
    ]
    assert activities._publish_run_sync(
        "20260716_12", partial, palettes, state_dir=tmp_path
    ) == []
    assert not (tmp_path / "manifest.json").exists()


def test_publish_run_publishes_each_complete_layer(monkeypatch, tmp_path):
    activities = import_activities_without_pygrib(monkeypatch)
    palettes = {"classic": {"pm25": {}, "ozone": {}}, "vivid": {"pm25": {}}}
    timestamps = _run_timestamps(activities, "20260716_12")

    # pm2.5 complete across two chunks, ozone incomplete → only pm2.5 lands.
    results = [
        activities.AqmChunkResult(
            layer="air-quality", start_msg=0, rendered_timestamps=timestamps[:36]
        ),
        activities.AqmChunkResult(
            layer="air-quality", start_msg=36, rendered_timestamps=timestamps[36:]
        ),
        activities.AqmChunkResult(
            layer="ozone", start_msg=0, rendered_timestamps=timestamps[:36]
        ),
    ]
    assert activities._publish_run_sync(
        "20260716_12", results, palettes, state_dir=tmp_path
    ) == ["air-quality"]

    manifest = json.loads((tmp_path / "manifest.json").read_text())
    layer = manifest["layers"]["air-quality"]
    assert layer["complete"] is True
    assert layer["run_id"] == "20260716_12"
    assert len(layer["frames"]) == activities.FORECAST_MESSAGES
    # pm25 exists in both palettes; ozone only in classic
    assert layer["palettes"] == ["classic", "vivid"]
    first = layer["frames"][0]
    assert first["path"] == "runs/20260716_12/2026-07-16T12:00:00+00:00"
    assert first["lead_minutes"] == 0
    assert first["source"] == "aqm"
    assert layer["frames"][-1]["lead_minutes"] == (activities.FORECAST_MESSAGES - 1) * 60
    assert "ozone" not in manifest["layers"]
