//! `tile_cleanup_sweep` against a tempdir tree covering every retention rule (spec-ingest-aux section 8).

use std::fs::{self, File};
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

use chrono::{DateTime, Utc};
use radar_aux::tile_cleanup::{CleanupError, TileCleanupConfig, tile_cleanup_sweep_at};
use radar_core::manifest::{ReplaceOptions, read_manifest, replace_layer};
use serde_json::{Map, Value};
use tempfile::TempDir;

/// Timestamp `hours` before now, in the canonical frame-name form.
fn ago(hours: f64) -> String {
    let t: DateTime<Utc> =
        Utc::now() - chrono::Duration::milliseconds((hours * 3_600_000.0) as i64);
    t.format("%Y-%m-%dT%H:%M:%S+00:00").to_string()
}

struct Env {
    _dir: TempDir,
    tile: PathBuf,
    state: PathBuf,
    grid: PathBuf,
    cfg: TileCleanupConfig,
}

fn env() -> Env {
    let dir = TempDir::new().unwrap();
    let tile = dir.path().join("tiles");
    let state = dir.path().join("state");
    let grid = dir.path().join("grid");
    for d in [&tile, &state, &grid] {
        fs::create_dir_all(d).unwrap();
    }
    let cfg = TileCleanupConfig::new(&tile, &state, &grid);
    Env {
        _dir: dir,
        tile,
        state,
        grid,
        cfg,
    }
}

fn mkdir(p: &Path) {
    fs::create_dir_all(p).unwrap();
}

fn set_mtime_hours_ago(p: &Path, hours: f64) {
    let t = SystemTime::now() - Duration::from_secs_f64(hours * 3600.0);
    File::open(p).unwrap().set_modified(t).unwrap();
}

/// Registers frames in the manifest so the removal path has something to de-list.
fn list_frames(
    state: &Path,
    layer: &str,
    timestamps: &[String],
    metadata: Option<Map<String, Value>>,
) {
    replace_layer(
        state,
        layer,
        timestamps,
        &ReplaceOptions {
            palettes: vec![],
            frames: vec![],
            layer_metadata: metadata,
        },
    )
    .unwrap();
}

fn manifest_has(state: &Path, layer: &str, ts: &str) -> bool {
    read_manifest(state)
        .unwrap()
        .layers
        .get(layer)
        .is_some_and(|l| l.timestamps.iter().any(|t| t == ts))
}

#[test]
fn legacy_layout_expired_frames_are_removed_and_fresh_ones_kept() {
    let e = env();
    let old = ago(6.0); // radar retention is 4 h
    let fresh = ago(1.0);
    for ts in [&old, &fresh] {
        mkdir(&e.tile.join("radar").join(ts));
    }
    list_frames(&e.state, "radar", &[old.clone(), fresh.clone()], None);

    let res = tile_cleanup_sweep_at(&e.cfg, SystemTime::now()).unwrap();
    assert!(
        !e.tile.join("radar").join(&old).exists(),
        "expired frame directory removed"
    );
    assert!(
        e.tile.join("radar").join(&fresh).exists(),
        "fresh frame kept"
    );
    assert!(
        !manifest_has(&e.state, "radar", &old),
        "expired frame de-listed"
    );
    assert!(
        manifest_has(&e.state, "radar", &fresh),
        "fresh frame still listed"
    );
    assert_eq!(res.tile_dirs_removed, 1);
}

#[test]
fn each_layer_uses_its_own_retention_window() {
    let e = env();
    // nowcast keeps 60 min, radar-hrrr keeps 12 h. Same age, different outcome.
    let nowcast_old = ago(2.0);
    let hrrr_mid = ago(2.0);
    mkdir(&e.tile.join("nowcast").join(&nowcast_old));
    mkdir(&e.tile.join("radar-hrrr").join(&hrrr_mid));
    list_frames(
        &e.state,
        "nowcast",
        std::slice::from_ref(&nowcast_old),
        None,
    );
    list_frames(
        &e.state,
        "radar-hrrr",
        std::slice::from_ref(&hrrr_mid),
        None,
    );

    tile_cleanup_sweep_at(&e.cfg, SystemTime::now()).unwrap();
    assert!(
        !e.tile.join("nowcast").join(&nowcast_old).exists(),
        "60 min window: 2 h old is expired"
    );
    assert!(
        e.tile.join("radar-hrrr").join(&hrrr_mid).exists(),
        "12 h window: 2 h old is kept"
    );
}

#[test]
fn palette_layout_timestamps_are_swept_the_same_way() {
    let e = env();
    let old = ago(13.0); // temperature keeps 12 h
    let fresh = ago(11.0);
    mkdir(&e.tile.join("temperature/classic").join(&old));
    mkdir(&e.tile.join("temperature/classic").join(&fresh));
    list_frames(&e.state, "temperature", &[old.clone(), fresh.clone()], None);

    tile_cleanup_sweep_at(&e.cfg, SystemTime::now()).unwrap();
    assert!(!e.tile.join("temperature/classic").join(&old).exists());
    assert!(e.tile.join("temperature/classic").join(&fresh).exists());
    assert!(!manifest_has(&e.state, "temperature", &old));
}

