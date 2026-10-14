//! Post-commit verification (spec-nowcast section 10, Python `backend/nowcast/skill.py`).
//!
//! After a run commits, the observation it started from is scored against every retained run that
//! forecast this valid time. Counts go to the rolling log in `nowcast-skill.json` (radar-core). The
//! contingency arithmetic is in [`crate::skill`]. This module is the file glue and matching rule.

use std::path::{Path, PathBuf};

use anyhow::Context;
use chrono::{DateTime, Utc};
use radar_core::grid_read::read_grid;
use radar_core::nowcast_skill::{self, Summary, prometheus_lines, summarize};
use radar_core::time::{isoformat_utc, parse_iso_as_utc, round_half_even};
use serde_json::{Map, Value};

use crate::config::NowcastConfig;
use crate::field::Field2;
use crate::skill::{self, Counts, GridBounds, HEAVY_THRESHOLD_DBZ, RAIN_THRESHOLD_DBZ};

/// Key of the observation grid stored inside each run (`runs/<anchor>/observed`).
pub const OBSERVED_KEY: &str = "observed";

/// Score the observation `observed_ts` against every retained run that predicted it. Returns the entries
/// appended to the log (empty when none matched). Matching rule (spec 10.2):
///
/// - a run is skipped when its anchor is not strictly before the observation;
/// - inside a run, the first `*.meta.json` (sorted, excluding `observed`) whose valid time is within
///   the tolerance is scored, then the scan of that run stops; a load failure also stops the scan;
/// - the persistence baseline is the run's own `observed` grid, or `null` when it is missing.
///
/// `observed` is the full-resolution science grid; `lats`/`lons` are its full axes. Its bounds (not the
/// downsampled axes) are used for resampling, as in Python.
pub fn score_observation(
    cfg: &NowcastConfig,
    observed: &Field2<f32>,
    observed_ts: &str,
    lats: &[f64],
    lons: &[f64],
    now: DateTime<Utc>,
) -> anyhow::Result<Vec<Map<String, Value>>> {
    let Some(obs_dt) = parse_iso_as_utc(observed_ts) else {
        return Ok(Vec::new());
    };
    let runs_root = cfg.paths.grid_dir.join("nowcast").join("runs");
    if !runs_root.is_dir() {
        return Ok(Vec::new());
    }
    let obs = skill::downsample_for_points(observed, cfg.point_grid_max_cells);
    let obs_bounds = axis_bounds(lats, lons);
    let scored_at = isoformat_utc(&now);
    let mut entries = Vec::new();

    for run_dir in sorted_children(&runs_root, true) {
        let Some(name) = run_dir
            .file_name()
            .and_then(|n| n.to_str())
            .map(str::to_string)
        else {
            continue;
        };
        let Some(anchor_dt) = parse_iso_as_utc(&name) else {
            continue;
        };
        if anchor_dt >= obs_dt {
            continue;
        }
        for meta_path in sorted_children(&run_dir, false)
            .into_iter()
            .filter(|p| is_meta(p))
        {
            let stem = stem_of(&meta_path);
            if stem == OBSERVED_KEY {
                continue;
            }
            let Some(valid_dt) = parse_iso_as_utc(&stem) else {
                continue;
            };
            let offset_s = (valid_dt - obs_dt).num_milliseconds() as f64 / 1000.0;
            if offset_s.abs() > cfg.skill_match_tolerance_s {
                continue;
            }
            let Some((forecast, fmeta)) = load_point(&meta_path) else {
                break;
            };
            let forecast = align(&forecast, bounds_of_meta(&fmeta), &obs, obs_bounds);
            let mut entry = Map::new();
            entry.insert("anchor".into(), Value::from(name.clone()));
            entry.insert("valid".into(), Value::from(observed_ts));
            let lead = (valid_dt - anchor_dt).num_milliseconds() as f64 / 1000.0 / 60.0;
            entry.insert(
                "lead_minutes".into(),
                Value::from(round_half_even(lead) as i64),
            );
            entry.insert("scored_at".into(), Value::from(scored_at.clone()));
            let mut thresholds = Map::new();
            let mut cells = 0u64;
            for (label, threshold) in thresholds_dbz() {
                let (counts, known) = skill::contingency(&forecast, &obs, threshold);
                cells = known;
                thresholds.insert(label.into(), counts_value(&counts));
            }
            entry.insert("cells".into(), Value::from(cells as i64));
            entry.insert("thresholds".into(), Value::Object(thresholds));
            entry.insert(
                "persistence".into(),
                persistence(&run_dir, &obs, obs_bounds),
            );
            entries.push(entry);
            break;
        }
    }

    if !entries.is_empty() {
        nowcast_skill::append_entries(
            &cfg.paths.state_dir,
            &entries,
            now,
            Some(cfg.skill.window_hours),
            &cfg.skill,
        )
        .context("appending nowcast skill entries")?;
    }
    Ok(entries)
}

