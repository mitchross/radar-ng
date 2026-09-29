import json
from pathlib import Path

import numpy as np

from backend.shared.storm_prefetch import _tiles_for_bbox, build_storm_prefetch_plan
from backend.shared.storms import detect_storms, write_storms_json


def _grid(col_start: int) -> tuple[np.ndarray, np.ndarray, np.ndarray]:
    data = np.zeros((20, 20), dtype=np.float32)
    data[7:10, col_start:col_start + 3] = 55.0
    lats = np.linspace(43.0, 42.81, 20)
    lons = np.linspace(-85.0, -84.81, 20)
    return data, lats, lons


def test_detect_storms_tracks_cell_and_emits_three_predicted_bboxes():
    first_data, lats, lons = _grid(7)
    first = detect_storms(
        first_data,
        lats,
        lons,
        timestamp="2026-07-10T12:00:00+00:00",
    )
    second_data, _, _ = _grid(8)
    second = detect_storms(
        second_data,
        lats,
        lons,
        timestamp="2026-07-10T12:05:00+00:00",
        previous=first,
    )

    first_props = first["features"][0]["properties"]
    second_props = second["features"][0]["properties"]
    assert second_props["cell_id"] == first_props["cell_id"]
    assert second_props["tracking_vector"]["east_kmh"] > 0
    assert [item["lead_minutes"] for item in second_props["predicted_bboxes"]] == [0, 5, 10]
    assert second_props["predicted_bboxes"][2]["bbox"][0] > second_props["predicted_bboxes"][0]["bbox"][0]
    assert len(second["tracking_vectors"]) == 1


def test_write_storms_json_does_not_regress_to_backfilled_frame(tmp_path: Path):
    data, lats, lons = _grid(7)
    write_storms_json(tmp_path, data, lats, lons, "2026-07-10T12:05:00+00:00")
    write_storms_json(tmp_path, data, lats, lons, "2026-07-10T12:00:00+00:00")

    payload = json.loads((tmp_path / "storms.json").read_text())
    assert payload["timestamp"] == "2026-07-10T12:05:00+00:00"


def test_prefetch_plan_has_exactly_three_bboxes_and_existing_tile_urls(tmp_path: Path):
    state_dir = tmp_path / "state"
    tile_dir = tmp_path / "tiles"
    state_dir.mkdir()
    data, lats, lons = _grid(7)
    storms = detect_storms(data, lats, lons, timestamp="2026-07-10T12:00:00+00:00")
    manifest = {
        "layers": {
            "radar": {"timestamps": ["2026-07-10T12:00:00+00:00"]},
            "nowcast": {"timestamps": [
                "2026-07-10T12:05:00+00:00",
                "2026-07-10T12:10:00+00:00",
            ]},
        },
        "tile_url_template": "/tiles/{layer}/{palette}/{timestamp}/{z}/{x}/{y}.png",
        "updated_at": "2026-07-10T12:00:00+00:00",
    }
    (state_dir / "manifest.json").write_text(json.dumps(manifest))

    predictions = storms["features"][0]["properties"]["predicted_bboxes"]
    frames = [
        ("radar", "2026-07-10T12:00:00+00:00"),
        ("nowcast", "2026-07-10T12:05:00+00:00"),
        ("nowcast", "2026-07-10T12:10:00+00:00"),
    ]
    for prediction, (layer, timestamp) in zip(predictions, frames):
        for x, y in _tiles_for_bbox(prediction["bbox"], 6):
            path = tile_dir / layer / "classic" / timestamp / "6" / str(x) / f"{y}.png"
            path.parent.mkdir(parents=True, exist_ok=True)
            path.write_bytes(b"png")

    plan = build_storm_prefetch_plan(
        storms=storms,
        state_dir=state_dir,
        tile_dir=tile_dir,
        base_url="https://radar.example",
        lat=42.9,
        lon=-84.9,
        zoom=6,
        palette="classic",
    )

    assert len(plan["bboxes"]) == 3
    assert [bbox["lead_minutes"] for bbox in plan["bboxes"]] == [0, 5, 10]
    assert plan["tile_urls"]
    assert all(url.startswith("https://radar.example/tiles/") for url in plan["tile_urls"])
    assert all(bbox["style_url"] for bbox in plan["bboxes"])


def _cell_frame(lon_center: float, lat_center: float = 42.0, size: int = 5):
    lats = 41.0 + 0.01 * np.arange(200)
    lons = -86.0 + 0.01 * np.arange(300)
    data = np.zeros((200, 300), dtype=np.float32)
    r = int(round((lat_center - 41.0) / 0.01))
    c = int(round((lon_center + 86.0) / 0.01))
    data[r:r + size, c:c + size] = 55.0
    return data, lats, lons


def test_motion_is_fitted_over_history_not_one_noisy_step():
    # 60 km/h due east with ±1 cell centroid wobble, sampled every 2 minutes.
    km_per_deg = 111.320 * np.cos(np.radians(42.0))
    wobble = [0.0, 0.01, -0.01, 0.01, 0.0, -0.01, 0.01, 0.0]
    previous = None
    for step, jitter in enumerate(wobble):
        lon = -85.5 + (60.0 * step * 2 / 60.0) / km_per_deg + jitter
        data, lats, lons = _cell_frame(lon)
        previous = detect_storms(data, lats, lons, timestamp=f"2026-07-10T12:{step * 2:02d}:00+00:00", previous=previous)
    props = previous["features"][0]["properties"]
    assert len(props["track_history"]) == len(wobble)
    assert 45 < props["tracking_vector"]["speed_kmh"] < 75
    assert 75 < props["tracking_vector"]["bearing_deg"] < 105
    assert props["tracking_confidence"] > 0.5


def test_new_cells_report_no_motion_until_history_spans_enough_time():
    data, lats, lons = _cell_frame(-85.5)
    first = detect_storms(data, lats, lons, timestamp="2026-07-10T12:00:00+00:00")
    data, _, _ = _cell_frame(-85.48)
    second = detect_storms(data, lats, lons, timestamp="2026-07-10T12:02:00+00:00", previous=first)
    props = second["features"][0]["properties"]
    assert props["cell_id"] == first["features"][0]["properties"]["cell_id"]
    assert props["tracking_vector"]["speed_kmh"] == 0.0 and props["tracking_confidence"] == 0.0


def test_distant_cell_is_not_associated_as_impossible_motion():
    data, lats, lons = _cell_frame(-85.5)
    first = detect_storms(data, lats, lons, timestamp="2026-07-10T12:00:00+00:00")
    data, _, _ = _cell_frame(-85.3)  # ~16 km in 2 minutes
    second = detect_storms(data, lats, lons, timestamp="2026-07-10T12:02:00+00:00", previous=first)
    assert second["features"][0]["properties"]["cell_id"] != first["features"][0]["properties"]["cell_id"]


def test_area_uses_the_source_grid_cell_size():
    data, lats, lons = _cell_frame(-85.5, size=10)
    props = detect_storms(data, lats, lons, timestamp="2026-07-10T12:00:00+00:00")["features"][0]["properties"]
    expected = 100 * (0.01 * 110.574) * (0.01 * 111.320 * np.cos(np.radians(42.05)))
    assert abs(props["area_km2"] - expected) / expected < 0.05
