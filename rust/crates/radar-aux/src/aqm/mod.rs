//! NAQFC air quality (AQMv7 CONUS, grid 227): spec-ingest-aux section 5.
//!
//! Activities, in the order the `IngestAirQualityWorkflow` calls them:
//!
//! 1. [`aqm_find_latest_run`]: HEAD probes for the newest cycle, then the processed-set check.
//! 2. [`aqm_render_chunk`]: one (layer, start message) chunk of 24 hourly messages. Several chunks run
//!    in parallel (the workflow bounds them). Each renders tiles for every palette and writes grid dumps.
//! 3. [`aqm_publish_run`]: commits a layer to the manifest only when all 72 frames rendered.
//! 4. [`aqm_mark_processed`]: records the run once `air-quality` published.
//! 5. [`aqm_cleanup`]: removes superseded run directories (and stale download cache entries).
//!
//! Rendering uses the Rust tile renderer (`radar-tiler`, renderer `indexed-rs1`). The Python legacy
//! RGBA renderer is not ported (`radar-tiler` README, deviation 1). Renderer identities never mix on
//! one run (PLAN collision rule), so Python and Rust pyramids can coexist under the same `TILE_DIR`.

mod cleanup;
mod download;
pub mod grib;
mod publish;
mod render;

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

use chrono::{DateTime, Utc};
use radar_core::grid_dump::GridConfig;
use radar_core::processed_set::ProcessedSet;
use radar_grib::naqfc::{self, AqmSources};
use radar_tiler::palettes::Palette;
use radar_types::payloads::airquality::{AqmChunkResult, AqmCleanupResult, AqmFindRunResult};
use tracing::{info, warn};

use crate::beat::Beat;

/// The two layers, in `AQM_LAYERS` insertion order.
pub const AQM_LAYER_SPECS: [LayerSpec; 2] = [
    LayerSpec {
        name: "air-quality",
        product: "ave_1hr_pm25_bc",
        color_key: "pm25",
        // "µg/m³", spelled with escapes so the bytes are explicit (U+00B5, U+00B3).
        unit: "\u{00B5}g/m\u{00B3}",
        title: "AQM PM2.5 (US AQI)",
    },
    LayerSpec {
        name: "ozone",
        product: "ave_1hr_o3_bc",
        color_key: "ozone",
        unit: "ppb",
        title: "AQM ozone",
    },
];

/// Forecast messages in one AQM file (`FORECAST_MESSAGES`).
pub const FORECAST_MESSAGES: usize = 72;
/// Messages per chunk (`CHUNK_MESSAGES`).
pub const CHUNK_MESSAGES: usize = 24;
/// Zoom levels rendered for every frame (`ZOOM_LEVELS`).
pub const ZOOM_LEVELS: [u32; 3] = [4, 5, 6];
/// `max_zoom` written into each frame.
pub const MAX_ZOOM: i64 = 6;
/// `spatial_resolution_km` written into each frame.
pub const SPATIAL_RESOLUTION_KM: f64 = 5.0;
/// Capacity of `STATE_DIR/ingest-airquality.json` (`max_entries`).
pub const PROCESSED_MAX_ENTRIES: usize = 100;
/// Name of the processed-set file in the state directory.
pub const PROCESSED_FILE: &str = "ingest-airquality.json";

/// One pollutant layer: its GRIB product, color key and display metadata.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LayerSpec {
    /// Layer name: tile subtree and manifest key.
    pub name: &'static str,
    /// GRIB2 product file stem (`ave_1hr_pm25_bc` or `ave_1hr_o3_bc`).
    pub product: &'static str,
    /// Key in each palette file (`pm25` or `ozone`).
    pub color_key: &'static str,
    /// Unit label written into grid dumps.
    pub unit: &'static str,
    /// Manifest title.
    pub title: &'static str,
}

/// Looks up a layer by name.
pub fn layer_spec(name: &str) -> Result<&'static LayerSpec, AqmError> {
    AQM_LAYER_SPECS
        .iter()
        .find(|s| s.name == name)
        .ok_or_else(|| AqmError::UnknownLayer(name.to_string()))
}

/// Configuration for the AQM activities. Build it with [`AqmConfig::from_env`] in production.
#[derive(Debug, Clone)]
pub struct AqmConfig {
    /// `TILE_DIR`.
    pub tile_dir: PathBuf,
    /// `STATE_DIR` (manifest, processed set).
    pub state_dir: PathBuf,
    /// `AQM_TMP_ROOT`. Download cache lives under `<tmp_root>/cache/<run_id>/`.
    pub tmp_root: PathBuf,
    /// Grid dump settings (`GRID_DIR`, `GRID_MAX_CELLS`, ...).
    pub grid: GridConfig,
    /// Palette names, from `PALETTES` (default `classic`).
    pub palette_names: Vec<String>,
    /// AWS mirror and NOMADS base URLs.
    pub sources: AqmSources,
    /// Heartbeat period while downloading or rendering (30 s in production).
    pub heartbeat_every: Duration,
    /// Per-read timeout for the GRIB download (300 s, as httpx `timeout=300`).
    pub read_timeout: Duration,
}

