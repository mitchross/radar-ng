//! Packing cost on a real NAQFC run. Ignored by default (network). Renders and publishes the three
//! chunks of each layer with and without packing and prints the time and bytes. The packed run stays
//! in `AQM_LIVE_OUT` (default: a temp dir) for `radar api`:
//!
//! `AQM_LIVE_OUT=/tmp/aqm cargo test --release -p radar-aux --test aqm_packed_live -- --ignored --nocapture`

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Instant;

use radar_aux::aqm::{self, AQM_LAYER_SPECS, AqmConfig};

fn bytes_under(dir: &Path) -> u64 {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return 0;
    };
    entries
        .flatten()
        .map(|e| {
            let p = e.path();
            if p.is_dir() {
                bytes_under(&p)
            } else {
                p.metadata().map(|m| m.len()).unwrap_or(0)
            }
        })
        .sum()
}

fn config(root: &Path, packed: bool) -> AqmConfig {
    let mut cfg = AqmConfig::new(
        root.join("tiles"),
        root.join("state"),
        root.join("grids"),
        root.join("tmp"),
    );
    cfg.palette_names = vec!["classic".into(), "muted".into(), "vivid".into()];
    cfg.packed_tiles = packed;
    cfg
}

#[tokio::test(flavor = "multi_thread")]
#[ignore = "network benchmark"]
async fn pack_real_aqm_run() {
    let tmp = tempfile::tempdir().unwrap();
    let out: PathBuf = std::env::var("AQM_LIVE_OUT")
        .map(PathBuf::from)
        .unwrap_or_else(|_| tmp.path().to_path_buf());
    let client = reqwest::Client::new();
    let packed = config(&out, true);
    // Both configs share one download cache.
    let mut raster = config(&out.join("raster"), false);
    raster.tmp_root = packed.tmp_root.clone();
    let run = aqm::aqm_find_latest_run(&packed, &client)
        .await
        .unwrap()
        .run_id
        .expect("a recent AQM run");
    for spec in AQM_LAYER_SPECS.iter() {
        let beat = || -> radar_aux::beat::Beat { Arc::new(|_| {}) };
        // Warm the download cache so both timed passes only render.
        aqm::aqm_render_chunk(&raster, &client, &run, spec.name, 72, beat())
            .await
            .unwrap();
        let mut results = Vec::new();
        let mut seconds = [0.0; 2];
        for (i, cfg) in [&raster, &packed].into_iter().enumerate() {
            results.clear();
            let started = Instant::now();
            for start in [0, 24, 48] {
                results.push(
                    aqm::aqm_render_chunk(cfg, &client, &run, spec.name, start, beat())
                        .await
                        .unwrap(),
                );
            }
            seconds[i] = started.elapsed().as_secs_f64();
        }
        let published = aqm::aqm_publish_run(&packed, &run, &results).await.unwrap();
        let frames: usize = results.iter().map(|r| r.rendered_timestamps.len()).sum();
        let buckets = out.join("tiles/packed").join(spec.name);
        println!(
            "{run} {}: {frames} frames, published {published:?}; render {:.1} s, render + pack {:.1} s",
            spec.name, seconds[0], seconds[1]
        );
        println!(
            "  packed buckets {:.0} kB total, classic PNG {:.0} kB",
            bytes_under(&buckets) as f64 / 1e3,
            bytes_under(&out.join("tiles").join(spec.name).join("classic")) as f64 / 1e3
        );
    }
}
