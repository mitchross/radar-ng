//! Tile and grid retention: `tile_cleanup_sweep` (spec-ingest-aux section 8).
//!
//! The Python activity is canonical. `cleanup.sh` is retired (DECIDE D-2): it ignored manifest state,
//! never removed versioned runs, and used directory mtime.
//!
//! Ordering rule (spec 8.2): for each expired timestamp directory, the manifest entry is removed first
//! (`remove_frame`, which takes `manifest.lock`), then the directory is deleted. The API therefore never
//! advertises a frame whose tiles are gone. If the manifest write fails, the directory is kept and the
//! sweep fails.

use std::fs;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

use chrono::{DateTime, NaiveDate, Utc};
use radar_core::grid_dump::{GridConfig, GridError, cleanup_old_grids};
use radar_core::manifest::{ManifestError, read_manifest, remove_frame};
use radar_core::time::parse_iso_as_utc;
use radar_types::payloads::cleanup::TileCleanupResult;
use tracing::info;

/// Per-layer retention in minutes (`LAYER_RETENTION_MIN`, verbatim, plus `precip-24h` and the two
/// global-model layers, whose run directories `global_model_cleanup` owns). `air-quality` and `ozone`
/// are not here: `aqm_cleanup` owns them.
pub const LAYER_RETENTION_MIN: [(&str, i64); 15] = [
    ("radar", 240),
    ("radar-composite", 240),
    ("precip-24h", 240),
    ("nowcast", 60),
    ("radar-hrrr", 720),
    ("temperature", 720),
    ("dewpoint", 720),
    ("humidity", 720),
    ("wind", 720),
    ("cape", 720),
    ("precip-type", 720),
    ("precip-accum", 720),
    ("cloud", 720),
    ("radar-gfs", 720),
    ("uv", 2160),
];

/// Age after which an orphaned `.tmp-` staging directory is removed (by its mtime).
pub const STAGING_GRACE_S: f64 = 3600.0;

/// Configuration for the sweep.
#[derive(Debug, Clone)]
pub struct TileCleanupConfig {
    /// `TILE_DIR`.
    pub tile_dir: PathBuf,
    /// `STATE_DIR` (manifest).
    pub state_dir: PathBuf,
    /// Grid retention settings (`GRID_DIR`, `GRID_MAX_AGE_S`, keep-N layers).
    pub grid: GridConfig,
}

impl TileCleanupConfig {
    /// Reads `TILE_DIR`, `STATE_DIR` and the grid settings, with the code defaults.
    pub fn from_env() -> Result<Self, CleanupError> {
        Ok(Self {
            tile_dir: radar_core::env::path_var("TILE_DIR", "/data/tiles"),
            state_dir: radar_core::env::path_var("STATE_DIR", "/data/state"),
            grid: GridConfig::from_env()?,
        })
    }

    /// Config rooted at the given directories, with default grid settings. Used by tests.
    pub fn new(
        tile_dir: impl Into<PathBuf>,
        state_dir: impl Into<PathBuf>,
        grid_dir: impl Into<PathBuf>,
    ) -> Self {
        Self {
            tile_dir: tile_dir.into(),
            state_dir: state_dir.into(),
            grid: GridConfig::with_dir(grid_dir),
        }
    }
}

