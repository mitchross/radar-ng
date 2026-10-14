//! MRMS ingest payloads (`IngestMrmsWorkflow` and the `mrms_*` activities).
//!
//! Source: `temporal/workflows/ingest_mrms.py`, `backend/ingest_mrms/activities.py`.

use serde::{Deserialize, Serialize};

use super::codec::{default_layer_name, default_mrms_prefix, finite_f64};

/// Python default `CONUS/MergedBaseReflectivityQC_00.50` (used when env `MRMS_PREFIX` is unset).
pub const DEFAULT_MRMS_PREFIX: &str = "CONUS/MergedBaseReflectivityQC_00.50";
/// Python default `radar` (used when env `LAYER_NAME` is unset).
pub const DEFAULT_LAYER_NAME: &str = "radar";
/// `RETENTION_HOURS` passed to `mrms_cleanup`.
pub const RETENTION_HOURS: i64 = 4;

/// `IngestMrmsArgs`: per-product config. Input to `IngestMrmsWorkflow` (which
/// accepts `null` and means `IngestMrmsArgs()`) and to `mrms_list_unprocessed_keys`.
///
/// `Default` and missing keys both read env `MRMS_PREFIX` / `LAYER_NAME` at call
/// time, as the Python dataclass defaults do at import time.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct IngestMrmsArgs {
    /// S3 prefix of the MRMS product.
    #[serde(default = "default_mrms_prefix")]
    pub mrms_prefix: String,
    /// Layer name used for tiles and grids.
    #[serde(default = "default_layer_name")]
    pub layer_name: String,
}

impl Default for IngestMrmsArgs {
    fn default() -> Self {
        Self {
            mrms_prefix: default_mrms_prefix(),
            layer_name: default_layer_name(),
        }
    }
}

/// `ListKeysResult`: output of `mrms_list_unprocessed_keys`.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ListKeysResult {
    /// Unprocessed S3 keys, newest first, capped per cycle.
    #[serde(default)]
    pub keys: Vec<String>,
    /// Size of the whole backlog before the cap.
    #[serde(default)]
    pub backlog_total: i64,
}

/// `ProcessFrameInput`: input to `mrms_process_frame`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProcessFrameInput {
    /// S3 key of the frame.
    pub key: String,
    /// Layer name.
    pub layer_name: String,
}

/// `ProcessFrameResult`: output of `mrms_process_frame`.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct ProcessFrameResult {
    /// S3 key of the frame.
    pub key: String,
    /// Frame timestamp, or `""` when not known.
    #[serde(default)]
    pub timestamp: String,
    /// Whether the frame was fully published.
    #[serde(default)]
    pub rendered: bool,
    /// Palettes rendered for this frame.
    #[serde(default)]
    pub palettes: Vec<String>,
    /// Wall-clock duration in seconds.
    #[serde(default, serialize_with = "finite_f64")]
    pub duration_s: f64,
}

/// `MarkProcessedInput`: input to `mrms_mark_processed`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MarkProcessedInput {
    /// S3 key to mark.
    pub key: String,
    /// Layer name.
    pub layer_name: String,
}

/// `CleanupInput`: input to `mrms_cleanup`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CleanupInput {
    /// Layer to sweep.
    pub layer_name: String,
    /// Retention in hours (the workflow passes [`RETENTION_HOURS`]).
    pub retention_hours: i64,
}

/// `CleanupResult` (MRMS): output of `mrms_cleanup`.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct CleanupResult {
    /// Tile directories removed.
    pub tile_dirs_removed: i64,
    /// Grid files removed.
    pub grid_files_removed: i64,
}

/// `IngestMrmsResult`: result of `IngestMrmsWorkflow`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct IngestMrmsResult {
    /// Layer name.
    pub layer: String,
    /// Backlog size reported by the listing.
    pub backlog_total: i64,
    /// Frames rendered and marked this run.
    pub rendered_count: i64,
    /// Cleanup counters, `null` when cleanup did not run.
    #[serde(default)]
    pub cleanup: Option<CleanupResult>,
}
