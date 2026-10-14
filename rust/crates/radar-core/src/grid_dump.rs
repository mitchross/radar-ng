//! Grid dumps: the byte-exact writer, generation marker, and retention (spec-shared §5).
//!
//! Layout under `<GRID_DIR>/<layer>/`:
//! - `<key>.<gen32>.bin`: little-endian f32, row-major, row 0 north. Written once, under its final unique name.
//! - `<key>.meta.json`: commit pointer (writer style A: sorted keys, compact, trailing newline). Replaced
//!   atomically, so readers see either the old or the new generation.
//! - `.grid.lock`: `flock` sidecar. Metadata commits, marker writes and prunes all serialize on it.
//! - `runs/<run>/`: run-scoped grids with a `.grid-generation-complete-v1.json` marker.
//!
//! Byte-compatibility with Python: a golden test (`testdata/grid/`) regenerates the Python output for fixed
//! inputs and a fixed generation id, and compares every byte. Read the code for the exact operation order:
//! layer lock, then directory create and fsync, then `.bin` write and fsync, then meta tmp, rename and
//! fsync.
//!
//! Deliberate deviations (details in README.md):
//! - Stored shape after stride uses `ceil(n / stride)`. The Python `numpy` slice does the same, even though the
//!   cell cap is computed with floor division.
//! - Non-finite `lats`/`lons` are rejected. Python would write NaN into the metadata.
//! - D11: `cleanup_old_grids` also removes `runs/<gen>` directories older than `GRID_MAX_AGE_S`, complete or not.

use crate::env::{self, EnvError};
use crate::fsutil::{self, DirSync, FlockGuard};
use crate::pycompat::{self, Json, Style};
use serde_json::Value;
use std::collections::{BTreeMap, BTreeSet, HashSet};
use std::fs::{self, File};
use std::io::{self, Write};
use std::path::{Component, Path, PathBuf};
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use thiserror::Error;

/// Name of the per-layer lock sidecar (`_LAYER_LOCK_FILE`).
pub const LAYER_LOCK_FILE: &str = ".grid.lock";
/// Directory under a layer that holds run-scoped generations.
pub const GENERATION_ROOT: &str = "runs";
/// Completion marker inside a run directory.
pub const GENERATION_COMPLETE_FILE: &str = ".grid-generation-complete-v1.json";
/// No-data sentinel written when the caller passes a NaN `fill`.
pub const FILL_SENTINEL: f64 = -9999.0;
/// `GRID_MAX_CELLS` default: 900 * 900.
pub const DEFAULT_MAX_CELLS: usize = 900 * 900;
/// Name of the nowcast input layer, keyed by default.
pub const NOWCAST_INPUT_LAYER: &str = "radar-nowcast-input";

