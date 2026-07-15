import json

from backend.tile_cleanup import activities


def test_sweep_removes_old_versioned_runs_but_keeps_manifest_run(monkeypatch, tmp_path):
    tiles = tmp_path / "tiles"
    state = tmp_path / "state"
    old_run = "2000-01-01T00:00:00+00:00"
    current_run = "2999-01-01T00:00:00+00:00"
    for palette in ("classic", "vivid"):
        (tiles / "nowcast" / palette / "runs" / old_run).mkdir(parents=True)
        (tiles / "nowcast" / palette / "runs" / current_run).mkdir(parents=True)
    state.mkdir()
    (state / "manifest.json").write_text(json.dumps({
        "layers": {"nowcast": {"run_id": current_run, "timestamps": []}}
    }))
    monkeypatch.setattr(activities, "TILE_DIR", tiles)
    monkeypatch.setattr(activities, "STATE_DIR", state)

    removed = activities._sweep_layer("nowcast", retention_min=60)

    assert removed == 2
    for palette in ("classic", "vivid"):
        assert not (tiles / "nowcast" / palette / "runs" / old_run).exists()
        assert (tiles / "nowcast" / palette / "runs" / current_run).exists()
