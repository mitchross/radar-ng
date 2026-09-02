"""Render-once classification, reviewed resampling, cache, and publication."""

from __future__ import annotations

import errno
import json
import struct
import threading
import time
from concurrent.futures import ThreadPoolExecutor
from pathlib import Path

import numpy as np
import pytest
from PIL import Image

from backend.shared import tiler
from backend.shared.png_palette import iter_chunks
from backend.shared.tiler import (
    apply_categorical_color_table,
    apply_color_table,
    build_categorical_class_model,
    build_class_model,
    clear_tile_geometry_cache,
    PublishStatus,
    render_frame_palettes,
    render_indexed_tiles_atomic,
    tile_geometry_cache_stats,
)

PALETTES_DIR = Path(__file__).resolve().parent / "palettes"
PALETTE_NAMES = ("classic", "muted", "vivid")


def _load_palettes() -> dict[str, dict]:
    return {
        name: json.loads((PALETTES_DIR / f"{name}.json").read_text())
        for name in PALETTE_NAMES
    }


def _legacy_apply_color_table(data: np.ndarray, color_table: dict) -> np.ndarray:
    """The pre-render-once implementation, kept verbatim as the reference."""
    h, w = data.shape
    rgba = np.zeros((h, w, 4), dtype=np.uint8)
    no_data = color_table.get("no_data_below", -999)
    for rng in color_table["ranges"]:
        mask = (data >= rng["min"]) & (data < rng["max"])
        rgba[mask] = rng["rgba"]
    rgba[data < no_data] = [0, 0, 0, 0]
    return rgba


def _legacy_apply_categorical(data, categories, category_map):
    h, w = data.shape
    rgba = np.zeros((h, w, 4), dtype=np.uint8)
    for value, name in category_map.items():
        if name in categories:
            rgba[data == value] = categories[name]
    return rgba


def _sample_values(table: dict, rng: np.random.Generator, n: int = 4000) -> np.ndarray:
    edges = sorted(
        {float(r["min"]) for r in table["ranges"]}
        | {float(r["max"]) for r in table["ranges"]}
    )
    lo, hi = edges[0] - 10, edges[-1] + 10
    vals = rng.uniform(lo, hi, size=n)
    exact = np.array(edges + [float(table.get("no_data_below", -999))])
    eps = np.nextafter(exact, np.inf) - exact
    specials = np.concatenate(
        [
            exact,
            exact - eps,
            exact + eps,
            [np.nan, np.inf, -np.inf, -9999.0, -999.0, -99.0],
        ]
    )
    return np.concatenate([vals, specials])


# ---------- palette invariants ----------


def test_all_shipped_palettes_share_bin_edges_per_color_key():
    palettes = _load_palettes()
    keys = {k for p in palettes.values() for k in p if not k.startswith("_")}
    for key in sorted(keys):
        tables = {name: p[key] for name, p in palettes.items() if key in p}
        assert len(tables) == len(palettes), f"{key} missing from a palette"
        if "categories" in next(iter(tables.values())):
            assert (
                len({tuple(sorted(t["categories"])) for t in tables.values()}) == 1
            ), key
            continue
        edge_sets = {
            name: tuple((float(r["min"]), float(r["max"])) for r in t["ranges"])
            + (float(t["no_data_below"]),)
            for name, t in tables.items()
        }
        assert len(set(edge_sets.values())) == 1, (
            f"{key}: palettes disagree on bin edges {edge_sets}"
        )
        model = build_class_model(tables)
        # Shared edges → one visible class per range, plus the transparent class 0.
        assert model.n_classes == len(tables["classic"]["ranges"]) + 1


# ---------- class model == legacy colour table ----------


@pytest.mark.parametrize("dtype", [np.float32, np.float64])
def test_class_model_matches_legacy_for_every_shipped_table(dtype):
    palettes = _load_palettes()
    rng = np.random.default_rng(7)
    for key in [
        k
        for k in palettes["classic"]
        if not k.startswith("_") and "ranges" in palettes["classic"][k]
    ]:
        tables = {name: p[key] for name, p in palettes.items()}
        model = build_class_model(tables)
        vals = _sample_values(tables["classic"], rng).astype(dtype).reshape(-1, 1)
        idx = model.classify(vals)
        assert idx.dtype == np.uint8
        for name, table in tables.items():
            expected = _legacy_apply_color_table(vals, table)
            assert np.array_equal(model.luts[name][idx], expected), (key, name)
            assert np.array_equal(apply_color_table(vals, table), expected), (key, name)