impl AqmConfig {
    /// Reads `TILE_DIR`, `STATE_DIR`, `AQM_TMP_ROOT`, the grid settings and `PALETTES`, with the code
    /// defaults (`/data/tiles`, `/data/state`, `/tmp/aqm_work`, `/data/grids`, `classic`).
    pub fn from_env() -> Result<Self, AqmError> {
        Ok(Self {
            tile_dir: radar_core::env::path_var("TILE_DIR", "/data/tiles"),
            state_dir: radar_core::env::path_var("STATE_DIR", "/data/state"),
            tmp_root: radar_core::env::path_var("AQM_TMP_ROOT", "/tmp/aqm_work"),
            grid: GridConfig::from_env()?,
            palette_names: radar_tiler::palette_names(),
            sources: AqmSources::default(),
            heartbeat_every: Duration::from_secs(30),
            read_timeout: Duration::from_secs(300),
        })
    }

    /// Defaults rooted at the given directories, with the production URLs and timings. Used by tests.
    pub fn new(
        tile_dir: impl Into<PathBuf>,
        state_dir: impl Into<PathBuf>,
        grid_dir: impl Into<PathBuf>,
        tmp_root: impl Into<PathBuf>,
    ) -> Self {
        Self {
            tile_dir: tile_dir.into(),
            state_dir: state_dir.into(),
            tmp_root: tmp_root.into(),
            grid: GridConfig::with_dir(grid_dir),
            palette_names: vec!["classic".to_string()],
            sources: AqmSources::default(),
            heartbeat_every: Duration::from_secs(30),
            read_timeout: Duration::from_secs(300),
        }
    }

    /// Path of `STATE_DIR/ingest-airquality.json`.
    pub fn processed_path(&self) -> PathBuf {
        self.state_dir.join(PROCESSED_FILE)
    }
}

