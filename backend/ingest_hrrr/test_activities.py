import importlib
import json
import sys
import types
from pathlib import Path

import numpy as np
import pytest


def import_activities_without_pygrib(monkeypatch):
    monkeypatch.setitem(sys.modules, "pygrib", types.SimpleNamespace())
    sys.modules.pop("backend.ingest_hrrr.activities", None)
    return importlib.import_module("backend.ingest_hrrr.activities")


def test_refc_selector_matches_current_noaa_metadata(monkeypatch, tmp_path):
    activities = import_activities_without_pygrib(monkeypatch)

    class Message:
        name = "Maximum/Composite radar reflectivity"
        shortName = "refc"
        typeOfLevel = "atmosphere"
        values = np.array([[10.0, 20.0], [30.0, 40.0]])

        @staticmethod
        def latlons():
            return (
                np.array([[40.0, 40.0], [39.0, 39.0]]),
                np.array([[-90.0, -89.0], [-90.0, -89.0]]),
            )

    class GribFile(list):
        def close(self):
            return None

    monkeypatch.setattr(
        activities.pygrib,
        "open",
        lambda _path: GribFile([Message()]),
        raising=False,
    )
    monkeypatch.setattr(activities, "_extract_native_projection", lambda *_args: None)

    grid = activities._extract_variable(
        tmp_path / "hrrr.grib2", activities.VAR_SELECTORS["refc"]
    )

    assert grid is not None
    assert grid.data.tolist() == [[10.0, 20.0], [30.0, 40.0]]


def test_missing_required_refc_fails_forecast_hour(monkeypatch, tmp_path):
    activities = import_activities_without_pygrib(monkeypatch)
    monkeypatch.setattr(activities, "_extract_variable", lambda *_args: None)

    with pytest.raises(RuntimeError, match="required HRRR variable refc"):
        activities._process_forecast_hour_sync(
            tmp_path / "hrrr.grib2",
            "20260715_12",
            1,
            {},
            tmp_path / "tiles",
        )


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

    assert (
        tmp_dir
        == tmp_path / "hrrr-sched_ingest_hrrr-run_one-3-attempt1-20260506_02-f01"
    )
    assert "/" not in tmp_dir.name


def test_publish_run_is_atomic_and_rejects_incomplete_hours(monkeypatch, tmp_path):
    activities = import_activities_without_pygrib(monkeypatch)
    result = activities.ForecastHourResult
    palettes = {"classic": {"reflectivity": {}}}
    complete = [
        result(1, ["radar-hrrr"], valid_timestamp="2026-07-15T13:00:00+00:00"),
        result(2, ["radar-hrrr"], valid_timestamp="2026-07-15T14:00:00+00:00"),
    ]

    assert (
        activities._publish_hrrr_run_sync(
            "20260715_12", complete[1:], palettes, state_dir=tmp_path, horizon=2
        )
        == []
    )
    assert not (tmp_path / "manifest.json").exists()

    assert activities._publish_hrrr_run_sync(
        "20260715_12", complete, palettes, state_dir=tmp_path, horizon=2
    ) == ["radar-hrrr"]
    manifest = json.loads((tmp_path / "manifest.json").read_text())
    layer = manifest["layers"]["radar-hrrr"]
    assert layer["complete"] is True
    assert layer["run_id"] == "20260715_12"
    assert [frame["path"] for frame in layer["frames"]] == [
        "runs/20260715_12/2026-07-15T13:00:00+00:00",
        "runs/20260715_12/2026-07-15T14:00:00+00:00",
    ]


def _run_activity(activity_fn, *args):
    import asyncio

    from temporalio.testing import ActivityEnvironment

    return asyncio.run(ActivityEnvironment().run(activity_fn, *args))