/// Errors from grid writes, finalization and retention.
#[derive(Debug, Error)]
pub enum GridError {
    /// Filesystem failure. Propagates from fsync and rename, as the Python tests require.
    #[error("grid I/O: {0}")]
    Io(#[from] io::Error),
    /// A grid key is empty or contains an empty, `.` or `..` segment.
    #[error("invalid grid key {0:?}")]
    InvalidKey(String),
    /// The input cannot be written (empty axis, non-finite coordinate, bad generation).
    #[error("invalid grid input: {0}")]
    InvalidInput(String),
    /// Environment configuration is invalid.
    #[error(transparent)]
    Env(#[from] EnvError),
}

/// Retention and location settings. Read once at startup with [`GridConfig::from_env`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GridConfig {
    /// `GRID_DIR`.
    pub grid_dir: PathBuf,
    /// `GRID_MAX_AGE_S`: age sweep cutoff in seconds.
    pub max_age_s: u64,
    /// `GRID_MAX_CELLS`: default cell cap for [`write_grid`] (floored at 1).
    pub max_cells: usize,
    /// `GRID_ORPHAN_GRACE_S`: grace before an unreferenced `.bin` is removed.
    pub orphan_grace_s: u64,
    /// `GRID_KEEP_LAST`: keep-N per layer. Built from `NOWCAST_INPUT_FRAMES` and `NOWCAST_INPUT_STEP_MIN`.
    pub keep_last: BTreeMap<String, u64>,
}

/// keep-N for a layer that reads `frames` inputs spaced `step_min` apart (spec §5.1).
pub fn keep_for_nowcast_inputs(frames: i64, step_min: f64) -> u64 {
    let stride = (step_min / 1.8).ceil().max(1.0) as i64;
    let keep = (frames - 1) * stride + 3;
    keep.max(0) as u64
}

impl GridConfig {
    /// Read `GRID_DIR`, `GRID_MAX_AGE_S`, `GRID_MAX_CELLS`, `GRID_ORPHAN_GRACE_S`, `NOWCAST_INPUT_FRAMES`,
    /// `NOWCAST_INPUT_STEP_MIN` and `NOWCAST_GRID_INPUT_LAYER`, with the spec defaults.
    pub fn from_env() -> Result<Self, EnvError> {
        let frames = env::int_var_clamped("NOWCAST_INPUT_FRAMES", 4, 3, i64::MAX)?;
        let step = env::float_var("NOWCAST_INPUT_STEP_MIN", 5.0)?;
        let layer =
            env::var("NOWCAST_GRID_INPUT_LAYER").unwrap_or_else(|| NOWCAST_INPUT_LAYER.to_string());
        let mut keep_last = BTreeMap::new();
        keep_last.insert(layer, keep_for_nowcast_inputs(frames, step));
        Ok(Self {
            grid_dir: env::path_var("GRID_DIR", "/data/grids"),
            max_age_s: env::int_var_clamped("GRID_MAX_AGE_S", 43200, 0, i64::MAX)? as u64,
            max_cells: env::int_var_clamped(
                "GRID_MAX_CELLS",
                DEFAULT_MAX_CELLS as i64,
                1,
                i64::MAX,
            )? as usize,
            orphan_grace_s: env::int_var_clamped("GRID_ORPHAN_GRACE_S", 300, 0, i64::MAX)? as u64,
            keep_last,
        })
    }

    /// Defaults rooted at `grid_dir`, with keep-N for the nowcast input layer.
    pub fn with_dir(grid_dir: impl Into<PathBuf>) -> Self {
        let mut keep_last = BTreeMap::new();
        keep_last.insert(
            NOWCAST_INPUT_LAYER.to_string(),
            keep_for_nowcast_inputs(4, 5.0),
        );
        Self {
            grid_dir: grid_dir.into(),
            max_age_s: 43200,
            max_cells: DEFAULT_MAX_CELLS,
            orphan_grace_s: 300,
            keep_last,
        }
    }

    fn layer_dir(&self, layer: &str) -> PathBuf {
        self.grid_dir.join(layer)
    }
}

/// One grid to write. `data` is row-major with `lats.len()` rows and `lons.len()` columns.
#[derive(Debug, Clone, Copy)]
pub struct GridInput<'a> {
    /// Row-major values, `lats.len() * lons.len()` long.
    pub data: &'a [f32],
    /// Row coordinates in degrees (ascending or descending).
    pub lats: &'a [f64],
    /// Column coordinates in degrees (either 0..360 or -180..180).
    pub lons: &'a [f64],
    /// Free-form unit label. Written with JSON `ensure_ascii` escaping.
    pub unit: &'a str,
    /// No-data sentinel. NaN means: replace NaN cells with `-9999.0`.
    pub fill: f64,
    /// Cell cap override. `None` or `Some(0)` falls back to [`GridConfig::max_cells`].
    pub max_cells: Option<usize>,
    /// Relative key below the layer. Defaults to the timestamp. Run writers use `runs/<run>/<valid>`.
    pub grid_key: Option<&'a str>,
}

impl<'a> GridInput<'a> {
    /// Input with NaN fill, the default cell cap, and the timestamp as key.
    pub fn new(data: &'a [f32], lats: &'a [f64], lons: &'a [f64], unit: &'a str) -> Self {
        Self {
            data,
            lats,
            lons,
            unit,
            fill: f64::NAN,
            max_cells: None,
            grid_key: None,
        }
    }
}

