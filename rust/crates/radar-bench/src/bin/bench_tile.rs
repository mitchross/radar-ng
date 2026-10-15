//! Tile render benchmark, Rust side: `radar_tiler::render::render_frame_palettes`.
//!
//! Same scene, palettes, zooms, overview and iteration rules as `bench/py/bench_tile.py`.
//! Each iteration writes a fresh pyramid tree; the last one is kept for `bench/py/verify.py`.

use std::fs;
use std::path::PathBuf;

use anyhow::{Context, Result};
use clap::Parser;
use radar_bench::{
    count_pngs, load_manifest, machine_info, measure, proc_status_mb, read_f32_le, read_f64_le,
    sha256_file, summarize, write_json,
};
use radar_geo::{Grid, GridGeo};
use radar_tiler::{
    ColorTable, FrameKind, FrameOptions, Overview, load_palette, render_frame_palettes,
};
use serde_json::json;

const PALETTES: [&str; 3] = ["classic", "muted", "vivid"];
const ZOOMS: [u32; 4] = [4, 5, 6, 7];

#[derive(Parser)]
struct Args {
    /// Directory written by bench/py/gen_inputs.py (contains manifest.json).
    #[arg(long)]
    input_dir: PathBuf,
    /// Root for the pyramid trees (one subdirectory per iteration).
    #[arg(long)]
    out_root: PathBuf,
    #[arg(long, default_value_t = 5)]
    iters: usize,
    #[arg(long, default_value_t = 1)]
    warmup: usize,
    #[arg(long)]
    json: PathBuf,
}

fn main() -> Result<()> {
    let args = Args::parse();
    let man = load_manifest(&args.input_dir)?;
    let h = man["mrms_grid"]["rows"].as_u64().context("rows")? as usize;
    let w = man["mrms_grid"]["cols"].as_u64().context("cols")? as usize;
    let src = args.input_dir.join("mrms_3500x7000_t0.f32");
    let values = read_f32_le(&src, h * w)?;
    let lats = read_f64_le(&args.input_dir.join("lats_3500.f64"))?;
    let lons = read_f64_le(&args.input_dir.join("lons_7000.f64"))?;
    let grid = Grid {
        width: w,
        height: h,
        values,
        geo: GridGeo::Regular { lats, lons },
    };
    let kind = FrameKind::Continuous {
        nodata_value: None,
        min_valid_weight: 1.0,
    };

    let mut palettes: Vec<(String, ColorTable)> = Vec::new();
    for name in PALETTES {
        let table = load_palette(name)?.color_table("reflectivity")?;
        palettes.push((name.to_string(), table));
    }
    let (rss_after_load, _) = proc_status_mb();

    let root = args.out_root.clone();
    let _ = fs::remove_dir_all(&root);
    fs::create_dir_all(&root)?;
    let total_iters = args.warmup + args.iters;

    let records = measure(args.warmup, args.iters, |i| {
        let iter_root = root.join(format!("iter{i}"));
        let outdirs: Vec<(String, PathBuf)> = PALETTES
            .iter()
            .map(|p| (p.to_string(), iter_root.join(p).join("ts")))
            .collect();
        let opts = FrameOptions {
            tile_size: 256,
            source_id: "mrms:bench:t0".into(),
            publication_lock_root: Some(iter_root.clone()),
            overview: Some(Overview::Max),
            png_compress_level: 6,
            keep_empty: false,
        };
        let res = render_frame_palettes(&grid, &kind, &palettes, &outdirs, &ZOOMS, &opts)?;
        let (tiles, bytes) = count_pngs(&iter_root)?;
        let mut rendered: Vec<&str> = res.rendered_palettes();
        rendered.sort();
        Ok(json!({ "tiles": tiles, "bytes": bytes, "rendered": rendered }))
    })?;
    let (_, peak) = proc_status_mb();
    let last = records.last().cloned().unwrap_or(json!({}));
    // Keep only the last iteration's tree (outside the timed loop).
    for i in 0..total_iters - 1 {
        let _ = fs::remove_dir_all(root.join(format!("iter{i}")));
    }

    let summary = summarize(&records);
    write_json(
        &args.json,
        &json!({
            "bench": "tile",
            "impl": "rust",
            "label": "Rust (radar-tiler, indexed-rs1, rayon)",
            "machine": machine_info(),
            "input": {
                "file": src.file_name().and_then(|s| s.to_str()),
                "sha256": sha256_file(&src)?,
                "shape": [h, w],
                "manifest_sha256": man["files"]["mrms_3500x7000_t0.f32"]["sha256"],
            },
            "config": {
                "palettes": PALETTES, "zooms": ZOOMS, "overview": "max",
                "png_compress_level": 6, "renderer": "indexed-rs1",
                "warmup": args.warmup, "iters": args.iters,
            },
            "records": records,
            "summary": summary,
            "tiles": last["tiles"],
            "bytes": last["bytes"],
            "memory": { "after_load_rss_mb": rss_after_load, "peak_rss_mb": peak },
            "output_root": root.display().to_string(),
        }),
    )?;
    println!(
        "rust tile: median {:.2}s tiles={} peak_rss={:.0}MB",
        summary["wall_s_median"].as_f64().unwrap_or(f64::NAN),
        last["tiles"],
        peak
    );
    Ok(())
}
