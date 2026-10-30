//! Manifest v2: the tile manifest served at `/api/manifest.json` (spec-shared §3).
//!
//! Writers hold `manifest.lock` (`flock`) across read-modify-write, and replace `manifest.json` atomically
//! with fsync (defect D8). Readers that find a structurally invalid document (missing or non-object
//! `layers`, non-object layer or frame, bad field type) return [`ManifestError::Malformed`]. Writers then
//! fail without touching the file (defect D7; PLAN "malformed manifest read as empty then wiped").
//!
//! Timestamps are canonicalized to UTC `+00:00` (defect D19). Non-UTC or naive input is rejected.

use crate::fsutil::{self, FlockGuard, JsonFileError};
use crate::pycompat::{self, Style};
use crate::time::{canonical_utc_timestamp, isoformat_utc};
use chrono::{DateTime, Utc};
use serde::Serialize;
use serde_json::{Map, Value};
use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use thiserror::Error;

/// `schema_version` written by every writer.
pub const MANIFEST_SCHEMA_VERSION: i64 = 2;
/// Tile URL template (spec §3.1).
pub const TILE_URL_TEMPLATE: &str = "/tiles/{layer}/{palette}/{path}/{z}/{x}/{y}.png";
/// File name of the manifest inside the state directory.
pub const MANIFEST_FILE: &str = "manifest.json";
/// Lock sidecar for read-modify-write of the manifest.
pub const MANIFEST_LOCK_FILE: &str = "manifest.lock";

/// Palette used when a frame gives none (spec §3.4 `_normalize_frame`).
const DEFAULT_PALETTE: &str = "classic";
/// Layer keys derived by `_sync_layer_indexes`. Metadata supplied under these names is ignored.
const DERIVED_LAYER_KEYS: [&str; 4] = ["frames", "timestamps", "latest", "palettes"];

/// Errors from manifest reads and writes.
#[derive(Debug, Error)]
pub enum ManifestError {
    /// Filesystem failure (including reading a directory where the file should be).
    #[error("manifest I/O: {0}")]
    Io(#[from] std::io::Error),
    /// The file is not valid JSON.
    #[error("manifest is not valid JSON: {0}")]
    Parse(#[source] serde_json::Error),
    /// Valid JSON with the wrong structure. The writer refuses to overwrite it (D7).
    #[error("manifest is structurally invalid: {0}")]
    Malformed(String),
    /// A timestamp was not UTC ISO-8601.
    #[error("invalid frame timestamp {timestamp:?}: {reason}")]
    InvalidTimestamp {
        /// The rejected input.
        timestamp: String,
        /// Why it was rejected.
        reason: String,
    },
}

impl From<JsonFileError> for ManifestError {
    fn from(err: JsonFileError) -> Self {
        match err {
            JsonFileError::Io { source, .. } => ManifestError::Io(source),
            JsonFileError::Parse { source, .. } => ManifestError::Parse(source),
        }
    }
}

/// The whole manifest document. Unknown top-level keys are kept in `extra`.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Manifest {
    /// Layers keyed by name.
    pub layers: BTreeMap<String, Layer>,
    /// Always [`MANIFEST_SCHEMA_VERSION`] after a write. A missing key reads as 1.
    pub schema_version: i64,
    /// [`TILE_URL_TEMPLATE`] unless the file said otherwise.
    pub tile_url_template: String,
    /// ISO-8601 UTC time of the last write.
    pub updated_at: String,
    /// Unknown top-level keys, preserved.
    #[serde(flatten)]
    pub extra: Map<String, Value>,
}

/// One layer. `frames`, `timestamps`, `latest` and `palettes` are derived on every write.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Layer {
    /// Frames sorted by timestamp, unique.
    pub frames: Vec<Frame>,
    /// Timestamps of `frames`, in order.
    pub timestamps: Vec<String>,
    /// Last frame's timestamp. Absent when there are no frames.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub latest: Option<String>,
    /// Sorted intersection of every frame's palettes. Empty when there are no frames.
    pub palettes: Vec<String>,
    /// Metadata such as `title`, `kind`, `run_id`, `complete`, preserved verbatim.
    #[serde(flatten)]
    pub extra: Map<String, Value>,
}

/// One frame of a layer.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Frame {
    /// Identity within the layer. Canonical UTC ISO-8601.
    pub timestamp: String,
    /// Tile path segment. Defaults to `timestamp`.
    pub path: String,
    /// Sorted palette names.
    pub palettes: Vec<String>,
    /// Caller-supplied keys (`source`, `kind`, `issued_at`, `lead_minutes`, ...), preserved verbatim.
    #[serde(flatten)]
    pub extra: Map<String, Value>,
}

