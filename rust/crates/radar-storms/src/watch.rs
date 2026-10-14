//! Storm-watch change detection (spec-shared 8.x and the pure parts of
//! `storm_watch_activities.py`).
//!
//! The watch samples a square window of the latest radar grid around a point and
//! decides whether the max reflectivity changed enough to push. Two Python
//! hazards are fixed here (spec-shared D2): a sample outside coverage or a
//! fill-only window is "no data", and never feeds a comparison, so it cannot
//! produce `dissipating` or `intensifying` events.

use std::fs;
use std::path::{Path, PathBuf};

use serde_json::Value;

use crate::numeric::{np_mean_f32, py_float, py_int, radians};

/// Radius of the watch window used by `compare_radar_frames` (km). Python hard-codes this value.
pub const COMPARE_RADIUS_KM: f64 = 20.0;

/// Values at or below this are fill or sentinel values, not reflectivity (spec-shared D2).
///
/// `sample_window` keeps `> -100` as Python does, so a `-99` fill survives sampling.
/// Comparisons treat `<= -99` as no data. That also covers the `-99.0` wire sentinel the API uses for `-inf`.
pub const NO_DATA_FLOOR_DBZ: f64 = -99.0;

/// Thresholds for change detection (`storm_watch.py` module constants).
#[derive(Debug, Clone, PartialEq)]
pub struct WatchConfig {
    /// `HAIL_DBZ_THRESHOLD` (default 60): at or above this, the change is `severe`.
    pub hail_dbz_threshold: f64,
    /// `INTENSIFY_DBZ_DELTA` (default 10).
    pub intensify_dbz_delta: f64,
    /// `DISSIPATE_DBZ_DELTA` (default 10).
    pub dissipate_dbz_delta: f64,
}

impl Default for WatchConfig {
    fn default() -> Self {
        Self {
            hail_dbz_threshold: 60.0,
            intensify_dbz_delta: 10.0,
            dissipate_dbz_delta: 10.0,
        }
    }
}

impl WatchConfig {
    /// Read the thresholds from the environment, falling back to the defaults.
    #[must_use]
    pub fn from_env() -> Self {
        let read = |name: &str, default: f64| {
            std::env::var(name)
                .ok()
                .and_then(|v| v.trim().parse::<f64>().ok())
                .unwrap_or(default)
        };
        let d = Self::default();
        Self {
            hail_dbz_threshold: read("HAIL_DBZ_THRESHOLD", d.hail_dbz_threshold),
            intensify_dbz_delta: read("INTENSIFY_DBZ_DELTA", d.intensify_dbz_delta),
            dissipate_dbz_delta: read("DISSIPATE_DBZ_DELTA", d.dissipate_dbz_delta),
        }
    }
}

/// Statistics of one sampled window (`storm_watch.FrameSample`).
#[derive(Debug, Clone, PartialEq)]
pub struct FrameSample {
    /// Frame timestamp, taken from the meta file name.
    pub timestamp: String,
    /// Max reflectivity in the window. `-inf` when there is no data.
    pub max_dbz: f64,
    /// Mean reflectivity in the window (float32 arithmetic). NaN when there is no data.
    pub mean_dbz: f64,
    /// Count of values at or above 50 dBZ.
    pub above_50_count: u64,
}

impl FrameSample {
    /// True when the sample carries usable reflectivity (finite and above the fill floor).
    #[must_use]
    pub fn has_data(&self) -> bool {
        is_usable_dbz(self.max_dbz)
    }
}

/// True for a finite reflectivity above the fill floor.
#[must_use]
pub fn is_usable_dbz(value: f64) -> bool {
    value.is_finite() && value > NO_DATA_FLOOR_DBZ
}

/// Kind of change (`storm_watch.ChangeKind.kind`). `appeared` and `disappeared` are never produced (D17).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChangeKind {
    /// Max reflectivity at or above the hail threshold.
    Severe,
    /// Max reflectivity rose by at least the intensify delta.
    Intensifying,
    /// Max reflectivity fell by at least the dissipate delta.
    Dissipating,
}

impl ChangeKind {
    /// Wire name used by the workflow (`severe`, `intensifying`, `dissipating`).
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Severe => "severe",
            Self::Intensifying => "intensifying",
            Self::Dissipating => "dissipating",
        }
    }
}

/// A detected change with its human-readable summary.
#[derive(Debug, Clone, PartialEq)]
pub struct ChangeEvent {
    /// Kind of change.
    pub kind: ChangeKind,
    /// Summary text, with the same wording and rounding as the Python code.
    pub summary: String,
}

/// Previous and current samples plus the delta the caller computed (`storm_watch.FrameDiff`).
#[derive(Debug, Clone, PartialEq)]
pub struct FrameDiff {
    /// Previous sample, or `None` on the first comparison.
    pub prev: Option<FrameSample>,
    /// Current sample.
    pub curr: FrameSample,
    /// `curr.max_dbz - prev.max_dbz` as the caller computed it.
    pub max_dbz_delta: f64,
}

