//! Helpers shared by the MRMS and HRRR activities: state files, grid dumps, timestamp directories.

use std::path::{Path, PathBuf};
use std::time::Instant;

use chrono::{DateTime, Utc};
use radar_core::grid_dump::{self, GridInput};
use radar_core::processed_set::ProcessedSet;
use radar_geo::{Grid, GridGeo};

use crate::config::IngestConfig;

/// Seconds since `start`, rounded to 2 places (Python `round(elapsed, 2)`).
pub fn elapsed_s(start: Instant) -> f64 {
    radar_core::time::round_digits(start.elapsed().as_secs_f64(), 2)
}

/// Run blocking file or lock work on the blocking pool (F5). Filesystem calls and `flock` waits must not run on
/// a runtime worker, because the Temporal core shares those workers and heartbeats and polls would stall.
pub async fn offload<T, F>(f: F) -> anyhow::Result<T>
where
    F: FnOnce() -> T + Send + 'static,
    T: Send + 'static,
{
    tokio::task::spawn_blocking(f)
        .await
        .map_err(|err| anyhow::anyhow!("blocking task failed: {err}"))
}

/// Open a ProcessedSet at `<STATE_DIR>/<file>` with the given capacity.
pub fn processed_set(
    cfg: &IngestConfig,
    file: &str,
    max_entries: usize,
) -> anyhow::Result<ProcessedSet> {
    Ok(ProcessedSet::open(cfg.paths.state_file(file), max_entries)?)
}

/// Regular 1-D axes of a grid. Errors on projected grids.
pub fn regular_axes(grid: &Grid) -> anyhow::Result<(&[f64], &[f64])> {
    match &grid.geo {
        GridGeo::Regular { lats, lons } => Ok((lats, lons)),
        GridGeo::Projected { .. } => anyhow::bail!("expected a regular lat/lon grid"),
    }
}

/// `write_grid` with NaN fill, logging `grid_dump_failed` on error (MRMS `_grids`).
pub fn dump_grid_logged(cfg: &IngestConfig, layer: &str, timestamp: &str, grid: &Grid, unit: &str) {
    let Ok((lats, lons)) = regular_axes(grid) else {
        return;
    };
    let input = GridInput::new(&grid.values, lats, lons, unit);
    if let Err(err) = grid_dump::write_grid(&cfg.grid, layer, timestamp, &input) {
        tracing::warn!(layer = %layer, err = %err, "grid_dump_failed");
    }
}

/// Remove `dir` and its manifest frame when `name` parses as a valid time older than `cutoff`.
/// Returns 1 when the directory was removed, else 0.
pub fn remove_timestamp_dir(
    state_dir: &Path,
    layer: &str,
    dir: &Path,
    name: &str,
    cutoff: DateTime<Utc>,
) -> anyhow::Result<u64> {
    let Some(ts) = radar_core::time::parse_iso_as_utc(name) else {
        return Ok(0);
    };
    if ts >= cutoff {
        return Ok(0);
    }
    radar_core::manifest::remove_frame(state_dir, layer, name)?;
    std::fs::remove_dir_all(dir)?;
    Ok(1)
}

/// Child directories of `dir` whose names satisfy `keep`, sorted.
pub fn child_dirs(dir: &Path, keep: impl Fn(&str) -> bool) -> std::io::Result<Vec<PathBuf>> {
    let mut out = Vec::new();
    for entry in std::fs::read_dir(dir)? {
        let path = entry?.path();
        let Some(name) = path.file_name().and_then(|n| n.to_str()) else {
            continue;
        };
        if path.is_dir() && keep(name) {
            out.push(path);
        }
    }
    out.sort();
    Ok(out)
}

/// `grid_dump::cleanup_old_grids` as a count, mapped into anyhow.
pub fn cleanup_grids(cfg: &IngestConfig) -> anyhow::Result<u64> {
    Ok(grid_dump::cleanup_old_grids(&cfg.grid)? as u64)
}