def test_class_model_handles_palettes_with_different_edges():
    a = {
        "ranges": [
            {"min": 0, "max": 10, "rgba": [1, 1, 1, 255]},
            {"min": 10, "max": 20, "rgba": [2, 2, 2, 255]},
        ],
        "no_data_below": 0,
    }
    b = {"ranges": [{"min": 5, "max": 15, "rgba": [3, 3, 3, 255]}], "no_data_below": 5}
    model = build_class_model({"a": a, "b": b})
    vals = np.array([[-1, 0, 4.9, 5, 9.9, 10, 14.9, 15, 19.9, 20]], dtype=np.float64)
    idx = model.classify(vals)
    assert np.array_equal(model.luts["a"][idx], _legacy_apply_color_table(vals, a))
    assert np.array_equal(model.luts["b"][idx], _legacy_apply_color_table(vals, b))
    assert (
        idx[0, 0] == 0 and idx[0, -1] == 0
    )  # outside every range → the shared transparent class


def test_class_model_later_overlapping_range_wins_like_legacy():
    table = {
        "ranges": [
            {"min": 0, "max": 20, "rgba": [1, 0, 0, 255]},
            {"min": 10, "max": 30, "rgba": [0, 1, 0, 255]},
        ],
        "no_data_below": -999,
    }
    vals = np.array([[5.0, 15.0, 25.0, 35.0]])
    assert np.array_equal(
        apply_color_table(vals, table), _legacy_apply_color_table(vals, table)
    )


def test_categorical_model_matches_legacy():
    palettes = _load_palettes()
    tables = {name: p["precip_type"] for name, p in palettes.items()}
    cmap = {1: "rain", 2: "snow", 3: "freezing_rain", 4: "ice_pellets"}
    model = build_categorical_class_model(tables, cmap)
    data = np.random.default_rng(3).integers(-1, 7, size=(40, 50)).astype(np.int32)
    idx = model.classify(data)
    for name, table in tables.items():
        expected = _legacy_apply_categorical(data, table["categories"], cmap)
        assert np.array_equal(model.luts[name][idx], expected)
        assert np.array_equal(
            apply_categorical_color_table(data, table["categories"], cmap), expected
        )


def test_classify_is_chunked_and_shape_preserving():
    model = build_class_model(
        {
            "p": {
                "ranges": [{"min": 0, "max": 1, "rgba": [9, 9, 9, 9]}],
                "no_data_below": 0,
            }
        }
    )
    tall = np.tile(np.array([[-1.0, 0.5, 2.0]]), (3000, 1))  # forces several row chunks
    idx = model.classify(tall)
    assert idx.shape == tall.shape and set(np.unique(idx)) == {0, 1}
    assert model.classify(np.array([0.5, 2.0])).tolist() == [1, 0]
    assert model.classify(np.zeros((0, 3))).shape == (0, 3)


def test_bilinear_samples_physical_values_with_explicit_nodata_policy():
    geom = tiler._TileGeometry(
        sampling="bilinear",
        rows=np.array([0.5], dtype=np.float32),
        cols=np.array([0.5], dtype=np.float32),
        flat=None,
        row_bounds=(0, 1),
        col_bounds=(0, 1),
    )
    data = np.array([[0.0, 10.0], [20.0, 30.0]], dtype=np.float32)
    assert (
        geom.sample_bilinear(data, nodata_value=None, min_valid_weight=1.0)[0, 0]
        == 15.0
    )

    data[1, 1] = -9999.0
    strict = geom.sample_bilinear(data, nodata_value=-9999.0, min_valid_weight=1.0)
    partial = geom.sample_bilinear(data, nodata_value=-9999.0, min_valid_weight=0.75)
    assert np.isnan(strict[0, 0])
    assert partial[0, 0] == 10.0


