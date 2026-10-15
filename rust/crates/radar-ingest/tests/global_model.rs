//! Global-model activities (`radar-gfs`, `uv`) against the fake bucket and synthetic 2.5 degree global
//! GRIB2 files laid out like GFS: north first, longitudes 0..357.5.

mod common;

use chrono::{TimeZone, Utc};
use radar_core::env::Paths;
use radar_core::grid_read::{Sample, read_grid};
use radar_core::manifest::read_manifest;
use radar_ingest::global_model::{Product, activities as act};
use radar_ingest::{IngestConfig, IngestCtx, heartbeat};
use radar_tiler::marker::{PYRAMID_COMPLETE_FILE, list_tile_paths};
use tempfile::TempDir;

const RUN: &str = "20261015_12";

/// One GRIB2 message: template 3.0 global 2.5 degree grid, 4.0 product, 5.0 simple packing (D = 3).
fn message(
    category: u8,
    number: u8,
    surface: u8,
    fhr: u32,
    field: impl Fn(f64, f64) -> f64,
) -> Vec<u8> {
    let (ni, nj) = (144u32, 73u32);
    let n = (ni * nj) as usize;
    let mut values = Vec::with_capacity(n);
    for j in 0..nj {
        let lat = 90.0 - 2.5 * f64::from(j);
        for i in 0..ni {
            let lon = 2.5 * f64::from(i);
            values.push(field(lat, if lon > 180.0 { lon - 360.0 } else { lon }) * 1000.0);
        }
    }
    let r = values.iter().cloned().fold(f64::INFINITY, f64::min);
    let sm = |v: i32| -> [u8; 4] {
        let m = v.unsigned_abs();
        (if v < 0 { m | 0x8000_0000 } else { m }).to_be_bytes()
    };
    let mut s1 = vec![0, 0, 0, 21, 1, 0, 7, 0, 0, 2, 1, 1];
    s1.extend_from_slice(&2026u16.to_be_bytes());
    s1.extend_from_slice(&[10, 15, 12, 0, 0, 0, 1]);
    let mut s3 = vec![0, 0, 0, 72, 3, 0];
    s3.extend_from_slice(&(ni * nj).to_be_bytes());
    s3.extend_from_slice(&[0, 0, 0, 0, 6, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0]);
    s3.extend_from_slice(&ni.to_be_bytes());
    s3.extend_from_slice(&nj.to_be_bytes());
    s3.extend_from_slice(&[0; 8]);
    s3.extend_from_slice(&sm(90_000_000));
    s3.extend_from_slice(&0u32.to_be_bytes());
    s3.push(48);
    s3.extend_from_slice(&sm(-90_000_000));
    s3.extend_from_slice(&357_500_000u32.to_be_bytes());
    s3.extend_from_slice(&2_500_000u32.to_be_bytes());
    s3.extend_from_slice(&2_500_000u32.to_be_bytes());
    s3.push(0);
    assert_eq!(s3.len(), 72);
    let mut s4 = vec![
        0, 0, 0, 34, 4, 0, 0, 0, 0, category, number, 2, 0, 96, 0, 0, 0, 1,
    ];
    s4.extend_from_slice(&fhr.to_be_bytes());
    s4.extend_from_slice(&[surface, 0, 0, 0, 0, 0, 255, 255, 255, 255, 255, 255]);
    let mut s5 = vec![0, 0, 0, 21, 5];
    s5.extend_from_slice(&(ni * nj).to_be_bytes());
    s5.extend_from_slice(&[0, 0]);
    s5.extend_from_slice(&(r as f32).to_be_bytes());
    s5.extend_from_slice(&[0, 0, 0, 3, 16, 0]);
    let s6 = vec![0, 0, 0, 6, 6, 255];
    let mut s7 = ((5 + 2 * n) as u32).to_be_bytes().to_vec();
    s7.push(7);
    for v in &values {
        s7.extend_from_slice(&(((v - r).round()) as u16).to_be_bytes());
    }
    let body: Vec<u8> = [s1, s3, s4, s5, s6, s7].concat();
    let total = (16 + body.len() + 4) as u64;
    let mut out = b"GRIB\0\0\0\x02".to_vec();
    out.extend_from_slice(&total.to_be_bytes());
    out.extend_from_slice(&body);
    out.extend_from_slice(b"7777");
    out
}

/// A storm box over the central US: 35 dBZ inside, -20 elsewhere.
fn storm(lat: f64, lon: f64) -> f64 {
    if (30.0..=45.0).contains(&lat) && (-100.0..=-80.0).contains(&lon) {
        35.0
    } else {
        -20.0
    }
}

