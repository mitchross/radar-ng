import asyncio
import sys
import types
from datetime import datetime, timedelta

import numpy as np

from backend.nowcast import activities


def test_nowcast_publishes_run_scoped_grid_keys_before_pruning(monkeypatch, tmp_path):
    anchor = "2026-07-15T16:00:00+00:00"
    input_timestamps = [
        "2026-07-15T15:56:00+00:00",
        "2026-07-15T15:58:00+00:00",
        anchor,
    ]
    meta = {
        "height": 2,
        "width": 2,
        "lat_min": 42.0,
        "lat_max": 43.0,
        "lon_min": -86.0,
        "lon_max": -85.0,
    }
    monkeypatch.setattr(activities, "TILE_DIR", tmp_path / "tiles")
    monkeypatch.setattr(activities, "STATE_DIR", tmp_path / "state")
    monkeypatch.setattr(activities, "HORIZON_MIN", 60)
    monkeypatch.setattr(activities, "STEP_MIN", 5)
    monkeypatch.setattr(
        activities,
        "_list_recent_grids",
        lambda: [tmp_path / f"{timestamp}.meta.json" for timestamp in input_timestamps],
    )
    monkeypatch.setattr(
        activities,
        "_load_grid",
        lambda _path: (
            np.full((2, 2), 30.0, dtype=np.float32),
            np.array([43.0, 42.0]),
            np.array([-86.0, -85.0]),
            meta,
        ),
    )
    monkeypatch.setattr(
        activities.activity, "heartbeat", lambda *_args, **_kwargs: None
    )

    async def to_thread(fn, *args, **kwargs):
        return fn(*args, **kwargs)

    async def run_sync(fn, *args, **kwargs):
        kwargs.pop("heartbeat_every", None)
        kwargs.pop("heartbeat_details", None)
        return fn(*args, **kwargs)

    monkeypatch.setattr(activities.asyncio, "to_thread", to_thread)
    monkeypatch.setattr(activities, "run_sync_with_heartbeat", run_sync)
    monkeypatch.setattr(
        activities,
        "_run_nowcast",
        lambda _frames, lead_steps: (
            np.full((len(lead_steps), 2, 2), 30.0, dtype=np.float32),
            "pysteps-sprog",
        ),
    )
    monkeypatch.setattr(activities, "get_palette_names", lambda: ["classic"])
    monkeypatch.setattr(
        activities,
        "load_palette",
        lambda _name: {
            "reflectivity": {
                "ranges": [{"min": 5, "max": 80, "rgba": [255, 0, 0, 255]}],
                "no_data_below": 5,
            }
        },
    )
    monkeypatch.setattr(activities, "_render_frame", lambda *_args: ["classic"])

    grid_keys: list[str] = []
    events: list[tuple[str, object]] = []

    def write(*_args, grid_key: str, **_kwargs):
        grid_keys.append(grid_key)
        return str(tmp_path / grid_key)

    def finalize(layer: str, generation: str):
        events.append(("finalize", (layer, generation)))

    def publish(layer: str, timestamps: list[str], **kwargs):
        events.append(("publish", (layer, timestamps, kwargs)))

    def prune(layer: str, *, keep: int, active_generation: str | None):
        events.append(("prune", (layer, keep, active_generation)))
        return 0

    monkeypatch.setattr(activities, "write_grid", write)
    monkeypatch.setattr(activities, "finalize_grid_generation", finalize)
    monkeypatch.setattr(activities, "replace_layer_manifest", publish)
    monkeypatch.setattr(activities, "prune_grid_generations", prune)

    result = asyncio.run(activities.nowcast_run())

    assert result.ran is True and result.leadtimes == 12
    assert len(grid_keys) == 12
    assert all(key.startswith(f"runs/{anchor}/") for key in grid_keys)
    assert [name for name, _ in events] == ["finalize", "publish", "prune"]
    published_frames = events[1][1][2]["frames"]
    assert [frame["grid_key"] for frame in published_frames] == grid_keys
    assert events[2][1] == ("nowcast", activities.POINT_GRID_RETENTION_RUNS, anchor)


