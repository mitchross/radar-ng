//! HRRR pure-logic invariants (spec §14.2): scratch names, publishable prefix, horizons, resume rule,
//! extended tail, needed variables, and grid-relative wind rotation.

use chrono::{TimeZone, Utc};
use radar_core::manifest::parse_manifest;
use radar_geo::{Crs, Grid, GridGeo};
use radar_ingest::hrrr::dump::{Resampler, target_axes};
use radar_ingest::hrrr::state;
use radar_ingest::scratch::{ActivityInfo, ScratchDir, dir_name, sanitize};
use radar_tiler::palettes::load_palette;
use radar_types::payloads::hrrr::ForecastHourResult;

fn hour(fhr: i64, layers: &[&str]) -> ForecastHourResult {
    ForecastHourResult {
        fhr,
        rendered_layers: layers.iter().map(|s| s.to_string()).collect(),
        duration_s: 0.0,
        valid_timestamp: String::new(),
    }
}

/// H3: scratch paths differ across attempts and activity ids, and sit under the root.
#[test]
fn h3_scratch_paths_distinct_and_under_root() {
    let root = tempfile::tempdir().unwrap();
    let a1 = ActivityInfo {
        workflow_id: "w".into(),
        run_id: "r".into(),
        activity_id: "3".into(),
        attempt: 1,
    };
    let a2 = ActivityInfo {
        attempt: 2,
        ..a1.clone()
    };
    let a3 = ActivityInfo {
        activity_id: "4".into(),
        ..a1.clone()
    };
    let d1 = ScratchDir::create(root.path(), "hrrr", &a1, &["run", "f01"]).unwrap();
    let d2 = ScratchDir::create(root.path(), "hrrr", &a2, &["run", "f01"]).unwrap();
    let d3 = ScratchDir::create(root.path(), "hrrr", &a3, &["run", "f01"]).unwrap();
    assert_ne!(d1.path(), d2.path());
    assert_ne!(d1.path(), d3.path());
    for d in [&d1, &d2, &d3] {
        assert_eq!(d.path().parent().unwrap(), root.path());
    }
}

/// H4: sanitisation of workflow ids, and the exact directory name from the spec example.
#[test]
fn h4_scratch_names_sanitised() {
    assert_eq!(sanitize("sched/ingest:hrrr"), "sched_ingest_hrrr");
    assert_eq!(sanitize("run+one"), "run_one");
    assert_eq!(sanitize("///"), "unknown");
    let info = ActivityInfo {
        workflow_id: "sched/ingest:hrrr".into(),
        run_id: "run+one".into(),
        activity_id: "3".into(),
        attempt: 1,
    };
    let name = dir_name("hrrr", &info, &["20260506_02", "f01"]);
    assert_eq!(
        name,
        "hrrr-sched_ingest_hrrr-run_one-3-attempt1-20260506_02-f01"
    );
    assert!(!name.contains('/'));
}

/// H8: publishable prefix.
#[test]
fn h8_publishable_prefix() {
    assert!(state::publishable_prefix(&[]).is_empty());
    assert!(state::publishable_prefix(&[hour(2, &["radar-hrrr"])]).is_empty());
    let results = vec![
        hour(3, &["radar-hrrr"]),
        hour(1, &["radar-hrrr"]),
        hour(2, &["temperature"]),
        hour(4, &["radar-hrrr"]),
    ];
    let prefix: Vec<i64> = state::publishable_prefix(&results)
        .iter()
        .map(|r| r.fhr)
        .collect();
    assert_eq!(prefix, vec![1]);
}

/// H9: horizon for extended and regular runs.
#[test]
fn h9_horizon_for_run() {
    assert_eq!(
        radar_ingest::hrrr::horizon_for("20260715_12", 18, 48).unwrap(),
        48
    );
    assert_eq!(
        radar_ingest::hrrr::horizon_for("20260715_13", 18, 48).unwrap(),
        18
    );
}

fn manifest_with(layer_json: &str) -> radar_core::manifest::Manifest {
    let text = format!("{{\"schema_version\": 2, \"layers\": {{\"radar-hrrr\": {layer_json}}}}}");
    parse_manifest(&text).unwrap()
}

/// H20: resume rule.
#[test]
fn h20_resume_rule() {
    let init = Utc.with_ymd_and_hms(2026, 7, 15, 12, 0, 0).unwrap();
    let young = init + chrono::Duration::minutes(30);
    let old = init + chrono::Duration::hours(3) + chrono::Duration::minutes(1);

    let partial = manifest_with(
        r#"{"run_id": "20260715_12", "complete": false, "frames": [], "timestamps": []}"#,
    );
    assert_eq!(
        state::resume_incomplete_run(&partial, young).as_deref(),
        Some("20260715_12")
    );
    assert_eq!(state::resume_incomplete_run(&partial, old), None);

    let complete = manifest_with(
        r#"{"run_id": "20260715_12", "complete": true, "frames": [], "timestamps": []}"#,
    );
    assert_eq!(state::resume_incomplete_run(&complete, young), None);

    let missing = manifest_with(r#"{"run_id": "20260715_12", "frames": [], "timestamps": []}"#);
    assert_eq!(
        state::resume_incomplete_run(&missing, young),
        None,
        "absent complete means complete"
    );

    let bogus = manifest_with(
        r#"{"run_id": "not-a-run", "complete": false, "frames": [], "timestamps": []}"#,
    );
    assert_eq!(state::resume_incomplete_run(&bogus, young), None);

    let empty = parse_manifest("{\"schema_version\": 2, \"layers\": {}}").unwrap();
    assert_eq!(state::resume_incomplete_run(&empty, young), None);
}

/// H22: the extended tail keeps only older extended runs that are valid after the last valid time.
#[test]
fn h22_extended_tail_ignores_non_extended_runs() {
    let manifest = parse_manifest(
        r#"{"schema_version": 2, "layers": {"radar-hrrr": {"complete": true, "frames": [
            {"timestamp": "2026-07-15T23:00:00+00:00", "path": "runs/20260715_12/2026-07-15T23:00:00+00:00", "run_id": "20260715_12", "palettes": ["classic"]},
            {"timestamp": "2026-07-15T20:00:00+00:00", "path": "runs/20260715_13/2026-07-15T20:00:00+00:00", "run_id": "20260715_13", "palettes": ["classic"]}
        ], "timestamps": []}}}"#,
    )
    .unwrap();
    let layer = manifest.layers.get("radar-hrrr");
    // Current run 14z (init 14:00): the 12z frame after 13:00 stays; the 13z frame is not extended.
    let tail = state::extended_tail(layer, "20260715_14", "2026-07-15T14:00:00+00:00");
    let stamps: Vec<&str> = tail.iter().map(|f| f.timestamp.as_str()).collect();
    assert_eq!(stamps, vec!["2026-07-15T23:00:00+00:00"]);
}

