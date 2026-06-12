from backend.shared.manifest import (
    read_manifest_file,
    replace_layer_manifest,
    update_manifest_file,
)


def test_update_manifest_file_adds_and_removes_timestamp(tmp_path):
    update_manifest_file("radar", "2026-05-20T12:00:00+00:00", palettes=["classic"], state_dir=tmp_path)
    update_manifest_file("radar", "2026-05-20T12:05:00+00:00", palettes=["vivid"], state_dir=tmp_path)

    manifest = read_manifest_file(tmp_path)
    assert manifest["layers"]["radar"]["timestamps"] == [
        "2026-05-20T12:00:00+00:00",
        "2026-05-20T12:05:00+00:00",
    ]
    assert manifest["layers"]["radar"]["latest"] == "2026-05-20T12:05:00+00:00"
    assert manifest["layers"]["radar"]["palettes"] == ["classic", "vivid"]

    update_manifest_file("radar", "2026-05-20T12:05:00+00:00", action="remove", state_dir=tmp_path)

    manifest = read_manifest_file(tmp_path)
    assert manifest["layers"]["radar"]["timestamps"] == ["2026-05-20T12:00:00+00:00"]
    assert manifest["layers"]["radar"]["latest"] == "2026-05-20T12:00:00+00:00"


def test_replace_layer_manifest_swaps_all_timestamps(tmp_path):
    # An older nowcast run published two frames…
    update_manifest_file("nowcast", "2026-05-20T12:05:00+00:00", palettes=["classic"], state_dir=tmp_path)
    update_manifest_file("nowcast", "2026-05-20T12:10:00+00:00", palettes=["classic"], state_dir=tmp_path)
    # …other layers must be untouched by the swap.
    update_manifest_file("radar", "2026-05-20T12:00:00+00:00", palettes=["classic"], state_dir=tmp_path)

    replace_layer_manifest(
        "nowcast",
        ["2026-05-20T12:12:00+00:00", "2026-05-20T12:17:00+00:00"],
        palettes=["classic", "vivid"],
        state_dir=tmp_path,
    )

    manifest = read_manifest_file(tmp_path)
    assert manifest["layers"]["nowcast"]["timestamps"] == [
        "2026-05-20T12:12:00+00:00",
        "2026-05-20T12:17:00+00:00",
    ]
    assert manifest["layers"]["nowcast"]["latest"] == "2026-05-20T12:17:00+00:00"
    assert manifest["layers"]["nowcast"]["palettes"] == ["classic", "vivid"]
    assert manifest["layers"]["radar"]["timestamps"] == ["2026-05-20T12:00:00+00:00"]


def test_replace_layer_manifest_empty_list_drops_layer(tmp_path):
    update_manifest_file("nowcast", "2026-05-20T12:05:00+00:00", state_dir=tmp_path)

    replace_layer_manifest("nowcast", [], state_dir=tmp_path)

    manifest = read_manifest_file(tmp_path)
    assert "nowcast" not in manifest["layers"]
