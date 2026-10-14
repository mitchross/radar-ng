//! Storm detection benchmark, Rust side: `radar_storms::detect_storms` with tracking.
//!
//! Same two-frame sequence as `bench/py/bench_storms.py`: frame A with no previous document,
//! then frame B with A's document as `previous`. Timing covers both calls.

use std::path::PathBuf;

use anyhow::{Context, Result};
use clap::Parser;
use radar_bench::{
    load_manifest, machine_info, measure, proc_status_mb, read_f32_le, read_f64_le, sha256_file,
    summarize, write_json,
};
use radar_storms::{Grid, StormConfig, StormsDoc, detect_storms};
use serde_json::{Value, json};

const TS_A: &str = "2026-10-14T00:00:00+00:00";
const TS_B: &str = "2026-10-14T00:05:00+00:00";

#[derive(Parser)]
struct Args {
    #[arg(long)]
    input_dir: PathBuf,
    /// Where the normalised detection summary is written.
    #[arg(long)]
    summary_out: PathBuf,
    #[arg(long, default_value_t = 5)]
    iters: usize,
    #[arg(long, default_value_t = 1)]
    warmup: usize,
    #[arg(long)]
    json: PathBuf,
}

/// Same shape as the Python `normalise` in bench_storms.py.
fn normalise(doc: &StormsDoc) -> Result<Value> {
    let v = serde_json::to_value(doc)?;
    let mut feats = Vec::new();
    for f in v["features"].as_array().context("features")? {
        let p = &f["properties"];
        feats.push(json!({
            "cell_id": p["cell_id"].as_i64().context("cell_id")?,
            "peak_dbz": p["peak_dbz"].as_f64().context("peak_dbz")?,
            "pixel_count": p["pixel_count"].as_u64().context("pixel_count")?,
            "area_km2": p["area_km2"].as_f64().context("area_km2")?,
            "bbox": p["bbox"],
            "coordinates": f["geometry"]["coordinates"],
        }));
    }
    Ok(json!({ "n_features": feats.len(), "features": feats }))
}

fn main() -> Result<()> {
    let args = Args::parse();
    let man = load_manifest(&args.input_dir)?;
    let h = man["mrms_grid"]["rows"].as_u64().context("rows")? as usize;
    let w = man["mrms_grid"]["cols"].as_u64().context("cols")? as usize;
    let a = read_f32_le(&args.input_dir.join("mrms_3500x7000_t0.f32"), h * w)?;
    let b = read_f32_le(&args.input_dir.join("mrms_3500x7000_t1.f32"), h * w)?;
    let lats = read_f64_le(&args.input_dir.join("lats_3500.f64"))?;
    let lons = read_f64_le(&args.input_dir.join("lons_7000.f64"))?;
    let ga = Grid::new(&a, h, w, &lats, &lons)?;
    let gb = Grid::new(&b, h, w, &lats, &lons)?;
    let cfg = StormConfig::default();
    let (rss_after_load, _) = proc_status_mb();

    let mut last: Option<(StormsDoc, StormsDoc)> = None;
    let records = measure(args.warmup, args.iters, |_i| {
        let doc_a = detect_storms(&ga, TS_A, None, &cfg);
        let prev = serde_json::to_value(&doc_a)?;
        let doc_b = detect_storms(&gb, TS_B, Some(&prev), &cfg);
        let out = json!({ "features_a": doc_a.features.len(), "features_b": doc_b.features.len() });
        last = Some((doc_a, doc_b));
        Ok(out)
    })?;
    let (_, peak) = proc_status_mb();
    let (doc_a, doc_b) = last.context("no iterations")?;
    write_json(
        &args.summary_out,
        &json!({ "frame_a": normalise(&doc_a)?, "frame_b": normalise(&doc_b)? }),
    )?;
    let summary = summarize(&records);
    let last_rec = records.last().cloned().unwrap_or(json!({}));
    write_json(
        &args.json,
        &json!({
            "bench": "storms",
            "impl": "rust",
            "label": "Rust (radar-storms detect_storms + tracking)",
            "machine": machine_info(),
            "input": {
                "files": ["mrms_3500x7000_t0.f32", "mrms_3500x7000_t1.f32"],
                "shape": [h, w],
                "sha256_t0": sha256_file(&args.input_dir.join("mrms_3500x7000_t0.f32"))?,
                "sha256_t1": sha256_file(&args.input_dir.join("mrms_3500x7000_t1.f32"))?,
            },
            "config": { "threshold_dbz": cfg.threshold_dbz, "min_pixels": cfg.min_pixels,
                        "max_storms": cfg.max_storms, "warmup": args.warmup, "iters": args.iters },
            "records": records,
            "summary": summary,
            "features_a": last_rec["features_a"],
            "features_b": last_rec["features_b"],
            "memory": { "after_load_rss_mb": rss_after_load, "peak_rss_mb": peak },
            "summary_file": args.summary_out.display().to_string(),
        }),
    )?;
    println!(
        "rust storms: median {:.3}s features_a={} features_b={} peak_rss={:.0}MB",
        summary["wall_s_median"].as_f64().unwrap_or(f64::NAN),
        last_rec["features_a"],
        last_rec["features_b"],
        peak
    );
    Ok(())
}