/// Errors from the AQM activities. Callers branch on the variant only where noted in the README.
#[derive(Debug, thiserror::Error)]
pub enum AqmError {
    /// A request failed before a status was received.
    #[error("transport error for {url}: {source}")]
    Transport {
        /// The URL that failed.
        url: String,
        /// Underlying reqwest error.
        #[source]
        source: reqwest::Error,
    },
    /// A response had a non-2xx status.
    #[error("HTTP {status} from {url}")]
    Status {
        /// Status code.
        status: u16,
        /// The URL.
        url: String,
    },
    /// A read stalled longer than the read timeout.
    #[error("timed out waiting for {url}")]
    Timeout {
        /// The URL being read.
        url: String,
    },
    /// Every source answered with a non-success status for this run.
    #[error("no AQM source for {run_id}")]
    NoSource {
        /// The run that was requested.
        run_id: String,
    },
    /// The layer name is not one of the two AQM layers.
    #[error("unknown AQM layer {0:?}")]
    UnknownLayer(String),
    /// A GRIB2 message is malformed or truncated.
    #[error("GRIB2 message {index}: {detail}")]
    Message {
        /// Zero-based message index in the file.
        index: usize,
        /// What is wrong.
        detail: String,
    },
    /// A GRIB2 feature the port does not interpret.
    #[error("unsupported GRIB2 feature: {0}")]
    Unsupported(String),
    /// The file has fewer messages than the chunk needs (Python: `grbs.message` raises).
    #[error("GRIB2 file has {found} messages; the chunk needs {needed}")]
    MissingMessage {
        /// Messages in the file.
        found: usize,
        /// Messages required.
        needed: usize,
    },
    /// No palette in `PALETTES` could be loaded.
    #[error("no palette could be loaded from PALETTES")]
    NoPalettes,
    /// Invalid caller input.
    #[error("{0}")]
    Invalid(String),
    /// Decoder error from `radar-grib`.
    #[error(transparent)]
    Grib(#[from] radar_grib::GribError),
    /// Tile renderer error from `radar-tiler`.
    #[error(transparent)]
    Tiler(#[from] radar_tiler::TilerError),
    /// Grid dump error from `radar-core`.
    #[error(transparent)]
    Grid(#[from] radar_core::grid_dump::GridError),
    /// Manifest read or write error from `radar-core`. A corrupt manifest is never overwritten.
    #[error(transparent)]
    Manifest(#[from] radar_core::manifest::ManifestError),
    /// Processed-set write error.
    #[error(transparent)]
    ProcessedSet(#[from] radar_core::processed_set::ProcessedSetError),
    /// Environment configuration error.
    #[error(transparent)]
    Env(#[from] radar_core::env::EnvError),
    /// Filesystem error.
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),
}

/// A palette and the color keys it carries. Publication needs only these (no color tables).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PaletteKeys {
    /// Palette name.
    pub name: String,
    /// Color keys present in the palette file.
    pub keys: BTreeSet<String>,
}

/// Loads the named palettes in order, skipping duplicates and palettes that do not exist (Python logs and
/// skips a missing file). Fails when none loads: the code's legacy fallback lacks the AQ keys (spec Q-4).
pub fn load_palettes(names: &[String]) -> Result<Vec<(String, Palette)>, AqmError> {
    let mut out: Vec<(String, Palette)> = Vec::new();
    for name in names {
        if out.iter().any(|(n, _)| n == name) {
            continue;
        }
        match radar_tiler::load_palette(name) {
            Ok(palette) => out.push((name.clone(), palette)),
            Err(err) => warn!(palette = %name, error = %err, "palette_missing"),
        }
    }
    if out.is_empty() {
        return Err(AqmError::NoPalettes);
    }
    Ok(out)
}

/// Key sets of the loaded palettes, for publication.
pub fn palette_keys(palettes: &[(String, Palette)]) -> Vec<PaletteKeys> {
    palettes
        .iter()
        .map(|(name, palette)| PaletteKeys {
            name: name.clone(),
            keys: palette.color_keys().map(str::to_string).collect(),
        })
        .collect()
}

/// `aqm_find_latest_run`: probes for the newest run and checks the processed set.
///
/// Probes use HEAD on the PM2.5 file, AWS then NOMADS, for the last three UTC days, cycles 12 then 6.
/// Only status 200 counts. `already_processed` is true when the run id is in the processed set.
pub async fn aqm_find_latest_run(
    cfg: &AqmConfig,
    http: &reqwest::Client,
) -> Result<AqmFindRunResult, AqmError> {
    find_latest_run_at(cfg, http, Utc::now()).await
}

/// [`aqm_find_latest_run`] with an explicit clock.
pub async fn find_latest_run_at(
    cfg: &AqmConfig,
    http: &reqwest::Client,
    now: DateTime<Utc>,
) -> Result<AqmFindRunResult, AqmError> {
    let Some(run_id) = naqfc::find_latest_run(http, &cfg.sources, now).await? else {
        return Ok(AqmFindRunResult {
            run_id: None,
            already_processed: false,
        });
    };
    let path = cfg.processed_path();
    let already = tokio::task::spawn_blocking({
        let run_id = run_id.clone();
        move || -> Result<bool, AqmError> {
            Ok(ProcessedSet::open(path, PROCESSED_MAX_ENTRIES)?.contains(&run_id))
        }
    })
    .await
    .map_err(|e| AqmError::Invalid(format!("processed-set task failed: {e}")))??;
    Ok(AqmFindRunResult {
        run_id: Some(run_id),
        already_processed: already,
    })
}

/// `aqm_render_chunk`: downloads the product file (cached per run, see README) and renders messages
/// `start_msg .. min(start_msg + 24, 72)` of one layer.
///
/// Frames are recorded only when every palette rendered (all-or-nothing). The result lists the recorded
/// timestamps in message order. A failed attempt returns an error and writes nothing to the manifest.
/// `beat` receives phase details every `cfg.heartbeat_every` while downloading or rendering.
pub async fn aqm_render_chunk(
    cfg: &AqmConfig,
    http: &reqwest::Client,
    run_id: &str,
    layer: &str,
    start_msg: i64,
    beat: Beat,
) -> Result<AqmChunkResult, AqmError> {
    let spec = layer_spec(layer)?;
    let start = usize::try_from(start_msg)
        .map_err(|_| AqmError::Invalid(format!("negative start_msg {start_msg}")))?;
    let end = (start + CHUNK_MESSAGES).min(FORECAST_MESSAGES);
    let started = std::time::Instant::now();

    let grib_path =
        download::ensure_product_file(cfg, http, run_id, spec, start_msg, &beat).await?;
    if start >= end {
        return Ok(AqmChunkResult {
            layer: layer.to_string(),
            start_msg,
            rendered_timestamps: Vec::new(),
        });
    }

    let palettes = {
        let names = cfg.palette_names.clone();
        tokio::task::spawn_blocking(move || load_palettes(&names))
            .await
            .map_err(|e| AqmError::Invalid(format!("palette task failed: {e}")))??
    };
    let tables = palettes
        .iter()
        .filter(|(_, p)| p.color_keys().any(|k| k == spec.color_key))
        .map(|(name, p)| p.color_table(spec.color_key).map(|t| (name.clone(), t)))
        .collect::<Result<Vec<_>, _>>()?;

    let details = serde_json::json!({
        "phase": "render",
        "run_id": run_id,
        "layer": layer,
        "start_msg": start_msg,
    });
    let beat_fn = {
        let beat = beat.clone();
        move || beat(details.clone())
    };
    let job = render::ChunkJob {
        layer: spec,
        run_id: run_id.to_string(),
        start,
        end,
        grib_path,
        tables,
        tile_dir: cfg.tile_dir.clone(),
        grid: cfg.grid.clone(),
    };
    let timestamps = radar_core::heartbeat::run_blocking_with_heartbeat(
        move || render::render_chunk_blocking(&job),
        cfg.heartbeat_every,
        beat_fn,
    )
    .await?;

    info!(
        run_id,
        layer,
        start_msg,
        rendered = timestamps.len(),
        duration_s = (started.elapsed().as_secs_f64() * 10.0).round() / 10.0,
        "chunk_done"
    );
    Ok(AqmChunkResult {
        layer: layer.to_string(),
        start_msg,
        rendered_timestamps: timestamps,
    })
}

/// `aqm_publish_run`: commits each layer whose 72 frames all rendered. Returns the published layer names
/// in `AQM_LAYERS` order. Partial layers are never written. An ozone-only publish is allowed.
pub async fn aqm_publish_run(
    cfg: &AqmConfig,
    run_id: &str,
    results: &[AqmChunkResult],
) -> Result<Vec<String>, AqmError> {
    let names = cfg.palette_names.clone();
    let keys = tokio::task::spawn_blocking(move || -> Result<Vec<PaletteKeys>, AqmError> {
        Ok(palette_keys(&load_palettes(&names)?))
    })
    .await
    .map_err(|e| AqmError::Invalid(format!("palette task failed: {e}")))??;
    let state_dir = cfg.state_dir.clone();
    let run_id = run_id.to_string();
    let results = results.to_vec();
    tokio::task::spawn_blocking(move || publish::publish_run(&state_dir, &run_id, &results, &keys))
        .await
        .map_err(|e| AqmError::Invalid(format!("publish task failed: {e}")))?
}

/// `aqm_mark_processed`: adds the run id to `STATE_DIR/ingest-airquality.json` (max 100, oldest dropped).
pub async fn aqm_mark_processed(cfg: &AqmConfig, run_id: &str) -> Result<(), AqmError> {
    let path = cfg.processed_path();
    let run_id = run_id.to_string();
    tokio::task::spawn_blocking(move || -> Result<(), AqmError> {
        ProcessedSet::open(path, PROCESSED_MAX_ENTRIES)?.add(&run_id)?;
        Ok(())
    })
    .await
    .map_err(|e| AqmError::Invalid(format!("processed-set task failed: {e}")))?
}

/// `aqm_cleanup`: removes run directories older than `retention_hours` (age from the run's issue time),
/// except the layer's current manifest run. Also prunes the download cache (see README).
pub async fn aqm_cleanup(
    cfg: &AqmConfig,
    retention_hours: i64,
) -> Result<AqmCleanupResult, AqmError> {
    aqm_cleanup_at(cfg, retention_hours, SystemTime::now()).await
}

/// [`aqm_cleanup`] with an explicit clock.
pub async fn aqm_cleanup_at(
    cfg: &AqmConfig,
    retention_hours: i64,
    now: SystemTime,
) -> Result<AqmCleanupResult, AqmError> {
    let cfg = cfg.clone();
    tokio::task::spawn_blocking(move || cleanup::cleanup_blocking(&cfg, retention_hours, now))
        .await
        .map_err(|e| AqmError::Invalid(format!("cleanup task failed: {e}")))?
}

/// The `runs/<run_id>/<ts>` tile path of a frame.
pub fn frame_path(run_id: &str, timestamp: &str) -> String {
    format!("runs/{run_id}/{timestamp}")
}

/// Parses a run id `YYYYMMDD_CC` (Python `strptime("%Y%m%d_%H")`) into its issue time, UTC.
pub fn parse_run_id(run_id: &str) -> Option<DateTime<Utc>> {
    let (date, hour) = run_id.split_once('_')?;
    if date.len() != 8
        || hour.len() != 2
        || !date.bytes().chain(hour.bytes()).all(|b| b.is_ascii_digit())
    {
        return None;
    }
    let day = chrono::NaiveDate::parse_from_str(date, "%Y%m%d").ok()?;
    let hour: u32 = hour.parse().ok()?;
    if hour > 23 {
        return None;
    }
    Some(day.and_hms_opt(hour, 0, 0)?.and_utc())
}

/// Directory of the download cache for one run.
pub(crate) fn cache_dir(tmp_root: &Path, run_id: &str) -> PathBuf {
    tmp_root.join("cache").join(run_id)
}
