//! Nowcast verification log (spec-shared §10): `<STATE_DIR>/nowcast-skill.json`.
//!
//! The log holds one entry per (run, lead) once the observed frame arrives. The summary gives POD, FAR and
//! CSI per lead and per threshold (rain >= 20 dBZ, heavy >= 35 dBZ), a persistence baseline, and a headline at
//! the lead nearest `headline_lead_min`. Output shapes and formulas match Python. The Prometheus text uses
//! Python `str(float)` (`0.833`, `1.0`).
//!
//! Fixes applied: D15 (appends take an exclusive flock on the `nowcast-skill.lock` sidecar, opened read-write,
//! as `manifest.lock` is; a corrupt or non-object log is an error, not a silent reset). Locking the directory
//! itself failed on NFS (EBADF), so the sidecar replaces it (review F4). Reads stay lenient (NSK-03). Unparseable
//! `scored_at` entries are dropped, as in Python.

use crate::fsutil::{self, FlockGuard};
use crate::pycompat::{self, Style, is_falsy, py_int, py_int_or_zero, py_str};
use crate::time::{isoformat_utc, parse_iso_as_utc};
use chrono::{DateTime, Duration as ChronoDuration, Utc};
use serde::Serialize;
use serde_json::{Map, Value};
use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use thiserror::Error;

/// File name in the state directory.
pub const SKILL_FILE: &str = "nowcast-skill.json";

/// The exclusive-lock sidecar for appends (review F4). It is opened read-write, which NFS requires for
/// `flock`, and it is never deleted.
pub const LOCK_FILE: &str = "nowcast-skill.lock";
/// Schema written by [`append_entries`].
pub const SCHEMA_VERSION: i64 = 1;
/// Rain threshold, dBZ.
pub const RAIN_DBZ: f64 = 20.0;
/// Heavy threshold, dBZ.
pub const HEAVY_DBZ: f64 = 35.0;

/// Runtime knobs. Read with [`SkillConfig::from_env`]. Defaults match spec §10.1.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SkillConfig {
    /// `NOWCAST_SKILL_WINDOW_HOURS`: rolling window.
    pub window_hours: i64,
    /// `NOWCAST_SKILL_HEADLINE_LEAD_MIN`: lead nearest this is the headline.
    pub headline_lead_min: i64,
    /// `NOWCAST_SKILL_MIN_RUNS`: fewer scored runs and the headline is withheld.
    pub min_runs_for_headline: usize,
}

impl Default for SkillConfig {
    fn default() -> Self {
        Self {
            window_hours: 24,
            headline_lead_min: 30,
            min_runs_for_headline: 6,
        }
    }
}

impl SkillConfig {
    /// Read the three `NOWCAST_SKILL_*` variables, falling back to the defaults.
    pub fn from_env() -> Result<Self, crate::env::EnvError> {
        let d = Self::default();
        Ok(Self {
            window_hours: crate::env::int_var(
                "NOWCAST_SKILL_WINDOW_HOURS",
                d.window_hours,
                1,
                i64::MAX,
            )?,
            headline_lead_min: crate::env::int_var(
                "NOWCAST_SKILL_HEADLINE_LEAD_MIN",
                d.headline_lead_min,
                0,
                i64::MAX,
            )?,
            min_runs_for_headline: crate::env::int_var(
                "NOWCAST_SKILL_MIN_RUNS",
                d.min_runs_for_headline as i64,
                0,
                i64::MAX,
            )? as usize,
        })
    }
}

