//! MRMS activities end to end against the fake bucket and the GRIB fixture (spec §14.1, M1 to M8).

mod common;

use std::sync::{Arc, Mutex};
use std::time::Duration;

use common::{MRMS_KEY, MRMS_PREFIX, MRMS_VALID};
use radar_core::env::Paths;
use radar_grib::mrms as gmrms;
use radar_ingest::heartbeat;
use radar_ingest::mrms;
use radar_ingest::scratch::ActivityInfo;
use radar_ingest::{IngestConfig, IngestCtx, NowcastKicker};
use radar_types::payloads::mrms::{
    CleanupInput, IngestMrmsArgs, MarkProcessedInput, ProcessFrameInput,
};
use tempfile::TempDir;

/// Records every kick and can be told to fail.
#[derive(Default)]
struct Recorder {
    calls: Mutex<Vec<(String, String, Duration)>>,
    fail: bool,
}

#[async_trait::async_trait]
impl NowcastKicker for Recorder {
    async fn trigger(
        &self,
        schedule_id: &str,
        timestamp: &str,
        timeout: Duration,
    ) -> anyhow::Result<()> {
        self.calls
            .lock()
            .unwrap()
            .push((schedule_id.into(), timestamp.into(), timeout));
        if self.fail {
            anyhow::bail!("temporal unreachable");
        }
        Ok(())
    }
}

struct Harness {
    _root: TempDir,
    paths: Paths,
    s3: common::FakeS3,
    ctx: IngestCtx,
    recorder: Arc<Recorder>,
}

async fn harness(kick: bool, kicker_fails: bool) -> Harness {
    let root = tempfile::tempdir().unwrap();
    let paths = Paths::new(
        root.path().join("state"),
        root.path().join("grids"),
        root.path().join("tiles"),
    );
    let s3 = common::start().await;
    s3.put(MRMS_KEY, common::mrms_gz());
    s3.bucket
        .lock()
        .unwrap()
        .list_keys
        .push(MRMS_KEY.to_string());
    let mut cfg = IngestConfig::with_roots(paths.clone(), root.path().join("work")).unwrap();
    cfg.mrms_base_url = s3.base.clone();
    cfg.nowcast_kick_enabled = kick;
    let recorder = Arc::new(Recorder {
        calls: Mutex::new(Vec::new()),
        fail: kicker_fails,
    });
    let ctx = IngestCtx::new(cfg, Some(recorder.clone() as Arc<dyn NowcastKicker>)).unwrap();
    Harness {
        _root: root,
        paths,
        s3,
        ctx,
        recorder,
    }
}

fn info() -> ActivityInfo {
    ActivityInfo {
        workflow_id: "sched/ingest:mrms".into(),
        run_id: "run-1".into(),
        activity_id: "2".into(),
        attempt: 1,
    }
}

fn frame_input(layer: &str) -> ProcessFrameInput {
    ProcessFrameInput {
        key: MRMS_KEY.into(),
        layer_name: layer.into(),
    }
}

fn dir_is_empty_or_missing(p: &std::path::Path) -> bool {
    !p.exists() || std::fs::read_dir(p).unwrap().next().is_none()
}

/// M1: regular lat/lon axes of the fixture (35 x 70). Longitudes normalised to -180..180.
#[test]
fn m1_regular_axes_from_header() {
    let grid = gmrms::decode_gz(&common::mrms_gz()).unwrap();
    let radar_geo::GridGeo::Regular { lats, lons } = &grid.geo else {
        panic!("expected regular")
    };
    assert_eq!(lats.len(), 35);
    assert_eq!(lons.len(), 70);
    assert!((lats[0] - 54.0).abs() < 1e-9);
    assert!((lats[34] - 20.0).abs() < 1e-9);
    assert!((lons[0] - (-130.0)).abs() < 1e-9);
    assert!((lons[69] - (-61.0)).abs() < 1e-9);
    assert!(
        (lats[1] - lats[0] + 1.0).abs() < 1e-9,
        "lat step is (20-54)/34 = -1"
    );
}