/// Write one grid (`write_grid`). Returns the `.bin` path, or `Ok(None)` when the shape does not match.
pub fn write_grid(
    cfg: &GridConfig,
    layer: &str,
    timestamp: &str,
    input: &GridInput<'_>,
) -> Result<Option<PathBuf>, GridError> {
    write_grid_with(
        cfg,
        layer,
        timestamp,
        input,
        &fsutil::fsync_dir,
        &new_generation(),
    )
}

/// A fresh generation id: `uuid4().hex`, 32 lowercase hex characters.
pub fn new_generation() -> String {
    uuid::Uuid::new_v4().simple().to_string()
}

/// [`write_grid`] with the directory-sync step and generation injected (tests and golden fixtures).
pub fn write_grid_with(
    cfg: &GridConfig,
    layer: &str,
    timestamp: &str,
    input: &GridInput<'_>,
    dir_sync: DirSync<'_>,
    generation: &str,
) -> Result<Option<PathBuf>, GridError> {
    let (mut h, mut w) = (input.lats.len(), input.lons.len());
    if input.data.len() != h * w {
        return Ok(None);
    }
    if h == 0 || w == 0 {
        return Err(GridError::InvalidInput(
            "empty latitude or longitude axis".into(),
        ));
    }
    if input
        .lats
        .iter()
        .chain(input.lons.iter())
        .any(|v| !v.is_finite())
    {
        return Err(GridError::InvalidInput(
            "non-finite latitude or longitude".into(),
        ));
    }
    let mut data: Vec<f32> = input.data.to_vec();
    let mut lats: Vec<f64> = input.lats.to_vec();
    let mut lons: Vec<f64> = input.lons.to_vec();

    // Latitude orientation: store north first. Only the endpoints are compared.
    if lats[0] < lats[h - 1] {
        for r in 0..h / 2 {
            let (a, b) = (r * w, (h - 1 - r) * w);
            for c in 0..w {
                data.swap(a + c, b + c);
            }
        }
        lats.reverse();
    }

    // Longitude normalization: shift values above 180 and sort columns ascending.
    let max_lon = lons.iter().copied().fold(f64::NEG_INFINITY, f64::max);
    if max_lon > 180.0 {
        for lon in lons.iter_mut() {
            if *lon > 180.0 {
                *lon -= 360.0;
            }
        }
        let mut order: Vec<usize> = (0..w).collect();
        order.sort_by(|&a, &b| lons[a].partial_cmp(&lons[b]).expect("finite longitudes"));
        let reordered_lons: Vec<f64> = order.iter().map(|&i| lons[i]).collect();
        let mut reordered = Vec::with_capacity(h * w);
        for r in 0..h {
            for &c in &order {
                reordered.push(data[r * w + c]);
            }
        }
        lons = reordered_lons;
        data = reordered;
    }

    // Stride decimation to the cell cap. The loop condition uses floor division, as the Python does.
    let cell_limit = input
        .max_cells
        .filter(|&m| m > 0)
        .unwrap_or(cfg.max_cells)
        .max(1);
    let mut stride = 1usize;
    while (h / stride) * (w / stride) > cell_limit {
        stride *= 2;
    }
    if stride > 1 {
        let (new_h, new_w) = (h.div_ceil(stride), w.div_ceil(stride));
        let mut decimated = Vec::with_capacity(new_h * new_w);
        for r in (0..h).step_by(stride) {
            for c in (0..w).step_by(stride) {
                decimated.push(data[r * w + c]);
            }
        }
        data = decimated;
        lats = lats.iter().step_by(stride).copied().collect();
        lons = lons.iter().step_by(stride).copied().collect();
        h = new_h;
        w = new_w;
    }

    // Dtype and fill: only the NaN-fill path rewrites NaN cells (D10).
    let fill_val = if input.fill.is_nan() {
        for v in data.iter_mut() {
            if v.is_nan() {
                *v = FILL_SENTINEL as f32;
            }
        }
        FILL_SENTINEL
    } else {
        input.fill
    };

    let layer_dir = cfg.layer_dir(layer);
    let key = input
        .grid_key
        .filter(|k| !k.is_empty())
        .unwrap_or(timestamp);
    let relative = relative_grid_key(key)?;
    let out_base = layer_dir.join(&relative);
    let parent = out_base.parent().unwrap_or(&layer_dir).to_path_buf();
    let base_name = out_base
        .file_name()
        .and_then(|n| n.to_str())
        .ok_or_else(|| GridError::InvalidKey(key.to_string()))?
        .to_string();
    let bin_name = format!("{base_name}.{generation}.bin");
    let bin_path = parent.join(&bin_name);
    let meta_name = format!("{base_name}.meta.json");
    let meta_path = parent.join(&meta_name);

    let lat_min = lats.iter().copied().fold(f64::INFINITY, f64::min);
    let lat_max = lats.iter().copied().fold(f64::NEG_INFINITY, f64::max);
    let lon_min = lons.iter().copied().fold(f64::INFINITY, f64::min);
    let lon_max = lons.iter().copied().fold(f64::NEG_INFINITY, f64::max);
    let meta = Json::Object(vec![
        ("height".into(), Json::Int(h as i64)),
        ("width".into(), Json::Int(w as i64)),
        ("lat_min".into(), Json::Float(lat_min)),
        ("lat_max".into(), Json::Float(lat_max)),
        ("lon_min".into(), Json::Float(lon_min)),
        ("lon_max".into(), Json::Float(lon_max)),
        ("unit".into(), Json::Str(input.unit.to_string())),
        ("fill".into(), Json::Float(fill_val)),
        ("stride".into(), Json::Int(stride as i64)),
        ("data_file".into(), Json::Str(bin_name.clone())),
    ]);
    let mut meta_text = pycompat::dumps(&meta, Style::Compact, true);
    meta_text.push('\n');

    let mut bytes = Vec::with_capacity(data.len() * 4);
    for v in &data {
        bytes.extend_from_slice(&v.to_le_bytes());
    }

    let _lock = layer_lock(&layer_dir, dir_sync)?;
    fsutil::durable_mkdir(&parent, dir_sync)?;
    {
        let mut file = File::create(&bin_path)?;
        file.write_all(&bytes)?;
        file.flush()?;
        file.sync_all()?;
    }
    dir_sync(&parent)?;
    fsutil::atomic_write_with(
        &meta_path,
        meta_text.as_bytes(),
        &format!(".{meta_name}."),
        ".tmp",
        dir_sync,
    )?;
    Ok(Some(bin_path))
}