def test_publish_partial_prefix_marks_layer_incomplete(monkeypatch, tmp_path):
    activities = import_activities_without_pygrib(monkeypatch)
    result = activities.ForecastHourResult
    palettes = {"classic": {"reflectivity": {}}}
    partial = [
        result(1, ["radar-hrrr"], valid_timestamp="2026-07-15T13:00:00+00:00"),
        result(2, ["radar-hrrr"], valid_timestamp="2026-07-15T14:00:00+00:00"),
        result(3, [], valid_timestamp="2026-07-15T15:00:00+00:00"),  # pending at NOAA
        result(4, ["radar-hrrr"], valid_timestamp="2026-07-15T16:00:00+00:00"),
    ]

    assert activities._publish_hrrr_run_sync(
        "20260715_12", partial, palettes, state_dir=tmp_path, horizon=4
    ) == ["radar-hrrr"]
    layer = json.loads((tmp_path / "manifest.json").read_text())["layers"]["radar-hrrr"]
    assert layer["complete"] is False
    assert layer["run_id"] == "20260715_12"
    # f04 is never advertised across the f03 gap.
    assert [frame["lead_minutes"] for frame in layer["frames"]] == [60, 120]

    partial[2] = result(3, ["radar-hrrr"], valid_timestamp="2026-07-15T15:00:00+00:00")
    assert activities._publish_hrrr_run_sync(
        "20260715_12", partial, palettes, state_dir=tmp_path, horizon=4
    ) == ["radar-hrrr"]
    layer = json.loads((tmp_path / "manifest.json").read_text())["layers"]["radar-hrrr"]
    assert layer["complete"] is True
    assert [frame["lead_minutes"] for frame in layer["frames"]] == [60, 120, 180, 240]


def test_publishable_prefix_stops_at_first_missing_hour(monkeypatch):
    activities = import_activities_without_pygrib(monkeypatch)
    result = activities.ForecastHourResult

    assert activities.publishable_prefix([]) == []
    assert activities.publishable_prefix([result(2, ["radar-hrrr"])]) == []
    prefix = activities.publishable_prefix(
        [
            result(3, ["radar-hrrr"]),
            result(1, ["radar-hrrr"]),
            result(2, ["temperature"]),  # reflectivity missing → prefix ends before it
            result(4, ["radar-hrrr"]),
        ]
    )
    assert [r.fhr for r in prefix] == [1]


def test_horizon_derived_from_run_hour(monkeypatch):
    activities = import_activities_without_pygrib(monkeypatch)
    assert (
        activities._horizon_for_run("20260715_12") == activities.EXTENDED_FORECAST_HOURS
    )
    assert activities._horizon_for_run("20260715_13") == activities.FORECAST_HOURS


def _mock_client(handler):
    import httpx

    return httpx.Client(transport=httpx.MockTransport(handler))


def test_download_treats_404_as_pending_without_raising(monkeypatch, tmp_path):
    import logging

    activities = import_activities_without_pygrib(monkeypatch)
    import httpx

    def not_uploaded(request: httpx.Request) -> httpx.Response:
        return httpx.Response(404, text="NoSuchKey")

    class Collect(logging.Handler):
        records: list[logging.LogRecord] = []

        def emit(self, record: logging.LogRecord) -> None:
            self.records.append(record)

    # The service logger is a non-propagating JSON logger, so listen on it directly.
    collector = Collect()
    activities.log.addHandler(collector)
    try:
        with _mock_client(not_uploaded) as client:
            out = activities._download_subset_sync(
                client, "20260715", "18", 42, ["refc"], tmp_path
            )
    finally:
        activities.log.removeHandler(collector)

    assert out is None
    assert not list(tmp_path.iterdir())
    pending = [rec for rec in collector.records if rec.getMessage() == "hour_pending"]
    assert len(pending) == 1
    assert pending[0].run_id == "20260715_18"
    assert pending[0].fhr == 42


