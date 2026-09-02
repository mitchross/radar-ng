import json

from backend.ingest_lightning import activities


def test_write_geojson_is_atomic_and_leaves_no_temp_files(tmp_path, monkeypatch):
    out = tmp_path / "state" / "lightning.json"
    monkeypatch.setattr(activities, "STATE_DIR", out.parent)
    monkeypatch.setattr(activities, "OUT_PATH", out)

    strikes = [{"t": 1.0, "lat": 40.0, "lon": -100.0, "pol": 1, "mds": 3}]
    activities._write_geojson(strikes)
    activities._write_geojson(iter(strikes))  # any iterable snapshot works

    body = json.loads(out.read_text())
    assert body["type"] == "FeatureCollection"
    assert body["features"][0]["geometry"]["coordinates"] == [-100.0, 40.0]
    assert [p.name for p in out.parent.iterdir()] == ["lightning.json"]