def test_fractional_indices_support_nonuniform_monotonic_axes():
    values = np.array([-1.0, 0.5, 2.0, 4.0])
    ascending = tiler._axis_to_fractional_indices(values, np.array([0.0, 1.0, 3.0]))
    descending = tiler._axis_to_fractional_indices(values, np.array([3.0, 1.0, 0.0]))
    assert np.allclose(ascending, [-1.0, 0.5, 1.5, 2.5])
    assert np.allclose(descending, [3.0, 1.5, 0.5, -0.5])
    with pytest.raises(ValueError, match="strictly monotonic"):
        tiler._axis_to_fractional_indices(values, np.array([0.0, 2.0, 1.0]))


# ---------- golden tiles ----------


def _synthetic_reflectivity(
    h: int = 240, w: int = 480
) -> tuple[np.ndarray, np.ndarray, np.ndarray]:
    """CONUS-ish grid of stepped echoes (clear border so the grid edge is never a class edge)."""
    lats = np.linspace(25.0, 50.0, h)
    lons = np.linspace(-125.0, -65.0, w)
    yy, xx = np.mgrid[0:h, 0:w]
    data = np.full((h, w), np.nan, dtype=np.float32)
    for cy, cx, r, base in (
        (70, 120, 45, 20.0),
        (150, 300, 60, 35.0),
        (110, 400, 30, 55.0),
    ):
        d = np.hypot(yy - cy, xx - cx)
        core = d < r
        data[core] = base + np.floor((r - d[core]) / 12) * 5.0
    data[:8, :] = data[-8:, :] = np.nan
    data[:, :8] = data[:, -8:] = np.nan
    return data, lats, lons


def _decode(path: Path) -> np.ndarray:
    return np.asarray(Image.open(path).convert("RGBA"))


def _regular_physical_reference(
    data: np.ndarray,
    lats: np.ndarray,
    lons: np.ndarray,
    z: int,
    tx: int,
    ty: int,
    tile_size: int,
    *,
    nodata_value: float | None = None,
) -> np.ndarray:
    """Independent scipy reference for strict-support physical bilinear sampling."""
    import scipy.ndimage

    n = 2**z
    px = (np.arange(tile_size) + 0.5) / tile_size
    tile_lons = (tx + px) / n * 360.0 - 180.0
    tile_lats = np.degrees(np.arctan(np.sinh(np.pi * (1.0 - 2.0 * (ty + px) / n))))
    rows = tiler._axis_to_fractional_indices(tile_lats, lats).astype(np.float32)
    cols = tiler._axis_to_fractional_indices(tile_lons, lons).astype(np.float32)
    rr, cc = np.meshgrid(rows, cols, indexing="ij")
    values = np.asarray(data, dtype=np.float32)
    if nodata_value is not None:
        values = np.where(values == np.float32(nodata_value), np.nan, values)
    return scipy.ndimage.map_coordinates(
        values,
        [rr, cc],
        order=1,
        mode="constant",
        cval=np.nan,
        prefilter=False,
    )


def test_continuous_tiles_match_physical_bilinear_then_classify_reference(tmp_path):
    palettes = _load_palettes()
    tables = {name: p["reflectivity"] for name, p in palettes.items()}
    data, lats, lons = _synthetic_reflectivity()
    root = tmp_path / "tiles"
    result = render_frame_palettes(
        data,
        lats,
        lons,
        tables,
        {name: str(root / name) for name in tables},
        [4, 5, 6],
        nodata_value=None,
        min_valid_weight=1.0,
    )
    assert len(set(result.counts.values())) == 1 and result.counts["classic"] > 0

    model = build_class_model(tables)
    for png in sorted((root / "classic").rglob("*.png")):
        z, tx, ty = int(png.parts[-3]), int(png.parts[-2]), int(png.stem)
        sampled = _regular_physical_reference(data, lats, lons, z, tx, ty, 256)
        ref_idx = model.classify(sampled)
        assert ref_idx.any()
        for name in tables:
            got = _decode(root / name / png.relative_to(root / "classic"))
            assert np.array_equal(got, model.luts[name][ref_idx]), (name, png.name)