struct Harness {
    _root: TempDir,
    paths: Paths,
    s3: common::FakeS3,
    ctx: IngestCtx,
}

async fn harness(edit: impl FnOnce(&mut IngestConfig)) -> Harness {
    let root = tempfile::tempdir().unwrap();
    let paths = Paths::new(
        root.path().join("state"),
        root.path().join("grids"),
        root.path().join("tiles"),
    );
    let s3 = common::start().await;
    let mut cfg = IngestConfig::with_roots(paths.clone(), root.path().join("work")).unwrap();
    cfg.gfs_base_url = s3.base.clone();
    cfg.uv_base_url = s3.base.clone();
    edit(&mut cfg);
    let ctx = IngestCtx::new(cfg, None).unwrap();
    Harness {
        _root: root,
        paths,
        s3,
        ctx,
    }
}

/// A GFS hour: a TMP message the index does not select, then REFC.
fn put_gfs_hour(s3: &common::FakeS3, fhr: u32) {
    let tmp = message(0, 0, 1, fhr, |_, _| 50.0);
    let refc = message(16, 196, 10, fhr, storm);
    let idx = format!(
        "1:0:d=2026101512:TMP:surface:{fhr} hour fcst:\n2:{}:d=2026101512:REFC:entire atmosphere:{fhr} hour fcst:\n",
        tmp.len()
    );
    let key = format!("gfs.20261015/12/atmos/gfs.t12z.pgrb2.0p25.f{fhr:03}");
    s3.put(&key, [tmp, refc].concat());
    s3.put(&format!("{key}.idx"), idx.into_bytes());
}

fn sample(h: &Harness, layer: &str, valid: &str, lat: f64, lon: f64) -> Sample {
    let meta = h
        .paths
        .grid_dir
        .join(layer)
        .join(format!("{valid}.meta.json"));
    read_grid(&meta).unwrap().sample(lat, lon).unwrap()
}

#[tokio::test]
async fn gfs_run_renders_publishes_and_resumes() {
    let h = harness(|c| {
        c.gfs_radar_first_hour = 24;
        c.gfs_radar_last_hour = 25;
    })
    .await;
    put_gfs_hour(&h.s3, 24);
    put_gfs_hour(&h.s3, 25);
    let now = Utc.with_ymd_and_hms(2026, 10, 15, 20, 0, 0).unwrap();
    let find = act::find_latest_run(&h.ctx, Product::GfsRadar, now)
        .await
        .unwrap();
    assert_eq!(find.run_id.as_deref(), Some(RUN));
    assert_eq!(find.hours, [24, 25]);
    assert!(!find.already_processed);

    let hb = heartbeat::noop();
    let mut results = Vec::new();
    for fhr in [24, 25] {
        let r = act::process_hour(&h.ctx, Product::GfsRadar, RUN, fhr, &hb)
            .await
            .unwrap();
        assert_eq!(r.rendered_layers, ["radar-gfs"]);
        results.push(r);
    }
    // Only the REFC record is fetched, by range: TMP (50 everywhere) would paint the whole box.
    let valid = "2026-10-16T12:00:00+00:00";
    assert_eq!(
        sample(&h, "radar-gfs", valid, 37.5, -90.0),
        Sample::Value(35.0)
    );
    assert_eq!(
        sample(&h, "radar-gfs", valid, 50.0, -120.0),
        Sample::Value(-20.0)
    );
    let frame = h
        .paths
        .tile_layer_dir("radar-gfs")
        .join("classic/runs")
        .join(RUN)
        .join(valid);
    let z4: Vec<String> = list_tile_paths(&frame)
        .unwrap()
        .into_iter()
        .filter(|p| p.starts_with("4/"))
        .collect();
    assert!(!z4.is_empty() && z4.len() <= 6, "{z4:?}");

    let published = act::publish_run(&h.ctx, Product::GfsRadar, RUN, &results)
        .await
        .unwrap();
    assert_eq!(published, 2);
    let manifest = read_manifest(&h.paths.state_dir).unwrap();
    let layer = &manifest.layers["radar-gfs"];
    assert_eq!(layer.frames.len(), 2);
    assert_eq!(layer.extra["run_id"], RUN);
    assert_eq!(layer.extra["complete"], true);
    assert_eq!(layer.frames[1].extra["source"], "gfs");

    // A second pass finds complete pyramids and fetches nothing.
    let before = h.s3.requests().len();
    let again = act::process_hour(&h.ctx, Product::GfsRadar, RUN, 24, &hb)
        .await
        .unwrap();
    assert_eq!(again.rendered_layers, ["radar-gfs"]);
    assert_eq!(h.s3.requests().len(), before);

    act::mark_processed(&h.ctx, Product::GfsRadar, RUN)
        .await
        .unwrap();
    let find = act::find_latest_run(&h.ctx, Product::GfsRadar, now)
        .await
        .unwrap();
    assert!(find.already_processed);
}

