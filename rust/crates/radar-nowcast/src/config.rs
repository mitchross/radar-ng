//! Runtime configuration for `nowcast_run` (spec-nowcast section 2.3).
//!
//! [`NowcastConfig::from_env`] reads the variables with the spec defaults. Values Python clamps
//! (`max(3, ...)` for the input frames, `max(1, ...)` for the stride) are clamped the same way.

use std::time::Duration;

use radar_core::env::{self, EnvError, Paths};
use radar_core::grid_dump::GridConfig;
use radar_core::nowcast_skill::SkillConfig;
use radar_tiler::palettes::parse_palette_names;

/// Layer that holds the 5-minute science grids (`NOWCAST_GRID_INPUT_LAYER`).
pub const DEFAULT_INPUT_LAYER: &str = "radar-nowcast-input";
/// Lead-time zoom levels (spec 8.5, `ZOOM_LEVELS`).
pub const ZOOM_LEVELS: [u32; 3] = [4, 5, 6];
/// Minimum number of input grids S-PROG needs (`N_INPUT_FRAMES` clamp).
pub const MIN_INPUT_FRAMES: usize = 3;

/// Every knob of one nowcast run.
#[derive(Debug, Clone, PartialEq)]
pub struct NowcastConfig {
    /// `STATE_DIR`, `GRID_DIR` and `TILE_DIR`.
    pub paths: Paths,
    /// Grid writer and retention settings (`GRID_*`, `NOWCAST_INPUT_*`).
    pub grid: GridConfig,
    /// `NOWCAST_HORIZON_MIN`: forecast horizon in minutes.
    pub horizon_min: i64,
    /// `NOWCAST_STEP_MIN`: output cadence in minutes.
    pub step_min: i64,
    /// `NOWCAST_INPUT_FRAMES` clamped to at least [`MIN_INPUT_FRAMES`].
    pub input_frames: usize,
    /// `NOWCAST_INPUT_STEP_MIN`: target spacing of the input grids, minutes.
    pub input_step_min: f64,
    /// `NOWCAST_GRID_INPUT_LAYER`.
    pub input_layer: String,
    /// `NOWCAST_MAX_INPUT_GAP_MIN`: cadence limit is `max(this, 1.5 * input_step_min)`.
    pub max_input_gap_min: f64,
    /// `NOWCAST_ALLOW_PERSISTENCE_FALLBACK` (DEC-7, off by default).
    pub allow_persistence_fallback: bool,
    /// `NOWCAST_MOTION_STRIDE`: stride of the Lucas-Kanade grid, at least 1.
    pub motion_stride: usize,
    /// `NOWCAST_POINT_GRID_MAX_CELLS`.
    pub point_grid_max_cells: usize,
    /// `NOWCAST_POINT_GRID_RETENTION_RUNS`, at least 2.
    pub retention_runs: u64,
    /// `NOWCAST_RENDER_WORKERS`: threads that render tiles within each lead in parallel.
    pub render_workers: usize,
    /// `PALETTES`: palette names, in order.
    pub palettes: Vec<String>,
    /// `TILE_PNG_COMPRESS_LEVEL`, `0..=9`.
    pub png_compress_level: u32,
    /// `NOWCAST_SKILL_*`.
    pub skill: SkillConfig,
    /// `NOWCAST_SKILL_MATCH_TOLERANCE_S`: a forecast valid time matches an observation within this many seconds.
    pub skill_match_tolerance_s: f64,
    /// Interval of the heartbeat callback (30 s in production, spec 2.1).
    pub heartbeat_every: Duration,
    /// `PACKED_TILES` (default on): write a packed data tile bucket per run.
    pub packed_tiles: bool,
}

impl NowcastConfig {
    /// Read every variable from the environment with the spec defaults.
    pub fn from_env() -> Result<Self, EnvError> {
        let paths = Paths::from_env();
        let grid = GridConfig::from_env()?;
        let step_min = env::int_var_clamped("NOWCAST_STEP_MIN", 5, 1, i64::MAX)?;
        let png_compress_level = env::int_var_clamped("TILE_PNG_COMPRESS_LEVEL", 6, 0, 9)? as u32;
        Ok(Self {
            paths,
            grid,
            horizon_min: env::int_var_clamped("NOWCAST_HORIZON_MIN", 60, step_min, i64::MAX)?,
            step_min,
            input_frames: env::int_var_clamped(
                "NOWCAST_INPUT_FRAMES",
                4,
                MIN_INPUT_FRAMES as i64,
                i64::MAX,
            )? as usize,
            input_step_min: env::float_var("NOWCAST_INPUT_STEP_MIN", 5.0)?,
            input_layer: env::var("NOWCAST_GRID_INPUT_LAYER")
                .unwrap_or_else(|| DEFAULT_INPUT_LAYER.to_string()),
            max_input_gap_min: env::float_var("NOWCAST_MAX_INPUT_GAP_MIN", 6.0)?,
            allow_persistence_fallback: env::bool_var("NOWCAST_ALLOW_PERSISTENCE_FALLBACK", false)?,
            motion_stride: env::int_var_clamped("NOWCAST_MOTION_STRIDE", 2, 1, i64::MAX)? as usize,
            point_grid_max_cells: env::int_var_clamped(
                "NOWCAST_POINT_GRID_MAX_CELLS",
                900_000,
                1,
                i64::MAX,
            )? as usize,
            retention_runs: env::int_var_clamped(
                "NOWCAST_POINT_GRID_RETENTION_RUNS",
                36,
                2,
                i64::MAX,
            )? as u64,
            render_workers: env::int_var_clamped("NOWCAST_RENDER_WORKERS", 4, 1, i64::MAX)?
                as usize,
            palettes: parse_palette_names(env::var("PALETTES").as_deref()),
            png_compress_level,
            skill: SkillConfig::from_env()?,
            skill_match_tolerance_s: env::float_var("NOWCAST_SKILL_MATCH_TOLERANCE_S", 150.0)?,
            heartbeat_every: Duration::from_secs(30),
            packed_tiles: env::bool_var("PACKED_TILES", true)?,
        })
    }

    /// Defaults (spec 2.3) rooted at explicit directories. Used by tests and the benchmark. Does not read
    /// the environment.
    pub fn with_paths(paths: Paths) -> Self {
        let grid = GridConfig::with_dir(paths.grid_dir.clone());
        Self {
            grid,
            paths,
            horizon_min: 60,
            step_min: 5,
            input_frames: 4,
            input_step_min: 5.0,
            input_layer: DEFAULT_INPUT_LAYER.to_string(),
            max_input_gap_min: 6.0,
            allow_persistence_fallback: false,
            motion_stride: 2,
            point_grid_max_cells: 900_000,
            retention_runs: 36,
            render_workers: 4,
            palettes: vec!["classic".to_string()],
            png_compress_level: 6,
            skill: SkillConfig::default(),
            skill_match_tolerance_s: 150.0,
            heartbeat_every: Duration::from_secs(30),
            packed_tiles: true,
        }
    }
}