def test_download_falls_back_to_full_file_when_only_idx_missing(monkeypatch, tmp_path):
    activities = import_activities_without_pygrib(monkeypatch)
    import httpx

    def grib_without_idx(request: httpx.Request) -> httpx.Response:
        if request.url.path.endswith(".idx"):
            return httpx.Response(404)
        return httpx.Response(200, content=b"GRIB-bytes")

    with _mock_client(grib_without_idx) as client:
        out = activities._download_subset_sync(
            client, "20260715", "18", 7, ["refc"], tmp_path
        )

    assert out == tmp_path / "hrrr_f07.grib2"
    assert out.read_bytes() == b"GRIB-bytes"


def test_download_non_404_errors_still_raise(monkeypatch, tmp_path):
    import pytest

    activities = import_activities_without_pygrib(monkeypatch)
    import httpx

    def broken(request: httpx.Request) -> httpx.Response:
        return httpx.Response(503)

    with _mock_client(broken) as client, pytest.raises(httpx.HTTPStatusError):
        activities._download_subset_sync(
            client, "20260715", "18", 7, ["refc"], tmp_path
        )


def _touch_pyramid(
    tile_base: Path,
    layer: str,
    palette: str,
    tile_path: str,
    color_table: dict,
    *,
    renderer: str = "legacy",
    algorithm: str | None = None,
    palette_payload: dict | None = None,
    zoom_levels: list[int] | None = None,
    semantic_policy: dict | None = None,
) -> None:
    from backend.shared import tiler

    root = tile_base / layer / palette / tile_path
    tile = tile_base / layer / palette / tile_path / "4" / "3" / "5.png"
    tile.parent.mkdir(parents=True, exist_ok=True)
    tile.write_bytes(b"png")
    tiler._write_completion_marker(
        root,
        {"4/3/5.png"},
        tiler.PyramidIdentity(
            renderer=renderer,
            algorithm=algorithm
            or (
                "rgba-bilinear-v1"
                if renderer == "legacy"
                else "physical-bilinear-classify-v1"
            ),
            source_id=f"hrrr:{layer}:{tile_path}",
            source_digest="fixture-source",
            grid_spec_digest="fixture-grid",
            palette_name=palette,
            palette_digest=tiler._stable_digest(palette_payload or color_table),
            tile_spec_digest=tiler._stable_digest(
                {
                    "tile_size": 256,
                    "zoom_levels": zoom_levels or [4, 5, 6],
                }
            ),
            semantic_digest=tiler._stable_digest(
                semantic_policy
                or {
                    "kind": "continuous",
                    "nodata_value": None,
                    "min_valid_weight": 1.0,
                }
            ),
            policy_digest=tiler._stable_digest(
                {
                    "sampling": "bilinear-rgba",
                    "category_map": None,
                    "png_compress_level": 1,
                }
                if renderer == "legacy"
                else {"fixture": "foreign-renderer-policy"}
            ),
        ),
    )


def test_existing_rendered_layers_requires_every_palette(monkeypatch, tmp_path):
    activities = import_activities_without_pygrib(monkeypatch)
    palettes = {"classic": {"reflectivity": {}}, "vivid": {"reflectivity": {}}}
    tile_path = "runs/20260715_12/2026-07-15T13:00:00+00:00"

    assert activities._existing_rendered_layers(tmp_path, tile_path, palettes) == []

    _touch_pyramid(
        tmp_path,
        "radar-hrrr",
        "classic",
        tile_path,
        palettes["classic"]["reflectivity"],
    )
    assert activities._existing_rendered_layers(tmp_path, tile_path, palettes) == []

    # An empty final dir (interrupted rename target) does not count as rendered.
    vivid = tmp_path / "radar-hrrr" / "vivid" / tile_path
    vivid.mkdir(parents=True)
    assert activities._existing_rendered_layers(tmp_path, tile_path, palettes) == []

    # Nor does a non-empty directory without a completion marker.
    unmarked = vivid / "4" / "3" / "5.png"
    unmarked.parent.mkdir(parents=True)
    unmarked.write_bytes(b"png")
    assert activities._existing_rendered_layers(tmp_path, tile_path, palettes) == []

    _touch_pyramid(
        tmp_path,
        "radar-hrrr",
        "vivid",
        tile_path,
        palettes["vivid"]["reflectivity"],
    )
    assert activities._existing_rendered_layers(tmp_path, tile_path, palettes) == [
        "radar-hrrr"
    ]