/// M8: one palette renders, the frame is marked rendered and every output is written.
#[tokio::test]
async fn m8_process_frame_publishes_tiles_grids_storms_and_manifest() {
    let h = harness(false, false).await;
    let out = mrms::process_frame(&h.ctx, &info(), &frame_input("radar"), &heartbeat::noop())
        .await
        .unwrap();
    assert!(out.rendered, "frame should render");
    assert_eq!(out.timestamp, MRMS_VALID);
    assert_eq!(out.palettes, vec!["classic".to_string()]);

    let tile_root = h
        .paths
        .tile_layer_dir("radar")
        .join("classic")
        .join(MRMS_VALID);
    assert!(
        tile_root.join(".render-complete-v2.json").is_file(),
        "completion marker"
    );
    assert!(tile_root.join("4").is_dir(), "zoom 4 tiles");
    assert!(
        h.paths
            .grid_layer_dir("radar")
            .join(format!("{MRMS_VALID}.meta.json"))
            .is_file(),
        "grid pointer"
    );
    assert!(
        h.paths
            .grid_layer_dir("radar-nowcast-input")
            .join(format!("{MRMS_VALID}.meta.json"))
            .is_file()
    );

    let storms: serde_json::Value =
        serde_json::from_slice(&std::fs::read(h.paths.state_dir.join("storms.json")).unwrap())
            .unwrap();
    assert_eq!(storms["timestamp"], MRMS_VALID);

    let manifest = radar_core::manifest::read_manifest(&h.paths.state_dir).unwrap();
    let layer = &manifest.layers["radar"];
    assert_eq!(layer.timestamps, vec![MRMS_VALID.to_string()]);
    assert_eq!(layer.frames[0].palettes, vec!["classic".to_string()]);
    assert_eq!(layer.frames[0].extra["source"], "mrms");
    assert_eq!(layer.frames[0].extra["max_zoom"], 7);
    assert!(
        dir_is_empty_or_missing(&h.ctx.cfg.mrms_tmp_root),
        "scratch removed after success (D3)"
    );
}

/// M6 and M7: only the `radar` frame kicks the nowcast, once, with the frame's valid time.
#[tokio::test]
async fn m6_m7_kick_only_for_radar_layer() {
    let h = harness(true, false).await;
    mrms::process_frame(&h.ctx, &info(), &frame_input("radar"), &heartbeat::noop())
        .await
        .unwrap();
    let calls = h.recorder.calls.lock().unwrap().clone();
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0].0, "nowcast");
    assert_eq!(calls[0].1, MRMS_VALID);

    let h2 = harness(true, false).await;
    mrms::process_frame(
        &h2.ctx,
        &info(),
        &frame_input("radar-composite"),
        &heartbeat::noop(),
    )
    .await
    .unwrap();
    assert!(
        h2.recorder.calls.lock().unwrap().is_empty(),
        "composite never kicks"
    );
    assert!(
        !h2.paths.grid_layer_dir("radar-nowcast-input").exists(),
        "no nowcast input for composite"
    );
    assert!(
        !h2.paths.state_dir.join("storms.json").exists(),
        "no storms for composite"
    );
}

/// A failing kicker never fails the frame (spec M4 inside the activity).
#[tokio::test]
async fn kick_failure_does_not_fail_the_frame() {
    let h = harness(true, true).await;
    let out = mrms::process_frame(&h.ctx, &info(), &frame_input("radar"), &heartbeat::noop())
        .await
        .unwrap();
    assert!(out.rendered);
    assert_eq!(h.recorder.calls.lock().unwrap().len(), 1);
}

/// M5: kick disabled never calls the kicker.
#[tokio::test]
async fn m5_disabled_kick_never_calls_kicker() {
    let h = harness(false, false).await;
    assert!(!radar_ingest::ctx::kick_nowcast(&h.ctx, MRMS_VALID).await);
    assert!(h.recorder.calls.lock().unwrap().is_empty());
}

