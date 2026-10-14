//! Grid invariants GRD-01..GRD-13 (spec §13.2, §13.3), derived DRV-01..04, and the byte-exact golden replay
//! of `testdata/grid/scenarios.json` against the Python writer's output.

use super::*;
use crate::fsutil::FlockGuard;
use serde_json::Value;
use std::sync::Mutex;

const TS0: &str = "2026-09-02T12:00:00+00:00";

fn stamps(n: usize) -> Vec<String> {
    (0..n)
        .map(|i| format!("2026-09-02T12:{:02}:00+00:00", i * 2))
        .collect()
}

fn small_grid_input(data: &[f32]) -> (Vec<f32>, Vec<f64>, Vec<f64>) {
    let lats: Vec<f64> = (0..8).map(|i| 50.0 - i as f64 * (30.0 / 7.0)).collect();
    let lons: Vec<f64> = (0..16).map(|i| -130.0 + i as f64 * (70.0 / 15.0)).collect();
    let mut v = data.to_vec();
    if v.is_empty() {
        v = vec![25.0; 8 * 16];
    }
    (v, lats, lons)
}

fn write_stamp(cfg: &GridConfig, layer: &str, ts: &str) -> PathBuf {
    let (data, lats, lons) = small_grid_input(&[]);
    let input = GridInput::new(&data, &lats, &lons, "dBZ");
    write_grid(cfg, layer, ts, &input)
        .unwrap()
        .expect("shape accepted")
}

fn names_in(dir: &Path) -> Vec<String> {
    let mut v: Vec<String> = std::fs::read_dir(dir)
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    v.sort();
    v
}

fn metas_in(dir: &Path) -> Vec<String> {
    names_in(dir)
        .into_iter()
        .filter(|n| n.ends_with(".meta.json"))
        .collect()
}

fn set_mtime_old(path: &Path, age: Duration) {
    let when = SystemTime::now() - age;
    std::fs::File::open(path)
        .unwrap()
        .set_modified(when)
        .unwrap();
}

#[test]
fn grd_01_second_write_gets_new_binary_and_meta_points_to_it() {
    let root = tempfile::tempdir().unwrap();
    let cfg = GridConfig::with_dir(root.path());
    let first = write_stamp(&cfg, "radar", TS0);
    let meta_path = root.path().join("radar").join(format!("{TS0}.meta.json"));
    let first_meta: Value =
        serde_json::from_str(&std::fs::read_to_string(&meta_path).unwrap()).unwrap();
    assert_eq!(
        first.file_name().unwrap().to_str().unwrap(),
        first_meta["data_file"].as_str().unwrap()
    );

    let (data, lats, lons) = small_grid_input(&[30.0; 8 * 16]);
    let second = write_grid(
        &cfg,
        "radar",
        TS0,
        &GridInput::new(&data, &lats, &lons, "dBZ"),
    )
    .unwrap()
    .unwrap();
    let second_meta: Value =
        serde_json::from_str(&std::fs::read_to_string(&meta_path).unwrap()).unwrap();
    assert_ne!(first, second);
    assert_eq!(
        second.file_name().unwrap().to_str().unwrap(),
        second_meta["data_file"].as_str().unwrap()
    );
    assert!(
        first.exists(),
        "previous generation is kept until retention"
    );
}

#[test]
fn grd_02_max_cells_caps_stored_size() {
    let root = tempfile::tempdir().unwrap();
    let cfg = GridConfig::with_dir(root.path());
    let data: Vec<f32> = (0..100).map(|v| v as f32).collect();
    let lats: Vec<f64> = (0..10).map(|i| 50.0 - i as f64).collect();
    let lons: Vec<f64> = (0..10).map(|i| -90.0 + i as f64).collect();
    let input = GridInput {
        max_cells: Some(25),
        ..GridInput::new(&data, &lats, &lons, "dBZ")
    };
    write_grid(&cfg, "radar", TS0, &input).unwrap().unwrap();
    let meta: Value = serde_json::from_str(
        &std::fs::read_to_string(root.path().join("radar").join(format!("{TS0}.meta.json")))
            .unwrap(),
    )
    .unwrap();
    let cells = meta["height"].as_u64().unwrap() * meta["width"].as_u64().unwrap();
    assert!(cells <= 25, "stored {cells} cells");
    assert_eq!(meta["stride"], 2);
}