/// `finalize_grid_generation`: mark `runs/<generation>` complete. Writes the marker atomically under the lock.
/// Returns the generation directory.
pub fn finalize_grid_generation(
    cfg: &GridConfig,
    layer: &str,
    generation: &str,
) -> Result<PathBuf, GridError> {
    let relative = relative_grid_key(generation)?;
    if relative.components().count() != 1 {
        return Err(GridError::InvalidInput(format!(
            "invalid grid generation {generation:?}"
        )));
    }
    let layer_dir = cfg.layer_dir(layer);
    let generation_dir = layer_dir.join(GENERATION_ROOT).join(&relative);
    let _lock = layer_lock(&layer_dir, &fsutil::fsync_dir)?;
    if !generation_dir.is_dir() {
        return Err(GridError::Io(io::Error::new(
            io::ErrorKind::NotFound,
            format!("{}: no such generation directory", generation_dir.display()),
        )));
    }
    let meta_names = meta_file_names(&generation_dir)?;
    if meta_names.is_empty() {
        return Err(GridError::InvalidInput(format!(
            "grid generation has no metadata: {}",
            generation_dir.display()
        )));
    }
    let marker = Json::Object(vec![
        ("schema_version".into(), Json::Int(1)),
        ("generation".into(), Json::Str(generation.to_string())),
        ("grid_count".into(), Json::Int(meta_names.len() as i64)),
        (
            "grid_metadata".into(),
            Json::Array(meta_names.iter().map(|n| Json::Str(n.clone())).collect()),
        ),
    ]);
    let mut text = pycompat::dumps(&marker, Style::Compact, true);
    text.push('\n');
    fsutil::atomic_write(
        &generation_dir.join(GENERATION_COMPLETE_FILE),
        text.as_bytes(),
        &format!(".{GENERATION_COMPLETE_FILE}."),
        ".tmp",
    )?;
    Ok(generation_dir)
}