/// M3: enabled kick triggers the nowcast schedule once with the configured timeout and returns true.
#[tokio::test]
async fn m3_enabled_kick_triggers_once() {
    let h = harness(true, false).await;
    assert!(radar_ingest::ctx::kick_nowcast(&h.ctx, MRMS_VALID).await);
    let calls = h.recorder.calls.lock().unwrap().clone();
    assert_eq!(
        calls,
        vec![(
            "nowcast".to_string(),
            MRMS_VALID.to_string(),
            Duration::from_secs(5)
        )]
    );
}

/// M4: a failing kicker returns false and does not raise.
#[tokio::test]
async fn m4_failing_kick_returns_false() {
    let h = harness(true, true).await;
    assert!(!radar_ingest::ctx::kick_nowcast(&h.ctx, MRMS_VALID).await);
}

/// List, process, mark, then list again: the processed key leaves the backlog.
#[tokio::test]
async fn list_process_mark_then_backlog_is_empty() {
    let h = harness(false, false).await;
    let args = IngestMrmsArgs {
        mrms_prefix: MRMS_PREFIX.into(),
        layer_name: "radar".into(),
    };
    let listed = mrms::list_unprocessed_keys(&h.ctx, &args).await.unwrap();
    assert_eq!(listed.keys, vec![MRMS_KEY.to_string()]);
    assert_eq!(listed.backlog_total, 1);

    mrms::process_frame(&h.ctx, &info(), &frame_input("radar"), &heartbeat::noop())
        .await
        .unwrap();
    mrms::mark_processed(
        &h.ctx,
        &MarkProcessedInput {
            key: MRMS_KEY.into(),
            layer_name: "radar".into(),
        },
    )
    .await
    .unwrap();
    let after = mrms::list_unprocessed_keys(&h.ctx, &args).await.unwrap();
    assert!(after.keys.is_empty());
    assert_eq!(after.backlog_total, 0);
}

/// D21: a malformed key fails before any download.
#[tokio::test]
async fn d21_bad_key_fails_without_download() {
    let h = harness(false, false).await;
    let bad = ProcessFrameInput {
        key: "CONUS/x/20260930/MRMS_A_notatime.grib2.gz".into(),
        layer_name: "radar".into(),
    };
    assert!(
        mrms::process_frame(&h.ctx, &info(), &bad, &heartbeat::noop())
            .await
            .is_err()
    );
    assert!(
        h.s3.requests().iter().all(|r| !r.starts_with("GET /CONUS")),
        "no object GET before key parse"
    );
    assert!(dir_is_empty_or_missing(&h.ctx.cfg.mrms_tmp_root));
}

/// D3: a failed download leaves no scratch directory behind.
#[tokio::test]
async fn d3_failed_download_removes_scratch() {
    let h = harness(false, false).await;
    h.s3.bucket
        .lock()
        .unwrap()
        .overrides
        .insert(MRMS_KEY.to_string(), 503);
    assert!(
        mrms::process_frame(&h.ctx, &info(), &frame_input("radar"), &heartbeat::noop())
            .await
            .is_err()
    );
    assert!(
        dir_is_empty_or_missing(&h.ctx.cfg.mrms_tmp_root),
        "scratch removed on failure"
    );
}

/// Cleanup removes timestamp directories older than the retention window and their manifest frames.
#[tokio::test]
async fn cleanup_removes_old_frames_and_keeps_new() {
    let h = harness(false, false).await;
    mrms::process_frame(&h.ctx, &info(), &frame_input("radar"), &heartbeat::noop())
        .await
        .unwrap();
    // The fixture is from 2026-09-30, far older than a 4 h window relative to today.
    let r = mrms::cleanup(
        &h.ctx,
        &CleanupInput {
            layer_name: "radar".into(),
            retention_hours: 4,
        },
    )
    .await
    .unwrap();
    assert_eq!(r.tile_dirs_removed, 1);
    assert!(
        !h.paths
            .tile_layer_dir("radar")
            .join("classic")
            .join(MRMS_VALID)
            .exists()
    );
    let manifest = radar_core::manifest::read_manifest(&h.paths.state_dir).unwrap();
    assert!(!manifest.layers.contains_key("radar"));
}
