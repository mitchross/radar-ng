//! Tile and grid cleanup payloads (`TileCleanupWorkflow` and `tile_cleanup_sweep`).
//!
//! Source: `temporal/workflows/tile_cleanup.py`, `backend/tile_cleanup/activities.py`.

use serde::{Deserialize, Serialize};

/// `TileCleanupResult`: output of `tile_cleanup_sweep` and `TileCleanupWorkflow`.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct TileCleanupResult {
    /// Layers swept (always 12 in the current retention table).
    pub layers_swept: i64,
    /// Tile directories removed.
    pub tile_dirs_removed: i64,
    /// Grid files removed.
    pub grid_files_removed: i64,
}
