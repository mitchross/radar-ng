//! Binary-free benchmark entry point (port of `backend/scripts/bench_render_once.py`): decode one GRIB
//! file (gzip or raw), render it into three palettes at zooms 4 to 7, and run storm detection. Times
//! each stage. The benchmark harness calls [`bench_render_once`] directly.

use std::path::Path;
use std::time::Instant;

use radar_grib::mrms::ensure_south_up;
use radar_storms::{Grid as StormGrid, StormConfig};
use radar_tiler::{FrameKind, FrameOptions, Overview};

/// Palettes rendered by the benchmark, as the Python script does.
pub const BENCH_PALETTES: [&str; 3] = ["classic", "muted", "vivid"];
/// Zooms rendered by the benchmark.
pub const BENCH_ZOOMS: [u32; 4] = [4, 5, 6, 7];

/// Timings and sizes for one `bench_render_once` run.
#[derive(Debug, Clone, PartialEq)]
pub struct BenchReport {
    /// Grid height (rows).
    pub height: usize,
    /// Grid width (columns).
    pub width: usize,
    /// Decoding (gunzip and GRIB2) in milliseconds.
    pub decode_ms: f64,
    /// Tile rendering for all palettes in milliseconds.
    pub render_ms: f64,
    /// Storm detection and tracking in milliseconds.
    pub storms_ms: f64,
    /// Storm cells detected in the frame.
    pub storm_cells: usize,
    /// Tile PNG count per palette, in [`BENCH_PALETTES`] order.
    pub tiles: Vec<(String, u64)>,
    /// Total bytes of published tile PNGs.
    pub tile_bytes: u64,
}

/// Decode, render and detect storms for one GRIB file, publishing tiles under `out_dir`.
///
/// Gzip input (`1f 8b` magic) is gunzipped first. Errors propagate: decoding, rendering and storm
/// detection are not best-effort in the benchmark.
pub fn bench_render_once(grib_path: &Path, out_dir: &Path) -> anyhow::Result<BenchReport> {
    let raw = std::fs::read(grib_path)?;
    let t0 = Instant::now();
    let mut grid = if raw.starts_with(&[0x1f, 0x8b]) {
        radar_grib::mrms::decode_gz(&raw)?
    } else {
        radar_grib::decode::decode_message(&raw)?
    };
    ensure_south_up(&mut grid);
    let decode_ms = t0.elapsed().as_secs_f64() * 1e3;

    let tables: Vec<(String, radar_tiler::ColorTable)> = BENCH_PALETTES
        .iter()
        .map(|name| {
            let t = radar_tiler::load_palette(name)?.color_table("reflectivity")?;
            Ok::<_, anyhow::Error>((name.to_string(), t))
        })
        .collect::<anyhow::Result<_>>()?;
    let dirs: Vec<(String, std::path::PathBuf)> = BENCH_PALETTES
        .iter()
        .map(|n| (n.to_string(), out_dir.join(n).join("bench")))
        .collect();
    let options = FrameOptions {
        source_id: "bench:radar".to_string(),
        publication_lock_root: Some(out_dir.to_path_buf()),
        overview: Some(Overview::Max),
        ..FrameOptions::default()
    };
    let t1 = Instant::now();
    let kind = FrameKind::Continuous {
        nodata_value: None,
        min_valid_weight: 1.0,
    };
    let result =
        radar_tiler::render_frame_palettes(&grid, &kind, &tables, &dirs, &BENCH_ZOOMS, &options)?;
    let render_ms = t1.elapsed().as_secs_f64() * 1e3;

    let t2 = Instant::now();
    let cfg = StormConfig::from_lookup(|_| None)?;
    let (lats, lons) = match &grid.geo {
        radar_geo::GridGeo::Regular { lats, lons } => (lats.as_slice(), lons.as_slice()),
        radar_geo::GridGeo::Projected { .. } => anyhow::bail!("benchmark needs a regular grid"),
    };
    let view = StormGrid::new(&grid.values, grid.height, grid.width, lats, lons)?;
    let doc = radar_storms::detect_storms(&view, "2026-01-01T00:00:00+00:00", None, &cfg);
    let storms_ms = t2.elapsed().as_secs_f64() * 1e3;

    let tiles: Vec<(String, u64)> = result
        .counts()
        .into_iter()
        .map(|(n, c)| (n.to_string(), c))
        .collect();
    let tile_bytes = dir_png_bytes(out_dir)?;
    Ok(BenchReport {
        height: grid.height,
        width: grid.width,
        decode_ms,
        render_ms,
        storms_ms,
        storm_cells: doc.features.len(),
        tiles,
        tile_bytes,
    })
}

/// Total size of `.png` files under `dir`.
fn dir_png_bytes(dir: &Path) -> std::io::Result<u64> {
    let mut total = 0u64;
    let mut stack = vec![dir.to_path_buf()];
    while let Some(d) = stack.pop() {
        for entry in std::fs::read_dir(&d)? {
            let entry = entry?;
            let path = entry.path();
            if path.is_dir() {
                stack.push(path);
            } else if path.extension().is_some_and(|e| e == "png") {
                total += entry.metadata()?.len();
            }
        }
    }
    Ok(total)
}
