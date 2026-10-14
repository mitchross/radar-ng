//! Lightning payloads (`IngestLightningWorkflow` and `lightning_consume_stream`).
//!
//! Source: `temporal/workflows/ingest_lightning.py`, `backend/ingest_lightning/activities.py`.

use serde::{Deserialize, Serialize};

use super::codec::finite_f64;

/// Stream duration passed to `lightning_consume_stream` as a bare JSON integer (50 min).
pub const RUN_DURATION_S: i64 = 3000;

/// `LightningRunResult`: output of `lightning_consume_stream`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct LightningRunResult {
    /// Run duration in seconds, rounded to one decimal place.
    #[serde(serialize_with = "finite_f64")]
    pub duration_s: f64,
    /// Raw websocket messages seen.
    pub msgs: i64,
    /// Frames that became a JSON object.
    pub parsed: i64,
    /// Strikes inside the bounding box with a nonzero time.
    pub in_bbox: i64,
    /// Strikes in the buffer at exit.
    pub final_buffer: i64,
    /// Strikes loaded from the previous output file.
    #[serde(default)]
    pub seeded: i64,
    /// Sessions that delivered no message.
    #[serde(default)]
    pub connect_failures: i64,
}
