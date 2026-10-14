//! `aqm_cleanup`: removes superseded AQM run directories and stale download cache entries.

use std::fs;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

use chrono::{DateTime, Utc};
use radar_core::manifest::read_manifest;
use radar_types::payloads::airquality::AqmCleanupResult;

use super::{AQM_LAYER_SPECS, AqmConfig, AqmError, parse_run_id};

/// Python `aqm_cleanup`: for each layer and palette, removes run directories whose issue time is before
/// `now - retention_hours`, except the layer's current manifest run. Only whole run directories go.
/// Directories whose names are not run ids are left alone.
pub(crate) fn cleanup_blocking(
    cfg: &AqmConfig,
    retention_hours: i64,
    now: SystemTime,
) -> Result<AqmCleanupResult, AqmError> {
    // Corrupt manifest: error (fail closed), as Python's read_manifest_file raises.
    let manifest = read_manifest(&cfg.state_dir)?;
    let now_secs = unix_secs(now);
    let cutoff = now_secs - (retention_hours as f64) * 3600.0;
    let mut removed = 0i64;
    for spec in AQM_LAYER_SPECS.iter() {
        let current = manifest
            .layers
            .get(spec.name)
            .and_then(|layer| layer.extra.get("run_id"))
            .and_then(|v| v.as_str())
            .map(str::to_string);
        let layer_dir = cfg.tile_dir.join(spec.name);
        if !layer_dir.is_dir() {
            continue;
        }
        for palette_dir in sorted_children(&layer_dir)? {
            let runs_root = palette_dir.join("runs");
            if !runs_root.is_dir() {
                continue;
            }
            for run_dir in fs::read_dir(&runs_root)? {
                let run_dir = run_dir?.path();
                if !run_dir.is_dir() {
                    continue;
                }
                let name = run_dir
                    .file_name()
                    .and_then(|n| n.to_str())
                    .unwrap_or_default()
                    .to_string();
                if current.as_deref() == Some(name.as_str()) {
                    continue;
                }
                let Some(run_dt) = parse_run_id(&name) else {
                    continue;
                };
                if unix_secs_of(&run_dt) < cutoff {
                    let _ = fs::remove_dir_all(&run_dir);
                    removed += 1;
                }
            }
        }
    }
    sweep_download_cache(&cfg.tmp_root);
    Ok(AqmCleanupResult {
        tile_dirs_removed: removed,
    })
}

/// Keeps only the newest run's download cache. The workflow always runs cleanup last, and the newest run is
/// the only one that can still be rendered. Best effort: errors are ignored.
fn sweep_download_cache(tmp_root: &Path) {
    let cache = tmp_root.join("cache");
    let Ok(entries) = fs::read_dir(&cache) else {
        return;
    };
    let mut runs: Vec<(DateTime<Utc>, PathBuf)> = Vec::new();
    for entry in entries.flatten() {
        let path = entry.path();
        let Some(name) = path.file_name().and_then(|n| n.to_str()) else {
            continue;
        };
        if let Some(dt) = parse_run_id(name) {
            runs.push((dt, path));
        }
    }
    runs.sort_by_key(|(dt, _)| *dt);
    if let Some((_, _newest)) = runs.pop() {
        for (_, path) in runs {
            let _ = fs::remove_dir_all(path);
        }
    }
}

fn sorted_children(dir: &Path) -> Result<Vec<PathBuf>, AqmError> {
    let mut out: Vec<PathBuf> = fs::read_dir(dir)?
        .map(|e| e.map(|e| e.path()))
        .collect::<Result<_, _>>()?;
    out.sort();
    Ok(out)
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