/// Errors from [`append_entries`].
#[derive(Debug, Error)]
pub enum SkillError {
    /// The existing log is unreadable or not the expected shape. It is left in place (D15).
    #[error("refusing to overwrite unreadable nowcast skill log {path}: {reason}")]
    CorruptLog {
        /// The log file.
        path: PathBuf,
        /// What was wrong.
        reason: String,
    },
    /// A new entry is not a JSON object, or has an integer field that Python could not coerce.
    #[error("invalid skill entry: {0}")]
    InvalidEntry(String),
    /// Filesystem failure.
    #[error("nowcast skill I/O: {0}")]
    Io(#[from] std::io::Error),
}

/// The log as stored. Unknown top-level keys are kept in `extra` and written back.
#[derive(Debug, Clone, PartialEq)]
pub struct SkillLog {
    /// `updated_at` string, or `None` for the empty log.
    pub updated_at: Option<String>,
    /// Stored window, when it is an integer.
    pub window_hours: Option<i64>,
    /// Entries as stored. Non-object entries are skipped by [`summarize`].
    pub entries: Vec<Value>,
    /// Other top-level keys.
    pub extra: Map<String, Value>,
}

impl SkillLog {
    /// `empty_log()`.
    pub fn empty(cfg: &SkillConfig) -> Self {
        Self {
            updated_at: None,
            window_hours: Some(cfg.window_hours),
            entries: Vec::new(),
            extra: Map::new(),
        }
    }
}

/// `skill_path(state_dir)`.
pub fn skill_path(state_dir: &Path) -> PathBuf {
    state_dir.join(SKILL_FILE)
}

/// `read_log(state_dir)`: the log, or an empty one when the file is missing, unreadable, not JSON, not an
/// object, or has no `entries` array (NSK-03).
pub fn read_log(state_dir: &Path, cfg: &SkillConfig) -> SkillLog {
    match parse_log_file(&skill_path(state_dir)) {
        Ok(Some(log)) => log,
        Ok(None) | Err(_) => SkillLog::empty(cfg),
    }
}

/// Parse the file. `Ok(None)` when it is missing. `Err` when present but unreadable or structurally wrong.
fn parse_log_file(path: &Path) -> Result<Option<SkillLog>, String> {
    let value = match fsutil::read_json_opt(path) {
        Ok(None) => return Ok(None),
        Ok(Some(v)) => v,
        Err(e) => return Err(e.to_string()),
    };
    let Value::Object(mut obj) = value else {
        return Err("top level is not an object".into());
    };
    let entries = match obj.remove("entries") {
        Some(Value::Array(items)) => items,
        _ => return Err("entries is not an array".into()),
    };
    let updated_at = match obj.remove("updated_at") {
        Some(Value::String(s)) => Some(s),
        _ => None,
    };
    let window_hours = obj.remove("window_hours").and_then(|v| py_int(&v));
    obj.remove("schema_version");
    obj.remove("thresholds_dbz");
    Ok(Some(SkillLog {
        updated_at,
        window_hours,
        entries,
        extra: obj,
    }))
}

/// `append_entries(state_dir, entries, now=now, window_hours=...)`.
///
/// Keeps entries whose `scored_at` falls within the window (future entries are kept), sorts by
/// `(scored_at, lead_minutes)`, and writes the file atomically with fsync. The directory is locked for the
/// duration on `nowcast-skill.lock` (D15, F4). A lock failure is logged and returned, never ignored. A corrupt
/// existing file is an error and is not overwritten.
pub fn append_entries(
    state_dir: &Path,
    entries: &[Map<String, Value>],
    now: DateTime<Utc>,
    window_hours: Option<i64>,
    cfg: &SkillConfig,
) -> Result<SkillLog, SkillError> {
    std::fs::create_dir_all(state_dir)?;
    let lock_path = state_dir.join(LOCK_FILE);
    let _lock = match FlockGuard::exclusive(&lock_path) {
        Ok(guard) => guard,
        Err(err) => {
            tracing::error!(path = %lock_path.display(), error = %err, "nowcast skill lock failed; log not updated");
            return Err(err.into());
        }
    };
    let path = skill_path(state_dir);
    let mut log = match parse_log_file(&path) {
        Ok(Some(log)) => log,
        Ok(None) => SkillLog::empty(cfg),
        Err(reason) => return Err(SkillError::CorruptLog { path, reason }),
    };
    let window_hours_value = window_hours.filter(|h| *h != 0).unwrap_or(cfg.window_hours);
    let window = ChronoDuration::hours(window_hours_value);

    let mut candidates: Vec<Value> = std::mem::take(&mut log.entries);
    candidates.extend(entries.iter().map(|e| Value::Object(e.clone())));
    let mut kept: Vec<(String, i64, Value)> = Vec::new();
    for entry in candidates {
        let Value::Object(obj) = &entry else {
            return Err(SkillError::InvalidEntry("entry is not an object".into()));
        };
        let scored_raw = match obj.get("scored_at") {
            Some(Value::String(s)) => s.clone(),
            _ => continue,
        };
        let Some(scored) = parse_iso_as_utc(&scored_raw) else {
            continue;
        };
        if now - scored > window {
            continue;
        }
        let lead = py_int_or_zero(obj.get("lead_minutes"))
            .ok_or_else(|| SkillError::InvalidEntry("lead_minutes is not an integer".into()))?;
        kept.push((scored_raw, lead, entry));
    }
    kept.sort_by(|a, b| a.0.cmp(&b.0).then(a.1.cmp(&b.1)));

    let mut doc = Map::new();
    doc.insert("schema_version".into(), Value::from(SCHEMA_VERSION));
    doc.insert("window_hours".into(), Value::from(window_hours_value));
    doc.insert("thresholds_dbz".into(), thresholds_value());
    doc.insert("updated_at".into(), Value::String(isoformat_utc(&now)));
    doc.insert(
        "entries".into(),
        Value::Array(kept.into_iter().map(|(_, _, e)| e).collect()),
    );
    for (k, v) in log.extra {
        doc.entry(k).or_insert(v);
    }
    let rendered = render_document(&Value::Object(doc));
    fsutil::atomic_write(&path, rendered.as_bytes(), ".nowcast-skill.json.", ".tmp")?;
    // Re-read so the caller gets exactly what is on disk.
    Ok(parse_log_file(&path)
        .ok()
        .flatten()
        .unwrap_or(SkillLog::empty(cfg)))
}

fn render_document(value: &Value) -> String {
    let mut text = pycompat::dumps_value(value, Style::Compact, true);
    text.push('\n');
    text
}

fn thresholds_value() -> Value {
    let mut m = Map::new();
    m.insert("heavy".into(), Value::from(HEAVY_DBZ));
    m.insert("rain".into(), Value::from(RAIN_DBZ));
    Value::Object(m)
}

/// Hit, miss and false-alarm counts for one threshold.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Counts {
    /// Forecast and observed.
    pub hits: i64,
    /// Observed, not forecast.
    pub misses: i64,
    /// Forecast, not observed.
    pub false_alarms: i64,
}

