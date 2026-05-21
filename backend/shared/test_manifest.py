from backend.shared.manifest import read_manifest_file, update_manifest_file


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
