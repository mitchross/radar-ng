//! The `precip-24h` layer end to end through the MRMS activities, against the fake bucket. The MRMS
//! fixture stands in for a QPE file: its values are read as millimetres.

mod common;

use radar_core::env::Paths;
use radar_ingest::mrms_qpe::{QPE_24H_LAYER, QPE_24H_PREFIX};
use radar_ingest::scratch::ActivityInfo;
use radar_ingest::{IngestConfig, IngestCtx, heartbeat, mrms};
use radar_types::payloads::mrms::{CleanupInput, IngestMrmsArgs, ProcessFrameInput};
use tempfile::TempDir;

const QPE_KEY: &str = "CONUS/MultiSensor_QPE_24H_Pass1_00.00/20260930/MRMS_MultiSensor_QPE_24H_Pass1_00.00_20260930-000000.grib2.gz";
const QPE_VALID: &str = "2026-09-30T00:00:00+00:00";

struct Harness {
    _root: TempDir,
    paths: Paths,
    ctx: IngestCtx,
    _s3: common::FakeS3,
}

async fn harness(enabled: bool) -> Harness {
    let root = tempfile::tempdir().unwrap();
    let paths = Paths::new(
        root.path().join("state"),
        root.path().join("grids"),
        root.path().join("tiles"),
    );
    let s3 = common::start().await;
    s3.put(QPE_KEY, common::mrms_gz());
    s3.bucket
        .lock()
        .unwrap()
        .list_keys
        .push(QPE_KEY.to_string());
    let mut cfg = IngestConfig::with_roots(paths.clone(), root.path().join("work")).unwrap();
    cfg.mrms_base_url = s3.base.clone();
    cfg.palettes = vec!["classic".into(), "vivid".into()];
    cfg.mrms_qpe_24h = enabled;
    let ctx = IngestCtx::new(cfg, None).unwrap();
    Harness {
        _root: root,
        paths,
        ctx,
        _s3: s3,
    }
}

fn args() -> IngestMrmsArgs {
    IngestMrmsArgs {
        mrms_prefix: QPE_24H_PREFIX.into(),
        layer_name: QPE_24H_LAYER.into(),
    }
}

fn input() -> ProcessFrameInput {
    ProcessFrameInput {
        key: QPE_KEY.into(),
        layer_name: QPE_24H_LAYER.into(),
    }
}

fn info() -> ActivityInfo {
    ActivityInfo {
        workflow_id: "sched-ingest-mrms-qpe-24h".into(),
        run_id: "run-1".into(),
        activity_id: "2".into(),
        attempt: 1,
    }
}

#[tokio::test]
async fn disabled_layer_lists_nothing() {
    let h = harness(false).await;
    let listed = mrms::list_unprocessed_keys(&h.ctx, &args()).await.unwrap();
    assert!(listed.keys.is_empty());
    assert_eq!(listed.backlog_total, 0);
}

#[tokio::test]
async fn enabled_layer_lists_renders_and_publishes_in_inches() {
    let h = harness(true).await;
    let listed = mrms::list_unprocessed_keys(&h.ctx, &args()).await.unwrap();
    assert_eq!(listed.keys, vec![QPE_KEY.to_string()]);

    let out = mrms::process_frame(&h.ctx, &info(), &input(), &heartbeat::noop())
        .await
        .unwrap();
    assert!(out.rendered);
    assert_eq!(out.timestamp, QPE_VALID);
    assert_eq!(
        out.palettes,
        vec!["classic".to_string(), "vivid".to_string()]
    );

    let tiles = h.paths.tile_layer_dir(QPE_24H_LAYER);
    for palette in ["classic", "vivid"] {
        assert!(
            tiles
                .join(palette)
                .join(QPE_VALID)
                .join(".render-complete-v2.json")
                .is_file(),
            "{palette} pyramid"
        );
    }
    let meta: serde_json::Value = serde_json::from_slice(
        &std::fs::read(
            h.paths
                .grid_layer_dir(QPE_24H_LAYER)
                .join(format!("{QPE_VALID}.meta.json")),
        )
        .unwrap(),
    )
    .unwrap();
    assert_eq!(meta["unit"], "in");

    let manifest = radar_core::manifest::read_manifest(&h.paths.state_dir).unwrap();
    let layer = &manifest.layers[QPE_24H_LAYER];
    assert_eq!(layer.timestamps, vec![QPE_VALID.to_string()]);
    assert_eq!(layer.frames[0].extra["accumulation_hours"], 24);
    assert_eq!(layer.extra["unit"], "in");
    assert!(layer.extra.get("packed").is_none(), "no packed bucket");
    assert!(
        !h.paths.state_dir.join("storms.json").exists(),
        "no storm detection"
    );
    assert!(!h.paths.grid_layer_dir("radar-nowcast-input").exists());

    let r = mrms::cleanup(
        &h.ctx,
        &CleanupInput {
            layer_name: QPE_24H_LAYER.into(),
            retention_hours: 4,
        },
    )
    .await
    .unwrap();
    assert_eq!(r.tile_dirs_removed, 2, "one directory per palette");
    let manifest = radar_core::manifest::read_manifest(&h.paths.state_dir).unwrap();
    assert!(!manifest.layers.contains_key(QPE_24H_LAYER));
}