/// Outcome of comparing the latest frame with the previous poll (`compare_radar_frames`).
///
/// `sampled == false` means "nothing to compare": no grid, no usable sample, or
/// the same frame as last time. In that case the other fields are zero.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct FrameComparison {
    /// Whether a usable comparison was made.
    pub sampled: bool,
    /// Timestamp of the sampled frame.
    pub curr_timestamp: String,
    /// Max reflectivity of the sampled frame.
    pub curr_max_dbz: f64,
    /// Previous max reflectivity (0 when there is no usable previous value).
    pub prev_max_dbz: f64,
    /// Whether a usable previous value was supplied.
    pub has_prev: bool,
    /// `curr_max_dbz - prev_max_dbz` when `has_prev`, else 0.
    pub max_dbz_delta: f64,
}

/// Newest `*.meta.json` in `grid_dir/radar` by file name (`latest_radar_meta`).
///
/// Dot files are skipped, as Python's `glob("*.meta.json")` skips them. Names
/// sort as ISO timestamps, so the last name is the newest frame.
#[must_use]
pub fn latest_radar_meta(grid_dir: &Path) -> Option<PathBuf> {
    let radar = grid_dir.join("radar");
    if !radar.exists() {
        return None;
    }
    let mut names: Vec<String> = fs::read_dir(&radar)
        .ok()?
        .filter_map(Result::ok)
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .filter(|n| n.ends_with(".meta.json") && !n.starts_with('.'))
        .collect();
    names.sort();
    names.pop().map(|n| radar.join(n))
}

/// Sample a `radius_km` square around `(lat, lon)` from a grid meta file (`sample_window`).
///
/// Returns `None` when the meta or binary cannot be read or parsed. Returns a
/// sample with `max_dbz = -inf` (not `None`) when the point is outside the grid
/// or the window holds no usable values.
#[must_use]
pub fn sample_window(meta_path: &Path, lat: f64, lon: f64, radius_km: f64) -> Option<FrameSample> {
    let meta: Value = serde_json::from_slice(&fs::read(meta_path).ok()?).ok()?;
    let obj = meta.as_object()?;
    let h = py_int(obj.get("height")?)?;
    let w = py_int(obj.get("width")?)?;
    let data_file = match obj.get("data_file") {
        Some(v) if truthy(v) => value_to_string(v),
        _ => meta_path
            .file_name()?
            .to_string_lossy()
            .replace(".meta.json", ".bin"),
    };
    let bin_path = meta_path.parent()?.join(data_file);
    let lat_max = py_float(obj.get("lat_max")?)?;
    let lat_min = py_float(obj.get("lat_min")?)?;
    let lon_min = py_float(obj.get("lon_min")?)?;
    let lon_max = py_float(obj.get("lon_max")?)?;
    if h <= 0 || w <= 0 {
        return None;
    }
    let (h, w) = (h as usize, w as usize);
    let bytes = fs::read(&bin_path).ok()?;
    if bytes.len() != h.checked_mul(w)?.checked_mul(4)? {
        // Python's reshape raises ValueError on a size mismatch.
        return None;
    }
    let values: Vec<f32> = bytes
        .chunks_exact(4)
        .map(|c| f32::from_le_bytes([c[0], c[1], c[2], c[3]]))
        .collect();

    let name = meta_path
        .file_name()?
        .to_string_lossy()
        .replace(".meta.json", "");
    let no_data = || FrameSample {
        timestamp: name.clone(),
        max_dbz: f64::NEG_INFINITY,
        mean_dbz: f64::NAN,
        above_50_count: 0,
    };
    let inside = lat_min <= lat && lat <= lat_max && lon_min <= lon && lon <= lon_max;
    if !inside {
        return Some(no_data());
    }
    let lat_span = lat_max - lat_min;
    let lon_span = lon_max - lon_min;
    if lat_span == 0.0 || lon_span == 0.0 {
        // Python raises ZeroDivisionError here; treat a degenerate grid as unreadable.
        return None;
    }

    // Row 0 is lat_max. round() is Python's half-to-even.
    let row_center = (((lat_max - lat) / lat_span) * (h as f64 - 1.0)).round_ties_even() as i64;
    let col_center = (((lon - lon_min) / lon_span) * (w as f64 - 1.0)).round_ties_even() as i64;

    let half_lat_deg = radius_km / 111.32;
    let half_lon_deg = radius_km / (111.32 * radians(lat).cos());
    let half_rows = ((half_lat_deg / lat_span * (h as f64 - 1.0)).round_ties_even() as i64).max(1);
    let half_cols = ((half_lon_deg / lon_span * (w as f64 - 1.0)).round_ties_even() as i64).max(1);

    let r0 = (row_center - half_rows).max(0);
    let r1 = (row_center + half_rows + 1).min(h as i64);
    let c0 = (col_center - half_cols).max(0);
    let c1 = (col_center + half_cols + 1).min(w as i64);

    let mut window: Vec<f32> = Vec::new();
    for r in r0..r1 {
        for c in c0..c1 {
            let v = values[r as usize * w + c as usize];
            // Drops fill values (-9999, -999, -99 is kept as Python does) and NaN.
            if v > -100.0 {
                window.push(v);
            }
        }
    }
    if window.is_empty() {
        return Some(no_data());
    }
    let max = window.iter().copied().fold(f32::NEG_INFINITY, f32::max);
    let mean = np_mean_f32(&window);
    let above = window.iter().filter(|&&v| v >= 50.0).count() as u64;
    Some(FrameSample {
        timestamp: name,
        max_dbz: f64::from(max),
        mean_dbz: f64::from(mean),
        above_50_count: above,
    })
}

