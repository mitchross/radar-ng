//! HRRR coherent publication invariants (spec §11.3, §14.2 H5, H6, H7, H21). Manifest only; no tiles.

use radar_core::manifest::read_manifest;
use radar_ingest::hrrr::publish::publish_run;
use radar_ingest::hrrr::render::load_hrrr_palettes;
use radar_ingest::hrrr::state::referenced_runs;
use radar_types::payloads::hrrr::ForecastHourResult;

fn hour(fhr: i64, layers: &[&str]) -> ForecastHourResult {
    ForecastHourResult {
        fhr,
        rendered_layers: layers.iter().map(|s| s.to_string()).collect(),
        duration_s: 0.0,
        valid_timestamp: String::new(),
    }
}

fn lead_minutes(layer: &radar_core::manifest::Layer) -> Vec<i64> {
    layer
        .frames
        .iter()
        .map(|f| f.extra["lead_minutes"].as_i64().unwrap())
        .collect()
}

/// H5: only f02 rendered means no publication and no manifest file.
#[test]
fn h5_no_f01_publishes_nothing_and_writes_no_manifest() {
    let dir = tempfile::tempdir().unwrap();
    let pals = load_hrrr_palettes(&["classic".into()]);
    let published = publish_run(
        dir.path(),
        "20260715_12",
        &[hour(2, &["radar-hrrr"])],
        &pals,
        18,
    )
    .unwrap();
    assert!(published.is_empty());
    assert!(!dir.path().join("manifest.json").exists());
}

/// H6: a complete two-hour run publishes radar-hrrr with complete true and the run-scoped paths.
#[test]
fn h6_complete_run_publishes_run_scoped_frames() {
    let dir = tempfile::tempdir().unwrap();
    let pals = load_hrrr_palettes(&["classic".into()]);
    let results = [hour(1, &["radar-hrrr"]), hour(2, &["radar-hrrr"])];
    let published = publish_run(dir.path(), "20260715_12", &results, &pals, 2).unwrap();
    assert_eq!(published, vec!["radar-hrrr".to_string()]);
    let m = read_manifest(dir.path()).unwrap();
    let layer = &m.layers["radar-hrrr"];
    assert_eq!(layer.extra["complete"], true);
    assert_eq!(layer.extra["run_id"], "20260715_12");
    let paths: Vec<&str> = layer.frames.iter().map(|f| f.path.as_str()).collect();
    assert_eq!(
        paths,
        vec![
            "runs/20260715_12/2026-07-15T13:00:00+00:00",
            "runs/20260715_12/2026-07-15T14:00:00+00:00",
        ]
    );
}

/// H7: a partial run advertises the coherent prefix with complete false, then extends when filled.
#[test]
fn h7_partial_then_extended_publication() {
    let dir = tempfile::tempdir().unwrap();
    let pals = load_hrrr_palettes(&["classic".into()]);
    let first = [
        hour(1, &["radar-hrrr"]),
        hour(2, &["radar-hrrr"]),
        hour(3, &[]),
        hour(4, &["radar-hrrr"]),
    ];
    publish_run(dir.path(), "20260715_12", &first, &pals, 4).unwrap();
    let m = read_manifest(dir.path()).unwrap();
    assert_eq!(lead_minutes(&m.layers["radar-hrrr"]), vec![60, 120]);
    assert_eq!(m.layers["radar-hrrr"].extra["complete"], false);
    assert_eq!(m.layers["radar-hrrr"].extra["run_id"], "20260715_12");

    let filled = [
        hour(1, &["radar-hrrr"]),
        hour(2, &["radar-hrrr"]),
        hour(3, &["radar-hrrr"]),
        hour(4, &["radar-hrrr"]),
    ];
    publish_run(dir.path(), "20260715_12", &filled, &pals, 4).unwrap();
    let m = read_manifest(dir.path()).unwrap();
    assert_eq!(
        lead_minutes(&m.layers["radar-hrrr"]),
        vec![60, 120, 180, 240]
    );
    assert_eq!(m.layers["radar-hrrr"].extra["complete"], true);
}

/// H21: a 13z run keeps the 12z extended tail after its last valid time; a later 01z run drops it.
#[test]
fn h21_extended_tail_carry_and_drop() {
    let dir = tempfile::tempdir().unwrap();
    let pals = load_hrrr_palettes(&["classic".into()]);
    let twelve: Vec<ForecastHourResult> = (1..=48).map(|f| hour(f, &["radar-hrrr"])).collect();
    publish_run(dir.path(), "20260716_12", &twelve, &pals, 48).unwrap();

    let thirteen: Vec<ForecastHourResult> = (1..=18).map(|f| hour(f, &["radar-hrrr"])).collect();
    publish_run(dir.path(), "20260716_13", &thirteen, &pals, 18).unwrap();
    let m = read_manifest(dir.path()).unwrap();
    let layer = &m.layers["radar-hrrr"];
    assert_eq!(layer.frames.len(), 18 + 29);
    assert!(
        layer.frames[..18]
            .iter()
            .all(|f| f.extra["run_id"] == "20260716_13")
    );
    assert!(
        layer.frames[18..]
            .iter()
            .all(|f| f.extra["run_id"] == "20260716_12")
    );
    assert_eq!(
        layer.timestamps.last().unwrap(),
        "2026-07-18T12:00:00+00:00"
    );
    assert_eq!(layer.extra["run_id"], "20260716_13");
    let runs = referenced_runs(Some(layer));
    assert_eq!(
        runs.into_iter().collect::<Vec<_>>(),
        vec!["20260716_12".to_string(), "20260716_13".to_string()]
    );

    let oh_one: Vec<ForecastHourResult> = (1..=18).map(|f| hour(f, &["radar-hrrr"])).collect();
    publish_run(dir.path(), "20260717_01", &oh_one, &pals, 18).unwrap();
    let m = read_manifest(dir.path()).unwrap();
    let layer = &m.layers["radar-hrrr"];
    assert_eq!(layer.frames.len(), 18);
    assert!(
        layer
            .frames
            .iter()
            .all(|f| f.extra["run_id"] == "20260717_01")
    );
}
