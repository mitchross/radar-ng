//! GRIB decode benchmark, Rust side.
//!
//! --kind mrms-gz     `radar_grib::mrms::decode_gz` (in-memory gunzip and decode of the first
//!                    message) followed by `ensure_south_up`, as the Python caller flips.
//! --kind grib-multi  every message of a multi-message file, split by the GRIB2 length field
//!                    and decoded with `decode_message`.

use std::fs;
use std::path::PathBuf;

use anyhow::{Context, Result, bail};
use clap::Parser;
use radar_bench::{
    machine_info, measure, proc_status_mb, sha256_f32_le, sha256_f64_le, sha256_file, summarize,
    write_json,
};
use radar_grib::decode::decode_message;
use radar_grib::mrms::{decode_gz, ensure_south_up};
use serde_json::{Value, json};

#[derive(Parser)]
struct Args {
    #[arg(long, value_parser = ["mrms-gz", "grib-multi"])]
    kind: String,
    #[arg(long)]
    file: PathBuf,
    #[arg(long)]
    label: String,
    #[arg(long, default_value_t = 5)]
    iters: usize,
    #[arg(long, default_value_t = 1)]
    warmup: usize,
    #[arg(long)]
    json: PathBuf,
}

/// Split a GRIB2 stream into messages using the 8-byte total length at octets 9 to 16.
fn split_messages(bytes: &[u8]) -> Result<Vec<&[u8]>> {
    let mut out = Vec::new();
    let mut at = 0usize;
    while at < bytes.len() {
        let rest = &bytes[at..];
        let Some(pos) = rest.windows(4).position(|w| w == b"GRIB") else {
            break;
        };
        let start = at + pos;
        if start + 16 > bytes.len() {
            bail!("truncated GRIB header at {start}");
        }
        let len = u64::from_be_bytes(bytes[start + 8..start + 16].try_into()?) as usize;
        if len < 16 || start + len > bytes.len() {
            bail!("bad GRIB length {len} at {start}");
        }
        out.push(&bytes[start..start + len]);
        at = start + len;
    }
    Ok(out)
}

fn main() -> Result<()> {
    let args = Args::parse();
    let raw = fs::read(&args.file).with_context(|| format!("read {}", args.file.display()))?;
    let (_, rss_after_load) = proc_status_mb();
    let mut last: Option<Value> = None;

    let records = measure(args.warmup, args.iters, |_i| {
        if args.kind == "mrms-gz" {
            let mut grid = decode_gz(&raw)?;
            ensure_south_up(&mut grid);
            let (lats, lons) = match &grid.geo {
                radar_geo::GridGeo::Regular { lats, lons } => (lats.clone(), lons.clone()),
                _ => bail!("expected a regular lat/lon grid"),
            };
            let nan_cells = grid.values.iter().filter(|v| v.is_nan()).count();
            let digest = json!({
                "shape": [grid.height, grid.width],
                "nan_cells": nan_cells,
                "sha256_values": sha256_f32_le(&grid.values),
                "sha256_lats": sha256_f64_le(&lats),
                "sha256_lons": sha256_f64_le(&lons),
                "lat_first": lats[0], "lat_last": lats[lats.len() - 1],
                "lon_first": lons[0], "lon_last": lons[lons.len() - 1],
            });
            last = Some(digest);
            Ok(json!({}))
        } else {
            let mut msgs = Vec::new();
            for (k, msg) in split_messages(&raw)?.into_iter().enumerate() {
                let grid = decode_message(msg)?;
                let nan_cells = grid.values.iter().filter(|v| v.is_nan()).count();
                msgs.push(json!({
                    "message": k + 1,
                    "shape": [grid.height, grid.width],
                    "nan_cells": nan_cells,
                    "sha256_values": sha256_f32_le(&grid.values),
                }));
            }
            let n = msgs.len();
            last = Some(json!({ "messages": msgs }));
            Ok(json!({ "messages": n }))
        }
    })?;
    let (_, peak) = proc_status_mb();
    let summary = summarize(&records);
    write_json(
        &args.json,
        &json!({
            "bench": "grib",
            "case": args.label,
            "impl": "rust",
            "label": "Rust (radar-grib, eccodes-matching decode)",
            "machine": machine_info(),
            "input": { "file": args.file.file_name().and_then(|s| s.to_str()),
                       "bytes": raw.len(), "sha256": sha256_file(&args.file)? },
            "config": { "kind": args.kind, "warmup": args.warmup, "iters": args.iters },
            "records": records,
            "summary": summary,
            "digest": last.context("no iterations")?,
            "memory": { "after_load_rss_mb": rss_after_load, "peak_rss_mb": peak },
        }),
    )?;
    println!(
        "rust grib {}: median {:.4}s peak_rss={:.0}MB",
        args.label,
        summary["wall_s_median"].as_f64().unwrap_or(f64::NAN),
        peak
    );
    Ok(())
}
