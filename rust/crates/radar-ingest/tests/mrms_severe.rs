//! The severe-weather layers (`hail-size`, `rotation-track`, `echo-top`) end to end through the MRMS
//! activities, against the fake bucket. The MRMS fixture stands in for each file; its values are
//! converted from the product's GRIB unit.

mod common;

use radar_core::env::Paths;
use radar_ingest::mrms_qpe::{PRODUCTS, QPE_24H_LAYER, product_for};
use radar_ingest::scratch::ActivityInfo;
use radar_ingest::{IngestConfig, IngestCtx, heartbeat, mrms};
use radar_types::payloads::mrms::{IngestMrmsArgs, ProcessFrameInput};
use tempfile::TempDir;

const VALID: &str = "2026-09-30T00:00:00+00:00";
const SEVERE: [&str; 3] = ["hail-size", "rotation-track", "echo-top"];

fn key_for(prefix: &str) -> String {
    let product = prefix.trim_start_matches("CONUS/");
    format!("{prefix}/20260930/MRMS_{product}_20260930-000000.grib2.gz")
}

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
    for layer in SEVERE {
        let key = key_for(product_for(layer).unwrap().prefix);
        s3.put(&key, common::mrms_gz());
        s3.bucket.lock().unwrap().list_keys.push(key);
    }
    let mut cfg = IngestConfig::with_roots(paths.clone(), root.path().join("work")).unwrap();
    cfg.mrms_base_url = s3.base.clone();
    cfg.palettes = vec!["classic".into(), "muted".into(), "vivid".into()];
    cfg.mrms_severe = enabled;
    let ctx = IngestCtx::new(cfg, None).unwrap();
    Harness {
        _root: root,
        paths,
        ctx,
        _s3: s3,
    }
}

fn info() -> ActivityInfo {
    ActivityInfo {
        workflow_id: "sched-ingest-mrms-severe".into(),
        run_id: "run-1".into(),
        activity_id: "2".into(),
        attempt: 1,
    }
}

#[test]
fn every_product_has_a_distinct_layer_prefix_and_palette_key() {
    let mut layers: Vec<_> = PRODUCTS.iter().map(|p| p.layer).collect();
    layers.sort_unstable();
    layers.dedup();
    assert_eq!(layers.len(), PRODUCTS.len());
    assert!(layers.contains(&QPE_24H_LAYER));
    for layer in SEVERE {
        let p = product_for(layer).unwrap();
        assert!(p.prefix.starts_with("CONUS/"), "{layer}");
    }
    assert_eq!(
        product_for("hail-size").unwrap().prefix,
        "CONUS/MESH_Max_60min_00.50"
    );
    assert_eq!(
        product_for("rotation-track").unwrap().prefix,
        "CONUS/RotationTrack60min_00.50"
    );
    assert_eq!(
        product_for("echo-top").unwrap().prefix,
        "CONUS/EchoTop_18_00.50"
    );
}

#[tokio::test]
async fn disabled_severe_layers_list_nothing() {
    let h = harness(false).await;
    for layer in SEVERE {
        let args = IngestMrmsArgs {
            mrms_prefix: product_for(layer).unwrap().prefix.into(),
            layer_name: layer.into(),
        };
        let listed = mrms::list_unprocessed_keys(&h.ctx, &args).await.unwrap();
        assert!(listed.keys.is_empty(), "{layer}");
    }
}

#[tokio::test]
async fn severe_layers_render_all_palettes_before_the_manifest_lists_them() {
    let h = harness(true).await;
    for layer in SEVERE {
        let product = product_for(layer).unwrap();
        let key = key_for(product.prefix);
        let args = IngestMrmsArgs {
            mrms_prefix: product.prefix.into(),
            layer_name: layer.into(),
        };
        let listed = mrms::list_unprocessed_keys(&h.ctx, &args).await.unwrap();
        assert_eq!(listed.keys, vec![key.clone()], "{layer}");

        let input = ProcessFrameInput {
            key,
            layer_name: layer.into(),
        };
        let out = mrms::process_frame(&h.ctx, &info(), &input, &heartbeat::noop())
            .await
            .unwrap();
        assert!(out.rendered, "{layer}");
        assert_eq!(out.timestamp, VALID);
        assert_eq!(out.palettes, ["classic", "muted", "vivid"], "{layer}");

        let tiles = h.paths.tile_layer_dir(layer);
        for palette in ["classic", "muted", "vivid"] {
            assert!(
                tiles
                    .join(palette)
                    .join(VALID)
                    .join(".render-complete-v2.json")
                    .is_file(),
                "{layer}/{palette} pyramid"
            );
        }
        let meta: serde_json::Value = serde_json::from_slice(
            &std::fs::read(
                h.paths
                    .grid_layer_dir(layer)
                    .join(format!("{VALID}.meta.json")),
            )
            .unwrap(),
        )
        .unwrap();
        assert_eq!(meta["unit"], product.unit, "{layer}");

        let manifest = radar_core::manifest::read_manifest(&h.paths.state_dir).unwrap();
        let entry = &manifest.layers[layer];
        assert_eq!(entry.timestamps, vec![VALID.to_string()], "{layer}");
        assert_eq!(entry.extra["unit"], product.unit);
        assert_eq!(entry.extra["palette_key"], product.color_key);
        assert_eq!(entry.frames[0].extra["product"], product.product);
    }
}
