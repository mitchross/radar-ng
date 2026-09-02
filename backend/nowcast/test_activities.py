import sys
import types

import numpy as np

from backend.nowcast import activities


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
    calls: list[tuple[str, int]] = []

    def prune(layer: str, keep: int) -> int:
        calls.append((layer, keep))
        return 7

    monkeypatch.setattr(activities, "prune_grid_layer", prune)

    assert activities._prune_nowcast_point_grids(12) == 7
    assert calls == [("nowcast", activities.POINT_GRID_RETENTION_RUNS * 12)]


def test_point_grid_retention_keep_zero_still_preserves_lock(tmp_path, monkeypatch):
    from backend.shared import grid_dump

    monkeypatch.setattr(grid_dump, "GRID_DIR", str(tmp_path))
    monkeypatch.setattr(activities, "prune_grid_layer", grid_dump.prune_grid_layer)
    lats = np.linspace(50.0, 20.0, 4)
    lons = np.linspace(-100.0, -90.0, 4)
    grid_dump.write_grid(
        "nowcast",
        "2026-09-02T12:00:00+00:00",
        np.ones((4, 4), dtype=np.float32),
        lats,
        lons,
        unit="dBZ",
    )

    assert activities._prune_nowcast_point_grids(0) == 2
    assert (tmp_path / "nowcast" / grid_dump._LAYER_LOCK_FILE).is_file()
