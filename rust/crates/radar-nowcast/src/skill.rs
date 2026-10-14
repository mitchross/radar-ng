//! Verification scores (spec section 10): contingency counts, the score formulas, the
//! rolling-window summary and the point-grid stride rule.
//!
//! Only the arithmetic lives here. Log files, JSON, Prometheus text and the observation
//! matching against stored runs are glue.

use chrono::{DateTime, Duration, Utc};

use crate::field::Field2;

/// Values at or below this are "unknown" (fill or outside coverage) for scoring.
pub const UNKNOWN_BELOW: f64 = -100.0;
/// Rain threshold in dBZ used for the headline.
pub const RAIN_THRESHOLD_DBZ: f64 = 20.0;
/// Heavy-rain threshold in dBZ.
pub const HEAVY_THRESHOLD_DBZ: f64 = 35.0;
/// Lead (minutes) closest to which the headline row is chosen.
pub const HEADLINE_LEAD_MIN: i64 = 30;
/// Minimum runs before a headline is published.
pub const MIN_RUNS_FOR_HEADLINE: usize = 6;

/// Hits, misses and false alarms for one threshold.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Counts {
    /// Forecast and observed events.
    pub hits: u64,
    /// Observed events the forecast missed.
    pub misses: u64,
    /// Forecast events not observed.
    pub false_alarms: u64,
}

impl Counts {
    /// Add another count in place.
    pub fn add(&mut self, other: &Counts) {
        self.hits += other.hits;
        self.misses += other.misses;
        self.false_alarms += other.false_alarms;
    }
}

/// Contingency counts over cells with a known observation (spec 10.3).
///
/// Returns the counts and `cells`, the number of known observation cells. Fill values in
/// the forecast are never events.
pub fn contingency(
    forecast: &Field2<f32>,
    observed: &Field2<f32>,
    threshold: f64,
) -> (Counts, u64) {
    let mut c = Counts::default();
    let mut cells = 0u64;
    for (&f, &o) in forecast.as_slice().iter().zip(observed.as_slice()) {
        let o = o as f64;
        let known = o.is_finite() && o > UNKNOWN_BELOW;
        if !known {
            continue;
        }
        cells += 1;
        let obs_event = o >= threshold;
        let f = f as f64;
        let fcst_event = f.is_finite() && f >= threshold;
        match (fcst_event, obs_event) {
            (true, true) => c.hits += 1,
            (false, true) => c.misses += 1,
            (true, false) => c.false_alarms += 1,
            (false, false) => {}
        }
    }
    (c, cells)
}

/// POD, FAR, CSI and bias, each `None` when its denominator is zero, rounded to 3 places.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Scores {
    /// Probability of detection.
    pub pod: Option<f64>,
    /// False alarm ratio.
    pub far: Option<f64>,
    /// Critical success index.
    pub csi: Option<f64>,
    /// Frequency bias.
    pub bias: Option<f64>,
}

/// Scores from counts (spec 10.3 `scores_from_counts`).
pub fn scores(c: &Counts) -> Scores {
    let (h, m, f) = (c.hits as f64, c.misses as f64, c.false_alarms as f64);
    Scores {
        pod: ratio(c.hits, c.hits + c.misses),
        far: ratio(c.false_alarms, c.hits + c.false_alarms),
        csi: ratio(c.hits, c.hits + c.misses + c.false_alarms),
        bias: if h + m > 0.0 {
            round3((h + f) / (h + m))
        } else {
            None
        },
    }
}

fn ratio(num: u64, den: u64) -> Option<f64> {
    if den == 0 {
        None
    } else {
        round3(num as f64 / den as f64)
    }
}

/// Python `round(x, 3)`: correctly rounded to 3 decimals, ties to even on the exact binary value.
pub fn round3(x: f64) -> Option<f64> {
    if !x.is_finite() {
        return None;
    }
    format!("{x:.3}").parse::<f64>().ok()
}

/// Point-grid stride rule (spec 8.2): start at 1, double while `(h//s)*(w//s) > max_cells`.
pub fn point_grid_stride(h: usize, w: usize, max_cells: usize) -> usize {
    let mut s = 1usize;
    while (h / s) * (w / s) > max_cells.max(1) {
        s *= 2;
    }
    s
}