def test_categorical_tiles_equal_nearest_class_reference(tmp_path):
    """Categorical IDs are nearest-sampled, never bilinearly interpolated."""
    import scipy.ndimage

    palettes = _load_palettes()
    tables = {name: p["precip_type"] for name, p in palettes.items()}
    category_map = {1: "rain", 2: "snow", 3: "freezing_rain", 4: "ice_pellets"}
    lats = np.linspace(25.0, 50.0, 120)
    lons = np.linspace(-125.0, -65.0, 240)
    data = np.zeros((120, 240), dtype=np.int16)
    data[15:105, 20:220] = np.indices((90, 200))[1] % 4 + 1
    model = build_categorical_class_model(tables, category_map)
    idx = model.classify(data)
    h, w = idx.shape
    root = tmp_path / "tiles"
    render_frame_palettes(
        data,
        lats,
        lons,
        tables,
        {name: str(root / name) for name in tables},
        [5],
        category_map=category_map,
    )

    for png in sorted((root / "classic").rglob("*.png")):
        z, tx, ty = int(png.parts[-3]), int(png.parts[-2]), int(png.stem)
        n = 2**z
        px = (np.arange(256) + 0.5) / 256
        lons_tile = (tx + px) / n * 360.0 - 180.0
        lats_tile = np.degrees(np.arctan(np.sinh(np.pi * (1.0 - 2.0 * (ty + px) / n))))
        rows = tiler._axis_to_fractional_indices(lats_tile, lats)
        cols = tiler._axis_to_fractional_indices(lons_tile, lons)
        rr, cc = np.meshgrid(rows, cols, indexing="ij")
        ref_idx = scipy.ndimage.map_coordinates(
            idx, [rr, cc], order=0, mode="constant", cval=0
        )
        for name in tables:
            got = _decode(root / name / png.relative_to(root / "classic"))
            assert np.array_equal(got, model.luts[name][ref_idx]), (name, png.name)


def test_indexed_tiles_share_idat_and_are_indexed_pngs(tmp_path):
    palettes = _load_palettes()
    tables = {name: p["reflectivity"] for name, p in palettes.items()}
    data, lats, lons = _synthetic_reflectivity()
    root = tmp_path / "tiles"
    render_frame_palettes(
        data, lats, lons, tables, {n: str(root / n) for n in tables}, [4, 5]
    )
    model = build_class_model(tables)
    for png in (root / "classic").rglob("*.png"):
        rel = png.relative_to(root / "classic")
        chunks = {
            name: dict(iter_chunks((root / name / rel).read_bytes())) for name in tables
        }
        ihdr = struct.unpack(">IIBBBBB", chunks["classic"][b"IHDR"])
        assert ihdr[3] == 3  # indexed colour
        assert len({c[b"IDAT"] for c in chunks.values()}) == 1
        for name in tables:
            assert chunks[name][b"PLTE"] == model.plte(name)
            assert chunks[name][b"tRNS"] == model.trns(name)
        # Indexed tiles are a fraction of the RGBA size.
        assert png.stat().st_size < 256 * 256


def test_legacy_renderer_is_a_marker_validated_rollback_path(tmp_path):
    palettes = _load_palettes()
    tables = {"classic": palettes["classic"]["reflectivity"]}
    data, lats, lons = _synthetic_reflectivity()
    output = tmp_path / "classic"

    first = render_frame_palettes(
        data,
        lats,
        lons,
        tables,
        {"classic": str(output)},
        [4],
        renderer="legacy",
    )
    assert first.outcomes["classic"].status is PublishStatus.CREATED
    assert first.counts["classic"] > 0
    assert tiler.is_complete_pyramid(output)
    assert Image.open(next(output.rglob("*.png"))).mode == "RGBA"

    retry = render_frame_palettes(
        data,
        lats,
        lons,
        tables,
        {"classic": str(output)},
        [4],
        renderer="legacy",
    )
    assert retry.outcomes["classic"].status is PublishStatus.EXISTING_VALID
    assert retry.counts == first.counts


