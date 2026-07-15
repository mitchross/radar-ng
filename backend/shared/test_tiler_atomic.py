"""Tests for render_tiles_atomic — atomic publish semantics."""

import tempfile
from pathlib import Path

import numpy as np


def _grid():
    rgba = np.zeros((64, 64, 4), dtype=np.uint8)
    rgba[:, :, 0] = 200
    rgba[:, :, 3] = 255
    lats = np.linspace(45.0, 40.0, 64)
    lons = np.linspace(-90.0, -85.0, 64)
    return rgba, lats, lons


def test_atomic_render_publishes_final_dir_without_tmp_leftover():
    from backend.shared.tiler import render_tiles_atomic

    rgba, lats, lons = _grid()
    with tempfile.TemporaryDirectory() as td:
        out = Path(td) / "radar" / "classic" / "2026-07-01T12:00:00+00:00"
        count = render_tiles_atomic(
            rgba=rgba, lats=lats, lons=lons, output_dir=str(out), zoom_levels=[4],
        )
        assert count > 0
        assert out.is_dir()
        assert not list(out.parent.glob(f".{out.name}.tmp-*"))
        assert list(out.glob("*/*/*.png"))


def test_atomic_render_keeps_existing_immutable_dir():
    from backend.shared.tiler import render_tiles_atomic

    rgba, lats, lons = _grid()
    with tempfile.TemporaryDirectory() as td:
        out = Path(td) / "nowcast" / "classic" / "2026-07-01T12:05:00+00:00"
        out.mkdir(parents=True)
        stale = out / "stale-marker"
        stale.write_text("old run")
        count = render_tiles_atomic(
            rgba=rgba, lats=lats, lons=lons, output_dir=str(out), zoom_levels=[4],
        )
        assert count > 0
        assert stale.exists()
        assert not list(out.parent.glob(f".{out.name}.tmp-*"))


def test_atomic_render_transparent_frame_publishes_nothing():
    from backend.shared.tiler import render_tiles_atomic

    rgba = np.zeros((64, 64, 4), dtype=np.uint8)  # fully transparent
    lats = np.linspace(45.0, 40.0, 64)
    lons = np.linspace(-90.0, -85.0, 64)
    with tempfile.TemporaryDirectory() as td:
        out = Path(td) / "radar" / "classic" / "2026-07-01T12:10:00+00:00"
        count = render_tiles_atomic(
            rgba=rgba, lats=lats, lons=lons, output_dir=str(out), zoom_levels=[4],
        )
        assert count == 0
        assert not out.exists()
        assert not list(out.parent.glob(f".{out.name}.tmp-*"))


def test_atomic_render_cleans_tmp_on_failure(monkeypatch):
    from backend.shared import tiler

    rgba, lats, lons = _grid()

    def boom(**kwargs):
        # Simulate a crash after partial writes.
        Path(kwargs["output_dir"], "4", "3").mkdir(parents=True)
        raise RuntimeError("render died")

    monkeypatch.setattr(tiler, "render_tiles", boom)
    with tempfile.TemporaryDirectory() as td:
        out = Path(td) / "radar" / "classic" / "2026-07-01T12:15:00+00:00"
        out.parent.mkdir(parents=True)
        try:
            tiler.render_tiles_atomic(
                rgba=rgba, lats=lats, lons=lons, output_dir=str(out), zoom_levels=[4],
            )
            raise AssertionError("expected RuntimeError")
        except RuntimeError:
            pass
        assert not out.exists()
        assert not list(out.parent.glob(f".{out.name}.tmp-*"))
