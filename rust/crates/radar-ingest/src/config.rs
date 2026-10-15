//! Environment-driven configuration (spec §3). Tests build this explicitly with tempdirs.

use std::path::PathBuf;
use std::time::Duration;

use radar_core::env::{self, Paths};
use radar_core::grid_dump::GridConfig;
use radar_storms::StormConfig;

/// `HRRR_ENABLED_LAYERS` names that the pipeline knows. `radar-hrrr` is always enabled.
pub const HRRR_LAYERS: [&str; 10] = [
    "radar-hrrr",
    "temperature",
    "feels-like",
    "dewpoint",
    "humidity",
    "wind",
    "cape",
    "precip-type",
    "precip-accum",
    "cloud",
];

/// Layers whose tile trees are swept by `hrrr_cleanup` (spec §12.2 `HRRR_TILE_LAYERS`).
pub const HRRR_TILE_LAYERS: [&str; 10] = HRRR_LAYERS;

/// Default MRMS base URL.
pub const MRMS_BASE_URL: &str = "https://noaa-mrms-pds.s3.amazonaws.com";

/// Everything the ingest activities read from the environment (spec §3).
#[derive(Debug, Clone)]
pub struct IngestConfig {
    /// `STATE_DIR`, `GRID_DIR`, `TILE_DIR`.
    pub paths: Paths,
    /// `MRMS_TMP_ROOT` (default `/tmp/mrms_work`).
    pub mrms_tmp_root: PathBuf,
    /// `HRRR_TMP_ROOT` (default `/tmp/hrrr_work`).
    pub hrrr_tmp_root: PathBuf,
    /// Base URL of the MRMS bucket.
    pub mrms_base_url: String,
    /// Base URL of the HRRR bucket.
    pub hrrr_base_url: String,
    /// `MRMS_PREFIX` default.
    pub mrms_prefix_default: String,
    /// `LAYER_NAME` default.
    pub layer_name_default: String,
    /// `BACKLOG_PER_CYCLE` (default 3).
    pub backlog_per_cycle: usize,
    /// `NOWCAST_KICK_ENABLED == "1"`.
    pub nowcast_kick_enabled: bool,
    /// `NOWCAST_SCHEDULE_ID` (default `nowcast`).
    pub nowcast_schedule_id: String,
    /// `NOWCAST_KICK_TIMEOUT_S` (default 5).
    pub nowcast_kick_timeout: Duration,
    /// `NOWCAST_SCIENCE_GRID_MAX_CELLS` (default 7000000).
    pub nowcast_science_grid_max_cells: usize,
    /// `FORECAST_HOURS` (default 18).
    pub forecast_hours: u32,
    /// `EXTENDED_FORECAST_HOURS` (default 48).
    pub extended_forecast_hours: u32,
    /// `HRRR_ENABLED_LAYERS` after parsing; always contains `radar-hrrr`.
    pub hrrr_enabled_layers: Vec<String>,
    /// `HRRR_GRID_DUMP_STEP_DEG` (default 0.03).
    pub hrrr_grid_dump_step_deg: f64,
    /// `PALETTES` (default `classic`).
    pub palettes: Vec<String>,
    /// Grid retention and cell caps (`GRID_*`, `NOWCAST_INPUT_*`).
    pub grid: GridConfig,
    /// Storm thresholds (`STORM_*`).
    pub storms: StormConfig,
    /// `PACKED_TILES` (default on): write packed data tile buckets for the MRMS layers.
    pub packed_tiles: bool,
    /// `PRECIP_TYPE` (default on): color MRMS frames by precipitation type from PrecipFlag.
    pub precip_type: bool,
    /// `PRECIP_FLAG_PREFIX` (default `CONUS/PrecipFlag_00.00`).
    pub precip_flag_prefix: String,
}