#[test]
fn staging_directories_go_by_mtime_not_by_name() {
    let e = env();
    let stale = e.tile.join("radar/.2026-01-01T00:00:00+00:00.tmp-abc");
    let stale_suffix = e.tile.join("radar/2026-01-01T00:00:00+00:00.tmp");
    let fresh = e.tile.join("radar/.tmp-fresh");
    for d in [&stale, &stale_suffix, &fresh] {
        mkdir(d);
    }
    set_mtime_hours_ago(&stale, 2.0);
    set_mtime_hours_ago(&stale_suffix, 2.0);
    set_mtime_hours_ago(&fresh, 0.1);

    tile_cleanup_sweep_at(&e.cfg, SystemTime::now()).unwrap();
    assert!(!stale.exists(), ".tmp- staging older than an hour removed");
    assert!(
        !stale_suffix.exists(),
        ".tmp staging older than an hour removed"
    );
    assert!(fresh.exists(), "recent staging kept (may be in progress)");
}

#[test]
fn names_that_are_not_timestamps_are_left_alone() {
    let e = env();
    let odd = e.tile.join("radar/notes/whatever-dir");
    mkdir(&odd);
    set_mtime_hours_ago(&odd, 100.0);
    mkdir(&e.tile.join("radar/legend"));
    fs::write(e.tile.join("radar/readme.txt"), b"x").unwrap();

    tile_cleanup_sweep_at(&e.cfg, SystemTime::now()).unwrap();
    assert!(odd.exists());
    assert!(e.tile.join("radar/readme.txt").exists());
}

#[test]
fn versioned_runs_are_removed_when_old_unless_current_or_future() {
    let e = env();
    // Both palettes carry the run, so the sweep removes two directories (spec 15.4).
    let runs = e.tile.join("nowcast/classic/runs");
    let vivid_runs = e.tile.join("nowcast/vivid/runs");
    let old = "2000-01-01T00:00:00+00:00";
    let current_old = "2001-01-01T00:00:00+00:00";
    let future = "2999-01-01T00:00:00+00:00";
    for r in [old, current_old, future] {
        mkdir(&runs.join(r));
        mkdir(&vivid_runs.join(r));
    }
    let mut meta = Map::new();
    meta.insert("run_id".into(), Value::String(current_old.into()));
    list_frames(&e.state, "nowcast", &[ago(0.1)], Some(meta));

    let res = tile_cleanup_sweep_at(&e.cfg, SystemTime::now()).unwrap();
    assert!(!runs.join(old).exists(), "old non-current run removed");
    assert!(
        !vivid_runs.join(old).exists(),
        "old non-current run removed in every palette"
    );
    assert!(
        runs.join(current_old).exists(),
        "current manifest run kept even when old"
    );
    assert!(
        vivid_runs.join(current_old).exists(),
        "current run kept in every palette"
    );
    assert!(runs.join(future).exists(), "run newer than the cutoff kept");
    assert_eq!(res.tile_dirs_removed, 2);
}

#[test]
fn removal_order_is_manifest_then_directory() {
    let e = env();
    let old = ago(6.0);
    let dir = e.tile.join("radar").join(&old);
    mkdir(&dir);
    list_frames(&e.state, "radar", std::slice::from_ref(&old), None);
    // Block manifest writes: the lock path is a directory, so flock cannot open it.
    let lock = e.state.join("manifest.lock");
    let _ = fs::remove_file(&lock);
    mkdir(&lock);

    let err = tile_cleanup_sweep_at(&e.cfg, SystemTime::now()).unwrap_err();
    assert!(matches!(err, CleanupError::Manifest(_)), "got {err:?}");
    assert!(
        dir.exists(),
        "manifest failed, so the frame directory must survive"
    );
    assert!(manifest_has(&e.state, "radar", &old), "manifest unchanged");
}

#[test]
fn successful_removal_de_lists_before_deleting() {
    let e = env();
    let old = ago(6.0);
    let dir = e.tile.join("radar").join(&old);
    mkdir(&dir);
    list_frames(&e.state, "radar", std::slice::from_ref(&old), None);
    tile_cleanup_sweep_at(&e.cfg, SystemTime::now()).unwrap();
    assert!(!dir.exists());
    assert!(!manifest_has(&e.state, "radar", &old));
}

#[test]
fn corrupt_manifest_fails_closed_and_deletes_nothing() {
    let e = env();
    let old = ago(6.0);
    let dir = e.tile.join("radar").join(&old);
    mkdir(&dir);
    fs::write(e.state.join("manifest.json"), b"{ corrupt").unwrap();

    assert!(tile_cleanup_sweep_at(&e.cfg, SystemTime::now()).is_err());
    assert!(
        dir.exists(),
        "no directory removed when the manifest cannot be read"
    );
}

#[test]
fn grid_store_files_older_than_the_grid_window_are_removed() {
    let e = env();
    let layer = e.grid.join("radar");
    mkdir(&layer);
    let old = layer.join("old.bin");
    let fresh = layer.join("fresh.bin");
    fs::write(&old, b"x").unwrap();
    fs::write(&fresh, b"x").unwrap();
    set_mtime_hours_ago(&old, 48.0); // grid window is 12 h by default
    set_mtime_hours_ago(&fresh, 1.0);

    let res = tile_cleanup_sweep_at(&e.cfg, SystemTime::now()).unwrap();
    assert!(!old.exists());
    assert!(fresh.exists());
    assert_eq!(res.grid_files_removed, 1);
}

#[test]
fn reports_all_twelve_layers_swept() {
    let e = env();
    let res = tile_cleanup_sweep_at(&e.cfg, SystemTime::now()).unwrap();
    assert_eq!(res.layers_swept, 12);
    assert_eq!(res.tile_dirs_removed, 0);
    assert_eq!(res.grid_files_removed, 0);
}