/// `data[::s, ::s]` for the stride chosen by [`point_grid_stride`] (shape `ceil(h/s) x ceil(w/s)`).
pub fn downsample_for_points(field: &Field2<f32>, max_cells: usize) -> Field2<f32> {
    let (h, w) = (field.height(), field.width());
    let s = point_grid_stride(h, w, max_cells);
    if s == 1 {
        return field.clone();
    }
    let (ow, oh) = (w.div_ceil(s), h.div_ceil(s));
    let mut out = Vec::with_capacity(ow * oh);
    for oy in 0..oh {
        let row = field.row(oy * s);
        for ox in 0..ow {
            out.push(row[ox * s]);
        }
    }
    Field2::new(ow, oh, out).expect("shape")
}

/// Geometry of a regular latitude/longitude grid, north-first.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct GridBounds {
    /// First (northern) latitude.
    pub lat_max: f64,
    /// Last (southern) latitude.
    pub lat_min: f64,
    /// Western longitude.
    pub lon_min: f64,
    /// Eastern longitude.
    pub lon_max: f64,
}

/// Nearest-neighbour resampling of `src` (with bounds `src_b`) onto the grid `dst_b` of
/// shape `dst_h x dst_w` (spec 10.2 step 3; pysteps-free `resample_nearest`). Indices use
/// `rint` (ties to even) and are clipped to the source.
pub fn resample_nearest(
    src: &Field2<f32>,
    src_b: GridBounds,
    dst_h: usize,
    dst_w: usize,
    dst_b: GridBounds,
) -> Field2<f32> {
    let (sh, sw) = (src.height(), src.width());
    let lat_span = (src_b.lat_max - src_b.lat_min).max(1e-9);
    let lon_span = (src_b.lon_max - src_b.lon_min).max(1e-9);
    let lin = |a: f64, b: f64, n: usize, i: usize| -> f64 {
        if n <= 1 {
            a
        } else {
            a + (b - a) * i as f64 / (n as f64 - 1.0)
        }
    };
    let rows: Vec<usize> = (0..dst_h)
        .map(|i| {
            let lat = lin(dst_b.lat_max, dst_b.lat_min, dst_h, i);
            let r = ((src_b.lat_max - lat) / lat_span * (sh as f64 - 1.0)).round_ties_even();
            (r.max(0.0) as usize).min(sh - 1)
        })
        .collect();
    let cols: Vec<usize> = (0..dst_w)
        .map(|j| {
            let lon = lin(dst_b.lon_min, dst_b.lon_max, dst_w, j);
            let c = ((lon - src_b.lon_min) / lon_span * (sw as f64 - 1.0)).round_ties_even();
            (c.max(0.0) as usize).min(sw - 1)
        })
        .collect();
    let mut out = Vec::with_capacity(dst_h * dst_w);
    for &r in &rows {
        for &c in &cols {
            out.push(src.get(c, r));
        }
    }
    Field2::new(dst_w, dst_h, out).expect("shape")
}

/// One scored (anchor, observation) pair in the rolling log.
#[derive(Clone, Debug, PartialEq)]
pub struct Entry {
    /// Anchor (run) timestamp.
    pub anchor: DateTime<Utc>,
    /// Observation time the forecast was matched to.
    pub valid: DateTime<Utc>,
    /// Lead in whole minutes, `round((valid_forecast - anchor)/60)`.
    pub lead_minutes: i64,
    /// When this entry was scored.
    pub scored_at: DateTime<Utc>,
    /// Known cells used by the scores.
    pub cells: u64,
    /// Rain-threshold counts of the forecast.
    pub rain: Counts,
    /// Heavy-threshold counts of the forecast.
    pub heavy: Counts,
    /// Rain-threshold counts of persistence, when available.
    pub persistence_rain: Option<Counts>,
    /// Heavy-threshold counts of persistence, when available.
    pub persistence_heavy: Option<Counts>,
}

/// Keep entries scored within `window` of `now` (spec 10.4 retention, dropping
/// entries scored more than `window` before `now`).
pub fn retain_window(entries: &mut Vec<Entry>, now: DateTime<Utc>, window: Duration) {
    entries.retain(|e| now - e.scored_at <= window);
}

/// Per-lead aggregate row (spec 10.5 step 2).
#[derive(Clone, Debug, PartialEq)]
pub struct LeadRow {
    /// Lead in minutes.
    pub lead_minutes: i64,
    /// Number of distinct anchors contributing.
    pub runs: usize,
    /// Rain-threshold scores.
    pub rain: Scores,
    /// Heavy-threshold scores.
    pub heavy: Scores,
    /// Rain-threshold raw counts.
    pub rain_counts: Counts,
    /// Heavy-threshold raw counts.
    pub heavy_counts: Counts,
    /// CSI of persistence at the rain threshold, `None` without persistence counts.
    pub persistence_csi: Option<f64>,
}