#[test]
fn grd_03_first_layer_creation_syncs_grid_dir_and_layer_dir() {
    let root = tempfile::tempdir().unwrap();
    let cfg = GridConfig::with_dir(root.path().join("grids"));
    let synced: Mutex<Vec<PathBuf>> = Mutex::new(Vec::new());
    let record = |p: &Path| -> io::Result<()> {
        synced.lock().unwrap().push(p.to_path_buf());
        Ok(())
    };
    let (data, lats, lons) = small_grid_input(&[]);
    write_grid_with(
        &cfg,
        "radar",
        TS0,
        &GridInput::new(&data, &lats, &lons, "dBZ"),
        &record,
        "00000000000000000000000000000001",
    )
    .unwrap()
    .unwrap();
    let got = synced.into_inner().unwrap();
    assert!(got.contains(&root.path().join("grids")), "{got:?}");
    assert!(
        got.contains(&root.path().join("grids").join("radar")),
        "{got:?}"
    );
}

#[test]
fn grd_06_parent_fsync_failure_during_write_propagates() {
    let root = tempfile::tempdir().unwrap();
    let grids = root.path().join("grids");
    std::fs::create_dir_all(&grids).unwrap();
    let cfg = GridConfig::with_dir(&grids);
    let fail_parent = |p: &Path| -> io::Result<()> {
        if p == grids.as_path() {
            Err(rustix::io::Errno::IO.into())
        } else {
            Ok(())
        }
    };
    let (data, lats, lons) = small_grid_input(&[]);
    let err = write_grid_with(
        &cfg,
        "radar",
        TS0,
        &GridInput::new(&data, &lats, &lons, "dBZ"),
        &fail_parent,
        "00000000000000000000000000000002",
    )
    .unwrap_err();
    assert!(matches!(err, GridError::Io(_)), "{err:?}");
}

#[test]
fn grd_04_directory_fsync_eio_is_not_suppressed() {
    let root = tempfile::tempdir().unwrap();
    let err = fsutil::fsync_dir_with(root.path(), |_| Err(rustix::io::Errno::IO)).unwrap_err();
    assert_eq!(
        err.raw_os_error(),
        Some(rustix::io::Errno::IO.raw_os_error())
    );
}

#[test]
fn grd_05_directory_fsync_einval_is_ignored() {
    let root = tempfile::tempdir().unwrap();
    fsutil::fsync_dir_with(root.path(), |_| Err(rustix::io::Errno::INVAL)).unwrap();
}

#[test]
fn grd_07_keep_n_removes_older_metas_and_their_data() {
    let root = tempfile::tempdir().unwrap();
    let cfg = GridConfig::with_dir(root.path());
    let ts = stamps(8);
    for t in &ts {
        write_stamp(&cfg, "radar-nowcast-input", t);
    }
    let layer_dir = root.path().join("radar-nowcast-input");
    let removed = prune_grid_layer(&cfg, "radar-nowcast-input", Some(4));
    assert_eq!(removed, 8, "4 metas + 4 data files");
    assert_eq!(
        metas_in(&layer_dir),
        ts[4..]
            .iter()
            .map(|t| format!("{t}.meta.json"))
            .collect::<Vec<_>>()
    );
    let bins: Vec<String> = names_in(&layer_dir)
        .into_iter()
        .filter(|n| n.ends_with(".bin"))
        .collect();
    assert_eq!(bins.len(), 4);
    let kept: BTreeSet<String> = bins.iter().map(|b| grid_timestamp_of(b)).collect();
    assert_eq!(kept, ts[4..].iter().cloned().collect());
}