/// Caller-supplied frame data (the Python `frame=` dict). Reserved keys are kept in typed fields.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct FrameSpec {
    /// Frame timestamp. Used as the key by [`replace_layer`]. Ignored by [`add_frame`], which uses its own argument.
    pub timestamp: String,
    /// Explicit tile path. Empty or `None` means `timestamp`.
    pub path: Option<String>,
    /// Frame palettes. Empty means "use the layer or default".
    pub palettes: Vec<String>,
    /// Any other frame keys, stored verbatim.
    pub extra: Map<String, Value>,
}

/// Options for [`add_frame`]. Mirrors the Python keyword arguments.
#[derive(Debug, Clone, Default)]
pub struct AddOptions {
    /// Explicit palettes. Empty falls through to the frame's own, then `classic`.
    pub palettes: Vec<String>,
    /// Frame data. Its timestamp is ignored.
    pub frame: Option<FrameSpec>,
    /// Layer metadata merged into the layer before indexes are recomputed.
    pub layer_metadata: Option<Map<String, Value>>,
}

/// Options for [`replace_layer`].
#[derive(Debug, Clone, Default)]
pub struct ReplaceOptions {
    /// Palettes applied to every new frame. Overrides each frame's own palettes when non-empty.
    pub palettes: Vec<String>,
    /// Frame data, matched to timestamps by `FrameSpec::timestamp`. Entries with other timestamps are ignored.
    pub frames: Vec<FrameSpec>,
    /// Layer metadata merged into the layer.
    pub layer_metadata: Option<Map<String, Value>>,
}

impl Layer {
    /// An empty layer, as created by a first `add`.
    pub fn empty() -> Self {
        Self {
            frames: Vec::new(),
            timestamps: Vec::new(),
            latest: None,
            palettes: Vec::new(),
            extra: Map::new(),
        }
    }
}

impl Manifest {
    /// `empty_manifest()`: no layers, default templates, `updated_at` now.
    pub fn empty(now: DateTime<Utc>) -> Self {
        Self {
            layers: BTreeMap::new(),
            schema_version: MANIFEST_SCHEMA_VERSION,
            tile_url_template: TILE_URL_TEMPLATE.to_string(),
            updated_at: isoformat_utc(&now),
            extra: Map::new(),
        }
    }
}

/// `manifest_path(state_dir)`.
pub fn manifest_path(state_dir: &Path) -> PathBuf {
    state_dir.join(MANIFEST_FILE)
}

/// `manifest.lock` in the state directory.
pub fn manifest_lock_path(state_dir: &Path) -> PathBuf {
    state_dir.join(MANIFEST_LOCK_FILE)
}

/// `read_manifest_file(state_dir)`. A missing file is an empty manifest. Any other failure is an error.
/// Callers that rewrite the file must use [`add_frame`], [`remove_frame`] or [`replace_layer`].
pub fn read_manifest(state_dir: &Path) -> Result<Manifest, ManifestError> {
    read_manifest_at(state_dir, Utc::now())
}

fn read_manifest_at(state_dir: &Path, now: DateTime<Utc>) -> Result<Manifest, ManifestError> {
    match fsutil::read_json_opt(&manifest_path(state_dir))? {
        None => Ok(Manifest::empty(now)),
        Some(value) => parse_manifest_value(value, now),
    }
}

/// Parse manifest JSON text. Used by readers that already hold the bytes.
pub fn parse_manifest(text: &str) -> Result<Manifest, ManifestError> {
    let value: Value = serde_json::from_str(text).map_err(ManifestError::Parse)?;
    parse_manifest_value(value, Utc::now())
}

