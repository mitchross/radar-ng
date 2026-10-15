//! Packing cost on an MRMS-sized grid. Ignored by default; run with
//! `cargo test --release -p radar-tiler --test packed_perf -- --ignored --nocapture`.

use std::time::Instant;

use radar_tiler::packed::{
    PACKED_TILE_SIZE, PackFrame, PackOutcome, Quantizer, ValueSampler, pack_frames,
};
use radar_tiler::{Grid, GridGeo, Overview};

/// 7000 x 3500 at 0.01 degree over CONUS, with echoes on about a quarter of the cells.
fn mrms_like(seed: f64) -> Grid {
    let (h, w) = (3500usize, 7000usize);
    let lats: Vec<f64> = (0..h).map(|i| 20.005 + 0.01 * i as f64).collect();
    let lons: Vec<f64> = (0..w).map(|j| -129.995 + 0.01 * j as f64).collect();
    let mut values = vec![f32::NAN; h * w];
    for i in 0..h {
        for j in 0..w {
            let (y, x) = (i as f64 / 90.0, j as f64 / 110.0);
            let v = (x + seed).sin() * (y * 1.3).cos() + 0.5 * (x * 2.7 + y * 1.9).sin();
            if v > 0.55 {
                values[i * w + j] = (5.0 + 45.0 * (v - 0.55)) as f32;
            }
        }
    }
    Grid {
        height: h,
        width: w,
        values,
        geo: GridGeo::Regular { lats, lons },
    }
}

#[test]
#[ignore = "benchmark"]
fn packing_cost_per_mrms_frame() {
    let dir = tempfile::tempdir().unwrap();
    let quant = Quantizer::for_color_key("reflectivity").unwrap();
    for slot in 0..15 {
        let grid = mrms_like(slot as f64 * 0.05);
        let started = Instant::now();
        let sampler = ValueSampler::new(
            &grid,
            None,
            &[3, 4, 5, 6],
            PACKED_TILE_SIZE,
            Some(Overview::Max),
        )
        .unwrap();
        let ts = format!("2026-10-15T18:{:02}:00+00:00", 30 + slot * 2);
        let frames = [PackFrame {
            timestamp: &ts,
            sampler: &sampler,
        }];
        let out = pack_frames(dir.path(), "2026-10-15T18:30:00Z", &frames, quant).unwrap();
        let elapsed = started.elapsed();
        let PackOutcome::Written { tiles_written, .. } = out else {
            panic!("full")
        };
        let bytes: u64 = walk(&dir.path().join("2026-10-15T18:30:00Z"));
        println!(
            "slot {slot:2}: {:6.0} ms, {tiles_written} tiles written, bucket {:.1} MB",
            elapsed.as_secs_f64() * 1000.0,
            bytes as f64 / 1e6
        );
    }
}

fn walk(dir: &std::path::Path) -> u64 {
    let mut total = 0;
    for entry in std::fs::read_dir(dir).unwrap() {
        let path = entry.unwrap().path();
        total += if path.is_dir() {
            walk(&path)
        } else {
            path.metadata().unwrap().len()
        };
    }
    total
}