/// `_is_complete_grid_generation`: the marker exists, parses, and its `generation`, `grid_count` and
/// `grid_metadata` match the directory right now. Adding a meta invalidates the generation.
pub fn is_complete_generation(path: &Path) -> bool {
    let Ok(Some(text)) = fsutil::read_text_opt(&path.join(GENERATION_COMPLETE_FILE)) else {
        return false;
    };
    let Ok(Value::Object(doc)) = serde_json::from_str::<Value>(&text) else {
        return false;
    };
    let Ok(names) = meta_file_names(path) else {
        return false;
    };
    let schema_ok = doc.get("schema_version").and_then(pycompat::py_int) == Some(1);
    let generation_ok = path
        .file_name()
        .and_then(|n| n.to_str())
        .is_some_and(|n| doc.get("generation") == Some(&Value::String(n.to_string())));
    let count_ok = doc.get("grid_count").and_then(pycompat::py_int) == Some(names.len() as i64);
    let listed = Value::Array(names.iter().map(|n| Value::String(n.clone())).collect());
    schema_ok && generation_ok && count_ok && doc.get("grid_metadata") == Some(&listed)
}

/// Retention for run-scoped layers: keep the newest `keep` complete generations (plus `active_generation`
/// even when it is older or incomplete). In-progress directories (no marker) are never removed here.
/// Returns the number of files removed.
pub fn prune_grid_generations(
    cfg: &GridConfig,
    layer: &str,
    keep: u64,
    active_generation: Option<&str>,
) -> Result<usize, GridError> {
    if let Some(active) = active_generation {
        let relative = relative_grid_key(active)?;
        if relative.components().count() != 1 {
            return Err(GridError::InvalidInput(format!(
                "invalid active grid generation {active:?}"
            )));
        }
    }
    let layer_dir = cfg.layer_dir(layer);
    let root = layer_dir.join(GENERATION_ROOT);
    if !root.is_dir() {
        return Ok(0);
    }
    let _lock = layer_lock(&layer_dir, &fsutil::fsync_dir)?;
    let mut complete: Vec<PathBuf> = Vec::new();
    for entry in fs::read_dir(&root)? {
        let path = entry?.path();
        let is_real_dir = fs::symlink_metadata(&path)
            .map(|m| m.is_dir())
            .unwrap_or(false);
        if is_real_dir && is_complete_generation(&path) {
            complete.push(path);
        }
    }
    complete.sort_by(|a, b| a.file_name().cmp(&b.file_name()));
    let mut retained: BTreeSet<PathBuf> = BTreeSet::new();
    if keep > 0 {
        let start = complete.len().saturating_sub(keep as usize);
        retained.extend(complete[start..].iter().cloned());
    }
    if let Some(active) = active_generation {
        retained.insert(root.join(active));
    }
    let mut removed = 0usize;
    for generation_dir in &complete {
        if retained.contains(generation_dir) {
            continue;
        }
        removed += count_files(generation_dir);
        fs::remove_dir_all(generation_dir)?;
    }
    if removed > 0 {
        fsutil::fsync_dir(&root)?;
    }
    Ok(removed)
}

/// `prune_grid_layer(layer, keep)`. `keep` defaults to the layer's `GRID_KEEP_LAST`. A layer with no keep-N
/// setting is a no-op. Errors other than `ENOENT`-like conditions return 0, as in Python.
pub fn prune_grid_layer(cfg: &GridConfig, layer: &str, keep: Option<u64>) -> usize {
    let Some(keep) = keep.or_else(|| cfg.keep_last.get(layer).copied()) else {
        return 0;
    };
    let layer_dir = cfg.layer_dir(layer);
    if !layer_dir.is_dir() {
        return 0;
    }
    match layer_lock(&layer_dir, &fsutil::fsync_dir) {
        Ok(_lock) => prune_layer_unlocked(cfg, &layer_dir, keep, SystemTime::now()),
        Err(_) => 0,
    }
}