#[test]
fn grd_08_orphans_and_legacy_removed_after_grace_and_in_flight_tmp_kept() {
    let root = tempfile::tempdir().unwrap();
    let cfg = GridConfig::with_dir(root.path());
    let ts = stamps(3);
    for t in &ts {
        write_stamp(&cfg, "radar-nowcast-input", t);
    }
    let layer_dir = root.path().join("radar-nowcast-input");
    let orphan = layer_dir.join("2026-09-02T11:00:00+00:00.deadbeefdeadbeefdeadbeefdeadbeef.bin");
    std::fs::write(&orphan, [0u8; 16]).unwrap();
    let legacy = layer_dir.join("2026-09-02T10:00:00+00:00.bin");
    std::fs::write(&legacy, [0u8; 16]).unwrap();
    let old = cfg.orphan_grace_s + 1;
    set_mtime_old(&orphan, Duration::from_secs(old));
    set_mtime_old(&legacy, Duration::from_secs(old));
    let inflight = layer_dir.join(".2026-09-02T12:06:00+00:00.meta.json.abc.tmp");
    std::fs::write(&inflight, "{}").unwrap();

    let removed = prune_grid_layer(&cfg, "radar-nowcast-input", Some(3));
    assert_eq!(removed, 2);
    assert!(!orphan.exists() && !legacy.exists());
    assert!(inflight.exists());
    assert_eq!(metas_in(&layer_dir).len(), 3);
}

#[test]
fn grd_08b_fresh_orphan_inside_grace_is_kept() {
    let root = tempfile::tempdir().unwrap();
    let cfg = GridConfig::with_dir(root.path());
    write_stamp(&cfg, "radar-nowcast-input", TS0);
    let layer_dir = root.path().join("radar-nowcast-input");
    let fresh = layer_dir.join("2026-09-02T11:00:00+00:00.00000000000000000000000000000000.bin");
    std::fs::write(&fresh, [0u8; 16]).unwrap();
    assert_eq!(prune_grid_layer(&cfg, "radar-nowcast-input", Some(1)), 0);
    assert!(fresh.exists());
}

#[test]
fn grd_09_unconfigured_layer_is_a_noop() {
    let root = tempfile::tempdir().unwrap();
    let cfg = GridConfig::with_dir(root.path());
    for t in stamps(5) {
        write_stamp(&cfg, "radar", &t);
    }
    assert_eq!(prune_grid_layer(&cfg, "radar", None), 0);
    assert_eq!(metas_in(&root.path().join("radar")).len(), 5);
}

#[test]
fn grd_10_keep_zero_removes_all_committed_and_keeps_lock() {
    let root = tempfile::tempdir().unwrap();
    let cfg = GridConfig::with_dir(root.path());
    for t in stamps(3) {
        write_stamp(&cfg, "radar-nowcast-input", &t);
    }
    let removed = prune_grid_layer(&cfg, "radar-nowcast-input", Some(0));
    let layer_dir = root.path().join("radar-nowcast-input");
    assert_eq!(removed, 6);
    assert!(metas_in(&layer_dir).is_empty());
    assert!(names_in(&layer_dir).iter().all(|n| !n.ends_with(".bin")));
    assert!(layer_dir.join(LAYER_LOCK_FILE).exists());
}

#[test]
fn grd_11_prune_blocks_while_the_layer_lock_is_held() {
    let root = tempfile::tempdir().unwrap();
    let cfg = GridConfig::with_dir(root.path());
    write_stamp(&cfg, "radar-nowcast-input", TS0);
    let layer_dir = root.path().join("radar-nowcast-input");
    let guard = layer_lock(&layer_dir, &fsutil::fsync_dir).unwrap();
    let cfg2 = cfg.clone();
    let handle =
        std::thread::spawn(move || prune_grid_layer(&cfg2, "radar-nowcast-input", Some(1)));
    std::thread::sleep(Duration::from_millis(150));
    assert!(
        !handle.is_finished(),
        "prune must wait for the writer's lock"
    );
    drop(guard);
    let removed = handle.join().unwrap();
    assert_eq!(removed, 0, "the committed generation is kept");
    assert_eq!(metas_in(&layer_dir).len(), 1);
}

