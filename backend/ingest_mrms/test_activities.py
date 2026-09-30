import asyncio
from datetime import timedelta

import numpy as np
from temporalio.client import ScheduleOverlapPolicy

from backend.ingest_mrms import activities
from backend.ingest_mrms.activities import _grid_axes


class _Msg(dict):
    def latlons(self):
        lats = np.array([[30.0, 30.0], [29.0, 29.0]])
        lons = np.array([[250.0, 251.0], [250.0, 251.0]])
        return lats, lons


def test_regular_grid_axes_come_from_the_header_without_meshes():
    msg = _Msg(
        gridType="regular_ll", Nj=3500, Ni=7000,
        latitudeOfFirstGridPointInDegrees=54.995, latitudeOfLastGridPointInDegrees=20.005001,
        longitudeOfFirstGridPointInDegrees=230.005, longitudeOfLastGridPointInDegrees=299.994998,
    )
    msg.latlons = None  # must not be called for regular grids
    lat, lon = _grid_axes(msg)
    assert lat.shape == (3500,) and lon.shape == (7000,)
    assert lat[0] == 54.995 and lat[-1] == 20.005001
    assert np.allclose(np.diff(lat), -0.01, atol=1e-6) and np.allclose(np.diff(lon), 0.01, atol=1e-6)


def test_other_grids_fall_back_to_latlons():
    lat, lon = _grid_axes(_Msg(gridType="lambert"))
    assert lat.tolist() == [30.0, 29.0] and lon.tolist() == [250.0, 251.0]


# ---------- nowcast kick ----------


class _FakeHandle:
    def __init__(self, calls: list) -> None:
        self.calls = calls

    async def trigger(self, **kwargs):
        self.calls.append(kwargs)


class _FakeClient:
    def __init__(self, calls: list, handles: list) -> None:
        self.calls = calls
        self.handles = handles

    def get_schedule_handle(self, schedule_id: str) -> _FakeHandle:
        self.handles.append(schedule_id)
        return _FakeHandle(self.calls)


def test_kick_triggers_the_nowcast_schedule_with_buffer_one(monkeypatch):
    calls: list = []
    handles: list = []
    monkeypatch.setattr(activities, "NOWCAST_KICK_ENABLED", True)
    monkeypatch.setattr(activities, "NOWCAST_SCHEDULE_ID", "nowcast")

    async def factory():
        return _FakeClient(calls, handles)

    assert asyncio.run(activities.kick_nowcast("2026-09-30T00:00:00+00:00", client_factory=factory))
    assert handles == ["nowcast"]
    assert calls == [
        {
            "overlap": ScheduleOverlapPolicy.BUFFER_ONE,
            "rpc_timeout": timedelta(seconds=activities.NOWCAST_KICK_TIMEOUT_S),
        }
    ]


def test_kick_failure_is_logged_not_raised(monkeypatch):
    monkeypatch.setattr(activities, "NOWCAST_KICK_ENABLED", True)

    async def factory():
        raise RuntimeError("temporal unreachable")

    assert asyncio.run(activities.kick_nowcast("ts", client_factory=factory)) is False


def test_kick_is_a_noop_when_disabled(monkeypatch):
    monkeypatch.setattr(activities, "NOWCAST_KICK_ENABLED", False)

    async def factory():
        raise AssertionError("must not connect when disabled")

    assert asyncio.run(activities.kick_nowcast("ts", client_factory=factory)) is False


def _run_frame(monkeypatch, tmp_path, layer_name: str) -> list[str]:
    """Drive mrms_process_frame with every heavy step stubbed; return kicked timestamps."""
    kicked: list[str] = []
    grid = np.full((4, 4), 30.0, dtype=np.float32)
    lats = np.linspace(45.0, 40.0, 4)
    lons = np.linspace(-100.0, -95.0, 4)

    monkeypatch.setattr(activities, "TILE_DIR", str(tmp_path / "tiles"))
    monkeypatch.setattr(activities, "STATE_DIR", str(tmp_path / "state"))
    monkeypatch.setattr(activities, "_load_palette_tables", lambda: {"classic": {}})
    monkeypatch.setattr(activities, "_current_activity_tmp_dir", lambda *a, **k: tmp_path / "tmp")
    monkeypatch.setattr(activities, "_download_and_decode_sync", lambda *a, **k: (grid, lats, lons))
    monkeypatch.setattr(activities, "_render_all_palettes", lambda *a, **k: ["classic"])
    monkeypatch.setattr(activities, "write_grid", lambda *a, **k: "written")
    monkeypatch.setattr(activities, "write_storms_json", lambda *a, **k: None)
    monkeypatch.setattr(activities, "prune_grid_layer", lambda *a, **k: 0)
    monkeypatch.setattr(activities, "update_manifest_file", lambda *a, **k: {})
    monkeypatch.setattr(activities.activity, "heartbeat", lambda *a, **k: None)

    async def to_thread(fn, *args, **kwargs):
        return fn(*args, **kwargs)

    async def run_sync(fn, *args, **kwargs):
        kwargs.pop("heartbeat_every", None)
        kwargs.pop("heartbeat_details", None)
        return fn(*args, **kwargs)

    async def kick(timestamp: str, **_kwargs) -> bool:
        kicked.append(timestamp)
        return True

    monkeypatch.setattr(activities.asyncio, "to_thread", to_thread)
    monkeypatch.setattr(activities, "run_sync_with_heartbeat", run_sync)
    monkeypatch.setattr(activities, "kick_nowcast", kick)

    key = "CONUS/Prefix/20260930/MRMS_Prefix_00.50_20260930-000000.grib2.gz"
    result = asyncio.run(
        activities.mrms_process_frame(activities.ProcessFrameInput(key=key, layer_name=layer_name))
    )
    assert result.rendered is True
    return kicked


def test_base_reflectivity_frame_kicks_the_nowcast_once(monkeypatch, tmp_path):
    assert _run_frame(monkeypatch, tmp_path, "radar") == ["2026-09-30T00:00:00+00:00"]


def test_composite_frame_does_not_kick_the_nowcast(monkeypatch, tmp_path):
    assert _run_frame(monkeypatch, tmp_path, "radar-composite") == []