/// Structural validation plus the v1 migration of spec §3.4.
pub fn parse_manifest_value(value: Value, now: DateTime<Utc>) -> Result<Manifest, ManifestError> {
    let Value::Object(mut root) = value else {
        return Err(malformed("top level is not an object"));
    };
    let layers_value = root
        .remove("layers")
        .ok_or_else(|| malformed("missing \"layers\""))?;
    let Value::Object(layers_map) = layers_value else {
        return Err(malformed("\"layers\" is not an object"));
    };
    let mut layers = BTreeMap::new();
    for (name, layer_value) in layers_map {
        let layer = parse_layer(&name, layer_value)?;
        layers.insert(name, layer);
    }
    let schema_version = match root.remove("schema_version") {
        None | Some(Value::Null) => 1,
        Some(v) => v
            .as_i64()
            .ok_or_else(|| malformed("\"schema_version\" is not an integer"))?,
    };
    let tile_url_template = take_string_or(&mut root, "tile_url_template", TILE_URL_TEMPLATE)?;
    // Nothing reads it any more; drop it from files written by older releases on the next write.
    root.remove("tile_url_template_legacy");
    let updated_at = take_string_or(&mut root, "updated_at", &isoformat_utc(&now))?;
    Ok(Manifest {
        layers,
        schema_version,
        tile_url_template,
        updated_at,
        extra: root,
    })
}

fn malformed(detail: impl Into<String>) -> ManifestError {
    ManifestError::Malformed(detail.into())
}

fn take_string_or(
    root: &mut Map<String, Value>,
    key: &str,
    default: &str,
) -> Result<String, ManifestError> {
    match root.remove(key) {
        None | Some(Value::Null) => Ok(default.to_string()),
        Some(Value::String(s)) => Ok(s),
        Some(_) => Err(malformed(format!("\"{key}\" is not a string"))),
    }
}

fn string_array(value: &Value, what: &str) -> Result<Vec<String>, ManifestError> {
    let Value::Array(items) = value else {
        return Err(malformed(format!("{what} is not an array")));
    };
    items
        .iter()
        .map(|item| match item {
            Value::String(s) => Ok(s.clone()),
            _ => Err(malformed(format!("{what} contains a non-string"))),
        })
        .collect()
}

fn parse_layer(name: &str, value: Value) -> Result<Layer, ManifestError> {
    let Value::Object(mut obj) = value else {
        return Err(malformed(format!("layer {name:?} is not an object")));
    };
    let palettes_raw = obj.remove("palettes");
    let timestamps_raw = obj.remove("timestamps");
    let frames_raw = obj.remove("frames");
    let palettes = match &palettes_raw {
        None | Some(Value::Null) => Vec::new(),
        Some(v) => string_array(v, &format!("layer {name:?} palettes"))?,
    };
    let timestamps = match &timestamps_raw {
        None | Some(Value::Null) => Vec::new(),
        Some(v) => string_array(v, &format!("layer {name:?} timestamps"))?,
    };
    let frames = match frames_raw {
        Some(Value::Array(items)) => items
            .into_iter()
            .map(|item| parse_frame(name, item))
            .collect::<Result<_, _>>()?,
        None | Some(Value::Null) => {
            // v1 migration: frames come from timestamps, with the layer's palettes (default classic).
            let palettes_for_migration = match palettes_raw {
                None | Some(Value::Null) => vec![DEFAULT_PALETTE.to_string()],
                Some(_) => palettes.clone(),
            };
            timestamps
                .iter()
                .map(|ts| Frame {
                    timestamp: ts.clone(),
                    path: ts.clone(),
                    palettes: palettes_for_migration.clone(),
                    extra: Map::new(),
                })
                .collect()
        }
        Some(_) => return Err(malformed(format!("layer {name:?} frames is not an array"))),
    };
    let latest = match obj.remove("latest") {
        None | Some(Value::Null) => None,
        Some(Value::String(s)) => Some(s),
        Some(_) => return Err(malformed(format!("layer {name:?} latest is not a string"))),
    };
    Ok(Layer {
        frames,
        timestamps,
        latest,
        palettes,
        extra: obj,
    })
}

fn parse_frame(layer: &str, value: Value) -> Result<Frame, ManifestError> {
    let Value::Object(mut obj) = value else {
        return Err(malformed(format!(
            "layer {layer:?} has a frame that is not an object"
        )));
    };
    let timestamp = match obj.remove("timestamp") {
        Some(Value::String(s)) if !s.is_empty() => s,
        _ => {
            return Err(malformed(format!(
                "layer {layer:?} has a frame without a string timestamp"
            )));
        }
    };
    let path = match obj.remove("path") {
        None | Some(Value::Null) => timestamp.clone(),
        Some(Value::String(s)) if s.is_empty() => timestamp.clone(),
        Some(Value::String(s)) => s,
        Some(_) => {
            return Err(malformed(format!(
                "layer {layer:?} frame {timestamp:?} path is not a string"
            )));
        }
    };
    let palettes = match obj.remove("palettes") {
        None | Some(Value::Null) => Vec::new(),
        Some(v) => string_array(&v, &format!("layer {layer:?} frame {timestamp:?} palettes"))?,
    };
    Ok(Frame {
        timestamp,
        path,
        palettes,
        extra: obj,
    })
}

