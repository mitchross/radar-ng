//! Manifest invariants MAN-01..MAN-10 (spec §13.1) plus the D7/D19 fixes and a golden replay.

use super::*;
use chrono::TimeZone;
use serde_json::json;

const T0: &str = "2026-05-20T12:00:00+00:00";
const T1: &str = "2026-05-20T12:05:00+00:00";
const T2: &str = "2026-05-20T12:10:00+00:00";

fn now() -> DateTime<Utc> {
    Utc.with_ymd_and_hms(2026, 5, 20, 12, 5, 3).unwrap() + chrono::Duration::microseconds(418221)
}

fn opts_palettes(p: &[&str]) -> AddOptions {
    AddOptions {
        palettes: p.iter().map(|s| s.to_string()).collect(),
        ..Default::default()
    }
}

fn timestamps_of(m: &Manifest, layer: &str) -> Vec<String> {
    m.layers[layer].timestamps.clone()
}

#[test]
fn man_01_02_add_two_frames_orders_and_intersects_palettes() {
    let dir = tempfile::tempdir().unwrap();
    add_frame_at(dir.path(), "radar", T0, &opts_palettes(&["classic"]), now()).unwrap();
    let m = add_frame_at(dir.path(), "radar", T1, &opts_palettes(&["vivid"]), now()).unwrap();
    let layer = &m.layers["radar"];
    assert_eq!(layer.timestamps, vec![T0, T1]);
    assert_eq!(layer.latest.as_deref(), Some(T1));
    assert!(
        layer.palettes.is_empty(),
        "intersection of [classic] and [vivid] is empty"
    );
    assert_eq!(layer.frames[0].palettes, vec!["classic"]);
    assert_eq!(layer.frames[1].palettes, vec!["vivid"]);
    // The file on disk matches what was returned.
    assert_eq!(
        read_manifest(dir.path()).unwrap().layers["radar"].timestamps,
        vec![T0, T1]
    );
}

#[test]
fn man_03_remove_frame_updates_timestamps_and_latest() {
    let dir = tempfile::tempdir().unwrap();
    add_frame_at(dir.path(), "radar", T0, &AddOptions::default(), now()).unwrap();
    add_frame_at(dir.path(), "radar", T1, &AddOptions::default(), now()).unwrap();
    let m = remove_frame_at(dir.path(), "radar", T1, now()).unwrap();
    assert_eq!(timestamps_of(&m, "radar"), vec![T0]);
    assert_eq!(m.layers["radar"].latest.as_deref(), Some(T0));
}

#[test]
fn man_04_replace_swaps_list_sorts_and_keeps_other_layers() {
    let dir = tempfile::tempdir().unwrap();
    add_frame_at(
        dir.path(),
        "nowcast",
        T1,
        &opts_palettes(&["classic"]),
        now(),
    )
    .unwrap();
    add_frame_at(
        dir.path(),
        "nowcast",
        T2,
        &opts_palettes(&["classic"]),
        now(),
    )
    .unwrap();
    add_frame_at(dir.path(), "radar", T0, &opts_palettes(&["classic"]), now()).unwrap();
    let wanted = vec![
        "2026-05-20T12:17:00+00:00".to_string(),
        "2026-05-20T12:12:00+00:00".to_string(),
    ];
    let opts = ReplaceOptions {
        palettes: vec!["vivid".into(), "classic".into()],
        ..Default::default()
    };
    let m = replace_layer_at(dir.path(), "nowcast", &wanted, &opts, now()).unwrap();
    let nc = &m.layers["nowcast"];
    assert_eq!(
        nc.timestamps,
        vec!["2026-05-20T12:12:00+00:00", "2026-05-20T12:17:00+00:00"]
    );
    assert_eq!(nc.latest.as_deref(), Some("2026-05-20T12:17:00+00:00"));
    assert_eq!(nc.palettes, vec!["classic", "vivid"]);
    assert_eq!(m.layers["radar"].timestamps, vec![T0]);
}

#[test]
fn man_05_replace_with_empty_list_drops_layer() {
    let dir = tempfile::tempdir().unwrap();
    add_frame_at(dir.path(), "nowcast", T1, &AddOptions::default(), now()).unwrap();
    let m = replace_layer_at(
        dir.path(),
        "nowcast",
        &[],
        &ReplaceOptions::default(),
        now(),
    )
    .unwrap();
    assert!(!m.layers.contains_key("nowcast"));
    assert!(
        !read_manifest(dir.path())
            .unwrap()
            .layers
            .contains_key("nowcast")
    );
}

#[test]
fn the_dropped_legacy_template_is_stripped_from_older_files() {
    let m = parse_manifest_value(
        json!({
            "layers": {},
            "tile_url_template_legacy": "/tiles/{layer}/{timestamp}/{z}/{x}/{y}.png",
        }),
        now(),
    )
    .unwrap();
    assert!(m.extra.is_empty());
    let out = serde_json::to_value(&m).unwrap();
    assert!(out.get("tile_url_template_legacy").is_none());
    assert_eq!(out["tile_url_template"], TILE_URL_TEMPLATE);
}