def test_unknown_renderer_fails_closed(tmp_path):
    palettes = _load_palettes()
    data, lats, lons = _synthetic_reflectivity()
    with pytest.raises(ValueError, match="unknown tile renderer"):
        render_frame_palettes(
            data,
            lats,
            lons,
            {"classic": palettes["classic"]["reflectivity"]},
            {"classic": str(tmp_path / "classic")},
            [4],
            renderer="typo",
        )


def test_projected_continuous_grid_matches_physical_bilinear_reference(tmp_path):
    import scipy.ndimage
    from pyproj import Transformer

    to_merc = Transformer.from_crs("EPSG:4326", "EPSG:3857", always_xy=True)
    to_geo = Transformer.from_crs("EPSG:3857", "EPSG:4326", always_xy=True)
    x0, y0 = to_merc.transform(-125.0, 25.0)
    x1, y1 = to_merc.transform(-65.0, 50.0)
    source_x = np.linspace(x0, x1, 160)
    source_y = np.linspace(y1, y0, 100)
    xs, ys = np.meshgrid(source_x, source_y, indexing="xy")
    lons, lats = to_geo.transform(xs, ys)
    data = np.full((100, 160), np.nan, dtype=np.float32)
    data[20:80, 30:130] = 30.0
    table = {
        "ranges": [{"min": 0, "max": 100, "rgba": [0, 128, 255, 200]}],
        "no_data_below": -1,
    }

    root = tmp_path / "tiles"
    result = render_frame_palettes(
        data,
        lats,
        lons,
        {"p": table},
        {"p": str(root)},
        [4, 5],
        tile_size=64,
        source_crs="EPSG:3857",
        source_x=source_x,
        source_y=source_y,
    )
    assert result.counts["p"] > 0
    model = build_class_model({"p": table})
    for png in root.rglob("*.png"):
        z, tx, ty = int(png.parts[-3]), int(png.parts[-2]), int(png.stem)
        n = 2**z
        px = (np.arange(64) + 0.5) / 64
        tile_lons = (tx + px) / n * 360.0 - 180.0
        tile_lats = np.degrees(np.arctan(np.sinh(np.pi * (1.0 - 2.0 * (ty + px) / n))))
        lon_mesh, lat_mesh = np.meshgrid(tile_lons, tile_lats, indexing="xy")
        mapped_x, mapped_y = to_merc.transform(lon_mesh, lat_mesh)
        cols = tiler._axis_to_fractional_indices(mapped_x, source_x).astype(np.float32)
        rows = tiler._axis_to_fractional_indices(mapped_y, source_y).astype(np.float32)
        sampled = scipy.ndimage.map_coordinates(
            data,
            [rows, cols],
            order=1,
            mode="constant",
            cval=np.nan,
            prefilter=False,
        )
        expected = model.luts["p"][model.classify(sampled)]
        assert np.array_equal(_decode(png), expected), png


# ---------- geometry cache ----------


def test_geometry_cache_hits_on_second_frame_and_misses_on_new_grid(tmp_path):
    clear_tile_geometry_cache()
    table = {
        "ranges": [{"min": 0, "max": 100, "rgba": [255, 0, 0, 200]}],
        "no_data_below": -1,
    }
    data, lats, lons = _synthetic_reflectivity()
    data = np.where(np.isnan(data), 20.0, data)

    render_frame_palettes(
        data, lats, lons, {"p": table}, {"p": str(tmp_path / "a")}, [4, 5]
    )
    first = tile_geometry_cache_stats()
    assert (
        first["hits"] == 0
        and first["misses"] > 0
        and first["entries"] == first["misses"]
    )

    render_frame_palettes(
        data * 0.5, lats, lons, {"p": table}, {"p": str(tmp_path / "b")}, [4, 5]
    )
    second = tile_geometry_cache_stats()
    assert second["misses"] == first["misses"]
    assert second["hits"] == first["misses"]

    render_frame_palettes(
        data[::2, ::2],
        lats[::2],
        lons[::2],
        {"p": table},
        {"p": str(tmp_path / "c")},
        [4, 5],
    )
    third = tile_geometry_cache_stats()
    assert third["misses"] > second["misses"]