impl IngestConfig {
    /// Read every knob from the environment with the spec defaults.
    pub fn from_env() -> anyhow::Result<Self> {
        let paths = Paths::from_env();
        let enabled_raw = env::var("HRRR_ENABLED_LAYERS").unwrap_or_else(|| "radar-hrrr".into());
        let hrrr_enabled_layers = parse_enabled_layers(&enabled_raw);
        let palettes = radar_tiler::palettes::parse_palette_names(env::var("PALETTES").as_deref());
        Ok(Self {
            mrms_tmp_root: env::path_var("MRMS_TMP_ROOT", "/tmp/mrms_work"),
            hrrr_tmp_root: env::path_var("HRRR_TMP_ROOT", "/tmp/hrrr_work"),
            mrms_base_url: MRMS_BASE_URL.to_string(),
            hrrr_base_url: radar_grib::hrrr::BASE_URL.to_string(),
            mrms_prefix_default: env::var("MRMS_PREFIX")
                .unwrap_or_else(|| "CONUS/MergedBaseReflectivityQC_00.50".into()),
            layer_name_default: env::var("LAYER_NAME").unwrap_or_else(|| "radar".into()),
            backlog_per_cycle: env::int_var_clamped("BACKLOG_PER_CYCLE", 3, 0, i64::MAX)? as usize,
            nowcast_kick_enabled: env::var("NOWCAST_KICK_ENABLED").unwrap_or_else(|| "1".into())
                == "1",
            nowcast_schedule_id: env::var("NOWCAST_SCHEDULE_ID")
                .unwrap_or_else(|| "nowcast".into()),
            nowcast_kick_timeout: Duration::from_secs(env::int_var_clamped(
                "NOWCAST_KICK_TIMEOUT_S",
                5,
                0,
                i64::MAX,
            )? as u64),
            nowcast_science_grid_max_cells: env::int_var_clamped(
                "NOWCAST_SCIENCE_GRID_MAX_CELLS",
                7_000_000,
                1,
                i64::MAX,
            )? as usize,
            forecast_hours: env::int_var_clamped("FORECAST_HOURS", 18, 1, i64::MAX)? as u32,
            extended_forecast_hours: env::int_var_clamped(
                "EXTENDED_FORECAST_HOURS",
                48,
                1,
                i64::MAX,
            )? as u32,
            hrrr_enabled_layers,
            hrrr_grid_dump_step_deg: env::float_var("HRRR_GRID_DUMP_STEP_DEG", 0.03)?,
            palettes,
            grid: GridConfig::from_env()?,
            storms: StormConfig::from_env()?,
            packed_tiles: env::bool_var("PACKED_TILES", true)?,
            precip_type: env::bool_var("PRECIP_TYPE", true)?,
            precip_flag_prefix: env::var("PRECIP_FLAG_PREFIX")
                .unwrap_or_else(|| crate::precip_type::PRECIP_FLAG_PREFIX.into()),
            paths,
        })
    }

    /// Config rooted at explicit directories, with spec defaults everywhere else. Used by tests.
    pub fn with_roots(paths: Paths, tmp_root: PathBuf) -> anyhow::Result<Self> {
        Ok(Self {
            grid: GridConfig::with_dir(paths.grid_dir.clone()),
            storms: StormConfig::from_lookup(|_| None)?,
            mrms_tmp_root: tmp_root.join("mrms"),
            hrrr_tmp_root: tmp_root.join("hrrr"),
            mrms_base_url: MRMS_BASE_URL.to_string(),
            hrrr_base_url: radar_grib::hrrr::BASE_URL.to_string(),
            mrms_prefix_default: "CONUS/MergedBaseReflectivityQC_00.50".into(),
            layer_name_default: "radar".into(),
            backlog_per_cycle: 3,
            nowcast_kick_enabled: false,
            nowcast_schedule_id: "nowcast".into(),
            nowcast_kick_timeout: Duration::from_secs(5),
            nowcast_science_grid_max_cells: 7_000_000,
            forecast_hours: 18,
            extended_forecast_hours: 48,
            hrrr_enabled_layers: vec!["radar-hrrr".into()],
            hrrr_grid_dump_step_deg: 0.03,
            palettes: vec!["classic".into()],
            packed_tiles: true,
            precip_type: false,
            precip_flag_prefix: crate::precip_type::PRECIP_FLAG_PREFIX.into(),
            paths,
        })
    }
}

/// Parse `HRRR_ENABLED_LAYERS`: comma list, `all` enables every layer, `radar-hrrr` always on.
pub fn parse_enabled_layers(raw: &str) -> Vec<String> {
    let names: Vec<&str> = raw
        .split(',')
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .collect();
    let mut out: Vec<String> = if names.contains(&"all") {
        HRRR_LAYERS.iter().map(|s| s.to_string()).collect()
    } else {
        names.iter().map(|s| s.to_string()).collect()
    };
    if !out.iter().any(|n| n == "radar-hrrr") {
        out.push("radar-hrrr".into());
    }
    out
}
