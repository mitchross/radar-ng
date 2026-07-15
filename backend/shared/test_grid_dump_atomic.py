import json
from pathlib import Path

import numpy as np

from backend.shared import grid_dump


def test_grid_metadata_atomically_points_to_versioned_binary(monkeypatch, tmp_path):
    monkeypatch.setattr(grid_dump, "GRID_DIR", str(tmp_path))
    data = np.arange(100, dtype=np.float32).reshape(10, 10)
    lats = np.linspace(40, 50, 10)
    lons = np.linspace(-90, -80, 10)

    first = Path(grid_dump.write_grid("radar", "2026-07-15T12:00:00+00:00", data, lats, lons, "dBZ", max_cells=25) or "")
    meta_path = tmp_path / "radar" / "2026-07-15T12:00:00+00:00.meta.json"
    first_meta = json.loads(meta_path.read_text())

    assert first.exists()
    assert first.name == first_meta["data_file"]
    assert first_meta["height"] * first_meta["width"] <= 25

    second = Path(grid_dump.write_grid("radar", "2026-07-15T12:00:00+00:00", data + 1, lats, lons, "dBZ", max_cells=25) or "")
    second_meta = json.loads(meta_path.read_text())

    assert second.exists()
    assert second != first
    assert second.name == second_meta["data_file"]
    assert first.exists()