def test_geometry_cache_skips_pyproj_on_cache_hit(tmp_path, monkeypatch):
    import pyproj

    clear_tile_geometry_cache()
    calls = {"transform": 0}
    real_from_crs = pyproj.Transformer.from_crs

    class _Counting:
        def __init__(self, inner):
            self._inner = inner

        def transform(self, *a, **k):
            calls["transform"] += 1
            return self._inner.transform(*a, **k)

    monkeypatch.setattr(
        pyproj.Transformer,
        "from_crs",
        lambda *a, **k: _Counting(real_from_crs(*a, **k)),
    )

    to_merc = real_from_crs("EPSG:4326", "EPSG:3857", always_xy=True)
    to_geo = real_from_crs("EPSG:3857", "EPSG:4326", always_xy=True)
    x0, y0 = to_merc.transform(-125.0, 25.0)
    x1, y1 = to_merc.transform(-65.0, 50.0)
    source_x = np.linspace(x0, x1, 160)
    source_y = np.linspace(y1, y0, 100)
    xs, ys = np.meshgrid(source_x, source_y, indexing="xy")
    lons, lats = to_geo.transform(xs, ys)
    data = np.full((100, 160), 20.0, dtype=np.float32)
    table = {
        "ranges": [{"min": 0, "max": 100, "rgba": [0, 128, 255, 200]}],
        "no_data_below": -1,
    }
    kwargs = dict(
        tile_size=64, source_crs="EPSG:3857", source_x=source_x, source_y=source_y
    )

    render_frame_palettes(
        data, lats, lons, {"p": table}, {"p": str(tmp_path / "a")}, [4, 5], **kwargs
    )
    first_calls = calls["transform"]
    assert first_calls > 0
    assert first_calls <= tile_geometry_cache_stats()["misses"]

    render_frame_palettes(
        data, lats, lons, {"p": table}, {"p": str(tmp_path / "b")}, [4, 5], **kwargs
    )
    assert calls["transform"] == first_calls


def test_geometry_cache_is_bounded(monkeypatch):
    clear_tile_geometry_cache()
    monkeypatch.setattr(tiler, "TILE_GEOMETRY_CACHE_MB", 0)
    for i in range(5):
        tiler._geometry_cache_put(
            ("k", i),
            tiler._TileGeometry(
                sampling="nearest",
                rows=np.zeros(256, np.int32),
                cols=np.zeros(256, np.int32),
                flat=None,
                row_bounds=(0, 1),
                col_bounds=(0, 1),
            ),
        )
    assert tile_geometry_cache_stats()["entries"] == 1
    clear_tile_geometry_cache()


def test_grid_spec_fingerprints_full_axes_not_only_shape_and_endpoints(tmp_path):
    clear_tile_geometry_cache()
    table = {
        "ranges": [{"min": 0, "max": 100, "rgba": [255, 0, 0, 255]}],
        "no_data_below": -1,
    }
    data = np.full((32, 48), 20.0, dtype=np.float32)
    lats = np.linspace(40.0, 45.0, 32)
    lons = np.linspace(-90.0, -85.0, 48)
    render_frame_palettes(
        data, lats, lons, {"p": table}, {"p": str(tmp_path / "a")}, [5], tile_size=32
    )
    first = tile_geometry_cache_stats()

    changed = lons.copy()
    changed[1:-1] += np.sin(np.linspace(0, np.pi, len(changed) - 2)) * 0.01
    render_frame_palettes(
        data, lats, changed, {"p": table}, {"p": str(tmp_path / "b")}, [5], tile_size=32
    )
    assert tile_geometry_cache_stats()["misses"] > first["misses"]