#[test]
fn man_06_missing_file_reads_as_empty() {
    let dir = tempfile::tempdir().unwrap();
    let m = read_manifest(dir.path()).unwrap();
    assert!(m.layers.is_empty());
    assert_eq!(m.schema_version, 2);
}

#[test]
fn man_07_corrupt_json_fails_read_and_write_and_keeps_bytes() {
    let dir = tempfile::tempdir().unwrap();
    add_frame_at(dir.path(), "radar", T0, &AddOptions::default(), now()).unwrap();
    let path = manifest_path(dir.path());
    std::fs::write(&path, r#"{"layers": {"radar": "#).unwrap();
    assert!(matches!(
        read_manifest(dir.path()),
        Err(ManifestError::Parse(_))
    ));
    let err = add_frame_at(dir.path(), "nowcast", T1, &AddOptions::default(), now()).unwrap_err();
    assert!(matches!(err, ManifestError::Parse(_)), "{err:?}");
    assert_eq!(
        std::fs::read_to_string(&path).unwrap(),
        r#"{"layers": {"radar": "#
    );
}

#[test]
fn man_08_directory_where_file_should_be_is_io_error() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::create_dir(manifest_path(dir.path())).unwrap();
    assert!(matches!(
        read_manifest(dir.path()),
        Err(ManifestError::Io(_))
    ));
}

#[test]
fn man_09_d7_structurally_invalid_manifest_fails_closed_for_writers() {
    let dir = tempfile::tempdir().unwrap();
    let path = manifest_path(dir.path());
    // Valid JSON but no layers object: the Python version silently became empty and wiped on write.
    for bad in [
        r#"[]"#,
        r#"{"schema_version": 2}"#,
        r#"{"layers": []}"#,
        r#"{"layers": {"radar": 3}}"#,
    ] {
        std::fs::write(&path, bad).unwrap();
        assert!(
            matches!(read_manifest(dir.path()), Err(ManifestError::Malformed(_))),
            "{bad}"
        );
        assert!(
            matches!(
                add_frame_at(dir.path(), "radar", T0, &AddOptions::default(), now()),
                Err(ManifestError::Malformed(_))
            ),
            "{bad}"
        );
        assert!(
            matches!(
                remove_frame_at(dir.path(), "radar", T0, now()),
                Err(ManifestError::Malformed(_))
            ),
            "{bad}"
        );
        assert_eq!(
            std::fs::read_to_string(&path).unwrap(),
            bad,
            "file must not be rewritten"
        );
    }
}

#[test]
fn man_10_v1_document_migrates_timestamps_to_frames_with_path_equal_timestamp() {
    let v1 = json!({
        "layers": {"radar": {"timestamps": [T0, T1], "palettes": ["classic", "vivid"], "title": "MRMS"}},
        "updated_at": "2026-05-20T12:00:00+00:00"
    });
    let m = parse_manifest_value(v1, now()).unwrap();
    assert_eq!(m.schema_version, 1, "missing schema_version reads as 1");
    let frames = &m.layers["radar"].frames;
    assert_eq!(frames.len(), 2);
    assert_eq!(frames[0].path, T0);
    assert_eq!(frames[1].timestamp, T1);
    assert_eq!(frames[1].palettes, vec!["classic", "vivid"]);
}

#[test]
fn d19_non_utc_timestamps_are_rejected_and_z_is_normalized() {
    let dir = tempfile::tempdir().unwrap();
    let err = add_frame_at(
        dir.path(),
        "radar",
        "2026-05-20T07:00:00-05:00",
        &AddOptions::default(),
        now(),
    )
    .unwrap_err();
    assert!(matches!(err, ManifestError::InvalidTimestamp { .. }));
    assert!(
        !manifest_path(dir.path()).exists(),
        "nothing written for a rejected timestamp"
    );
    let m = add_frame_at(
        dir.path(),
        "radar",
        "2026-05-20T12:00:00Z",
        &AddOptions::default(),
        now(),
    )
    .unwrap();
    assert_eq!(m.layers["radar"].timestamps, vec![T0]);
}

#[test]
fn explicit_palettes_override_frame_palettes_and_empty_falls_through() {
    let dir = tempfile::tempdir().unwrap();
    let frame = FrameSpec {
        palettes: vec!["vivid".into()],
        ..Default::default()
    };
    let m = add_frame_at(
        dir.path(),
        "radar",
        T0,
        &AddOptions {
            palettes: vec![],
            frame: Some(frame.clone()),
            layer_metadata: None,
        },
        now(),
    )
    .unwrap();
    assert_eq!(
        m.layers["radar"].frames[0].palettes,
        vec!["vivid"],
        "empty explicit list falls through"
    );
    let m = add_frame_at(
        dir.path(),
        "radar",
        T0,
        &AddOptions {
            palettes: vec!["classic".into()],
            frame: Some(frame),
            layer_metadata: None,
        },
        now(),
    )
    .unwrap();
    assert_eq!(m.layers["radar"].frames[0].palettes, vec!["classic"]);
}