/// Errors from the sweep. A corrupt manifest is an error, so nothing is deleted (fail closed).
#[derive(Debug, thiserror::Error)]
pub enum CleanupError {
    /// Manifest read or write failed.
    #[error(transparent)]
    Manifest(#[from] ManifestError),
    /// Grid retention failed.
    #[error(transparent)]
    Grid(#[from] GridError),
    /// Grid settings from the environment are invalid.
    #[error(transparent)]
    Env(#[from] radar_core::env::EnvError),
    /// Filesystem error.
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),
    /// The blocking task panicked or was cancelled.
    #[error("cleanup task failed: {0}")]
    Join(String),
}

/// `tile_cleanup_sweep`: sweeps all 14 layers, then the grid store.
pub async fn tile_cleanup_sweep(
    cfg: &TileCleanupConfig,
) -> Result<TileCleanupResult, CleanupError> {
    let cfg = cfg.clone();
    tokio::task::spawn_blocking(move || tile_cleanup_sweep_at(&cfg, SystemTime::now()))
        .await
        .map_err(|e| CleanupError::Join(e.to_string()))?
}

/// [`tile_cleanup_sweep`] with an explicit clock for the tile sweep. Grid retention uses the system clock
/// (radar-core), as the Python does.
pub fn tile_cleanup_sweep_at(
    cfg: &TileCleanupConfig,
    now: SystemTime,
) -> Result<TileCleanupResult, CleanupError> {
    let mut total = 0i64;
    for (layer, retention_min) in LAYER_RETENTION_MIN {
        let removed = sweep_layer(cfg, layer, retention_min, now)?;
        if removed > 0 {
            info!(layer, removed, retention_min, "layer_swept");
        }
        total += removed;
    }
    let grid_files_removed = cleanup_old_grids(&cfg.grid)?;
    Ok(TileCleanupResult {
        layers_swept: LAYER_RETENTION_MIN.len() as i64,
        tile_dirs_removed: total,
        grid_files_removed: grid_files_removed as i64,
    })
}

/// `_sweep_layer(layer, retention_min)`. Returns the number of directories removed.
///
/// Two layouts are handled: legacy `<layer>/<ts>/` and palette `<layer>/<palette>/<ts>/`. Versioned runs
/// live in `<layer>/<palette>/runs/<run>/`. Removal rules:
/// - a versioned run older than the cutoff goes, unless it is the layer's current manifest run (no manifest edit);
/// - a timestamp directory older than the cutoff: manifest `remove` first, then delete;
/// - a `.tmp-` or `.tmp` staging directory goes when its mtime is over an hour old;
/// - a name that does not parse as a timestamp is left alone.
pub fn sweep_layer(
    cfg: &TileCleanupConfig,
    layer: &str,
    retention_min: i64,
    now: SystemTime,
) -> Result<i64, CleanupError> {
    let base = cfg.tile_dir.join(layer);
    if !base.exists() {
        return Ok(0);
    }
    let now_secs = unix_secs(now);
    let cutoff = now_secs - (retention_min as f64) * 60.0;
    let manifest = read_manifest(&cfg.state_dir)?;
    let current_run: Option<String> = manifest
        .layers
        .get(layer)
        .and_then(|l| l.extra.get("run_id"))
        .and_then(|v| v.as_str())
        .map(str::to_string);

    let mut candidates: Vec<PathBuf> = Vec::new();
    let mut versioned: Vec<PathBuf> = Vec::new();
    for entry in sorted_children(&base)? {
        if !entry.is_dir() {
            continue;
        }
        let name = file_name(&entry);
        // Staging leftovers (8.5) are candidates wherever they sit. Python's pseudocode would descend into a
        // top-level `.tmp-` entry as if it were a palette directory and never remove it (deviation D-13).
        if is_staging(&name) || name.as_bytes().first().is_some_and(u8::is_ascii_digit) {
            candidates.push(entry);
            continue;
        }
        let runs_root = entry.join("runs");
        if runs_root.is_dir() {
            versioned.extend(child_dirs(&runs_root)?);
            candidates.extend(
                child_dirs(&entry)?
                    .into_iter()
                    .filter(|p| file_name(p) != "runs"),
            );
        } else {
            candidates.extend(child_dirs(&entry)?);
        }
    }

    let mut removed = 0i64;
    for run_dir in versioned {
        let name = file_name(&run_dir);
        if current_run.as_deref() == Some(name.as_str()) {
            continue;
        }
        let Some(issued) = parse_stamp(&name) else {
            continue;
        };
        if unix_secs_of(&issued) < cutoff {
            let _ = fs::remove_dir_all(&run_dir);
            removed += 1;
        }
    }

    for ts_dir in candidates {
        let name = file_name(&ts_dir);
        if is_staging(&name) {
            if let Ok(modified) = ts_dir.metadata().and_then(|m| m.modified())
                && unix_secs(modified) < now_secs - STAGING_GRACE_S
            {
                let _ = fs::remove_dir_all(&ts_dir);
                removed += 1;
            }
            continue;
        }
        let Some(valid) = parse_stamp(&name) else {
            continue;
        };
        if unix_secs_of(&valid) < cutoff {
            // Manifest first: de-list the frame before its tiles disappear.
            remove_frame(&cfg.state_dir, layer, &name)?;
            let _ = fs::remove_dir_all(&ts_dir);
            removed += 1;
        }
    }
    Ok(removed)
}

/// Parses a frame or run directory name as a timestamp. ISO 8601 with an offset is exact. A naive value or a
/// bare date is taken as UTC (the container is UTC; Python's naive `.timestamp()` would use local time).
pub fn parse_stamp(name: &str) -> Option<DateTime<Utc>> {
    if let Some(dt) = parse_iso_as_utc(name) {
        return Some(dt);
    }
    NaiveDate::parse_from_str(name, "%Y-%m-%d")
        .ok()
        .and_then(|d| d.and_hms_opt(0, 0, 0))
        .map(|n| n.and_utc())
}

/// Atomic-render staging names (spec 8.5): contains `.tmp-` or ends in `.tmp`.
fn is_staging(name: &str) -> bool {
    name.contains(".tmp-") || name.ends_with(".tmp")
}

fn file_name(path: &Path) -> String {
    path.file_name()
        .and_then(|n| n.to_str())
        .unwrap_or_default()
        .to_string()
}

fn sorted_children(dir: &Path) -> Result<Vec<PathBuf>, std::io::Error> {
    let mut out: Vec<PathBuf> = fs::read_dir(dir)?
        .map(|e| e.map(|e| e.path()))
        .collect::<Result<_, _>>()?;
    out.sort();
    Ok(out)
}

fn child_dirs(dir: &Path) -> Result<Vec<PathBuf>, std::io::Error> {
    Ok(fs::read_dir(dir)?
        .map(|e| e.map(|e| e.path()))
        .collect::<Result<Vec<_>, _>>()?
        .into_iter()
        .filter(|p| p.is_dir())
        .collect())
}

fn unix_secs(t: SystemTime) -> f64 {
    match t.duration_since(SystemTime::UNIX_EPOCH) {
        Ok(d) => d.as_secs_f64(),
        Err(e) => -e.duration().as_secs_f64(),
    }
}

fn unix_secs_of(dt: &DateTime<Utc>) -> f64 {
    dt.timestamp_micros() as f64 / 1_000_000.0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn retention_table_has_fifteen_layers_and_no_aqm() {
        assert_eq!(LAYER_RETENTION_MIN.len(), 15);
        assert!(
            LAYER_RETENTION_MIN
                .iter()
                .all(|(l, _)| *l != "air-quality" && *l != "ozone")
        );
    }

    #[test]
    fn stamps_parse_with_offset_naive_and_date_only() {
        assert!(parse_stamp("2026-07-16T12:00:00+00:00").is_some());
        assert!(parse_stamp("2026-07-16T12:00:00").is_some());
        assert!(parse_stamp("2026-07-16").is_some());
        assert!(parse_stamp(".2026-07-16T12:00:00+00:00.tmp-abc").is_none());
        assert!(parse_stamp("not-a-time").is_none());
    }
}
