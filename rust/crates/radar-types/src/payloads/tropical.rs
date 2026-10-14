//! Tropical cyclone payloads (`IngestTropicalWorkflow` and `tropical_fetch_and_publish`).
//!
//! Source: `temporal/workflows/ingest_tropical.py`, `backend/ingest_tropical/activities.py`.

use serde::{Deserialize, Serialize};

/// `TropicalResult`: output of `tropical_fetch_and_publish` and `IngestTropicalWorkflow`.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct TropicalResult {
    /// Storms published.
    pub storm_count: i64,
    /// GeoJSON features published.
    pub feature_count: i64,
}
