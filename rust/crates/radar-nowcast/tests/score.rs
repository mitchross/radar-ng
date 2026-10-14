#![allow(clippy::needless_range_loop)]
//! Verification matching (spec-nowcast §10.2, §13 item 24) on hand-built run directories.

use std::path::Path;

use chrono::{TimeDelta, TimeZone, Utc};
use radar_core::env::Paths;
use radar_core::grid_dump::{GridConfig, GridInput, write_grid};
use radar_core::time::{isoformat_utc, parse_iso_as_utc};
use radar_nowcast::config::NowcastConfig;
use radar_nowcast::field::Field2;
use radar_nowcast::inputs::linspace;
use radar_nowcast::score::score_observation;
use serde_json::Value;

const H: usize = 40;
const W: usize = 60;

fn ts(minute: i64) -> String {
    let base = Utc.with_ymd_and_hms(2026, 7, 15, 16, 0, 0).unwrap();
    isoformat_utc(&(base + TimeDelta::minutes(minute)))
}

fn axes() -> (Vec<f64>, Vec<f64>) {
    (linspace(54.0, 20.0, H), linspace(-130.0, -60.0, W))
}

/// A run-scoped point grid at `runs/<anchor>/<key>` with `value` everywhere except the no-coverage top row.
fn put_point_grid(cfg: &NowcastConfig, anchor: &str, key: &str, value: f32) {
    let (lats, lons) = axes();
    let mut data = vec![value; H * W];
    for x in 0..W {
        data[x] = -9999.0;
    }
    write_grid(
        &cfg.grid,
        "nowcast",
        key,
        &GridInput {
            data: &data,
            lats: &lats,
            lons: &lons,
            unit: "dBZ",
            fill: -9999.0,
            max_cells: None,
            grid_key: Some(&format!("runs/{anchor}/{key}")),
        },
    )
    .unwrap()
    .unwrap();
}

fn observed() -> Field2<f32> {
    let mut v = vec![30.0f32; H * W];
    for x in 0..W {
        v[x] = -9999.0;
    }
    Field2::new(W, H, v).unwrap()
}

fn config(root: &Path) -> NowcastConfig {
    let paths = Paths::new(root.join("state"), root.join("grids"), root.join("tiles"));
    let mut cfg = NowcastConfig::with_paths(paths);
    cfg.grid = GridConfig::with_dir(root.join("grids"));
    cfg
}

fn score(cfg: &NowcastConfig, at: i64) -> Vec<serde_json::Map<String, Value>> {
    let (lats, lons) = axes();
    let now = parse_iso_as_utc(&ts(at + 1)).unwrap();
    score_observation(cfg, &observed(), &ts(at), &lats, &lons, now).unwrap()
}

/// Run A (anchor 0) forecast leads 5 and 10 at 30 dBZ and stored an observed grid. Run B (anchor 20) is later
/// than every observation used here and must be skipped. Run C (anchor -10) has no observed baseline.
fn setup(cfg: &NowcastConfig) {
    let a = ts(0);
    put_point_grid(cfg, &a, &ts(5), 30.0);
    put_point_grid(cfg, &a, &ts(10), 30.0);
    put_point_grid(cfg, &a, "observed", 30.0);
    put_point_grid(cfg, &ts(20), &ts(25), 30.0);
    put_point_grid(cfg, &ts(-10), &ts(-5), 30.0);
}

/// §13 item 24: a valid time is matched within 150 s. The observations at 16:04 and 16:06 both match the 16:05
/// forecast (D3). Observation 16:08 matches the 16:10 forecast (120 s). Observation 16:15 matches nothing.
/// Later anchors are skipped.
#[test]
fn matches_within_tolerance_and_counts_each_observation() {
    let dir = tempfile::tempdir().unwrap();
    let cfg = config(dir.path());
    setup(&cfg);

    let at_4 = score(&cfg, 4);
    assert_eq!(
        at_4.len(),
        1,
        "run A matched; run C is out of its window: {at_4:?}"
    );
    let e = &at_4[0];
    assert_eq!(e["anchor"], Value::from(ts(0)));
    assert_eq!(e["valid"], Value::from(ts(4)));
    assert_eq!(e["lead_minutes"], Value::from(5));

    // Forecast 30 dBZ everywhere except the top row, observed 30 dBZ likewise: all cells are hits.
    let cells = ((H - 1) * W) as i64;
    assert_eq!(e["cells"], Value::from(cells));
    assert_eq!(e["thresholds"]["rain"]["hits"], Value::from(cells));
    assert_eq!(e["thresholds"]["rain"]["misses"], Value::from(0));
    assert_eq!(e["thresholds"]["heavy"]["false_alarms"], Value::from(0));
    // The run's own observed grid is the baseline: same as the new observation here.
    assert_eq!(e["persistence"]["rain"]["hits"], Value::from(cells));

    let at_6 = score(&cfg, 6);
    assert_eq!(
        at_6.len(),
        1,
        "D3: a second observation for the same valid time is also scored"
    );
    assert_eq!(at_6[0]["valid"], Value::from(ts(6)));

    // 16:08 is 120 s before the 16:10 forecast: inside the window, so lead 10 matches.
    let at_8 = score(&cfg, 8);
    assert_eq!(at_8.len(), 1);
    assert_eq!(at_8[0]["lead_minutes"], Value::from(10));

    // 16:15 is 300 s from both forecast valid times and before run B's anchor: nothing matches.
    assert!(score(&cfg, 15).is_empty(), "outside the window");
}

/// A run with no observed baseline is still scored, with `persistence` set to null.
#[test]
fn missing_observed_baseline_gives_null_persistence() {
    let dir = tempfile::tempdir().unwrap();
    let cfg = config(dir.path());
    setup(&cfg);
    // Observation at -4 matches only run C's forecast at -5 (run C has no observed grid).
    let entries = score(&cfg, -4);
    assert_eq!(entries.len(), 1, "{entries:?}");
    assert_eq!(entries[0]["anchor"], Value::from(ts(-10)));
    assert!(entries[0]["persistence"].is_null());
}

/// The log is written once per call, with the window and schema keys, and an empty match writes nothing.
#[test]
fn log_is_written_only_when_something_matched() {
    let dir = tempfile::tempdir().unwrap();
    let cfg = config(dir.path());
    setup(&cfg);
    assert!(score(&cfg, 15).is_empty());
    assert!(!cfg.paths.state_dir.join("nowcast-skill.json").exists());
    score(&cfg, 4);
    let log: Value = serde_json::from_str(
        &std::fs::read_to_string(cfg.paths.state_dir.join("nowcast-skill.json")).unwrap(),
    )
    .unwrap();
    assert_eq!(log["schema_version"], Value::from(1));
    assert_eq!(log["window_hours"], Value::from(24));
    assert_eq!(log["entries"].as_array().unwrap().len(), 1);
}
