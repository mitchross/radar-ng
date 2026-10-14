#![allow(clippy::needless_range_loop)]
//! Wall-time check on the production-size grid (stride-2 of 1750x3500 -> 875x1750).
//! Run with: cargo test -p radar-nowcast --test perf --release -- --ignored --nocapture

use std::time::Instant;

use radar_nowcast::field::Field2;
use radar_nowcast::motion::{MotionParams, Velocity, estimate_motion};
use radar_nowcast::sprog::{SprogParams, forecast};

/// Smooth moving blobs on a 1750x3500 grid, four frames, values in dBZ-like range.
fn synthetic_frames(w: usize, h: usize, n: usize) -> Vec<Field2<f32>> {
    (0..n)
        .map(|t| {
            let mut v = vec![-10.0f32; w * h];
            for b in 0..12 {
                let cx = (w as f64) * (0.1 + 0.07 * b as f64) + 3.0 * t as f64;
                let cy = (h as f64) * (0.2 + 0.05 * b as f64) + 2.0 * t as f64;
                let r = 60.0 + 10.0 * b as f64;
                let y0 = (cy - 3.0 * r).max(0.0) as usize;
                let y1 = ((cy + 3.0 * r) as usize).min(h);
                let x0 = (cx - 3.0 * r).max(0.0) as usize;
                let x1 = ((cx + 3.0 * r) as usize).min(w);
                for y in y0..y1 {
                    for x in x0..x1 {
                        let d2 = (x as f64 - cx).powi(2) + (y as f64 - cy).powi(2);
                        let val = (35.0 * (-d2 / (2.0 * r * r)).exp() - 10.0) as f32;
                        let idx = y * w + x;
                        if val > v[idx] {
                            v[idx] = val;
                        }
                    }
                }
            }
            Field2::new(w, h, v).expect("shape")
        })
        .collect()
}

#[test]
#[ignore = "performance measurement; run in release mode"]
fn perf_motion_and_sprog_875x1750() {
    let (w, h) = (3500usize, 1750usize);
    let frames = synthetic_frames(w, h, 4);

    let t0 = Instant::now();
    let uv: Velocity = estimate_motion(&frames, 2, &MotionParams::radar_ng()).expect("motion");
    let t_motion = t0.elapsed();
    // estimate_motion returns the full-resolution field; LK itself ran on 1750x875.
    assert_eq!(uv.x.width(), w);
    assert_eq!(uv.x.height(), h);

    let t1 = Instant::now();
    let leads: Vec<f64> = (1..=12).map(|i| i as f64).collect();
    let fc = forecast(&frames[1..], &uv, &leads, &SprogParams::radar_ng()).expect("forecast");
    let t_sprog = t1.elapsed();

    println!(
        "perf: motion (LK stride 2 on 875x1750, 4 frames, full-res output {}x{}) {:.3} s; S-PROG 12 leads {:.3} s; fields {}",
        w,
        h,
        t_motion.as_secs_f64(),
        t_sprog.as_secs_f64(),
        fc.fields.len()
    );
    assert_eq!(fc.fields.len(), 12);
}
