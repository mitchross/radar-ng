import errno
import json
from pathlib import Path

import numpy as np
import pytest

from backend.shared import grid_dump


def test_grid_metadata_atomically_points_to_versioned_binary(monkeypatch, tmp_path):
    monkeypatch.setattr(grid_dump, "GRID_DIR", str(tmp_path))
    data = np.arange(100, dtype=np.float32).reshape(10, 10)
    lats = np.linspace(40, 50, 10)
    lons = np.linspace(-90, -80, 10)

    first = Path(
        grid_dump.write_grid(
            "radar", "2026-07-15T12:00:00+00:00", data, lats, lons, "dBZ", max_cells=25
        )
        or ""
    )
    meta_path = tmp_path / "radar" / "2026-07-15T12:00:00+00:00.meta.json"
    first_meta = json.loads(meta_path.read_text())

    assert first.exists()
    assert first.name == first_meta["data_file"]
    assert first_meta["height"] * first_meta["width"] <= 25

    second = Path(
        grid_dump.write_grid(
            "radar",
            "2026-07-15T12:00:00+00:00",
            data + 1,
            lats,
            lons,
            "dBZ",
            max_cells=25,
        )
        or ""
    )
    second_meta = json.loads(meta_path.read_text())

    assert second.exists()
    assert second != first
    assert second.name == second_meta["data_file"]
    assert first.exists()


def test_first_layer_creation_syncs_its_parent(monkeypatch, tmp_path):
    monkeypatch.setattr(grid_dump, "GRID_DIR", str(tmp_path))
    synced: list[Path] = []
    real_fsync_dir = grid_dump._fsync_dir

    def observe(path: Path) -> None:
        synced.append(Path(path))
        real_fsync_dir(path)

    monkeypatch.setattr(grid_dump, "_fsync_dir", observe)
    data = np.ones((2, 2), dtype=np.float32)
    lats = np.array([43.0, 42.0])
    lons = np.array([-86.0, -85.0])

    grid_dump.write_grid("radar", "2026-07-15T12:00:00+00:00", data, lats, lons, "dBZ")

    assert tmp_path in synced
    assert tmp_path / "radar" in synced


def test_directory_fsync_eio_is_not_suppressed(monkeypatch, tmp_path):
    def fail(_fd: int) -> None:
        raise OSError(errno.EIO, "simulated directory I/O failure")

    monkeypatch.setattr(grid_dump.os, "fsync", fail)

    with pytest.raises(OSError, match="directory I/O failure"):
        grid_dump._fsync_dir(tmp_path)


def test_write_grid_propagates_initial_parent_fsync_failure(monkeypatch, tmp_path):
    monkeypatch.setattr(grid_dump, "GRID_DIR", str(tmp_path))
    real_fsync_dir = grid_dump._fsync_dir

    def fail_parent(path: Path) -> None:
        if Path(path) == tmp_path:
            raise OSError(errno.EIO, "simulated parent fsync failure")
        real_fsync_dir(path)

    monkeypatch.setattr(grid_dump, "_fsync_dir", fail_parent)
    data = np.ones((2, 2), dtype=np.float32)
    lats = np.array([43.0, 42.0])
    lons = np.array([-86.0, -85.0])

    with pytest.raises(OSError, match="parent fsync failure"):
        grid_dump.write_grid(
            "radar", "2026-07-15T12:00:00+00:00", data, lats, lons, "dBZ"
        )


def test_directory_fsync_unsupported_error_is_ignored(monkeypatch, tmp_path):
    def unsupported(_fd: int) -> None:
        raise OSError(errno.EINVAL, "directory fsync unsupported")

    monkeypatch.setattr(grid_dump.os, "fsync", unsupported)

    grid_dump._fsync_dir(tmp_path)