#[test]
fn grd_12_cleanup_applies_keep_n_before_age_sweep() {
    let root = tempfile::tempdir().unwrap();
    let mut cfg = GridConfig::with_dir(root.path());
    cfg.keep_last = BTreeMap::from([("radar-nowcast-input".to_string(), 2)]);
    for t in stamps(5) {
        write_stamp(&cfg, "radar-nowcast-input", &t);
        write_stamp(&cfg, "radar", &t);
    }
    let removed = cleanup_old_grids(&cfg).unwrap();
    assert_eq!(removed, 6);
    assert_eq!(metas_in(&root.path().join("radar-nowcast-input")).len(), 2);
    assert_eq!(
        metas_in(&root.path().join("radar")).len(),
        5,
        "young enough for the age rule"
    );
}

#[test]
fn grd_13_default_keep_covers_nowcast_inputs_plus_slack() {
    let cfg = GridConfig::with_dir("/unused");
    assert!(cfg.keep_last["radar-nowcast-input"] >= 3 + 2);
    assert_eq!(cfg.keep_last["radar-nowcast-input"], 12);
    assert_eq!(keep_for_nowcast_inputs(4, 5.0), 12);
}

#[test]
fn drv_01_latitudes_descend_and_row_zero_is_north() {
    let root = tempfile::tempdir().unwrap();
    let cfg = GridConfig::with_dir(root.path());
    // Ascending latitudes (south first): the stored grid must put lat 41 (values 4,5,6) in row 0.
    let data = [1.0f32, 2.0, 3.0, 4.0, 5.0, 6.0];
    let lats = [40.0, 41.0];
    let lons = [-2.0, -1.0, 0.0];
    let input = GridInput::new(&data, &lats, &lons, "dBZ");
    let path = write_grid(&cfg, "radar", TS0, &input).unwrap().unwrap();
    let meta_path = root.path().join("radar").join(format!("{TS0}.meta.json"));
    let dump = crate::grid_read::read_grid(&meta_path).unwrap();
    assert_eq!(
        path.file_name().unwrap().to_str().unwrap(),
        dump.meta.data_file
    );
    assert_eq!(dump.meta.lat_max, 41.0);
    assert_eq!(dump.meta.lat_min, 40.0);
    assert_eq!(dump.at(0, 0), 4.0);
    assert_eq!(dump.at(0, 2), 6.0);
    assert_eq!(dump.at(1, 0), 1.0);
}

#[test]
fn drv_02_longitudes_above_180_shift_and_columns_sort_ascending() {
    let root = tempfile::tempdir().unwrap();
    let cfg = GridConfig::with_dir(root.path());
    // Column 1 (359, which is -1 after the shift) must move to the front, carrying its values with it.
    let data = [1.0f32, 2.0, 3.0, 4.0, 5.0, 6.0];
    let lats = [41.0, 40.0];
    let lons = [1.0, 359.0, 0.5];
    let input = GridInput::new(&data, &lats, &lons, "dBZ");
    write_grid(&cfg, "radar", TS0, &input).unwrap().unwrap();
    let dump =
        crate::grid_read::read_grid(&root.path().join("radar").join(format!("{TS0}.meta.json")))
            .unwrap();
    assert_eq!(dump.meta.lon_min, -1.0);
    assert_eq!(dump.meta.lon_max, 1.0);
    assert_eq!(dump.at(0, 0), 2.0, "column for 359 is first");
    assert_eq!(dump.at(0, 1), 3.0, "column for 0.5 is second");
    assert_eq!(dump.at(0, 2), 1.0, "column for 1.0 is last");
    assert_eq!(dump.at(1, 0), 5.0);
}

#[test]
fn drv_03_stride_is_a_power_of_two_within_the_cap_using_floor_division() {
    let root = tempfile::tempdir().unwrap();
    let cfg = GridConfig::with_dir(root.path());
    let data = vec![1.0f32; 100];
    let lats: Vec<f64> = (0..10).map(|i| 50.0 - i as f64).collect();
    let lons: Vec<f64> = (0..10).map(|i| -90.0 + i as f64).collect();
    let input = GridInput {
        max_cells: Some(10),
        ..GridInput::new(&data, &lats, &lons, "dBZ")
    };
    write_grid(&cfg, "radar", TS0, &input).unwrap().unwrap();
    let meta: Value = serde_json::from_str(
        &std::fs::read_to_string(root.path().join("radar").join(format!("{TS0}.meta.json")))
            .unwrap(),
    )
    .unwrap();
    let stride = meta["stride"].as_u64().unwrap();
    assert!(stride.is_power_of_two());
    assert_eq!(
        stride, 4,
        "floor(10/4)^2 = 4 <= 10, floor(10/2)^2 = 25 > 10"
    );
}

