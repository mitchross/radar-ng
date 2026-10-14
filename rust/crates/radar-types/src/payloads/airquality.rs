//! Air-quality (NAQFC AQM) payloads (`IngestAirQualityWorkflow` and the `aqm_*` activities).
//!
//! Source: `temporal/workflows/ingest_airquality.py`, `backend/ingest_airquality/activities.py`.

use serde::{Deserialize, Serialize};

/// `RETENTION_HOURS` passed to `aqm_cleanup`.
pub const RETENTION_HOURS: i64 = 36;
/// `AQM_LAYERS` in insertion order.
pub const AQM_LAYERS: [&str; 2] = ["air-quality", "ozone"];
/// Chunk start messages (`range(0, 72, 24)`).
pub const CHUNK_START_MSGS: [i64; 3] = [0, 24, 48];

/// `AqmFindRunResult`: output of `aqm_find_latest_run`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AqmFindRunResult {
    /// Run ID, or `null`.
    pub run_id: Option<String>,
    /// Whether the run is already processed.
    pub already_processed: bool,
}

/// `AqmChunkResult`: output of `aqm_render_chunk`, and the placeholder for a failed chunk.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AqmChunkResult {
    /// Layer name.
    pub layer: String,
    /// First message in the chunk.
    pub start_msg: i64,
    /// Valid timestamps rendered for this chunk.
    #[serde(default)]
    pub rendered_timestamps: Vec<String>,
}

impl AqmChunkResult {
    /// The result the workflow records for a failed chunk: `AqmChunkResult(layer, start_msg)`.
    pub fn failed(layer: impl Into<String>, start_msg: i64) -> Self {
        Self {
            layer: layer.into(),
            start_msg,
            rendered_timestamps: Vec::new(),
        }
    }
}

/// `AqmCleanupResult`: output of `aqm_cleanup`.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct AqmCleanupResult {
    /// Tile directories removed.
    pub tile_dirs_removed: i64,
}

/// `IngestAirQualityResult`: result of `IngestAirQualityWorkflow`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct IngestAirQualityResult {
    /// Run ID, or `null`.
    pub run_id: Option<String>,
    /// True when the run was skipped because it was already processed.
    pub skipped_already_processed: bool,
    /// Layers published by `aqm_publish_run`.
    #[serde(default)]
    pub published_layers: Vec<String>,
    /// Frames rendered across all layers.
    #[serde(default)]
    pub frames_rendered: i64,
    /// Cleanup counters, `null` when cleanup did not run.
    #[serde(default)]
    pub cleanup: Option<AqmCleanupResult>,
}
