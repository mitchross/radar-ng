#![allow(clippy::needless_range_loop)]
//! End-to-end tests of `nowcast_run` on synthetic grid dumps in a temporary directory (spec-nowcast §13).
//!
//! The grids are written with `radar-core`'s grid writer, so the layout is the production one. Each test
//! checks the manifest layer, tiles, point grids, status, processed set and skill log.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

use chrono::{TimeDelta, TimeZone, Utc};
use radar_core::env::Paths;
use radar_core::grid_dump::{GridConfig, GridInput, NOWCAST_INPUT_LAYER, write_grid};
use radar_core::time::isoformat_utc;
use radar_nowcast::bench_nowcast;
use radar_nowcast::config::NowcastConfig;
use radar_nowcast::run::{NowcastOutput, nowcast_run};
use serde_json::Value;

const H: usize = 40;
const W: usize = 60;
const PALETTES: [&str; 3] = ["classic", "muted", "vivid"];

/// Timestamp of `minute` after the base anchor time (2026-07-15 16:00 UTC).
fn ts(minute: i64) -> String {
    let base = Utc.with_ymd_and_hms(2026, 7, 15, 16, 0, 0).unwrap();
    isoformat_utc(&(base + TimeDelta::minutes(minute)))
}

fn axes() -> (Vec<f64>, Vec<f64>) {
    let lats = radar_nowcast::inputs::linspace(54.0, 20.0, H);
    let lons = radar_nowcast::inputs::linspace(-130.0, -60.0, W);
    (lats, lons)
}

/// Two Gaussian blobs moving east by one column per 5-minute step, with a no-coverage top row.
fn blobs(minute: i64) -> Vec<f32> {
    let step = (minute / 5) as f64;
    let mut v = vec![-10.0f32; H * W];
    for (cx0, cy0, amp) in [(20.0f64, 15.0f64, 45.0f64), (42.0, 26.0, 35.0)] {
        let (cx, cy) = (cx0 + step, cy0);
        for y in 0..H {
            for x in 0..W {
                let d2 = (x as f64 - cx).powi(2) + (y as f64 - cy).powi(2);
                let val = (amp * (-d2 / (2.0 * 4.0 * 4.0)).exp() - 10.0) as f32;
                let idx = y * W + x;
                if val > v[idx] {
                    v[idx] = val;
                }
            }
        }
    }
    for x in 0..W {
        v[x] = -9999.0;
    }
    v
}

fn dry(_minute: i64) -> Vec<f32> {
    let mut v = vec![-10.0f32; H * W];
    for x in 0..W {
        v[x] = -9999.0;
    }
    v
}

/// Write the science grid for `minute` into the input layer.
fn write_frame(cfg: &NowcastConfig, minute: i64, values: &[f32]) {
    let (lats, lons) = axes();
    write_grid(
        &cfg.grid,
        NOWCAST_INPUT_LAYER,
        &ts(minute),
        &GridInput {
            data: values,
            lats: &lats,
            lons: &lons,
            unit: "dBZ",
            fill: f64::NAN,
            max_cells: None,
            grid_key: None,
        },
    )
    .unwrap()
    .expect("shape matches");
}

fn config(root: &Path) -> NowcastConfig {
    let paths = Paths::new(root.join("state"), root.join("grids"), root.join("tiles"));
    let mut cfg = NowcastConfig::with_paths(paths);
    cfg.grid = GridConfig::with_dir(root.join("grids"));
    cfg.palettes = PALETTES.iter().map(|s| s.to_string()).collect();
    cfg.render_workers = 2;
    cfg.heartbeat_every = Duration::from_millis(5);
    cfg
}

/// One palette and a 20-minute horizon: enough for the flow, cheap in a debug build.
fn light_config(root: &Path) -> NowcastConfig {
    let mut cfg = config(root);
    cfg.palettes = vec!["classic".to_string()];
    cfg.horizon_min = 20;
    cfg
}

fn beats() -> (Arc<dyn Fn() + Send + Sync>, Arc<AtomicUsize>) {
    let counter = Arc::new(AtomicUsize::new(0));
    let c = Arc::clone(&counter);
    (
        Arc::new(move || {
            c.fetch_add(1, Ordering::SeqCst);
        }),
        counter,
    )
}