#[test]
fn drv_04_nan_becomes_sentinel_only_when_fill_is_nan() {
    let root = tempfile::tempdir().unwrap();
    let cfg = GridConfig::with_dir(root.path());
    let data = [f32::NAN, 1.0, 2.0, 3.0];
    let lats = [43.0, 42.0];
    let lons = [-86.0, -85.0];
    let nan_fill = GridInput::new(&data, &lats, &lons, "dBZ");
    write_grid(&cfg, "a", TS0, &nan_fill).unwrap();
    let dump = crate::grid_read::read_grid(&root.path().join("a").join(format!("{TS0}.meta.json")))
        .unwrap();
    assert_eq!(dump.values[0], -9999.0);

    let explicit = GridInput {
        fill: -1.5,
        ..GridInput::new(&data, &lats, &lons, "dBZ")
    };
    write_grid(&cfg, "b", TS0, &explicit).unwrap();
    let dump = crate::grid_read::read_grid(&root.path().join("b").join(format!("{TS0}.meta.json")))
        .unwrap();
    assert!(
        dump.values[0].is_nan(),
        "D10: NaN is kept when fill is explicit"
    );
    assert_eq!(dump.meta.fill, -1.5);
}

#[test]
fn shape_mismatch_and_empty_axis_handling() {
    let root = tempfile::tempdir().unwrap();
    let cfg = GridConfig::with_dir(root.path());
    let data = [1.0f32; 3];
    let lats = [43.0, 42.0];
    let lons = [-86.0, -85.0];
    assert!(
        write_grid(
            &cfg,
            "radar",
            TS0,
            &GridInput::new(&data, &lats, &lons, "dBZ")
        )
        .unwrap()
        .is_none()
    );
    assert!(
        !root.path().join("radar").exists(),
        "nothing written for a rejected shape"
    );
    let empty: [f64; 0] = [];
    assert!(matches!(
        write_grid(
            &cfg,
            "radar",
            TS0,
            &GridInput::new(&[], &empty, &lons, "dBZ")
        ),
        Err(GridError::InvalidInput(_))
    ));
    assert!(matches!(
        write_grid(
            &cfg,
            "radar",
            TS0,
            &GridInput::new(&[1.0; 4], &[43.0, f64::NAN], &lons, "dBZ")
        ),
        Err(GridError::InvalidInput(_))
    ));
}

#[test]
fn relative_keys_reject_traversal_and_empty_segments() {
    assert_eq!(
        relative_grid_key("runs/a/b").unwrap(),
        PathBuf::from("runs/a/b")
    );
    for bad in ["", "a//b", "a/./b", "../x", "/abs", "a/../b"] {
        assert!(relative_grid_key(bad).is_err(), "{bad}");
    }
}

#[test]
fn grid_timestamp_of_names() {
    assert_eq!(
        grid_timestamp_of("2026-09-02T12:00:00+00:00.meta.json"),
        "2026-09-02T12:00:00+00:00"
    );
    assert_eq!(
        grid_timestamp_of("2026-09-02T12:00:00+00:00.0123456789abcdef0123456789abcdef.bin"),
        "2026-09-02T12:00:00+00:00"
    );
    assert_eq!(
        grid_timestamp_of("2026-09-02T10:00:00+00:00.bin"),
        "2026-09-02T10:00:00+00:00"
    );
    assert_eq!(
        grid_timestamp_of("x.0123456789ABCDEF0123456789abcdef.bin"),
        "x.0123456789ABCDEF0123456789abcdef"
    );
    assert_eq!(grid_timestamp_of("weird"), "weird");
}