/// Why no headline is available.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Reason {
    /// Nothing inside the window.
    NoScoredRuns,
    /// Fewer than [`MIN_RUNS_FOR_HEADLINE`] runs at the headline lead.
    WarmingUp,
    /// No rain observed at the headline lead.
    NoRainObserved,
}

/// Headline shown to the app (spec 10.5 step 5).
#[derive(Clone, Debug, PartialEq)]
pub struct Headline {
    /// Headline lead in minutes.
    pub lead_minutes: i64,
    /// Runs contributing.
    pub runs: usize,
    /// Rain POD.
    pub pod: Option<f64>,
    /// Rain FAR.
    pub far: Option<f64>,
    /// Rain CSI.
    pub csi: Option<f64>,
    /// Persistence CSI at the rain threshold.
    pub persistence_csi: Option<f64>,
}

/// Summary of the rolling log (spec 10.5).
#[derive(Clone, Debug, PartialEq)]
pub struct Summary {
    /// Whether a headline is published.
    pub available: bool,
    /// Reason when not available.
    pub reason: Option<Reason>,
    /// Distinct anchors inside the window.
    pub runs_scored: usize,
    /// Per-lead rows, ascending by lead.
    pub by_lead: Vec<LeadRow>,
    /// Headline when available.
    pub headline: Option<Headline>,
}