async fn run(cfg: &NowcastConfig) -> NowcastOutput {
    let (hb, _) = beats();
    nowcast_run(Arc::new(cfg.clone()), hb)
        .await
        .expect("nowcast_run")
}

fn read_json(path: &Path) -> Value {
    serde_json::from_str(
        &std::fs::read_to_string(path).unwrap_or_else(|e| panic!("{}: {e}", path.display())),
    )
    .expect("valid JSON")
}

fn manifest(cfg: &NowcastConfig) -> Value {
    read_json(&cfg.paths.state_dir.join("manifest.json"))
}

fn status(cfg: &NowcastConfig) -> Value {
    read_json(&cfg.paths.state_dir.join("nowcast-status.json"))
}

fn png_count(dir: &Path) -> usize {
    let Ok(read) = std::fs::read_dir(dir) else {
        return 0;
    };
    read.flatten()
        .map(|e| {
            let p: PathBuf = e.path();
            if p.is_dir() {
                png_count(&p)
            } else {
                usize::from(p.extension().is_some_and(|x| x == "png"))
            }
        })
        .sum()
}

fn write_frames(cfg: &NowcastConfig, minutes: &[i64]) {
    for &m in minutes {
        write_frame(cfg, m, &blobs(m));
    }
}

/// §13 items 14, 15, 16, 18 and D4: manifest replacement, tiles, point grids, observed anchor, status after commit.
#[tokio::test]
async fn full_run_commits_manifest_tiles_points_and_status() {
    let dir = tempfile::tempdir().unwrap();
    let cfg = config(dir.path());
    write_frames(&cfg, &[-15, -10, -5, 0]);

    let out = run(&cfg).await;
    let r = &out.result;
    assert!(r.ran);
    assert_eq!(r.anchor_ts.as_deref(), Some(ts(0).as_str()));
    assert_eq!(r.leadtimes, 12);
    assert_eq!(r.palettes, PALETTES.to_vec());
    assert!(r.duration_s >= 0.0);

    // Manifest: the layer holds exactly the 12 new leads, in lead order, with the run metadata.
    let m = manifest(&cfg);
    let layer = &m["layers"]["nowcast"];
    let timestamps: Vec<String> = (1..=12).map(|i| ts(5 * i)).collect();
    assert_eq!(
        layer["timestamps"],
        serde_json::to_value(&timestamps).unwrap()
    );
    assert_eq!(layer["latest"], Value::from(ts(60)));
    assert_eq!(layer["palettes"], serde_json::json!(PALETTES));
    assert_eq!(layer["run_id"], Value::from(ts(0)));
    assert_eq!(layer["method"], Value::from("pysteps-sprog"));
    assert_eq!(layer["horizon_minutes"], Value::from(60));
    let frames = layer["frames"].as_array().unwrap();
    assert_eq!(frames.len(), 12);
    for (i, frame) in frames.iter().enumerate() {
        let lead = 5 * (i as i64 + 1);
        assert_eq!(frame["lead_minutes"], Value::from(lead));
        assert_eq!(frame["timestamp"], Value::from(ts(lead)));
        assert_eq!(
            frame["path"],
            Value::from(format!("runs/{}/{}", ts(0), ts(lead)))
        );
        assert_eq!(frame["grid_key"], frame["path"]);
        assert_eq!(frame["max_zoom"], Value::from(6));
        assert_eq!(frame["source"], Value::from("mrms-nowcast"));
        assert_eq!(frame["issued_at"], Value::from(ts(0)));
        assert_eq!(frame["input_interval_minutes"], Value::from(5.0));
    }
    // Spatial resolution: 34 deg over 39 cell gaps, times 111 km, rounded to 2 places.
    let res = frames[0]["spatial_resolution_km"].as_f64().unwrap();
    let expect: f64 = (34.0_f64 / 39.0 * 111.0 * 100.0).round() / 100.0;
    assert!((res - expect).abs() < 1e-9, "{res}");

    // Tiles: one complete pyramid per palette per lead, with the indexed-rs1 identity and PNG tiles.
    for palette in PALETTES {
        for lead in 1..=12 {
            let root = cfg
                .paths
                .tile_dir
                .join("nowcast")
                .join(palette)
                .join("runs")
                .join(ts(0))
                .join(ts(5 * lead));
            let marker =
                std::fs::read_to_string(root.join(".render-complete-v2.json")).expect("marker");
            assert!(
                marker.contains("indexed-rs1"),
                "renderer identity in marker"
            );
            assert!(png_count(&root) > 0, "tiles under {}", root.display());
        }
    }

    // Point grids: 12 leads plus the observed anchor, and the generation marker.
    let run_dir = cfg.paths.grid_dir.join("nowcast").join("runs").join(ts(0));
    for lead in 1..=12 {
        assert!(run_dir.join(format!("{}.meta.json", ts(5 * lead))).exists());
    }
    assert!(run_dir.join("observed.meta.json").exists());
    let marker = read_json(&run_dir.join(".grid-generation-complete-v1.json"));
    assert_eq!(marker["grid_count"], Value::from(13));
    assert_eq!(marker["generation"], Value::from(ts(0)));

    // Status is written only after the commit: ok, no reason. Processed set holds the anchor.
    let s = status(&cfg);
    assert_eq!(s["status"], Value::from("ok"));
    assert!(s["reason"].is_null());
    let processed = read_json(&cfg.paths.state_dir.join("nowcast.json"));
    assert_eq!(processed["items"], serde_json::json!([ts(0)]));

    // No earlier run to score, so no skill log is created.
    assert!(!cfg.paths.state_dir.join("nowcast-skill.json").exists());
    assert_eq!(out.skill_summary.as_ref().map(|s| s.runs_scored), Some(0));
}