def test_legacy_render_adopts_matching_pre_marker_hrrr_pyramid(monkeypatch, tmp_path):
    activities = import_activities_without_pygrib(monkeypatch)
    from backend.shared import tiler

    monkeypatch.setattr(activities, "ZOOM_LEVELS", [4])
    monkeypatch.setenv("TILE_RENDERER", "legacy")
    data = np.full((64, 64), 25.0, dtype=np.float32)
    lats = np.linspace(40.0, 45.0, 64)
    lons = np.linspace(-90.0, -85.0, 64)
    grid = activities.ExtractedGrid(data=data, lats=lats, lons=lons)
    table = {
        "ranges": [{"min": 0, "max": 100, "rgba": [200, 0, 0, 255]}],
        "no_data_below": -1,
    }
    tile_path = "runs/20260715_12/2026-07-15T13:00:00+00:00"
    output = tmp_path / "radar-hrrr" / "classic" / tile_path
    count = tiler.render_tiles(
        tiler.apply_color_table(data, table),
        lats,
        lons,
        str(output),
        [4],
    )
    assert count > 0 and not (output / tiler._PYRAMID_COMPLETE_FILE).exists()

    result = activities._write_palette_tiles(
        tmp_path,
        "radar-hrrr",
        tile_path,
        data,
        grid,
        {"classic": table},
    )

    assert result.outcomes["classic"].status is tiler.PublishStatus.ADOPTED
    identity = tiler.complete_pyramid_identity(output)
    assert identity is not None
    assert identity.renderer == "legacy"
    assert identity.source_id == f"hrrr:radar-hrrr:{tile_path}"


def test_process_forecast_hour_resumes_from_existing_tiles(monkeypatch, tmp_path):
    activities = import_activities_without_pygrib(monkeypatch)
    palettes = {"classic": {"reflectivity": {}}, "vivid": {"reflectivity": {}}}
    tile_base = tmp_path / "tiles"
    monkeypatch.setattr(activities, "TILE_DIR", str(tile_base))
    monkeypatch.setattr(activities, "TMP_ROOT", tmp_path / "work")
    monkeypatch.setattr(activities, "ENABLED_LAYERS", {"radar-hrrr"})
    monkeypatch.setattr(activities, "_load_palette_tables", lambda: palettes)

    def must_not_download(*args, **kwargs):
        raise AssertionError("download attempted for an hour that is already rendered")

    monkeypatch.setattr(activities, "_download_subset_sync", must_not_download)

    tile_path = "runs/20260715_12/2026-07-15T13:00:00+00:00"
    for palette in palettes:
        _touch_pyramid(
            tile_base,
            "radar-hrrr",
            palette,
            tile_path,
            palettes[palette]["reflectivity"],
        )

    result = _run_activity(activities.hrrr_process_forecast_hour, "20260715_12", 1)

    assert result.rendered_layers == ["radar-hrrr"]
    assert result.valid_timestamp == "2026-07-15T13:00:00+00:00"
    assert not (tmp_path / "work").exists()


def test_existing_rendered_layers_accepts_complete_foreign_renderer_set(
    monkeypatch, tmp_path
):
    activities = import_activities_without_pygrib(monkeypatch)
    palettes = {"classic": {"reflectivity": {}}, "vivid": {"reflectivity": {}}}
    tile_path = "runs/20260715_12/2026-07-15T13:00:00+00:00"
    for palette in palettes:
        _touch_pyramid(
            tmp_path,
            "radar-hrrr",
            palette,
            tile_path,
            palettes[palette]["reflectivity"],
            renderer="indexed",
        )

    assert activities._existing_rendered_layers(tmp_path, tile_path, palettes) == [
        "radar-hrrr"
    ]