/// H23: needed variables and the palette color keys a layer needs.
#[test]
fn h23_needed_vars_and_required_layers() {
    let both = state::needed_vars(&["radar-hrrr".into(), "wind".into()]);
    let keys: Vec<&str> = both.iter().map(|v| v.key()).collect();
    assert_eq!(keys, vec!["refc", "u10", "v10"]);
    let only_radar = state::needed_vars(&["radar-hrrr".into()]);
    assert_eq!(
        only_radar.iter().map(|v| v.key()).collect::<Vec<_>>(),
        vec!["refc"]
    );

    let classic = load_palette("classic").unwrap();
    let enabled = ["radar-hrrr", "wind"];
    let required: Vec<&str> = enabled
        .into_iter()
        .filter(|l| state::layer_color_key(l).is_some_and(|k| classic.color_keys().any(|c| c == k)))
        .collect();
    assert_eq!(required, vec!["radar-hrrr", "wind"]);
}

/// Lambert conformal source grid over CONUS (HRRR parameters), with meshes from the projection.
fn lcc_grid() -> Grid {
    let crs = Crs::LambertConformal {
        lat_1: 38.5,
        lat_2: 38.5,
        lat_0: 38.5,
        lon_0: -97.5,
        radius_m: 6_371_229.0,
    };
    let (nx, ny) = (260usize, 170usize);
    let x: Vec<f64> = (0..nx).map(|i| -2.6e6 + i as f64 * 20_000.0).collect();
    let y: Vec<f64> = (0..ny).map(|j| -1.7e6 + j as f64 * 20_000.0).collect();
    let mut lat_mesh = Vec::with_capacity(nx * ny);
    let mut lon_mesh = Vec::with_capacity(nx * ny);
    for &yy in &y {
        for &xx in &x {
            let (lon, lat) = crs.inverse(xx, yy);
            lon_mesh.push(lon);
            lat_mesh.push(lat);
        }
    }
    Grid {
        width: nx,
        height: ny,
        values: vec![0.0; nx * ny],
        geo: GridGeo::Projected {
            crs,
            x,
            y,
            lat_mesh,
            lon_mesh,
        },
    }
}

fn probe(u: f32, v: f32, grid_relative: bool) -> (f32, f32) {
    let grid = lcc_grid();
    let GridGeo::Projected { crs, x, y, .. } = &grid.geo else {
        unreachable!()
    };
    let r = Resampler::new(crs, x, y, grid.height, grid.width, &[40.0], &[-85.0]).unwrap();
    let uu = vec![u; grid.values.len()];
    let vv = vec![v; grid.values.len()];
    let (wu, wv) = r.wind_components(&uu, &vv, grid_relative);
    (wu[0], wv[0])
}

/// H24: grid-relative (1, 0) rotates to the convergence angle of the Lambert grid, within 0.5 degree.
#[test]
fn h24_grid_relative_winds_rotate_by_convergence() {
    let (wu, wv) = probe(1.0, 0.0, true);
    let speed = (wu * wu + wv * wv).sqrt();
    assert!((speed - 1.0).abs() < 1e-3, "speed {speed}");
    let angle = (wv as f64).atan2(wu as f64).to_degrees();
    // Spec H24: radians(-sin(radians(38.5)) * (lon + 97.5)); compare in degrees.
    let expected = -(38.5f64.to_radians().sin()) * (-85.0 + 97.5);
    assert!(
        (angle - expected).abs() < 0.5,
        "angle {angle} vs expected {expected}"
    );
}

/// H25: earth-relative (not rotated) winds pass through unchanged.
#[test]
fn h25_unrotated_winds_are_resampled_only() {
    let (wu, wv) = probe(1.0, 0.0, false);
    assert!((wu - 1.0).abs() < 1e-6);
    assert!(wv.abs() < 1e-6);
}

/// Target axes from a projected grid cover its meshes and end before the maximum (numpy arange).
#[test]
fn target_axes_follow_numpy_arange() {
    let grid = lcc_grid();
    let (lats, lons) = target_axes(&grid, 0.5).unwrap();
    assert!(!lats.is_empty() && !lons.is_empty());
    assert!(lats.windows(2).all(|w| (w[1] - w[0] - 0.5).abs() < 1e-9));
    assert!(lons.windows(2).all(|w| (w[1] - w[0] - 0.5).abs() < 1e-9));
}