/// `cleanup_old_grids()`: apply keep-N per layer, then sweep files older than `GRID_MAX_AGE_S`. Run
/// directories older than the cutoff are removed too (D11). Layers that cannot take the lock are skipped.
pub fn cleanup_old_grids(cfg: &GridConfig) -> Result<usize, GridError> {
    if !cfg.grid_dir.exists() {
        return Ok(0);
    }
    let now = SystemTime::now();
    let cutoff = now
        .checked_sub(Duration::from_secs(cfg.max_age_s))
        .unwrap_or(UNIX_EPOCH);
    let mut removed = 0usize;
    for entry in fs::read_dir(&cfg.grid_dir)? {
        let layer_dir = entry?.path();
        if !layer_dir.is_dir() {
            continue;
        }
        let layer_name = layer_dir
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or_default()
            .to_string();
        let Ok(_lock) = layer_lock(&layer_dir, &fsutil::fsync_dir) else {
            continue;
        };
        if let Some(keep) = cfg.keep_last.get(&layer_name).copied() {
            removed += prune_layer_unlocked(cfg, &layer_dir, keep, SystemTime::now());
        }
        let Ok(children) = fs::read_dir(&layer_dir) else {
            continue;
        };
        for child in children.flatten() {
            let path = child.path();
            let name = child.file_name();
            if name == LAYER_LOCK_FILE {
                continue;
            }
            if name == GENERATION_ROOT && path.is_dir() {
                removed += sweep_run_directories(&path, cutoff);
                continue;
            }
            let Ok(meta) = fs::metadata(&path) else {
                continue;
            };
            let Ok(modified) = meta.modified() else {
                continue;
            };
            if modified < cutoff && fs::remove_file(&path).is_ok() {
                removed += 1;
            }
        }
    }
    Ok(removed)
}

/// Remove `runs/<gen>` directories older than the cutoff, complete or not (D11). Caller holds the layer lock.
fn sweep_run_directories(runs_root: &Path, cutoff: SystemTime) -> usize {
    let Ok(entries) = fs::read_dir(runs_root) else {
        return 0;
    };
    let mut removed = 0usize;
    for entry in entries.flatten() {
        let path = entry.path();
        let Ok(meta) = fs::symlink_metadata(&path) else {
            continue;
        };
        if !meta.is_dir() {
            continue;
        }
        let Ok(modified) = meta.modified() else {
            continue;
        };
        if modified < cutoff {
            let files = count_files(&path);
            if fs::remove_dir_all(&path).is_ok() {
                removed += files;
            }
        }
    }
    if removed > 0 {
        let _ = fsutil::fsync_dir(runs_root);
    }
    removed
}

/// `_prune_grid_layer_unlocked`. Caller holds the layer lock.
fn prune_layer_unlocked(cfg: &GridConfig, layer_dir: &Path, keep: u64, now: SystemTime) -> usize {
    let Ok(read) = fs::read_dir(layer_dir) else {
        return 0;
    };
    let mut files: Vec<(String, PathBuf)> = Vec::new();
    for entry in read.flatten() {
        let path = entry.path();
        if fs::metadata(&path).map(|m| m.is_file()).unwrap_or(false)
            && let Some(name) = entry.file_name().to_str()
        {
            files.push((name.to_string(), path));
        }
    }
    let mut metas: Vec<(String, PathBuf)> = files
        .iter()
        .filter(|(n, _)| n.ends_with(".meta.json"))
        .cloned()
        .collect();
    metas.sort_by(|a, b| a.0.cmp(&b.0));
    let kept_start = if keep > 0 {
        metas.len().saturating_sub(keep as usize)
    } else {
        metas.len()
    };
    let mut keep_names: HashSet<String> = HashSet::new();
    for (name, path) in &metas[kept_start..] {
        keep_names.insert(name.clone());
        keep_names.insert(read_meta_data_file(path));
    }
    let meta_names: HashSet<String> = metas.iter().map(|(n, _)| n.clone()).collect();
    let committed: HashSet<String> = metas.iter().map(|(_, p)| read_meta_data_file(p)).collect();

    let mut removed = 0usize;
    for (name, path) in &files {
        if name == LAYER_LOCK_FILE || keep_names.contains(name) {
            continue;
        }
        if name.starts_with('.') && name.ends_with(".tmp") {
            continue;
        }
        if !(name.ends_with(".bin") || meta_names.contains(name)) {
            continue;
        }
        if name.ends_with(".bin") && !committed.contains(name) {
            // Unreferenced data may belong to an older worker that does not take the lock. The grace
            // window makes a rolling upgrade safe.
            let Ok(meta) = fs::metadata(path) else {
                continue;
            };
            let Ok(modified) = meta.modified() else {
                continue;
            };
            let age = now.duration_since(modified).unwrap_or(Duration::ZERO);
            if age < Duration::from_secs(cfg.orphan_grace_s) {
                continue;
            }
        }
        if fs::remove_file(path).is_ok() {
            removed += 1;
        }
    }
    removed
}