/// §13 item 24 and D3-style scoring: the second run scores the first run's lead-5 forecast against the new
/// observation, and the log gets the persistence baseline.
#[tokio::test]
async fn second_run_scores_the_previous_forecast_into_the_skill_log() {
    let dir = tempfile::tempdir().unwrap();
    let cfg = light_config(dir.path());
    write_frames(&cfg, &[-15, -10, -5, 0]);
    run(&cfg).await;

    write_frames(&cfg, &[5]);
    let out = run(&cfg).await;
    assert_eq!(out.result.anchor_ts.as_deref(), Some(ts(5).as_str()));
    assert_eq!(out.result.skill_scored, 1);

    let log = read_json(&cfg.paths.state_dir.join("nowcast-skill.json"));
    let entries = log["entries"].as_array().unwrap();
    assert_eq!(entries.len(), 1, "{log}");
    let e = &entries[0];
    assert_eq!(e["anchor"], Value::from(ts(0)));
    assert_eq!(e["valid"], Value::from(ts(5)));
    assert_eq!(e["lead_minutes"], Value::from(5));
    assert!(e["cells"].as_i64().unwrap() > 0);
    assert!(e["thresholds"]["rain"]["hits"].is_i64());
    assert!(
        e["persistence"]["rain"].is_object(),
        "persistence baseline present"
    );
    assert_eq!(log["schema_version"], Value::from(1));

    assert!(
        out.prometheus
            .iter()
            .any(|l| l == "radar_ng_nowcast_skill_runs 1"),
        "{:?}",
        out.prometheus
    );
}

/// §13 item 5 and §4.3 step 3: a run whose newest grid is already processed returns before loading anything.
#[tokio::test]
async fn already_processed_anchor_is_not_rerun() {
    let dir = tempfile::tempdir().unwrap();
    let cfg = light_config(dir.path());
    write_frames(&cfg, &[-15, -10, -5, 0]);
    run(&cfg).await;
    let before = std::fs::read_to_string(cfg.paths.state_dir.join("nowcast-status.json")).unwrap();

    let out = run(&cfg).await;
    assert!(!out.result.ran);
    assert_eq!(out.result.anchor_ts.as_deref(), Some(ts(0).as_str()));
    assert_eq!(out.result.leadtimes, 0);
    assert_eq!(
        std::fs::read_to_string(cfg.paths.state_dir.join("nowcast-status.json")).unwrap(),
        before
    );
}