/// Classify a change between two samples (`detect_change`).
///
/// Precedence is severe, then intensifying, then dissipating (DRV-07). Any
/// no-data side or non-finite delta yields `None` (D2).
#[must_use]
pub fn detect_change(diff: &FrameDiff, cfg: &WatchConfig) -> Option<ChangeEvent> {
    let prev = diff.prev.as_ref()?;
    let curr = &diff.curr;
    if !prev.has_data() || !curr.has_data() || !diff.max_dbz_delta.is_finite() {
        return None;
    }
    let delta = diff.max_dbz_delta;
    if curr.max_dbz >= cfg.hail_dbz_threshold {
        return Some(ChangeEvent {
            kind: ChangeKind::Severe,
            summary: format!(
                "reflectivity {:.0} dBZ — possible hail/severe",
                curr.max_dbz
            ),
        });
    }
    if delta >= cfg.intensify_dbz_delta {
        return Some(ChangeEvent {
            kind: ChangeKind::Intensifying,
            summary: format!(
                "+{:.0} dBZ since last frame ({:.0}→{:.0})",
                delta, prev.max_dbz, curr.max_dbz
            ),
        });
    }
    if -delta >= cfg.dissipate_dbz_delta {
        return Some(ChangeEvent {
            kind: ChangeKind::Dissipating,
            summary: format!(
                "{:.0} dBZ since last frame ({:.0}→{:.0})",
                delta, prev.max_dbz, curr.max_dbz
            ),
        });
    }
    None
}

/// Compare a sample with the previous poll (the decision part of `compare_radar_frames`).
///
/// - No sample, no usable data, or the same timestamp as the previous poll: not sampled.
/// - A previous value that is missing, non-finite or a fill sentinel counts as "no previous".
#[must_use]
pub fn compare_sample(
    sample: Option<&FrameSample>,
    prev_timestamp: Option<&str>,
    prev_max_dbz: Option<f64>,
) -> FrameComparison {
    let Some(sample) = sample else {
        return FrameComparison::default();
    };
    if !sample.has_data() || prev_timestamp == Some(sample.timestamp.as_str()) {
        return FrameComparison::default();
    }
    let prev_usable = prev_max_dbz.filter(|v| is_usable_dbz(*v));
    let has_prev = prev_usable.is_some();
    let delta = prev_usable.map_or(0.0, |p| sample.max_dbz - p);
    FrameComparison {
        sampled: true,
        curr_timestamp: sample.timestamp.clone(),
        curr_max_dbz: sample.max_dbz,
        prev_max_dbz: prev_usable.unwrap_or(0.0),
        has_prev,
        max_dbz_delta: delta,
    }
}

/// Full `compare_radar_frames`: find the latest meta, sample the window, compare.
#[must_use]
pub fn compare_radar_frames(
    grid_dir: &Path,
    lat: f64,
    lng: f64,
    prev_timestamp: Option<&str>,
    prev_max_dbz: Option<f64>,
) -> FrameComparison {
    let Some(meta) = latest_radar_meta(grid_dir) else {
        return FrameComparison::default();
    };
    let sample = sample_window(&meta, lat, lng, COMPARE_RADIUS_KM);
    compare_sample(sample.as_ref(), prev_timestamp, prev_max_dbz)
}

/// `detect_storm_change`: classify the change given a comparison result.
///
/// Returns `None` when there is no previous frame, as the Python activity does.
#[must_use]
pub fn detect_storm_change(comparison: &FrameComparison, cfg: &WatchConfig) -> Option<ChangeEvent> {
    if !comparison.has_prev {
        return None;
    }
    let diff = FrameDiff {
        prev: Some(FrameSample {
            timestamp: String::new(),
            max_dbz: comparison.prev_max_dbz,
            mean_dbz: f64::NAN,
            above_50_count: 0,
        }),
        curr: FrameSample {
            timestamp: String::new(),
            max_dbz: comparison.curr_max_dbz,
            mean_dbz: f64::NAN,
            above_50_count: 0,
        },
        max_dbz_delta: comparison.max_dbz_delta,
    };
    detect_change(&diff, cfg)
}

fn truthy(v: &Value) -> bool {
    match v {
        Value::Null => false,
        Value::Bool(b) => *b,
        Value::Number(n) => n.as_f64().is_some_and(|f| f != 0.0),
        Value::String(s) => !s.is_empty(),
        Value::Array(a) => !a.is_empty(),
        Value::Object(o) => !o.is_empty(),
    }
}

fn value_to_string(v: &Value) -> String {
    match v {
        Value::String(s) => s.clone(),
        other => other.to_string(),
    }
}