/// Acquire the layer's `.grid.lock` exclusively. Creates the layer directory durably first.
fn layer_lock(layer_dir: &Path, dir_sync: DirSync<'_>) -> io::Result<FlockGuard> {
    fsutil::durable_mkdir(layer_dir, dir_sync)?;
    FlockGuard::exclusive(&layer_dir.join(LAYER_LOCK_FILE))
}

/// `_relative_grid_key`: split on `/`, reject empty, `.` and `..` segments, and never produce an absolute path.
pub fn relative_grid_key(value: &str) -> Result<PathBuf, GridError> {
    if value.is_empty() {
        return Err(GridError::InvalidKey(value.to_string()));
    }
    let parts: Vec<&str> = value.split('/').collect();
    if parts
        .iter()
        .any(|p| p.is_empty() || *p == "." || *p == "..")
    {
        return Err(GridError::InvalidKey(value.to_string()));
    }
    let path: PathBuf = parts.iter().collect();
    if path.is_absolute()
        || path
            .components()
            .any(|c| !matches!(c, Component::Normal(_)))
    {
        return Err(GridError::InvalidKey(value.to_string()));
    }
    Ok(path)
}

/// `_grid_timestamp_of`: `{ts}.meta.json` gives `ts`. `{ts}.{gen32}.bin` gives `ts`. A legacy `{stem}.bin`
/// gives `stem`. Anything else is returned unchanged.
pub fn grid_timestamp_of(name: &str) -> String {
    if let Some(ts) = name.strip_suffix(".meta.json") {
        return ts.to_string();
    }
    if let Some(stem) = name.strip_suffix(".bin") {
        if let Some((ts, generation)) = stem.rsplit_once('.')
            && generation.len() == 32
            && generation
                .chars()
                .all(|c| matches!(c, '0'..='9' | 'a'..='f'))
        {
            return ts.to_string();
        }
        return stem.to_string();
    }
    name.to_string()
}

/// `_read_meta_data_file`: the `data_file` named in a meta, if it is a plain file name. Otherwise the
/// legacy `{ts}.bin` name derived from the meta file name.
pub fn read_meta_data_file(meta_path: &Path) -> String {
    let doc = fs::read_to_string(meta_path)
        .ok()
        .and_then(|t| serde_json::from_str::<Value>(&t).ok());
    if let Some(Value::Object(obj)) = doc
        && let Some(Value::String(value)) = obj.get("data_file")
        && is_plain_file_name(value)
    {
        return value.clone();
    }
    let name = meta_path
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or_default();
    format!("{}.bin", grid_timestamp_of(name))
}

fn is_plain_file_name(value: &str) -> bool {
    !value.is_empty()
        && value != "."
        && value != ".."
        && !value.contains('/')
        && !value.contains('\0')
}

/// Sorted names of entries ending in `.meta.json` in `dir` (non-recursive).
fn meta_file_names(dir: &Path) -> io::Result<Vec<String>> {
    let mut names = Vec::new();
    for entry in fs::read_dir(dir)? {
        let entry = entry?;
        if let Some(name) = entry.file_name().to_str()
            && name.ends_with(".meta.json")
        {
            names.push(name.to_string());
        }
    }
    names.sort();
    Ok(names)
}

/// Number of non-directory entries (files and symlinks) under `dir`, recursively. Matches the Python `rglob`
/// count used before `rmtree`.
fn count_files(dir: &Path) -> usize {
    walkdir::WalkDir::new(dir)
        .min_depth(1)
        .follow_links(false)
        .into_iter()
        .filter_map(Result::ok)
        .filter(|e| !e.file_type().is_dir())
        .count()
}

#[cfg(test)]
mod tests;