#[test]
fn finalize_writes_marker_and_adding_a_meta_invalidates_it() {
    let root = tempfile::tempdir().unwrap();
    let cfg = GridConfig::with_dir(root.path());
    let run_id = "2026-07-15T12:00:00+00:00";
    let key = format!("runs/{run_id}/2026-07-15T12:05:00+00:00");
    let (data, lats, lons) = small_grid_input(&[]);
    let input = GridInput {
        grid_key: Some(&key),
        ..GridInput::new(&data, &lats, &lons, "dBZ")
    };
    write_grid(&cfg, "nowcast", "2026-07-15T12:05:00+00:00", &input).unwrap();
    let run_dir = root.path().join("nowcast").join("runs").join(run_id);
    assert!(!is_complete_generation(&run_dir));
    finalize_grid_generation(&cfg, "nowcast", run_id).unwrap();
    assert!(is_complete_generation(&run_dir));
    let input2 = GridInput {
        grid_key: Some(&format!("runs/{run_id}/2026-07-15T12:10:00+00:00")),
        ..GridInput::new(&data, &lats, &lons, "dBZ")
    };
    write_grid(&cfg, "nowcast", "2026-07-15T12:10:00+00:00", &input2).unwrap();
    assert!(
        !is_complete_generation(&run_dir),
        "a new meta invalidates the marker"
    );
}

#[test]
fn prune_generations_keeps_newest_complete_and_active_and_never_touches_in_progress() {
    let root = tempfile::tempdir().unwrap();
    let cfg = GridConfig::with_dir(root.path());
    let (data, lats, lons) = small_grid_input(&[]);
    let gens = [
        "2026-07-15T10:00:00+00:00",
        "2026-07-15T11:00:00+00:00",
        "2026-07-15T12:00:00+00:00",
        "2026-07-15T13:00:00+00:00",
    ];
    for (i, run_id) in gens.iter().enumerate() {
        let key = format!("runs/{run_id}/{}", stamps(1)[0]);
        let input = GridInput {
            grid_key: Some(&key),
            ..GridInput::new(&data, &lats, &lons, "dBZ")
        };
        write_grid(&cfg, "nowcast", &format!("v{i}"), &input).unwrap();
        if i != 3 {
            finalize_grid_generation(&cfg, "nowcast", run_id).unwrap();
        }
    }
    let in_progress = root.path().join("nowcast/runs/2026-07-15T13:00:00+00:00");
    let removed =
        prune_grid_generations(&cfg, "nowcast", 1, Some("2026-07-15T10:00:00+00:00")).unwrap();
    // Complete: 10:00 (active, kept), 11:00 (removed), 12:00 (newest complete, kept). 13:00 is in progress.
    assert_eq!(
        removed, 3,
        "the meta, the bin and the completion marker of the removed generation"
    );
    assert!(
        root.path()
            .join("nowcast/runs/2026-07-15T10:00:00+00:00")
            .exists()
    );
    assert!(
        !root
            .path()
            .join("nowcast/runs/2026-07-15T11:00:00+00:00")
            .exists()
    );
    assert!(
        root.path()
            .join("nowcast/runs/2026-07-15T12:00:00+00:00")
            .exists()
    );
    assert!(
        in_progress.exists(),
        "in-progress generations are never removed here"
    );
}

#[test]
fn d11_cleanup_sweeps_old_run_directories() {
    let root = tempfile::tempdir().unwrap();
    let mut cfg = GridConfig::with_dir(root.path());
    cfg.max_age_s = 3600;
    let stale = root.path().join("hrrr/runs/2026-01-01T00:00:00+00:00");
    std::fs::create_dir_all(&stale).unwrap();
    std::fs::write(stale.join("a.bin"), [0u8; 4]).unwrap();
    std::fs::write(stale.join("a.meta.json"), "{}").unwrap();
    set_mtime_old(&stale.join("a.bin"), Duration::from_secs(7200));
    set_mtime_old(&stale, Duration::from_secs(7200));
    let fresh = root.path().join("hrrr/runs/2026-06-01T00:00:00+00:00");
    std::fs::create_dir_all(&fresh).unwrap();
    std::fs::write(fresh.join("b.bin"), [0u8; 4]).unwrap();
    let removed = cleanup_old_grids(&cfg).unwrap();
    assert_eq!(removed, 2);
    assert!(!stale.exists());
    assert!(fresh.exists());
}