/// Scores derived from [`Counts`]. `None` where the denominator is zero.
#[derive(Debug, Clone, Copy, PartialEq, Serialize)]
pub struct Scores {
    /// Hits.
    pub hits: i64,
    /// Misses.
    pub misses: i64,
    /// False alarms.
    pub false_alarms: i64,
    /// Probability of detection: `hits / (hits + misses)`.
    pub pod: Option<f64>,
    /// False alarm ratio: `false_alarms / (hits + false_alarms)`.
    pub far: Option<f64>,
    /// Critical success index: `hits / (hits + misses + false_alarms)`.
    pub csi: Option<f64>,
    /// Frequency bias: `(hits + false_alarms) / (hits + misses)`.
    pub bias: Option<f64>,
}

/// `_ratio(num, den)`: `round(num / den, 3)` when `den > 0`, else `None`.
pub fn ratio(numerator: i64, denominator: i64) -> Option<f64> {
    if denominator > 0 {
        Some(crate::time::round_digits(
            numerator as f64 / denominator as f64,
            3,
        ))
    } else {
        None
    }
}

/// `scores_from_counts`: the classic contingency-table scores (NSK-01).
pub fn scores_from_counts(counts: &Counts) -> Scores {
    let Counts {
        hits,
        misses,
        false_alarms,
    } = *counts;
    Scores {
        hits,
        misses,
        false_alarms,
        pod: ratio(hits, hits + misses),
        far: ratio(false_alarms, hits + false_alarms),
        csi: ratio(hits, hits + misses + false_alarms),
        bias: ratio(hits + false_alarms, hits + misses),
    }
}