def test_existing_rendered_layers_rejects_mixed_renderer_set(monkeypatch, tmp_path):
    activities = import_activities_without_pygrib(monkeypatch)
    palettes = {"classic": {"reflectivity": {}}, "vivid": {"reflectivity": {}}}
    tile_path = "runs/20260715_12/2026-07-15T13:00:00+00:00"
    _touch_pyramid(
        tmp_path,
        "radar-hrrr",
        "classic",
        tile_path,
        palettes["classic"]["reflectivity"],
    )
    _touch_pyramid(
        tmp_path,
        "radar-hrrr",
        "vivid",
        tile_path,
        palettes["vivid"]["reflectivity"],
        renderer="indexed",
    )

    assert activities._existing_rendered_layers(tmp_path, tile_path, palettes) == []


@pytest.mark.parametrize(
    "marker_overrides",
    [
        {"algorithm": "future-render-v9"},
        {"zoom_levels": [4, 5]},
        {"palette_payload": {"ranges": [], "no_data_below": -999}},
        {
            "semantic_policy": {
                "kind": "continuous",
                "nodata_value": -9999.0,
                "min_valid_weight": 1.0,
            }
        },
    ],
    ids=["unknown-algorithm", "old-zoom", "old-palette", "old-semantics"],
)
def test_existing_rendered_layers_rejects_noncurrent_render_contract(
    monkeypatch, tmp_path, marker_overrides
):
    activities = import_activities_without_pygrib(monkeypatch)
    monkeypatch.setenv("TILE_RENDERER", "legacy")
    palettes = {"classic": {"reflectivity": {}}}
    tile_path = "runs/20260715_12/2026-07-15T13:00:00+00:00"
    _touch_pyramid(
        tmp_path,
        "radar-hrrr",
        "classic",
        tile_path,
        palettes["classic"]["reflectivity"],
        **marker_overrides,
    )

    assert activities._existing_rendered_layers(tmp_path, tile_path, palettes) == []


def test_process_forecast_hour_pending_returns_empty_without_raising(
    monkeypatch, tmp_path
):
    activities = import_activities_without_pygrib(monkeypatch)
    palettes = {"classic": {"reflectivity": {}}}
    monkeypatch.setattr(activities, "TILE_DIR", str(tmp_path / "tiles"))
    monkeypatch.setattr(activities, "TMP_ROOT", tmp_path / "work")
    monkeypatch.setattr(activities, "ENABLED_LAYERS", {"radar-hrrr"})
    monkeypatch.setattr(activities, "_load_palette_tables", lambda: palettes)
    monkeypatch.setattr(activities, "_download_subset_sync", lambda *a, **k: None)

    result = _run_activity(activities.hrrr_process_forecast_hour, "20260715_12", 42)

    assert result.rendered_layers == []
    assert result.fhr == 42
    assert not list((tmp_path / "work").iterdir())


def test_resume_incomplete_run_prefers_young_partial_run(monkeypatch):
    from datetime import datetime, timedelta, timezone

    activities = import_activities_without_pygrib(monkeypatch)
    now = datetime(2026, 7, 15, 14, 30, tzinfo=timezone.utc)

    partial = {"layers": {"radar-hrrr": {"run_id": "20260715_12", "complete": False}}}
    assert activities._resume_incomplete_run(partial, now) == "20260715_12"

    complete = {"layers": {"radar-hrrr": {"run_id": "20260715_12", "complete": True}}}
    assert activities._resume_incomplete_run(complete, now) is None

    legacy = {"layers": {"radar-hrrr": {"run_id": "20260715_12"}}}
    assert activities._resume_incomplete_run(legacy, now) is None

    stale = now + activities.INCOMPLETE_RUN_MAX_AGE + timedelta(minutes=1)
    assert activities._resume_incomplete_run(partial, stale) is None

    assert activities._resume_incomplete_run({"layers": {}}, now) is None
    garbage = {"layers": {"radar-hrrr": {"run_id": "not-a-run", "complete": False}}}
    assert activities._resume_incomplete_run(garbage, now) is None
