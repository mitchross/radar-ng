//! Input selection, cadence validation and lead steps (spec-nowcast sections 4.2, 4.3, 4.4).
//!
//! Everything here is pure: it works on candidate names and times, so the same rules are testable
//! without a filesystem.

use chrono::{DateTime, Utc};

/// One candidate input grid: its file stem (the ISO timestamp), its parsed time and a payload.
#[derive(Debug, Clone, PartialEq)]
pub struct Candidate<T> {
    /// File stem, the ISO-8601 timestamp without `.meta.json`.
    pub stem: String,
    /// Parsed UTC time, `None` when the stem does not parse (Python skips these in the timed list).
    pub time: Option<DateTime<Utc>>,
    /// Caller payload (for example the meta path).
    pub item: T,
}

/// Newest grid plus the grids nearest each earlier `step_min` mark (spec 4.2).
///
/// `items` must be sorted by stem (chronological). Returns the chosen candidates oldest first. When
/// no full set of marks can be found, returns the newest `frames` items, never stretching the step.
pub fn select_input_grids<T: Clone>(
    items: &[Candidate<T>],
    step_min: f64,
    frames: usize,
) -> Vec<Candidate<T>> {
    let timed: Vec<(DateTime<Utc>, &Candidate<T>)> = items
        .iter()
        .filter_map(|c| c.time.map(|t| (t, c)))
        .collect();
    let Some(&(newest, _)) = timed.last() else {
        return tail(items, frames);
    };
    let mut chosen: Vec<(DateTime<Utc>, &Candidate<T>)> = vec![*timed.last().expect("non-empty")];
    for k in 1..frames {
        let target = newest - seconds(k as f64 * step_min * 60.0);
        let last_chosen = chosen.last().expect("at least one").0;
        let min_gap = 0.6 * step_min * 60.0;
        // `earlier` is measured from the most recently chosen grid, not from the newest.
        let mut best: Option<(f64, (DateTime<Utc>, &Candidate<T>))> = None;
        for &(t, c) in &timed {
            // Candidates must be at least `min_gap` seconds older than the last chosen grid.
            if secs_between(t, last_chosen) < min_gap {
                continue;
            }
            let dist = secs_between(t, target).abs();
            // Strictly smaller keeps the first candidate on ties, as Python's min() does.
            if best.is_none_or(|(d, _)| dist < d) {
                best = Some((dist, (t, c)));
            }
        }
        let Some((dist, pick)) = best else {
            break;
        };
        if dist > 0.45 * step_min * 60.0 {
            break;
        }
        chosen.push(pick);
    }
    if chosen.len() < frames {
        return tail(items, frames);
    }
    chosen.into_iter().rev().map(|(_, c)| c.clone()).collect()
}

fn tail<T: Clone>(items: &[Candidate<T>], frames: usize) -> Vec<Candidate<T>> {
    items[items.len().saturating_sub(frames)..].to_vec()
}

/// `(later - earlier)` in seconds, as f64. Python's `total_seconds()`.
fn secs_between(earlier: DateTime<Utc>, later: DateTime<Utc>) -> f64 {
    (later - earlier).num_milliseconds() as f64 / 1000.0
}

fn seconds(s: f64) -> chrono::TimeDelta {
    chrono::TimeDelta::milliseconds((s * 1000.0).round() as i64)
}

/// Input spacing in minutes between consecutive grids, or the list of intervals when they are invalid
/// (spec 4.3 step 7). Valid means non-empty, all positive, and none longer than
/// `max(max_gap_min, 1.5 * step_min)`. Returns the median on success.
pub fn input_interval_min(
    times: &[DateTime<Utc>],
    max_gap_min: f64,
    step_min: f64,
) -> Result<f64, Vec<f64>> {
    let intervals: Vec<f64> = times
        .windows(2)
        .map(|w| secs_between(w[0], w[1]) / 60.0)
        .collect();
    let limit = max_gap_min.max(1.5 * step_min);
    let valid = !intervals.is_empty()
        && intervals.iter().copied().fold(f64::INFINITY, f64::min) > 0.0
        && intervals.iter().copied().fold(f64::NEG_INFINITY, f64::max) <= limit;
    if !valid {
        return Err(intervals);
    }
    Ok(median(&intervals))
}

/// `numpy.median` for a non-empty slice: the mean of the two middle values for even lengths.
pub fn median(values: &[f64]) -> f64 {
    let mut sorted = values.to_vec();
    sorted.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    let n = sorted.len();
    if n % 2 == 1 {
        sorted[n / 2]
    } else {
        (sorted[n / 2 - 1] + sorted[n / 2]) / 2.0
    }
}

