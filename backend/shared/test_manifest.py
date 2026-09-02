import json

import pytest

from backend.shared.manifest import (
    manifest_path,
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
    # Layer-level palettes are the safe intersection across every frame; each
    # frame retains its exact palette availability.
    assert manifest["layers"]["radar"]["palettes"] == []
    assert manifest["layers"]["radar"]["frames"][0]["palettes"] == ["classic"]
    assert manifest["layers"]["radar"]["frames"][1]["palettes"] == ["vivid"]

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


def test_read_manifest_file_missing_is_empty(tmp_path):
    manifest = read_manifest_file(tmp_path)
    assert manifest["layers"] == {}


def test_read_manifest_file_corrupt_raises_instead_of_wiping(tmp_path):
    update_manifest_file("radar", "2026-05-20T12:00:00+00:00", state_dir=tmp_path)
    manifest_path(tmp_path).write_text('{"layers": {"radar": ')

    with pytest.raises(json.JSONDecodeError):
        read_manifest_file(tmp_path)
    # The writer must see the error too, so a transient bad read never
    # rewrites the file with a single layer.
    with pytest.raises(json.JSONDecodeError):
        update_manifest_file("nowcast", "2026-05-20T12:05:00+00:00", state_dir=tmp_path)
    assert manifest_path(tmp_path).read_text() == '{"layers": {"radar": '


def test_read_manifest_file_unreadable_raises(tmp_path):
    manifest_path(tmp_path).mkdir()

    with pytest.raises(OSError):
        read_manifest_file(tmp_path)