/// Per-threshold row of a lead. `persistence_csi` is `None` when the baseline had no observed rain.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct LabelScores {
    /// The forecast scores.
    #[serde(flatten)]
    pub scores: Scores,
    /// CSI of the "nothing moves" baseline.
    pub persistence_csi: Option<f64>,
}

/// One lead time in the summary.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct LeadRow {
    /// Lead in minutes.
    pub lead_minutes: i64,
    /// Distinct anchors scored at this lead.
    pub runs: usize,
    /// Rain (>= 20 dBZ) scores.
    pub rain: LabelScores,
    /// Heavy (>= 35 dBZ) scores.
    pub heavy: LabelScores,
}

/// Headline shown in the app.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Headline {
    /// Lead in minutes.
    pub lead_minutes: i64,
    /// Window used.
    pub window_hours: i64,
    /// Runs scored at this lead.
    pub runs: usize,
    /// Rain POD.
    pub pod: Option<f64>,
    /// Rain FAR.
    pub far: Option<f64>,
    /// Rain CSI.
    pub csi: Option<f64>,
    /// Persistence CSI for rain.
    pub persistence_csi: Option<f64>,
}

/// Threshold values exposed in the summary.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Thresholds {
    /// Heavy threshold, dBZ.
    pub heavy: f64,
    /// Rain threshold, dBZ.
    pub rain: f64,
}

/// `summarize(log)`: per-lead aggregation and the headline decision.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Summary {
    /// True when a headline is available.
    pub available: bool,
    /// Window used.
    pub window_hours: i64,
    /// Thresholds.
    pub thresholds_dbz: Thresholds,
    /// Distinct anchors across all leads.
    pub runs_scored: usize,
    /// The log's `updated_at`, passed through.
    pub updated_at: Option<String>,
    /// Rows, ascending by lead.
    pub by_lead: Vec<LeadRow>,
    /// Headline, when available.
    pub headline: Option<Headline>,
    /// Why no headline is available. Absent when available.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reason: Option<&'static str>,
}

#[derive(Default)]
struct Bucket {
    anchors: BTreeSet<String>,
    counts: BTreeMap<&'static str, Counts>,
    persistence: BTreeMap<&'static str, Counts>,
}

const LABELS: [&str; 2] = ["rain", "heavy"];

/// `_add_counts`: accumulate `hits`, `misses`, `false_alarms` from a JSON object (missing or non-object is a no-op).
fn add_counts(into: &mut Counts, value: Option<&Value>) {
    let Some(Value::Object(obj)) = value else {
        return;
    };
    let get = |key: &str| -> i64 {
        obj.get(key)
            .filter(|v| !is_falsy(v))
            .and_then(py_int)
            .unwrap_or(0)
    };
    into.hits += get("hits");
    into.misses += get("misses");
    into.false_alarms += get("false_alarms");
}