/// Lead times in input steps: `lead_i = ((i + 1) * step_min) / input_interval_min` for
/// `i` in `0..horizon_min / step_min` (spec 4.4). Fractional values are kept.
pub fn lead_steps(horizon_min: i64, step_min: i64, input_interval_min: f64) -> Vec<f64> {
    let n = (horizon_min / step_min).max(0) as usize;
    (0..n)
        .map(|i| ((i as i64 + 1) * step_min) as f64 / input_interval_min)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    /// Minutes after 22:00 on 2026-07-15 as a candidate.
    fn cand(min: f64) -> Candidate<usize> {
        let base = Utc.with_ymd_and_hms(2026, 7, 15, 22, 0, 0).unwrap();
        let t = base + chrono::TimeDelta::milliseconds((min * 60_000.0).round() as i64);
        Candidate {
            stem: t.to_rfc3339(),
            time: Some(t),
            item: min as usize,
        }
    }

    fn mins(sel: &[Candidate<usize>]) -> Vec<f64> {
        let base = Utc.with_ymd_and_hms(2026, 7, 15, 22, 0, 0).unwrap();
        sel.iter()
            .map(|c| (c.time.unwrap() - base).num_milliseconds() as f64 / 60_000.0)
            .collect()
    }

    /// §13 item 1: five-minute marks chosen from the 9-element fixture.
    #[test]
    fn selects_nearest_five_minute_marks() {
        let items: Vec<_> = [0.0, 1.9, 4.1, 6.0, 8.2, 10.1, 12.0, 14.2, 16.1]
            .iter()
            .map(|&m| cand(m))
            .collect();
        let sel = select_input_grids(&items, 5.0, 4);
        assert_eq!(mins(&sel), vec![1.9, 6.0, 12.0, 16.1]);
    }

    /// §13 item 2: short history with consecutive grids returns the newest four.
    #[test]
    fn consecutive_history_returns_last_four() {
        let items: Vec<_> = [0.0, 2.0, 4.0, 6.0, 8.0].iter().map(|&m| cand(m)).collect();
        let sel = select_input_grids(&items, 5.0, 4);
        assert_eq!(mins(&sel), vec![2.0, 4.0, 6.0, 8.0]);
    }

    /// §13 item 3: a missing mark returns the newest four, never stretches the step.
    #[test]
    fn missing_mark_returns_newest_four() {
        // Marks at 5 and 10 are both far from any grid: 0, 1, 2, 3, 20.
        let items: Vec<_> = [0.0, 1.0, 2.0, 3.0, 20.0]
            .iter()
            .map(|&m| cand(m))
            .collect();
        let sel = select_input_grids(&items, 5.0, 4);
        assert_eq!(mins(&sel), vec![1.0, 2.0, 3.0, 20.0]);
    }

    /// The only other selection path: fewer candidates than frames.
    #[test]
    fn fewer_candidates_than_frames_returns_all() {
        let items: Vec<_> = [0.0, 5.0].iter().map(|&m| cand(m)).collect();
        assert_eq!(mins(&select_input_grids(&items, 5.0, 4)), vec![0.0, 5.0]);
    }

    /// Candidates whose stems do not parse are skipped when choosing marks.
    #[test]
    fn unparseable_stems_are_skipped() {
        let mut items: Vec<_> = [0.0, 5.0, 10.0, 15.0].iter().map(|&m| cand(m)).collect();
        items.insert(
            0,
            Candidate {
                stem: "garbage".into(),
                time: None,
                item: 99,
            },
        );
        let sel = select_input_grids(&items, 5.0, 4);
        assert_eq!(mins(&sel), vec![0.0, 5.0, 10.0, 15.0]);
        assert!(sel.iter().all(|c| c.item != 99));
    }

    /// §13 item 4: a cadence violation is reported with its intervals.
    #[test]
    fn cadence_violation_reports_intervals() {
        let times: Vec<_> = [0.0, 10.0, 15.0]
            .iter()
            .map(|&m| cand(m).time.unwrap())
            .collect();
        let err = input_interval_min(&times, 6.0, 5.0).unwrap_err();
        assert_eq!(err, vec![10.0, 5.0]);
    }

    #[test]
    fn valid_cadence_returns_median() {
        let times: Vec<_> = [0.0, 4.0, 9.0, 14.5]
            .iter()
            .map(|&m| cand(m).time.unwrap())
            .collect();
        // Intervals 4, 5, 5.5: median 5.
        let m = input_interval_min(&times, 6.0, 5.0).unwrap();
        assert!((m - 5.0).abs() < 1e-9, "{m}");
    }

    #[test]
    fn non_positive_interval_is_invalid() {
        let times: Vec<_> = [0.0, 0.0].iter().map(|&m| cand(m).time.unwrap()).collect();
        assert!(input_interval_min(&times, 6.0, 5.0).is_err());
        assert!(input_interval_min(&times[..1], 6.0, 5.0).is_err());
    }

    #[test]
    fn median_matches_numpy_for_even_and_odd_lengths() {
        assert_eq!(median(&[3.0, 1.0, 2.0]), 2.0);
        assert_eq!(median(&[4.0, 1.0, 2.0, 3.0]), 2.5);
    }

    #[test]
    fn lead_steps_follow_spec_examples() {
        let five = lead_steps(60, 5, 5.0);
        assert_eq!(five, (1..=12).map(|i| i as f64).collect::<Vec<_>>());
        let two = lead_steps(60, 5, 2.0);
        assert_eq!(two[0], 2.5);
        assert_eq!(two[1], 5.0);
        assert_eq!(*two.last().unwrap(), 30.0);
        let four = lead_steps(60, 5, 4.0);
        assert_eq!(four[0], 1.25);
        assert_eq!(four[1], 2.5);
    }
}