/// The rolling summary and its Prometheus lines, read from the log after scoring (spec 10.5, 10.6).
pub fn skill_summary(cfg: &NowcastConfig, now: DateTime<Utc>) -> (Summary, Vec<String>) {
    let log = nowcast_skill::read_log(&cfg.paths.state_dir, &cfg.skill);
    let summary = summarize(&log, now, None, &cfg.skill);
    let lines = prometheus_lines(&summary);
    (summary, lines)
}

fn thresholds_dbz() -> [(&'static str, f64); 2] {
    [("rain", RAIN_THRESHOLD_DBZ), ("heavy", HEAVY_THRESHOLD_DBZ)]
}

fn counts_value(c: &Counts) -> Value {
    let mut m = Map::new();
    m.insert("hits".into(), Value::from(c.hits as i64));
    m.insert("misses".into(), Value::from(c.misses as i64));
    m.insert("false_alarms".into(), Value::from(c.false_alarms as i64));
    Value::Object(m)
}

/// Persistence baseline: the run's own observed grid against the new observation, or `null`.
fn persistence(run_dir: &Path, obs: &Field2<f32>, obs_bounds: GridBounds) -> Value {
    let Some((base, bmeta)) = load_point(&run_dir.join(format!("{OBSERVED_KEY}.meta.json"))) else {
        return Value::Null;
    };
    let base = align(&base, bounds_of_meta(&bmeta), obs, obs_bounds);
    let mut out = Map::new();
    for (label, threshold) in thresholds_dbz() {
        let (counts, _) = skill::contingency(&base, obs, threshold);
        out.insert(label.into(), counts_value(&counts));
    }
    Value::Object(out)
}

/// Resample `src` onto the observed shape when they differ (`resample_nearest`).
fn align(
    src: &Field2<f32>,
    src_bounds: GridBounds,
    obs: &Field2<f32>,
    obs_bounds: GridBounds,
) -> Field2<f32> {
    if src.width() == obs.width() && src.height() == obs.height() {
        return src.clone();
    }
    skill::resample_nearest(src, src_bounds, obs.height(), obs.width(), obs_bounds)
}

/// Raw point-grid read (Python `load_point_grid`: no fill rewrite).
fn load_point(meta_path: &Path) -> Option<(Field2<f32>, radar_core::grid_read::GridMeta)> {
    let dump = read_grid(meta_path).ok()?;
    let values = Field2::new(dump.meta.width, dump.meta.height, dump.values).ok()?;
    Some((values, dump.meta))
}

fn bounds_of_meta(meta: &radar_core::grid_read::GridMeta) -> GridBounds {
    GridBounds {
        lat_max: meta.lat_max,
        lat_min: meta.lat_min,
        lon_min: meta.lon_min,
        lon_max: meta.lon_max,
    }
}

/// Bounds of the full axes (`_observed_meta`: min and max of each axis).
fn axis_bounds(lats: &[f64], lons: &[f64]) -> GridBounds {
    let min = |v: &[f64]| v.iter().copied().fold(f64::INFINITY, f64::min);
    let max = |v: &[f64]| v.iter().copied().fold(f64::NEG_INFINITY, f64::max);
    GridBounds {
        lat_max: max(lats),
        lat_min: min(lats),
        lon_min: min(lons),
        lon_max: max(lons),
    }
}

fn sorted_children(dir: &Path, dirs_only: bool) -> Vec<PathBuf> {
    let Ok(read) = std::fs::read_dir(dir) else {
        return Vec::new();
    };
    let mut out: Vec<PathBuf> = read
        .flatten()
        .map(|e| e.path())
        .filter(|p| {
            if dirs_only {
                p.is_dir()
            } else {
                p.is_file() || p.is_dir()
            }
        })
        .collect();
    out.sort_by(|a, b| a.file_name().cmp(&b.file_name()));
    out
}

fn is_meta(path: &Path) -> bool {
    path.file_name()
        .and_then(|n| n.to_str())
        .is_some_and(|n| n.ends_with(".meta.json"))
}

fn stem_of(path: &Path) -> String {
    path.file_name()
        .and_then(|n| n.to_str())
        .unwrap_or_default()
        .trim_end_matches(".meta.json")
        .to_string()
}