#[test]
fn guard_is_exclusive_and_grid_lock_file_is_created() {
    let root = tempfile::tempdir().unwrap();
    let guard = FlockGuard::exclusive(&root.path().join(LAYER_LOCK_FILE)).unwrap();
    drop(guard);
    assert!(root.path().join(LAYER_LOCK_FILE).exists());
}

/// Byte-exact replay of `testdata/grid/scenarios.json`. Every file the Python writer produced must match
/// byte for byte, and the Rust output must contain no extra files.
#[test]
fn golden_grid_outputs_match_python_bytes() {
    let scenarios: Vec<Value> =
        serde_json::from_str(include_str!("../../testdata/grid/scenarios.json")).unwrap();
    let golden_root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("testdata/grid/expected");
    for scenario in scenarios {
        let name = scenario["name"].as_str().unwrap();
        let tmp = tempfile::tempdir().unwrap();
        let grids = tmp.path().join("grids");
        let cfg = GridConfig::with_dir(&grids);
        for op in scenario["ops"].as_array().unwrap() {
            match op["op"].as_str().unwrap() {
                "write" | "write_expect_none" => {
                    let rows: Vec<Vec<Option<f64>>> =
                        serde_json::from_value(op["data"].clone()).unwrap();
                    let height = rows.len();
                    let width = rows[0].len();
                    let data: Vec<f32> = rows
                        .iter()
                        .flatten()
                        .map(|v| v.map(|x| x as f32).unwrap_or(f32::NAN))
                        .collect();
                    let lats: Vec<f64> = serde_json::from_value(op["lats"].clone()).unwrap();
                    let lons: Vec<f64> = serde_json::from_value(op["lons"].clone()).unwrap();
                    assert_eq!((height, width), (rows.len(), rows[0].len()));
                    let fill = op["fill"].as_f64().unwrap_or(f64::NAN);
                    let max_cells = op["max_cells"].as_u64().map(|m| m as usize);
                    let grid_key = op["grid_key"].as_str();
                    let input = GridInput {
                        data: &data,
                        lats: &lats,
                        lons: &lons,
                        unit: op["unit"].as_str().unwrap(),
                        fill,
                        max_cells,
                        grid_key,
                    };
                    let generation = op["generation"].as_str().unwrap();
                    let result = write_grid_with(
                        &cfg,
                        op["layer"].as_str().unwrap(),
                        op["timestamp"].as_str().unwrap_or(""),
                        &input,
                        &fsutil::fsync_dir,
                        generation,
                    )
                    .unwrap();
                    if op["op"] == "write_expect_none" {
                        assert!(result.is_none(), "{name}");
                    } else {
                        assert!(result.is_some(), "{name}");
                    }
                }
                "finalize" => {
                    finalize_grid_generation(
                        &cfg,
                        op["layer"].as_str().unwrap(),
                        op["generation"].as_str().unwrap(),
                    )
                    .unwrap();
                }
                other => panic!("unknown op {other}"),
            }
        }
        let expected = golden_root.join(name);
        let mut expected_files = collect_files(&expected);
        let mut actual_files = collect_files(&grids);
        expected_files.sort();
        actual_files.sort();
        assert_eq!(actual_files, expected_files, "{name}: file set differs");
        for rel in &expected_files {
            let want = std::fs::read(expected.join(rel)).unwrap();
            let got = std::fs::read(grids.join(rel)).unwrap();
            assert_eq!(got, want, "{name}: bytes differ in {rel}");
        }
    }
}

/// Relative file paths under `root` (empty when `root` does not exist). The writer's empty-layer case creates no
/// files, so an absent golden directory and an absent output directory are equal.
fn collect_files(root: &Path) -> Vec<String> {
    let mut out = Vec::new();
    if !root.exists() {
        return out;
    }
    for entry in walkdir::WalkDir::new(root).min_depth(1) {
        let entry = entry.unwrap();
        if entry.file_type().is_file() {
            out.push(
                entry
                    .path()
                    .strip_prefix(root)
                    .unwrap()
                    .to_string_lossy()
                    .into_owned(),
            );
        }
    }
    out
}
