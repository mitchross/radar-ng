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
        None,
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
    let published = publish_run(dir.path(), None, "20260715_12", &results, &pals, 2).unwrap();
    assert_eq!(published, vec!["radar-hrrr".to_string()]);
    let m = read_manifest(dir.path()).unwrap();
    let layer = &m.layers["radar-hrrr"];
    assert_eq!(layer.extra["complete"], true);
    assert_eq!(layer.extra["palette_key"], "reflectivity");
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
    publish_run(dir.path(), None, "20260715_12", &first, &pals, 4).unwrap();
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
    publish_run(dir.path(), None, "20260715_12", &filled, &pals, 4).unwrap();
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
    publish_run(dir.path(), None, "20260716_12", &twelve, &pals, 48).unwrap();

    let thirteen: Vec<ForecastHourResult> = (1..=18).map(|f| hour(f, &["radar-hrrr"])).collect();
    publish_run(dir.path(), None, "20260716_13", &thirteen, &pals, 18).unwrap();
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
    publish_run(dir.path(), None, "20260717_01", &oh_one, &pals, 18).unwrap();
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

/// Packed buckets follow the frames: the 13z run lists its own bucket plus the slots of the 12z
/// extended tail; slots the 13z run replaced are blank. Publishing without packing drops them.
#[test]
fn packed_buckets_follow_the_published_frames_and_the_extended_tail() {
    use radar_core::manifest::packed::packed_buckets;
    use radar_ingest::hrrr::packed::pack_hour;
    use radar_ingest::hrrr::{run_init, valid_time};

    let dir = tempfile::tempdir().unwrap();
    let paths = radar_core::env::Paths::new(
        dir.path().join("state"),
        dir.path().join("grids"),
        dir.path().join("tiles"),
    );
    let lats: Vec<f64> = (0..50).map(|i| 35.0 + 0.04 * f64::from(i)).collect();
    let lons: Vec<f64> = (0..50).map(|i| -98.0 + 0.04 * f64::from(i)).collect();
    let grid = radar_geo::Grid {
        height: 50,
        width: 50,
        values: vec![40.0; 2500],
        geo: radar_geo::GridGeo::Regular { lats, lons },
    };
    let pals = load_hrrr_palettes(&["classic".into()]);
    let state = &paths.state_dir;
    for (run, hours) in [("20260716_12", 48), ("20260716_13", 18)] {
        let init = run_init(run).unwrap();
        for fhr in 1..=hours {
            pack_hour(&paths, "radar-hrrr", run, &valid_time(init, fhr), &grid).unwrap();
        }
        let results: Vec<ForecastHourResult> = (1..=i64::from(hours))
            .map(|f| hour(f, &["radar-hrrr"]))
            .collect();
        publish_run(state, Some(&paths), run, &results, &pals, hours).unwrap();
    }
    let m = read_manifest(state).unwrap();
    let buckets = packed_buckets(&m, "radar-hrrr");
    let ids: Vec<&str> = buckets.iter().map(|b| b.id.as_str()).collect();
    assert_eq!(ids, ["20260716_12", "20260716_12.p1", "20260716_13"]);
    let shown = |i: usize| buckets[i].slots.iter().filter(|s| !s.is_empty()).count();
    // 12z f20 to f24 (after 13z's last hour, f19 of 12z), all of 12z f25 to f48, all of 13z.
    assert_eq!((shown(0), shown(1), shown(2)), (5, 24, 18));
    let layer = &m.layers["radar-hrrr"];
    for frame in &layer.frames {
        let hits = buckets
            .iter()
            .filter(|b| b.slots.contains(&frame.timestamp))
            .count();
        assert_eq!(hits, 1, "{} is in exactly one bucket", frame.timestamp);
    }

    publish_run(
        state,
        None,
        "20260716_13",
        &(1..=18)
            .map(|f| hour(f, &["radar-hrrr"]))
            .collect::<Vec<_>>(),
        &pals,
        18,
    )
    .unwrap();
    let m = read_manifest(state).unwrap();
    assert!(packed_buckets(&m, "radar-hrrr").is_empty());
}