def test_pysteps_receives_fractional_input_timesteps(monkeypatch, tmp_path):
    monkeypatch.setattr(activities, "STATE_DIR", tmp_path)
    captured: dict[str, object] = {}

    def optical_flow(stack):
        return np.zeros((2, *stack.shape[1:]), dtype=np.float32)

    def nowcast(stack, velocity, timesteps, **kwargs):
        captured["timesteps"] = timesteps
        return np.repeat(stack[-1][None, :, :], len(timesteps), axis=0)

    fake = types.SimpleNamespace(
        motion=types.SimpleNamespace(get_method=lambda _: optical_flow),
        nowcasts=types.SimpleNamespace(get_method=lambda _: nowcast),
    )
    monkeypatch.setitem(sys.modules, "pysteps", fake)
    frames = [np.ones((8, 8), dtype=np.float32) * value for value in (10, 20, 30)]

    forecast, method = activities._run_nowcast(frames, [2.5, 5.0, 7.5])

    assert captured["timesteps"] == [2.5, 5.0, 7.5]
    assert forecast is not None and forecast.shape == (3, 8, 8)
    assert method == "pysteps-sprog"


def test_nowcast_fails_closed_when_pysteps_is_unavailable(monkeypatch, tmp_path):
    monkeypatch.setattr(activities, "STATE_DIR", tmp_path)
    monkeypatch.setattr(activities, "ALLOW_PERSISTENCE_FALLBACK", False)
    monkeypatch.setitem(sys.modules, "pysteps", None)
    frames = [np.ones((4, 4), dtype=np.float32)] * 3

    forecast, method = activities._run_nowcast(frames, [2.5])

    assert forecast is None
    assert method == "unavailable"


def test_nowcast_tile_path_is_scoped_to_anchor_run():
    valid = "2026-07-15T15:05:00+00:00"

    first = activities._nowcast_tile_path("2026-07-15T14:50:00+00:00", valid)
    later = activities._nowcast_tile_path("2026-07-15T15:00:00+00:00", valid)

    assert first != later
    assert first.endswith(f"/{valid}")


def test_point_grid_retention_uses_shared_locked_pruner(monkeypatch):
    calls: list[tuple[str, int, str | None]] = []

    def prune(layer: str, *, keep: int, active_generation: str | None) -> int:
        calls.append((layer, keep, active_generation))
        return 7

    monkeypatch.setattr(activities, "prune_grid_generations", prune)

    assert activities._prune_nowcast_point_grids("2026-09-02T12:00:00+00:00") == 7
    assert calls == [
        (
            "nowcast",
            activities.POINT_GRID_RETENTION_RUNS,
            "2026-09-02T12:00:00+00:00",
        )
    ]


def test_point_grid_retention_prunes_whole_runs_and_preserves_active(
    tmp_path, monkeypatch
):
    from backend.shared import grid_dump

    monkeypatch.setattr(grid_dump, "GRID_DIR", str(tmp_path))
    monkeypatch.setattr(
        activities, "prune_grid_generations", grid_dump.prune_grid_generations
    )
    lats = np.linspace(50.0, 20.0, 4)
    lons = np.linspace(-100.0, -90.0, 4)
    anchors = [
        "2026-09-02T12:00:00+00:00",
        "2026-09-02T12:05:00+00:00",
        "2026-09-02T12:10:00+00:00",
    ]
    for anchor_index, anchor in enumerate(anchors):
        anchor_dt = datetime.fromisoformat(anchor)
        for lead in range(1, 13):
            timestamp = (anchor_dt + timedelta(minutes=lead * 5)).isoformat()
            grid_dump.write_grid(
                "nowcast",
                timestamp,
                np.full((4, 4), anchor_index + 1, dtype=np.float32),
                lats,
                lons,
                unit="dBZ",
                grid_key=activities._nowcast_grid_key(anchor, timestamp),
            )
        grid_dump.finalize_grid_generation("nowcast", anchor)

    removed = activities._prune_nowcast_point_grids(anchors[1])

    assert removed == 25  # 12 metadata + 12 binaries + the completion marker
    runs = tmp_path / "nowcast" / grid_dump._GENERATION_ROOT
    assert not (runs / anchors[0]).exists()
    for anchor in anchors[1:]:
        generation = runs / anchor
        assert len(list(generation.glob("*.meta.json"))) == 12
        assert len(list(generation.glob("*.bin"))) == 12
        assert (generation / grid_dump._GENERATION_COMPLETE_FILE).is_file()
    assert (tmp_path / "nowcast" / grid_dump._LAYER_LOCK_FILE).is_file()
