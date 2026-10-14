//! Benchmark entry point for the benchmark harness (`bench/`, PLAN "nowcast" row).
//!
//! [`bench_nowcast`] runs the same input selection, loading, masking, motion and S-PROG as `nowcast_run`
//! on the grid dumps in a directory, with no rendering and no writes except one JSON report. Motion and
//! forecast are timed separately. The checksum lets two builds be compared for identical output.

use std::path::Path;
use std::time::Instant;

use anyhow::{Context, bail};
use radar_core::env::Paths;
use serde::Serialize;

use crate::config::{MIN_INPUT_FRAMES, NowcastConfig};
use crate::field::Field2;
use crate::inputs;
use crate::motion::{MotionParams, estimate_motion};
use crate::select::{input_interval_min, lead_steps};
use crate::sprog::{SprogParams, forecast};

/// File written into the `out` directory.
pub const REPORT_FILE: &str = "nowcast-bench.json";

/// Timings and sizes of one benchmark run. Written as JSON to `out/nowcast-bench.json`.
#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct BenchReport {
    /// Stems of the grids used, oldest first.
    pub frames: Vec<String>,
    /// Science-grid width (columns) and height (rows).
    pub width: usize,
    /// Science-grid height (rows).
    pub height: usize,
    /// Motion stride (`NOWCAST_MOTION_STRIDE`).
    pub motion_stride: usize,
    /// Number of leads forecast.
    pub leads: usize,
    /// Rayon worker threads available to the run.
    pub threads: usize,
    /// Wall seconds of `estimate_motion`.
    pub motion_s: f64,
    /// Wall seconds of `forecast` (S-PROG, all leads).
    pub forecast_s: f64,
    /// Wall seconds of motion plus forecast.
    pub total_s: f64,
    /// True when the window was dry (S-PROG returned the constant minimum).
    pub no_rain: bool,
    /// Sum of the finite forecast values, in lead order. Equal across builds when the output is equal.
    pub checksum: f64,
}

/// Time motion and S-PROG on the grid dumps in `frames_dir` and write `out/nowcast-bench.json`.
///
/// `frames_dir` is a grid layer directory (`<layer>/<ts>.meta.json` plus `.bin`). Four grids are chosen with
/// the production rules. Errors cover missing or unreadable inputs and a failed forecast.
pub fn bench_nowcast(frames_dir: &Path, out: &Path) -> anyhow::Result<BenchReport> {
    let paths = Paths::new(out, frames_dir, out);
    let cfg = NowcastConfig::with_paths(paths);
    std::fs::create_dir_all(out).with_context(|| format!("creating {}", out.display()))?;

    let candidates = inputs::list_metas(frames_dir);
    let chosen =
        crate::select::select_input_grids(&candidates, cfg.input_step_min, cfg.input_frames);
    let mut loaded = Vec::new();
    for c in &chosen {
        let Some(grid) = inputs::load_grid(&c.item) else {
            bail!("cannot load grid {}", c.item.display());
        };
        loaded.push((grid, c.time, c.stem.clone()));
    }
    if loaded.len() < MIN_INPUT_FRAMES {
        bail!(
            "need at least {MIN_INPUT_FRAMES} grids in {}",
            frames_dir.display()
        );
    }
    let times: Vec<_> = loaded.iter().filter_map(|(_, t, _)| *t).collect();
    let interval = input_interval_min(&times, cfg.max_input_gap_min, cfg.input_step_min)
        .map_err(|iv| anyhow::anyhow!("invalid input cadence, intervals_minutes={iv:?}"))?;
    let (width, height) = (loaded[0].0.values.width(), loaded[0].0.values.height());
    let frames_names: Vec<String> = loaded.iter().map(|(_, _, s)| s.clone()).collect();
    let mut frames: Vec<Field2<f32>> = loaded.into_iter().map(|(g, _, _)| g.values).collect();
    for f in &mut frames {
        f.mask_below(-100.0);
    }
    let steps = lead_steps(cfg.horizon_min, cfg.step_min, interval);

    let t0 = Instant::now();
    let uv =
        estimate_motion(&frames, cfg.motion_stride, &MotionParams::radar_ng()).context("motion")?;
    let motion_s = t0.elapsed().as_secs_f64();

    let t1 = Instant::now();
    let fc = forecast(&frames, &uv, &steps, &SprogParams::radar_ng()).context("S-PROG")?;
    let forecast_s = t1.elapsed().as_secs_f64();

    let checksum: f64 = fc
        .fields
        .iter()
        .flat_map(|f| f.as_slice().iter())
        .filter(|v| v.is_finite())
        .map(|&v| f64::from(v))
        .sum();
    let report = BenchReport {
        frames: frames_names,
        width,
        height,
        motion_stride: cfg.motion_stride,
        leads: steps.len(),
        threads: rayon::current_num_threads(),
        motion_s,
        forecast_s,
        total_s: motion_s + forecast_s,
        no_rain: fc.no_rain,
        checksum,
    };
    let text = serde_json::to_string_pretty(&report)?;
    std::fs::write(out.join(REPORT_FILE), text + "\n")
        .with_context(|| format!("writing {REPORT_FILE}"))?;
    Ok(report)
}