/// D2: a dry window is reported as dry. The run commits with no leads, the layer is removed, and the status
/// says ok with reason `no_rain`. Nothing is reported as a failure.
#[tokio::test]
async fn dry_window_commits_an_empty_run_reported_as_no_rain() {
    let dir = tempfile::tempdir().unwrap();
    let cfg = config(dir.path());
    for m in [-15, -10, -5, 0] {
        write_frame(&cfg, m, &dry(m));
    }
    let out = run(&cfg).await;
    assert!(out.result.ran);
    assert_eq!(out.result.leadtimes, 0);
    assert!(out.result.palettes.is_empty());

    let m = manifest(&cfg);
    assert!(
        m["layers"].get("nowcast").is_none(),
        "nowcast layer removed: {m}"
    );
    let s = status(&cfg);
    assert_eq!(s["status"], Value::from("ok"));
    assert_eq!(s["reason"], Value::from("no_rain"));
    assert!(
        !cfg.paths
            .tile_dir
            .join("nowcast")
            .join("classic")
            .join("runs")
            .join(ts(0))
            .exists()
    );
    // The observed anchor is still stored and the anchor is processed.
    assert!(
        cfg.paths
            .grid_dir
            .join("nowcast/runs")
            .join(ts(0))
            .join("observed.meta.json")
            .exists()
    );
    let processed = read_json(&cfg.paths.state_dir.join("nowcast.json"));
    assert_eq!(processed["items"], serde_json::json!([ts(0)]));
}

/// §4.3 step 2: fewer than three grids is a warming-up state with the spec's detail string.
#[tokio::test]
async fn too_few_grids_is_warming_up() {
    let dir = tempfile::tempdir().unwrap();
    let cfg = config(dir.path());
    write_frames(&cfg, &[-5, 0]);
    let out = run(&cfg).await;
    assert!(!out.result.ran);
    assert!(out.result.anchor_ts.is_none());
    let s = status(&cfg);
    assert_eq!(s["status"], Value::from("warming_up"));
    assert_eq!(s["reason"], Value::from("insufficient_inputs"));
    assert_eq!(s["detail"], Value::from("count=2,required=3"));
}

/// §4.3 step 7 and §13 item 4: a cadence violation is degraded, with the intervals in the detail.
#[tokio::test]
async fn cadence_violation_is_degraded_without_running() {
    let dir = tempfile::tempdir().unwrap();
    let cfg = config(dir.path());
    write_frames(&cfg, &[-20, -10, 0]);
    let out = run(&cfg).await;
    assert!(!out.result.ran);
    assert_eq!(out.result.anchor_ts.as_deref(), Some(ts(0).as_str()));
    let s = status(&cfg);
    assert_eq!(s["status"], Value::from("degraded"));
    assert_eq!(s["reason"], Value::from("invalid_input_cadence"));
    assert_eq!(s["detail"], Value::from("intervals_minutes=[10.0, 10.0]"));
    assert!(!cfg.paths.state_dir.join("manifest.json").exists());
}

/// §13 item 19: retention removes whole old runs, keeps the newest complete ones and the active run.
#[tokio::test]
async fn retention_removes_old_whole_runs() {
    let dir = tempfile::tempdir().unwrap();
    let mut cfg = light_config(dir.path());
    cfg.retention_runs = 2;
    write_frames(&cfg, &[-15, -10, -5, 0]);
    run(&cfg).await;
    write_frames(&cfg, &[5]);
    run(&cfg).await;
    write_frames(&cfg, &[10]);
    run(&cfg).await;

    let runs = cfg.paths.grid_dir.join("nowcast").join("runs");
    assert!(!runs.join(ts(0)).exists(), "oldest run removed");
    assert!(
        runs.join(ts(5))
            .join(".grid-generation-complete-v1.json")
            .exists()
    );
    assert!(
        runs.join(ts(10))
            .join(".grid-generation-complete-v1.json")
            .exists()
    );
    // The manifest references only the newest run.
    let m = manifest(&cfg);
    assert_eq!(m["layers"]["nowcast"]["run_id"], Value::from(ts(10)));
}

/// The benchmark entry point runs the same selection and writes its report.
#[tokio::test]
async fn bench_entry_point_reports_timings_and_checksum() {
    let dir = tempfile::tempdir().unwrap();
    let cfg = config(dir.path());
    write_frames(&cfg, &[-15, -10, -5, 0]);
    let layer = cfg.paths.grid_dir.join(NOWCAST_INPUT_LAYER);
    let out = dir.path().join("bench");
    let report = bench_nowcast(&layer, &out).expect("bench");
    assert_eq!(report.width, W);
    assert_eq!(report.height, H);
    assert_eq!(report.leads, 12);
    assert_eq!(report.frames.len(), 4);
    assert!(report.checksum.is_finite() && report.checksum != 0.0);
    assert!(report.total_s >= report.motion_s);
    assert!(out.join("nowcast-bench.json").exists());
}