/// `_normalize_frame`: explicit palettes > the frame's own palettes > `["classic"]`.
/// Palettes are deduplicated and sorted. Reserved keys in `extra` are dropped.
fn normalize_frame(timestamp: &str, palettes_arg: &[String], frame: Option<&FrameSpec>) -> Frame {
    let own: &[String] = frame.map(|f| f.palettes.as_slice()).unwrap_or(&[]);
    let default = [DEFAULT_PALETTE.to_string()];
    let chosen: &[String] = if !palettes_arg.is_empty() {
        palettes_arg
    } else if !own.is_empty() {
        own
    } else {
        &default
    };
    let palettes: Vec<String> = chosen
        .iter()
        .cloned()
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect();
    let path = frame
        .and_then(|f| f.path.clone())
        .filter(|p| !p.is_empty())
        .unwrap_or_else(|| timestamp.to_string());
    let mut extra = frame.map(|f| f.extra.clone()).unwrap_or_default();
    for reserved in ["timestamp", "path", "palettes"] {
        extra.remove(reserved);
    }
    Frame {
        timestamp: timestamp.to_string(),
        path,
        palettes,
        extra,
    }
}

/// `_sync_layer_indexes`: dedupe by timestamp (last wins), sort, and recompute `timestamps`, `latest` and
/// `palettes`.
fn sync_layer(layer: &mut Layer) {
    let mut by_timestamp: BTreeMap<String, Frame> = BTreeMap::new();
    for frame in std::mem::take(&mut layer.frames) {
        by_timestamp.insert(frame.timestamp.clone(), frame);
    }
    let frames: Vec<Frame> = by_timestamp.into_values().collect();
    layer.timestamps = frames.iter().map(|f| f.timestamp.clone()).collect();
    match frames.last() {
        Some(last) => {
            layer.latest = Some(last.timestamp.clone());
            let mut common: Option<BTreeSet<String>> = None;
            for frame in &frames {
                let set: BTreeSet<String> = frame.palettes.iter().cloned().collect();
                common = Some(match common {
                    None => set,
                    Some(acc) => acc.intersection(&set).cloned().collect(),
                });
            }
            layer.palettes = common.unwrap_or_default().into_iter().collect();
        }
        None => {
            layer.latest = None;
            layer.palettes = Vec::new();
        }
    }
    layer.frames = frames;
}

/// Metadata merge (`layer.update`), skipping keys that `sync_layer` derives.
fn merge_metadata(layer: &mut Layer, metadata: &Map<String, Value>) {
    for (key, value) in metadata {
        if DERIVED_LAYER_KEYS.contains(&key.as_str()) {
            continue;
        }
        layer.extra.insert(key.clone(), value.clone());
    }
}

/// Canonicalize a timestamp or fail with [`ManifestError::InvalidTimestamp`].
pub fn canonical_timestamp(timestamp: &str) -> Result<String, ManifestError> {
    canonical_utc_timestamp(timestamp).map_err(|e| ManifestError::InvalidTimestamp {
        timestamp: timestamp.to_string(),
        reason: e.to_string(),
    })
}

/// Serialize a manifest exactly as Python writes it: compact, sorted keys, trailing newline (writer style A).
pub fn render_manifest(manifest: &Manifest) -> Result<String, ManifestError> {
    let value = serde_json::to_value(manifest).map_err(|e| malformed(format!("serialize: {e}")))?;
    let mut text = pycompat::dumps_value(&value, Style::Compact, true);
    text.push('\n');
    Ok(text)
}

/// Run `mutate` on the manifest under `manifest.lock`, then stamp `schema_version`/`updated_at` and write
/// atomically. Any error (including a malformed file) leaves `manifest.json` untouched.
fn with_locked_manifest<F>(
    state_dir: &Path,
    now: DateTime<Utc>,
    mutate: F,
) -> Result<Manifest, ManifestError>
where
    F: FnOnce(&mut Manifest) -> Result<(), ManifestError>,
{
    std::fs::create_dir_all(state_dir)?;
    let _lock = FlockGuard::exclusive(&manifest_lock_path(state_dir))?;
    let mut manifest = read_manifest_at(state_dir, now)?;
    mutate(&mut manifest)?;
    manifest.schema_version = MANIFEST_SCHEMA_VERSION;
    manifest.updated_at = isoformat_utc(&now);
    let text = render_manifest(&manifest)?;
    fsutil::atomic_write(
        &manifest_path(state_dir),
        text.as_bytes(),
        ".manifest.json.",
        ".tmp",
    )?;
    Ok(manifest)
}

