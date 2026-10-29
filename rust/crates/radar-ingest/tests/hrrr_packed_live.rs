//! Packing cost on real HRRR data. Ignored by default (network). Downloads the 13-variable subset of
//! a few forecast hours, renders every layer with and without packing, publishes, and prints the time
//! and bytes. The packed run stays in `HRRR_LIVE_OUT` (default: a temp dir) for `radar api`:
//!
//! `HRRR_LIVE_OUT=/tmp/hrrr HRRR_LIVE_HOURS=3 cargo test --release -p radar-ingest --test hrrr_packed_live -- --ignored --nocapture`

use std::path::{Path, PathBuf};
use std::time::Instant;

use radar_core::env::Paths;
use radar_grib::hrrr::{self, Fetched};
use radar_grib::s3::S3Client;
use radar_ingest::config::{HRRR_LAYERS, IngestConfig};
use radar_ingest::hrrr::render::{load_hrrr_palettes, process_hour_sync};
use radar_ingest::hrrr::state::needed_vars;
use radar_ingest::hrrr::{publish::publish_run, run_init, valid_time};
use radar_types::payloads::hrrr::ForecastHourResult;

fn config(root: &Path, packed: bool) -> IngestConfig {
    let paths = Paths::new(root.join("state"), root.join("grids"), root.join("tiles"));
    let mut cfg = IngestConfig::with_roots(paths, root.join("tmp")).unwrap();
    cfg.hrrr_enabled_layers = HRRR_LAYERS.iter().map(|s| s.to_string()).collect();
    cfg.palettes = vec!["classic".into(), "muted".into(), "vivid".into()];
    cfg.packed_tiles = packed;
    cfg
}

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

#[tokio::test(flavor = "multi_thread")]
#[ignore = "network benchmark"]
async fn pack_real_hrrr_hours() {
    let hours: u32 = std::env::var("HRRR_LIVE_HOURS")
        .ok()
        .and_then(|h| h.parse().ok())
        .unwrap_or(3);
    let tmp = tempfile::tempdir().unwrap();
    let out: PathBuf = std::env::var("HRRR_LIVE_OUT")
        .map(PathBuf::from)
        .unwrap_or_else(|_| tmp.path().to_path_buf());
    let s3 = S3Client::new(
        radar_grib::http::client().unwrap(),
        hrrr::BASE_URL.to_string(),
    );
    let run = match std::env::var("HRRR_LIVE_RUN") {
        Ok(run) => run,
        Err(_) => hrrr::find_latest_run(&s3, chrono::Utc::now())
            .await
            .unwrap()
            .expect("a recent HRRR run"),
    };
    let init = run_init(&run).unwrap();
    let (raster, packed) = (config(&out.join("raster"), false), config(&out, true));
    let palettes = load_hrrr_palettes(&packed.palettes);
    let needed = needed_vars(&packed.hrrr_enabled_layers);
    let mut results = Vec::new();
    for fhr in 1..=hours {
        let grib = out.join("cache").join(format!("{run}_f{fhr:02}.grib2"));
        if !grib.exists() {
            let Fetched::Data { bytes, .. } =
                hrrr::fetch_hour(&s3, init, fhr, &needed).await.unwrap()
            else {
                panic!("f{fhr:02} of {run} is not available");
            };
            std::fs::create_dir_all(grib.parent().unwrap()).unwrap();
            std::fs::write(&grib, &bytes).unwrap();
        }
        let valid = valid_time(init, fhr);
        let mut seconds = [0.0; 2];
        let bytes = std::fs::read(&grib).unwrap();
        let mut layers = Vec::new();
        for (i, cfg) in [&raster, &packed].into_iter().enumerate() {
            let started = Instant::now();
            layers = process_hour_sync(cfg, &bytes, &run, &valid, &palettes).unwrap();
            seconds[i] = started.elapsed().as_secs_f64();
        }
        println!(
            "{run} f{fhr:02}: render {:.2} s, render + pack {:.2} s ({} layers)",
            seconds[0],
            seconds[1],
            layers.len()
        );
        results.push(ForecastHourResult {
            fhr: i64::from(fhr),
            rendered_layers: layers,
            duration_s: seconds[1],
            valid_timestamp: valid,
        });
    }
    let published = publish_run(
        &packed.paths.state_dir,
        Some(&packed.paths),
        &run,
        &results,
        &palettes,
        hours,
    )
    .unwrap();
    println!("published {published:?}");
    let packed_root = packed.paths.tile_dir.join("packed");
    for layer in HRRR_LAYERS {
        let dir = packed_root.join(layer).join(&run);
        let png = bytes_under(&packed.paths.tile_dir.join(layer).join("classic"));
        println!(
            "{layer:>12}: packed bucket {:7.0} kB ({hours} slots), classic PNG {:7.0} kB",
            bytes_under(&dir) as f64 / 1e3,
            png as f64 / 1e3
        );
    }
}