#[test]
fn default_palette_is_classic_and_default_path_is_timestamp() {
    let dir = tempfile::tempdir().unwrap();
    let m = add_frame_at(dir.path(), "radar", T0, &AddOptions::default(), now()).unwrap();
    assert_eq!(m.layers["radar"].frames[0].palettes, vec!["classic"]);
    assert_eq!(m.layers["radar"].frames[0].path, T0);
}

#[test]
fn layer_metadata_is_merged_and_derived_keys_are_ignored() {
    let dir = tempfile::tempdir().unwrap();
    let mut meta = Map::new();
    meta.insert("title".into(), json!("MRMS observed reflectivity"));
    meta.insert("kind".into(), json!("observation"));
    meta.insert("latest".into(), json!("1999-01-01T00:00:00+00:00"));
    let opts = AddOptions {
        layer_metadata: Some(meta),
        ..Default::default()
    };
    let m = add_frame_at(dir.path(), "radar", T0, &opts, now()).unwrap();
    let layer = &m.layers["radar"];
    assert_eq!(layer.extra["title"], json!("MRMS observed reflectivity"));
    assert_eq!(layer.latest.as_deref(), Some(T0));
}

#[test]
fn render_is_compact_sorted_with_trailing_newline_and_round_trips() {
    let dir = tempfile::tempdir().unwrap();
    add_frame_at(dir.path(), "radar", T0, &AddOptions::default(), now()).unwrap();
    let text = std::fs::read_to_string(manifest_path(dir.path())).unwrap();
    assert!(text.ends_with("}\n"));
    assert!(!text.contains(": "), "compact separators");
    assert!(
        text.starts_with(r#"{"layers":{"radar":{"frames":"#),
        "sorted keys: {text}"
    );
    let parsed = parse_manifest(&text).unwrap();
    assert_eq!(parsed.layers["radar"].timestamps, vec![T0]);
}

#[test]
fn golden_replay_matches_python_bytes() {
    // The same operations run through backend/shared/manifest.py with `datetime` pinned to `now()`.
    // The input is a v1 document (no frames); the output is byte-for-byte the Python file.
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(
        manifest_path(dir.path()),
        include_str!("../../testdata/manifest/v1_input.json"),
    )
    .unwrap();
    add_frame_at(
        dir.path(),
        "radar",
        T1,
        &AddOptions {
            palettes: vec!["classic".into(), "vivid".into()],
            frame: Some(FrameSpec {
                extra: json!({"source": "mrms", "kind": "observation", "issued_at": T1, "lead_minutes": 0,
                              "spatial_resolution_km": 1.0, "max_zoom": 7})
                .as_object()
                .unwrap()
                .clone(),
                ..Default::default()
            }),
            layer_metadata: Some(json!({"title": "MRMS observed reflectivity", "kind": "observation"}).as_object().unwrap().clone()),
        },
        now(),
    )
    .unwrap();
    let nowcast_frames = vec![
        FrameSpec {
            timestamp: T2.into(),
            path: Some("runs/2026-05-20T12:00:00+00:00/2026-05-20T12:10:00+00:00".into()),
            palettes: vec![],
            extra: json!({"grid_key": "runs/2026-05-20T12:00:00+00:00/2026-05-20T12:10:00+00:00",
                          "source": "mrms-nowcast", "kind": "nowcast", "issued_at": "2026-05-20T12:00:00+00:00",
                          "lead_minutes": 10, "input_interval_minutes": 2.0, "method": "pysteps-sprog",
                          "spatial_resolution_km": 1.0, "max_zoom": 6})
            .as_object()
            .unwrap()
            .clone(),
        },
    ];
    replace_layer_at(
        dir.path(),
        "nowcast",
        &[T2.to_string()],
        &ReplaceOptions {
            palettes: vec!["classic".into()],
            frames: nowcast_frames,
            layer_metadata: Some(
                json!({"horizon_minutes": 60, "kind": "nowcast", "method": "pysteps-sprog",
                                        "run_id": "2026-05-20T12:00:00+00:00", "step_minutes": 5,
                                        "title": "MRMS motion nowcast"})
                .as_object()
                .unwrap()
                .clone(),
            ),
        },
        now(),
    )
    .unwrap();
    remove_frame_at(dir.path(), "radar", "2026-05-20T11:55:00+00:00", now()).unwrap();
    let got = std::fs::read_to_string(manifest_path(dir.path())).unwrap();
    assert_eq!(
        got,
        include_str!("../../testdata/manifest/v1_migrated_final.json")
    );
}