/// `update_manifest_file(action="add")`. Adds a frame or replaces the one with the same timestamp, then
/// reindexes the layer. The layer is created on first use.
pub fn add_frame(
    state_dir: &Path,
    layer: &str,
    timestamp: &str,
    options: &AddOptions,
) -> Result<Manifest, ManifestError> {
    add_frame_at(state_dir, layer, timestamp, options, Utc::now())
}

fn add_frame_at(
    state_dir: &Path,
    layer: &str,
    timestamp: &str,
    options: &AddOptions,
    now: DateTime<Utc>,
) -> Result<Manifest, ManifestError> {
    let ts = canonical_timestamp(timestamp)?;
    with_locked_manifest(state_dir, now, |manifest| {
        let entry = manifest
            .layers
            .entry(layer.to_string())
            .or_insert_with(Layer::empty);
        entry.frames.retain(|f| f.timestamp != ts);
        entry.frames.push(normalize_frame(
            &ts,
            &options.palettes,
            options.frame.as_ref(),
        ));
        if let Some(metadata) = &options.layer_metadata {
            merge_metadata(entry, metadata);
        }
        sync_layer(entry);
        Ok(())
    })
}

/// `update_manifest_file(action="remove")`. Drops the frame with this timestamp. The layer is removed when
/// it has no frames left. Like the Python version, this rewrites the file even when nothing changed.
pub fn remove_frame(
    state_dir: &Path,
    layer: &str,
    timestamp: &str,
) -> Result<Manifest, ManifestError> {
    remove_frame_at(state_dir, layer, timestamp, Utc::now())
}

fn remove_frame_at(
    state_dir: &Path,
    layer: &str,
    timestamp: &str,
    now: DateTime<Utc>,
) -> Result<Manifest, ManifestError> {
    let ts = canonical_utc_timestamp(timestamp).unwrap_or_else(|_| timestamp.to_string());
    with_locked_manifest(state_dir, now, |manifest| {
        let now_empty = match manifest.layers.get_mut(layer) {
            Some(entry) => {
                entry.frames.retain(|f| f.timestamp != ts);
                sync_layer(entry);
                entry.frames.is_empty()
            }
            None => false,
        };
        if now_empty {
            manifest.layers.remove(layer);
        }
        Ok(())
    })
}

/// `replace_layer_manifest`. Replaces the whole frame list of a forecast layer. An empty `timestamps` list
/// removes the layer. Unknown layer keys are kept.
pub fn replace_layer(
    state_dir: &Path,
    layer: &str,
    timestamps: &[String],
    options: &ReplaceOptions,
) -> Result<Manifest, ManifestError> {
    replace_layer_at(state_dir, layer, timestamps, options, Utc::now())
}

fn replace_layer_at(
    state_dir: &Path,
    layer: &str,
    timestamps: &[String],
    options: &ReplaceOptions,
    now: DateTime<Utc>,
) -> Result<Manifest, ManifestError> {
    let wanted: BTreeSet<String> = timestamps
        .iter()
        .map(|t| canonical_timestamp(t))
        .collect::<Result<_, _>>()?;
    let mut supplied: BTreeMap<String, &FrameSpec> = BTreeMap::new();
    for spec in options.frames.iter().filter(|f| !f.timestamp.is_empty()) {
        let key =
            canonical_utc_timestamp(&spec.timestamp).unwrap_or_else(|_| spec.timestamp.clone());
        supplied.insert(key, spec);
    }
    with_locked_manifest(state_dir, now, |manifest| {
        if wanted.is_empty() {
            manifest.layers.remove(layer);
            return Ok(());
        }
        let entry = manifest
            .layers
            .entry(layer.to_string())
            .or_insert_with(Layer::empty);
        entry.frames = wanted
            .iter()
            .map(|ts| normalize_frame(ts, &options.palettes, supplied.get(ts).copied()))
            .collect();
        if let Some(metadata) = &options.layer_metadata {
            merge_metadata(entry, metadata);
        }
        sync_layer(entry);
        Ok(())
    })
}

pub mod packed;

#[cfg(test)]
mod tests;