#[tokio::test]
async fn gfs_hour_without_index_is_pending_not_a_full_download() {
    let h = harness(|_| {}).await;
    let hb = heartbeat::noop();
    let r = act::process_hour(&h.ctx, Product::GfsRadar, RUN, 30, &hb)
        .await
        .unwrap();
    assert!(r.rendered_layers.is_empty());
    assert!(
        h.s3.requests().iter().all(|r| r.ends_with(".idx")),
        "{:?}",
        h.s3.requests()
    );
}

#[tokio::test]
async fn uv_converts_to_index_and_keeps_night_hours_as_empty_frames() {
    let h = harness(|c| c.uv_last_hour = 2).await;
    // f01: night everywhere. f02: 0.2 W/m2 (UV Index 8) over the box.
    h.s3.put(
        "uvi.20261015/uv.t12z.grbf01.grib2",
        message(7, 196, 1, 1, |_, _| 0.0),
    );
    h.s3.put(
        "uvi.20261015/uv.t12z.grbf02.grib2",
        message(
            7,
            196,
            1,
            2,
            |la, lo| if storm(la, lo) > 0.0 { 0.2 } else { 0.0 },
        ),
    );
    let now = Utc.with_ymd_and_hms(2026, 10, 15, 19, 0, 0).unwrap();
    let find = act::find_latest_run(&h.ctx, Product::Uv, now)
        .await
        .unwrap();
    assert_eq!(find.run_id.as_deref(), Some(RUN));
    assert_eq!(find.hours, [1, 2]);

    let hb = heartbeat::noop();
    let mut results = Vec::new();
    for fhr in [1, 2] {
        let r = act::process_hour(&h.ctx, Product::Uv, RUN, fhr, &hb)
            .await
            .unwrap();
        assert_eq!(r.rendered_layers, ["uv"], "f{fhr:02}");
        results.push(r);
    }
    let classic = h.paths.tile_layer_dir("uv").join("classic/runs").join(RUN);
    let night = classic.join("2026-10-15T13:00:00+00:00");
    assert!(night.join(PYRAMID_COMPLETE_FILE).is_file());
    assert!(list_tile_paths(&night).unwrap().is_empty());
    assert!(
        !list_tile_paths(&classic.join("2026-10-15T14:00:00+00:00"))
            .unwrap()
            .is_empty()
    );
    let Sample::Value(uvi) = sample(&h, "uv", "2026-10-15T14:00:00+00:00", 37.5, -90.0) else {
        panic!("no UV value");
    };
    assert!((uvi - 8.0).abs() < 1e-3, "{uvi}");

    assert_eq!(
        act::publish_run(&h.ctx, Product::Uv, RUN, &results)
            .await
            .unwrap(),
        2
    );
    let manifest = read_manifest(&h.paths.state_dir).unwrap();
    assert_eq!(manifest.layers["uv"].frames.len(), 2);
}

#[tokio::test]
async fn disabled_product_finds_nothing_and_asks_nothing() {
    let h = harness(|c| c.gfs_radar_enabled = false).await;
    let find = act::find_latest_run(&h.ctx, Product::GfsRadar, Utc::now())
        .await
        .unwrap();
    assert_eq!(find.run_id, None);
    assert!(h.s3.requests().is_empty());
}

#[tokio::test]
async fn cleanup_keeps_live_and_recent_runs() {
    let h = harness(|c| {
        c.gfs_radar_first_hour = 24;
        c.gfs_radar_last_hour = 24;
    })
    .await;
    put_gfs_hour(&h.s3, 24);
    let hb = heartbeat::noop();
    let r = act::process_hour(&h.ctx, Product::GfsRadar, RUN, 24, &hb)
        .await
        .unwrap();
    act::publish_run(&h.ctx, Product::GfsRadar, RUN, &[r])
        .await
        .unwrap();
    let runs = h.paths.tile_layer_dir("radar-gfs").join("classic/runs");
    std::fs::create_dir_all(runs.join("20261014_00")).unwrap();
    std::fs::create_dir_all(runs.join("20261015_06")).unwrap();
    let cutoff = Utc.with_ymd_and_hms(2026, 10, 15, 0, 0, 0).unwrap();
    let res = act::cleanup_sync(&h.ctx.cfg, Product::GfsRadar, cutoff).unwrap();
    assert_eq!(res.tile_dirs_removed, 1);
    assert!(!runs.join("20261014_00").exists());
    assert!(runs.join("20261015_06").exists());
    assert!(runs.join(RUN).exists());
}
