//! Nowcast benchmark, Rust side: `radar_nowcast::estimate_motion` (LK, stride 2) followed by
//! `radar_nowcast::forecast` (S-PROG, 6 cascade levels, 5 dBZ, 12 leads).
//!
//! Same inputs and steps as the Python `_run_nowcast` in `bench/py/bench_nowcast.py`: values
//! below -100 become NaN before the motion step, and the forecast is written with NaN as -9999.

use std::fs;
use std::path::PathBuf;

use anyhow::{Context, Result};
use clap::Parser;
use radar_bench::{
    load_manifest, machine_info, measure, proc_status_mb, read_f32_le, sha256_file, summarize,
    write_json,
};
use radar_nowcast::{Field2, MotionParams, SprogParams, estimate_motion, forecast};
use serde_json::json;

/// Mirrors `MOTION_STRIDE` in backend/nowcast/activities.py.
const MOTION_STRIDE: usize = 2;

#[derive(Parser)]
struct Args {
    /// HxW of the science grid, for example 1750x3500.
    #[arg(long)]
    shape: String,
    #[arg(long)]
    input_dir: PathBuf,
    /// Directory for the forecast dump (`rust_HxW_fc.f32`).
    #[arg(long)]
    out_dir: PathBuf,
    #[arg(long, default_value_t = 2)]
    iters: usize,
    #[arg(long, default_value_t = 0)]
    warmup: usize,
    #[arg(long)]
    json: PathBuf,
}

fn main() -> Result<()> {
    let args = Args::parse();
    let (h, w) = {
        let (a, b) = args.shape.split_once('x').context("--shape must be HxW")?;
        (a.parse::<usize>()?, b.parse::<usize>()?)
    };
    let man = load_manifest(&args.input_dir)?;
    let mut frame_paths = Vec::new();
    let mut frames: Vec<Field2<f32>> = Vec::new();
    for t in 0..4 {
        let path = args.input_dir.join(format!("nowcast_{h}x{w}_t{t}.f32"));
        let mut data = read_f32_le(&path, h * w)?;
        for v in data.iter_mut() {
            if *v < -100.0 {
                *v = f32::NAN;
            }
        }
        frames.push(Field2::new(w, h, data)?);
        frame_paths.push(path);
    }
    let _ = man; // manifest is read for its existence check; shapes are in the file names
    // Lead times as in activities.py: ((i + 1) * STEP_MIN) / INPUT_STEP_MIN, 60 / 5 = 12 leads.
    let lead_steps: Vec<f64> = (0..12).map(|i| ((i + 1) as f64 * 5.0) / 5.0).collect();
    let motion = MotionParams::radar_ng();
    let sprog = SprogParams::radar_ng();
    let (rss_after_load, _) = proc_status_mb();

    let mut last: Option<Vec<f32>> = None;
    let records = measure(args.warmup, args.iters, |_i| {
        let velocity = estimate_motion(&frames, MOTION_STRIDE, &motion)?;
        let fc = forecast(&frames, &velocity, &lead_steps, &sprog)?;
        let no_rain = fc.no_rain;
        let mut flat = Vec::with_capacity(fc.fields.len() * h * w);
        for field in &fc.fields {
            flat.extend(
                field
                    .as_slice()
                    .iter()
                    .map(|v| if v.is_nan() { -9999.0f32 } else { *v }),
            );
        }
        last = Some(flat);
        Ok(json!({ "method": "radar-nowcast", "leads": fc.fields.len(), "no_rain": no_rain }))
    })?;
    let (_, peak) = proc_status_mb();

    let flat = last.context("no iterations")?;
    fs::create_dir_all(&args.out_dir)?;
    let dump = args.out_dir.join(format!("rust_{h}x{w}_fc.f32"));
    let mut bytes = Vec::with_capacity(flat.len() * 4);
    for v in &flat {
        bytes.extend_from_slice(&v.to_le_bytes());
    }
    fs::write(&dump, &bytes)?;
    drop(bytes);

    let summary = summarize(&records);
    let last_rec = records.last().cloned().unwrap_or(json!({}));
    write_json(
        &args.json,
        &json!({
            "bench": "nowcast",
            "impl": "rust",
            "label": "Rust (radar-nowcast: LK stride 2 + S-PROG)",
            "machine": machine_info(),
            "input": {
                "shape": [h, w], "frames": 4,
                "sha256": frame_paths.iter().map(|p| sha256_file(p)).collect::<Result<Vec<_>>>()?,
            },
            "config": { "stride": MOTION_STRIDE, "leads": lead_steps.len(), "lead_steps": lead_steps,
                        "n_cascade_levels": sprog.n_cascade_levels, "precip_thr": sprog.precip_thr,
                        "warmup": args.warmup, "iters": args.iters },
            "records": records,
            "summary": summary,
            "forecast_file": dump.display().to_string(),
            "forecast_shape": [lead_steps.len(), h, w],
            "no_rain": last_rec["no_rain"],
            "memory": { "after_load_rss_mb": rss_after_load, "peak_rss_mb": peak },
        }),
    )?;
    println!(
        "rust nowcast {h}x{w}: median {:.2}s peak_rss={:.0}MB",
        summary["wall_s_median"].as_f64().unwrap_or(f64::NAN),
        peak
    );
    Ok(())
}
