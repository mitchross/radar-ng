"""Keep-N retention for layers that only need their newest generations."""

import os
import threading
import time
from concurrent.futures import ThreadPoolExecutor
from pathlib import Path

import numpy as np

from backend.shared import grid_dump


def _write(layer: str, ts: str) -> str:
    lats = np.linspace(50.0, 20.0, 8)
    lons = np.linspace(-130.0, -60.0, 16)
    data = np.full((8, 16), 25.0, dtype=np.float32)
    return grid_dump.write_grid(layer, ts, data, lats, lons, unit="dBZ")


def _stamps(n: int) -> list[str]:
    return [f"2026-09-02T12:{m:02d}:00+00:00" for m in range(0, 2 * n, 2)]


def test_prune_keeps_newest_n_with_their_data_files(tmp_path, monkeypatch):
    monkeypatch.setattr(grid_dump, "GRID_DIR", str(tmp_path))
    stamps = _stamps(8)
    for ts in stamps:
        _write("radar-nowcast-input", ts)
    layer_dir = tmp_path / "radar-nowcast-input"
    assert len(list(layer_dir.glob("*.meta.json"))) == 8

    removed = grid_dump.prune_grid_layer("radar-nowcast-input", keep=4)

    assert removed == 8  # 4 metas + 4 data files
    metas = sorted(p.name for p in layer_dir.glob("*.meta.json"))
    assert metas == [f"{ts}.meta.json" for ts in stamps[-4:]]
    bins = list(layer_dir.glob("*.bin"))
    assert len(bins) == 4
    assert {grid_dump._grid_timestamp_of(p.name) for p in bins} == set(stamps[-4:])


def test_prune_removes_orphan_data_files_and_keeps_in_flight_tmp(tmp_path, monkeypatch):
    monkeypatch.setattr(grid_dump, "GRID_DIR", str(tmp_path))
    stamps = _stamps(3)
    for ts in stamps:
        _write("radar-nowcast-input", ts)
    layer_dir = tmp_path / "radar-nowcast-input"
    orphan = (
        layer_dir / "2026-09-02T11:00:00+00:00.deadbeefdeadbeefdeadbeefdeadbeef.bin"
    )
    orphan.write_bytes(b"\0" * 16)
    legacy = layer_dir / "2026-09-02T10:00:00+00:00.bin"
    legacy.write_bytes(b"\0" * 16)
    old = time.time() - grid_dump.GRID_ORPHAN_GRACE_S - 1
    os.utime(orphan, (old, old))
    os.utime(legacy, (old, old))
    inflight = layer_dir / ".2026-09-02T12:06:00+00:00.meta.json.abc.tmp"
    inflight.write_text("{}")

    removed = grid_dump.prune_grid_layer("radar-nowcast-input", keep=3)

    assert removed == 2
    assert not orphan.exists() and not legacy.exists()
    assert inflight.exists()
    assert len(list(layer_dir.glob("*.meta.json"))) == 3


def test_prune_is_noop_for_unconfigured_layer(tmp_path, monkeypatch):
    monkeypatch.setattr(grid_dump, "GRID_DIR", str(tmp_path))
    for ts in _stamps(5):
        _write("radar", ts)
    assert grid_dump.prune_grid_layer("radar") == 0
    assert len(list((tmp_path / "radar").glob("*.meta.json"))) == 5


def test_prune_keep_zero_removes_all_committed_generations(tmp_path, monkeypatch):
    monkeypatch.setattr(grid_dump, "GRID_DIR", str(tmp_path))
    for ts in _stamps(3):
        _write("radar-nowcast-input", ts)

    removed = grid_dump.prune_grid_layer("radar-nowcast-input", keep=0)

    layer_dir = tmp_path / "radar-nowcast-input"
    assert removed == 6
    assert not list(layer_dir.glob("*.meta.json"))
    assert not list(layer_dir.glob("*.bin"))
    assert (layer_dir / grid_dump._LAYER_LOCK_FILE).exists()


def test_prune_waits_for_grid_metadata_commit(tmp_path, monkeypatch):
    monkeypatch.setattr(grid_dump, "GRID_DIR", str(tmp_path))
    real_replace = grid_dump.os.replace
    writer_at_commit = threading.Event()
    release_writer = threading.Event()
    prune_entered = threading.Event()
    real_prune = grid_dump._prune_grid_layer_unlocked

    def paused_replace(src, dst):
        writer_at_commit.set()
        assert release_writer.wait(timeout=5)
        return real_replace(src, dst)

    def observed_prune(*args, **kwargs):
        prune_entered.set()
        return real_prune(*args, **kwargs)

    monkeypatch.setattr(grid_dump.os, "replace", paused_replace)
    monkeypatch.setattr(grid_dump, "_prune_grid_layer_unlocked", observed_prune)
    timestamp = _stamps(1)[0]

    with ThreadPoolExecutor(max_workers=2) as pool:
        write_future = pool.submit(_write, "radar-nowcast-input", timestamp)
        assert writer_at_commit.wait(timeout=5)
        prune_future = pool.submit(grid_dump.prune_grid_layer, "radar-nowcast-input", 1)
        assert not prune_entered.wait(timeout=0.05)
        release_writer.set()
        bin_path = Path(write_future.result(timeout=5))
        assert prune_future.result(timeout=5) == 0

    meta_path = tmp_path / "radar-nowcast-input" / f"{timestamp}.meta.json"
    assert meta_path.is_file()
    assert bin_path.is_file()


def test_cleanup_old_grids_applies_keep_n_before_age_sweep(tmp_path, monkeypatch):
    monkeypatch.setattr(grid_dump, "GRID_DIR", str(tmp_path))
    monkeypatch.setattr(grid_dump, "GRID_KEEP_LAST", {"radar-nowcast-input": 2})
    for ts in _stamps(5):
        _write("radar-nowcast-input", ts)
        _write("radar", ts)
    removed = grid_dump.cleanup_old_grids()
    assert removed == 6
    assert len(list((tmp_path / "radar-nowcast-input").glob("*.meta.json"))) == 2
    assert (
        len(list((tmp_path / "radar").glob("*.meta.json"))) == 5
    )  # young enough for the age rule


def test_default_keep_covers_nowcast_inputs_plus_slack():
    assert grid_dump.GRID_KEEP_LAST["radar-nowcast-input"] >= 3 + 2