def test_geometry_cache_single_flight_computes_one_value():
    clear_tile_geometry_cache()
    workers = 4
    start = threading.Barrier(workers)
    release = threading.Event()
    calls = 0

    def compute():
        nonlocal calls
        calls += 1
        assert release.wait(timeout=5)
        return tiler._TileGeometry(
            sampling="nearest",
            rows=np.array([0], dtype=np.int32),
            cols=np.array([0], dtype=np.int32),
            flat=None,
            row_bounds=(0, 0),
            col_bounds=(0, 0),
        )

    def get():
        start.wait(timeout=5)
        return tiler._geometry_cache_get_or_compute(("single-flight",), compute)

    with ThreadPoolExecutor(max_workers=workers) as pool:
        futures = [pool.submit(get) for _ in range(workers)]
        deadline = time.monotonic() + 5
        while (
            tile_geometry_cache_stats()["waits"] < workers - 1
            and time.monotonic() < deadline
        ):
            time.sleep(0.001)
        release.set()
        results = [future.result(timeout=5) for future in futures]

    assert calls == 1
    assert all(result is results[0] for result in results)
    stats = tile_geometry_cache_stats()
    assert stats["misses"] == 1
    assert stats["waits"] == workers - 1


# ---------- atomic multi-palette publish ----------


def _index_frame():
    idx = np.zeros((64, 64), dtype=np.uint8)
    idx[8:56, 8:56] = 1
    luts = {
        "classic": np.array([[0, 0, 0, 0], [200, 0, 0, 255]], dtype=np.uint8),
        "muted": np.array([[0, 0, 0, 0], [0, 0, 200, 255]], dtype=np.uint8),
    }
    lats = np.linspace(40.0, 45.0, 64)
    lons = np.linspace(-90.0, -85.0, 64)
    return idx, luts, lats, lons


def test_atomic_multi_palette_publishes_each_pyramid_without_tmp_leftover(tmp_path):
    idx, luts, lats, lons = _index_frame()
    outs = {p: str(tmp_path / "radar" / p / "2026-07-01T12:00:00+00:00") for p in luts}
    result = render_indexed_tiles_atomic(idx, lats, lons, outs, luts, [4])
    assert result.counts["classic"] == result.counts["muted"] > 0
    assert {outcome.status for outcome in result.outcomes.values()} == {
        PublishStatus.CREATED
    }
    for p, out in outs.items():
        out = Path(out)
        assert out.is_dir() and list(out.glob("*/*/*.png"))
        assert (out / tiler._PYRAMID_COMPLETE_FILE).is_file()
        assert not list(out.parent.glob(f".{out.name}.tmp-*"))
        tile = _decode(next(out.glob("*/*/*.png")))
        opaque = tile[:, :, 3] > 0
        assert opaque.any()
        assert np.all(tile[opaque] == luts[p][1])


def test_atomic_multi_palette_rejects_incomplete_existing_dir(tmp_path):
    idx, luts, lats, lons = _index_frame()
    outs = {p: str(tmp_path / "radar" / p / "2026-07-01T12:05:00+00:00") for p in luts}
    existing = Path(outs["classic"])
    existing.mkdir(parents=True)
    (existing / "stale-marker").write_text("not a completed pyramid")
    with pytest.raises(RuntimeError, match="no PNG tiles"):
        render_indexed_tiles_atomic(idx, lats, lons, outs, luts, [4])
    assert not Path(outs["muted"]).exists()
    assert not list((tmp_path / "radar").rglob(".*.tmp-*"))


def test_atomic_multi_palette_retry_reports_existing_valid(tmp_path):
    idx, luts, lats, lons = _index_frame()
    outs = {p: str(tmp_path / "radar" / p / "2026-07-01T12:07:00+00:00") for p in luts}
    first = render_indexed_tiles_atomic(idx, lats, lons, outs, luts, [4])
    second = render_indexed_tiles_atomic(idx, lats, lons, outs, luts, [4])
    assert second.counts == first.counts
    assert {outcome.status for outcome in second.outcomes.values()} == {
        PublishStatus.EXISTING_VALID,
    }


def test_completion_marker_detects_same_count_path_tamper(tmp_path):
    idx, luts, lats, lons = _index_frame()
    out = tmp_path / "radar" / "classic" / "ts"
    render_indexed_tiles_atomic(
        idx,
        lats,
        lons,
        {"classic": str(out)},
        {"classic": luts["classic"]},
        [4],
    )
    tile = next(out.rglob("*.png"))
    tile.rename(tile.with_name("tampered.png"))
    assert not tiler.is_complete_pyramid(out)
    with pytest.raises(RuntimeError, match="completion path mismatch"):
        render_indexed_tiles_atomic(
            idx,
            lats,
            lons,
            {"classic": str(out)},
            {"classic": luts["classic"]},
            [4],
        )