/// Aggregate entries inside `window` of `now` per lead and pick the headline (spec 10.5).
pub fn summarize(entries: &[Entry], now: DateTime<Utc>, window: Duration) -> Summary {
    use std::collections::{BTreeMap, BTreeSet};
    struct Bucket {
        anchors: BTreeSet<DateTime<Utc>>,
        rain: Counts,
        heavy: Counts,
        p_rain: Counts,
        p_heavy: Counts,
    }
    let mut by_lead: BTreeMap<i64, Bucket> = BTreeMap::new();
    let mut anchors: BTreeSet<DateTime<Utc>> = BTreeSet::new();
    for e in entries {
        if now - e.scored_at > window {
            continue;
        }
        let b = by_lead.entry(e.lead_minutes).or_insert_with(|| Bucket {
            anchors: BTreeSet::new(),
            rain: Counts::default(),
            heavy: Counts::default(),
            p_rain: Counts::default(),
            p_heavy: Counts::default(),
        });
        b.anchors.insert(e.anchor);
        anchors.insert(e.anchor);
        b.rain.add(&e.rain);
        b.heavy.add(&e.heavy);
        if let Some(p) = &e.persistence_rain {
            b.p_rain.add(p);
        }
        if let Some(p) = &e.persistence_heavy {
            b.p_heavy.add(p);
        }
    }
    let rows: Vec<LeadRow> = by_lead
        .iter()
        .map(|(&lead, b)| {
            let p_rain = scores(&b.p_rain);
            let persistence_csi = if b.p_rain.hits + b.p_rain.misses > 0 {
                p_rain.csi
            } else {
                None
            };
            // The persistence heavy counts are carried for completeness but unused by the headline.
            let _ = &b.p_heavy;
            LeadRow {
                lead_minutes: lead,
                runs: b.anchors.len(),
                rain: scores(&b.rain),
                heavy: scores(&b.heavy),
                rain_counts: b.rain,
                heavy_counts: b.heavy,
                persistence_csi,
            }
        })
        .collect();
    let mut summary = Summary {
        available: false,
        reason: None,
        runs_scored: anchors.len(),
        by_lead: rows.clone(),
        headline: None,
    };
    if rows.is_empty() {
        summary.reason = Some(Reason::NoScoredRuns);
        return summary;
    }
    let head = rows
        .iter()
        .min_by_key(|r| ((r.lead_minutes - HEADLINE_LEAD_MIN).abs(), r.lead_minutes))
        .expect("non-empty");
    if head.runs < MIN_RUNS_FOR_HEADLINE {
        summary.reason = Some(Reason::WarmingUp);
        return summary;
    }
    if head.rain.pod.is_none() {
        summary.reason = Some(Reason::NoRainObserved);
        return summary;
    }
    summary.available = true;
    summary.headline = Some(Headline {
        lead_minutes: head.lead_minutes,
        runs: head.runs,
        pod: head.rain.pod,
        far: head.rain.far,
        csi: head.rain.csi,
        persistence_csi: head.persistence_csi,
    });
    summary
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    fn field(w: usize, h: usize, v: &[f32]) -> Field2<f32> {
        Field2::new(w, h, v.to_vec()).unwrap()
    }

    #[test]
    fn contingency_excludes_unknown_and_treats_fill_as_dry() {
        // Observations: 1 unknown (-9999), 1 unknown (NaN); the rest known.
        let obs = field(3, 2, &[30.0, -9999.0, 10.0, f32::NAN, 25.0, 0.0]);
        // Forecast: fill (-9999) never counts as an event.
        let fc = field(3, 2, &[30.0, -9999.0, 25.0, 40.0, -9999.0, 0.0]);
        let (c, cells) = contingency(&fc, &obs, 20.0);
        assert_eq!(cells, 4);
        assert_eq!(
            c,
            Counts {
                hits: 1,
                misses: 1,
                false_alarms: 1
            }
        );
    }

    #[test]
    fn scores_round_half_even_on_binary_value() {
        // 0.0625 is exact in binary: Python round(0.0625, 3) = 0.062.
        assert_eq!(round3(0.0625), Some(0.062));
        assert_eq!(round3(2.0 / 3.0), Some(0.667));
        let s = scores(&Counts {
            hits: 2,
            misses: 2,
            false_alarms: 1,
        });
        assert_eq!(s.pod, Some(0.5));
        assert_eq!(s.far, Some(0.333));
        assert_eq!(s.csi, Some(0.4));
        assert_eq!(s.bias, Some(0.75));
        assert_eq!(scores(&Counts::default()).pod, None);
    }

    #[test]
    fn point_grid_stride_rule() {
        assert_eq!(point_grid_stride(8, 8, 20), 2);
        assert_eq!(point_grid_stride(8, 8, 3), 8);
        assert_eq!(point_grid_stride(1750, 3500, 900_000), 4);
        let f = Field2::filled(8, 8, 1.0f32);
        assert_eq!(downsample_for_points(&f, 20).width(), 4);
        assert_eq!(downsample_for_points(&f, 3).width(), 1);
    }

    #[test]
    fn resample_nearest_matches_fixture() {
        // 4x4 source with value 4*row + col, resampled to 2x2 on the same extent.
        let src = Field2::new(4, 4, (0..16).map(|i| (i / 4 * 4 + i % 4) as f32).collect()).unwrap();
        let b = GridBounds {
            lat_max: 3.0,
            lat_min: 0.0,
            lon_min: 0.0,
            lon_max: 3.0,
        };
        let out = resample_nearest(&src, b, 2, 2, b);
        // Rows: lat 3 -> index 0, lat 0 -> index 3 ; cols: lon 0 -> 0, lon 3 -> 3 (clipped)
        assert_eq!(out.get(0, 0), 0.0);
        assert_eq!(out.get(1, 1), 15.0);
    }

    #[test]
    fn summary_warms_up_and_picks_headline() {
        let now = Utc.with_ymd_and_hms(2026, 10, 13, 12, 0, 0).unwrap();
        let mk = |anchor_m: i64, lead: i64, pod_hits: u64| Entry {
            anchor: now - Duration::minutes(anchor_m),
            valid: now,
            lead_minutes: lead,
            scored_at: now - Duration::minutes(1),
            cells: 10,
            rain: Counts {
                hits: pod_hits,
                misses: 1,
                false_alarms: 1,
            },
            heavy: Counts::default(),
            persistence_rain: Some(Counts {
                hits: 1,
                misses: 1,
                false_alarms: 0,
            }),
            persistence_heavy: None,
        };
        let few: Vec<Entry> = (0..3).map(|i| mk(60 + i * 5, 30, 4)).collect();
        assert_eq!(
            summarize(&few, now, Duration::hours(24)).reason,
            Some(Reason::WarmingUp)
        );
        let many: Vec<Entry> = (0..7).map(|i| mk(60 + i * 5, 30, 4)).collect();
        let s = summarize(&many, now, Duration::hours(24));
        assert!(s.available);
        let h = s.headline.unwrap();
        assert_eq!(h.lead_minutes, 30);
        assert_eq!(h.runs, 7);
        assert_eq!(h.persistence_csi, Some(0.5));
        let empty = summarize(&[], now, Duration::hours(24));
        assert_eq!(empty.reason, Some(Reason::NoScoredRuns));
    }

    #[test]
    fn retention_drops_old_entries() {
        let now = Utc.with_ymd_and_hms(2026, 10, 13, 12, 0, 0).unwrap();
        let mut v = vec![Entry {
            anchor: now,
            valid: now,
            lead_minutes: 5,
            scored_at: now - Duration::hours(25),
            cells: 0,
            rain: Counts::default(),
            heavy: Counts::default(),
            persistence_rain: None,
            persistence_heavy: None,
        }];
        retain_window(&mut v, now, Duration::hours(24));
        assert!(v.is_empty());
    }
}