/// `summarize(log, now=now, window_hours=...)`.
pub fn summarize(
    log: &SkillLog,
    now: DateTime<Utc>,
    window_hours: Option<i64>,
    cfg: &SkillConfig,
) -> Summary {
    let window_h = window_hours
        .filter(|h| *h != 0)
        .or(log.window_hours.filter(|h| *h != 0))
        .unwrap_or(cfg.window_hours);
    let window = ChronoDuration::hours(window_h);

    let mut by_lead: BTreeMap<i64, Bucket> = BTreeMap::new();
    let mut anchors: BTreeSet<String> = BTreeSet::new();
    for entry in &log.entries {
        let Value::Object(obj) = entry else {
            continue;
        };
        let Some(scored) = obj.get("scored_at").and_then(|v| match v {
            Value::String(s) => parse_iso_as_utc(s),
            _ => None,
        }) else {
            continue;
        };
        if now - scored > window {
            continue;
        }
        let Some(lead) = obj.get("lead_minutes").and_then(py_int) else {
            continue;
        };
        let bucket = by_lead.entry(lead).or_default();
        let anchor = obj
            .get("anchor")
            .map(py_str)
            .unwrap_or_else(|| "None".to_string());
        bucket.anchors.insert(anchor.clone());
        anchors.insert(anchor);
        let thresholds = obj.get("thresholds").filter(|v| !is_falsy(v));
        let persistence = obj.get("persistence").filter(|v| !is_falsy(v));
        for label in LABELS {
            let t = thresholds.and_then(|v| v.get(label));
            let p = persistence.and_then(|v| v.get(label));
            add_counts(bucket.counts.entry(label).or_default(), t);
            add_counts(bucket.persistence.entry(label).or_default(), p);
        }
    }

    let mut leads = Vec::new();
    for (lead, bucket) in &by_lead {
        let mut row_scores = Vec::new();
        for label in LABELS {
            let scores = scores_from_counts(&bucket.counts.get(label).copied().unwrap_or_default());
            let persistence =
                scores_from_counts(&bucket.persistence.get(label).copied().unwrap_or_default());
            let persistence_csi = if persistence.hits + persistence.misses != 0 {
                persistence.csi
            } else {
                None
            };
            row_scores.push(LabelScores {
                scores,
                persistence_csi,
            });
        }
        let heavy = row_scores.pop().expect("two labels");
        let rain = row_scores.pop().expect("two labels");
        leads.push(LeadRow {
            lead_minutes: *lead,
            runs: bucket.anchors.len(),
            rain,
            heavy,
        });
    }

    let mut summary = Summary {
        available: false,
        window_hours: window_h,
        thresholds_dbz: Thresholds {
            heavy: HEAVY_DBZ,
            rain: RAIN_DBZ,
        },
        runs_scored: anchors.len(),
        updated_at: log.updated_at.clone(),
        by_lead: Vec::new(),
        headline: None,
        reason: None,
    };
    if leads.is_empty() {
        summary.reason = Some("no_scored_runs");
        summary.by_lead = leads;
        return summary;
    }
    let target = cfg.headline_lead_min;
    let headline_row = leads
        .iter()
        .min_by_key(|row| ((row.lead_minutes - target).abs(), row.lead_minutes))
        .expect("non-empty")
        .clone();
    if headline_row.runs < cfg.min_runs_for_headline {
        summary.reason = Some("warming_up");
        summary.by_lead = leads;
        return summary;
    }
    if headline_row.rain.scores.pod.is_none() {
        summary.reason = Some("no_rain_observed");
        summary.by_lead = leads;
        return summary;
    }
    summary.available = true;
    summary.headline = Some(Headline {
        lead_minutes: headline_row.lead_minutes,
        window_hours: window_h,
        runs: headline_row.runs,
        pod: headline_row.rain.scores.pod,
        far: headline_row.rain.scores.far,
        csi: headline_row.rain.scores.csi,
        persistence_csi: headline_row.rain.persistence_csi,
    });
    summary.by_lead = leads;
    summary
}

/// `prometheus_lines(summary)`: gauges for `/api/metrics`. Values use Python `str(float)`.
pub fn prometheus_lines(summary: &Summary) -> Vec<String> {
    let mut lines = vec![
        "# TYPE radar_ng_nowcast_skill_runs gauge".to_string(),
        format!("radar_ng_nowcast_skill_runs {}", summary.runs_scored),
    ];
    if !summary.by_lead.is_empty() {
        for metric in ["pod", "far", "csi"] {
            lines.push(format!("# TYPE radar_ng_nowcast_{metric} gauge"));
            for row in &summary.by_lead {
                for (label, scores) in [("rain", &row.rain.scores), ("heavy", &row.heavy.scores)] {
                    let value = match metric {
                        "pod" => scores.pod,
                        "far" => scores.far,
                        _ => scores.csi,
                    };
                    if let Some(v) = value {
                        lines.push(format!(
                            "radar_ng_nowcast_{metric}{{lead_minutes=\"{}\",threshold=\"{label}\"}} {}",
                            row.lead_minutes,
                            pycompat::float_repr(v)
                        ));
                    }
                }
            }
        }
    }
    lines
}

#[cfg(test)]
mod tests;