def test_atomic_partial_publish_converges_on_retry(tmp_path, monkeypatch):
    idx, luts, lats, lons = _index_frame()
    outs = {p: str(tmp_path / "radar" / p / "2026-07-01T12:08:00+00:00") for p in luts}
    real_rename = tiler.os.rename
    calls = 0

    def fail_second_publish(src, dst):
        nonlocal calls
        calls += 1
        if calls == 2:
            raise OSError(errno.EIO, "simulated storage failure", str(dst))
        return real_rename(src, dst)

    monkeypatch.setattr(tiler.os, "rename", fail_second_publish)
    with pytest.raises(OSError, match="simulated storage failure"):
        render_indexed_tiles_atomic(idx, lats, lons, outs, luts, [4])
    assert Path(outs["classic"]).is_dir()
    assert not Path(outs["muted"]).exists()

    monkeypatch.setattr(tiler.os, "rename", real_rename)
    result = render_indexed_tiles_atomic(idx, lats, lons, outs, luts, [4])
    assert result.outcomes["classic"].status is PublishStatus.EXISTING_VALID
    assert result.outcomes["muted"].status is PublishStatus.CREATED
    assert result.counts["classic"] == result.counts["muted"] > 0


def test_atomic_eexist_requires_winner_completion_marker(tmp_path, monkeypatch):
    idx, luts, lats, lons = _index_frame()
    outs = {"classic": str(tmp_path / "radar" / "classic" / "ts")}

    def incomplete_winner(src, dst):
        src_path, dst_path = Path(src), Path(dst)
        winner_tile = next(src_path.rglob("*.png"))
        target = dst_path / winner_tile.relative_to(src_path)
        target.parent.mkdir(parents=True)
        target.write_bytes(winner_tile.read_bytes())
        raise OSError(errno.ENOTEMPTY, "simulated publish race", str(dst_path))

    monkeypatch.setattr(tiler.os, "rename", incomplete_winner)
    with pytest.raises(RuntimeError, match="no completion marker"):
        render_indexed_tiles_atomic(
            idx,
            lats,
            lons,
            outs,
            {"classic": luts["classic"]},
            [4],
        )
    assert not list(Path(outs["classic"]).parent.glob(".*.tmp-*"))


def test_atomic_multi_palette_transparent_frame_publishes_nothing(tmp_path):
    idx, luts, lats, lons = _index_frame()
    outs = {p: str(tmp_path / "radar" / p / "2026-07-01T12:10:00+00:00") for p in luts}
    result = render_indexed_tiles_atomic(
        np.zeros_like(idx), lats, lons, outs, luts, [4]
    )
    assert result.counts == {"classic": 0, "muted": 0}
    assert {outcome.status for outcome in result.outcomes.values()} == {
        PublishStatus.EMPTY
    }
    assert not (tmp_path / "radar").exists() or not list(
        (tmp_path / "radar").rglob("*")
    )


def test_atomic_multi_palette_cleans_all_staging_on_failure(tmp_path, monkeypatch):
    idx, luts, lats, lons = _index_frame()
    outs = {p: str(tmp_path / "radar" / p / "2026-07-01T12:15:00+00:00") for p in luts}
    for out in outs.values():
        Path(out).parent.mkdir(parents=True)

    def boom(idx, lats, lons, output_dirs, *a, **k):
        for d in output_dirs.values():
            Path(d, "4", "3").mkdir(parents=True)
        raise RuntimeError("render died")

    monkeypatch.setattr(tiler, "render_indexed_tiles", boom)
    with pytest.raises(RuntimeError):
        render_indexed_tiles_atomic(idx, lats, lons, outs, luts, [4])
    for out in outs.values():
        assert not Path(out).exists()
        assert not list(Path(out).parent.glob(".*.tmp-*"))
