//! Micro-benchmark on an MRMS-sized grid (spec 15.4). Ignored by default because it takes
//! seconds and writes a few thousand PNGs. Run with:
//!
//!     cargo test -p radar-tiler --release --test bench -- --ignored --nocapture

use std::path::PathBuf;
use std::time::Instant;

use radar_tiler::{FrameKind, FrameOptions, Grid, GridGeo, load_palette, render_frame_palettes};

#[test]
#[ignore = "micro-benchmark; run with --ignored --nocapture"]
fn mrms_like_grid_z4_to_z7_three_palettes() {
    let (h, w) = (3500usize, 7000usize);
    let lats: Vec<f64> = (0..h).map(|i| 20.0 + 0.01 * i as f64).collect();
    let lons: Vec<f64> = (0..w).map(|j| -130.0 + 0.01 * j as f64).collect();
    // Deterministic blobs of reflectivity on a background of missing values.
    let values: Vec<f32> = (0..h * w)
        .map(|k| {
            let (r, c) = (k / w, k % w);
            let blob = ((r / 40 + c / 60) % 7) as f32;
            if (r * 13 + c * 7) % 11 == 0 {
                f32::NAN
            } else {
                5.0 + blob * 9.0 + ((r ^ c) % 5) as f32
            }
        })
        .collect();
    let grid = Grid {
        width: w,
        height: h,
        values,
        geo: GridGeo::Regular { lats, lons },
    };
    let names = ["classic", "muted", "vivid"];
    let palettes: Vec<_> = names
        .iter()
        .map(|n| {
            (
                n.to_string(),
                load_palette(n)
                    .unwrap()
                    .color_table("reflectivity")
                    .unwrap(),
            )
        })
        .collect();
    let tmp = tempfile::tempdir().unwrap();
    let outputs: Vec<(String, PathBuf)> = names
        .iter()
        .map(|n| (n.to_string(), tmp.path().join(n)))
        .collect();
    let kind = FrameKind::Continuous {
        nodata_value: None,
        min_valid_weight: 1.0,
    };
    let start = Instant::now();
    let result = render_frame_palettes(
        &grid,
        &kind,
        &palettes,
        &outputs,
        &[4, 5, 6, 7],
        &FrameOptions::default(),
    )
    .expect("render");
    let elapsed = start.elapsed();
    let tiles: Vec<(String, u64)> = result
        .counts()
        .into_iter()
        .map(|(n, c)| (n.to_string(), c))
        .collect();
    eprintln!(
        "radar-tiler bench: {h}x{w} grid, zooms 4..=7, {} palettes",
        names.len()
    );
    eprintln!("radar-tiler bench: tiles per palette {tiles:?}");
    eprintln!(
        "radar-tiler bench: wall time {:.3} s",
        elapsed.as_secs_f64()
    );
}
