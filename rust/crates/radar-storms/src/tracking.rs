//! Cell association, motion fit and predicted bboxes (spec-shared 6.5 and 6.6).
//!
//! Association is a globally optimal assignment (`pathfinding::kuhn_munkres_min`)
//! over predicted-position distances, gated by speed and cell size. IDs come from
//! a counter that never goes backwards: it is the larger of the previous frame's
//! `next_cell_id` and `max(previous ids) + 1` (defect D3).

use std::collections::HashMap;

use pathfinding::kuhn_munkres::kuhn_munkres_min;
use pathfinding::matrix::Matrix;
use serde_json::{Map, Value};

use crate::config::StormConfig;
use crate::doc::{CellFeature, CellProperties, PointGeometry, PredictedBbox, TrackingVector};
use crate::geo::{advance, haversine_km, translate_bbox};
use crate::numeric::{degrees, np_mean_f64, np_sum_f64, py_float, py_int, round_digits};
use crate::time::{Instant, parse_timestamp};

/// Distance used for impossible pairs (no prediction). Same magnitude as the Python `1e9`.
const NO_PREDICTION_KM: f64 = 1e9;

/// Scale for converting kilometres to the integer costs `kuhn_munkres` needs (1 mm).
const COST_SCALE: f64 = 1e6;

/// A cell found by detection, before tracking fields are attached.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Candidate {
    /// Centroid longitude (degrees).
    pub lon: f64,
    /// Centroid latitude (degrees).
    pub lat: f64,
    /// Peak reflectivity (dBZ).
    pub peak_dbz: f64,
    /// Area (km^2).
    pub area_km2: f64,
    /// Pixel count.
    pub pixel_count: u64,
    /// Padded bbox, unrounded.
    pub bbox: [f64; 4],
}

/// Previous-frame feature with the parts tracking reads, all optional so a
/// malformed entry is skipped instead of aborting the frame (D6).
struct PrevFeature<'a> {
    geometry: Option<&'a Map<String, Value>>,
    props: Option<&'a Map<String, Value>>,
}

/// Attach cell ids, motion and predicted bboxes to `candidates`.
///
/// Returns the features in candidate order plus the next unused cell id, which
/// the caller stores as `next_cell_id`.
///
/// `previous` is the parsed previous `storms.json` (or `None`). `wall_now` is
/// used as "now" only when `timestamp` does not parse, as in the Python code.
#[must_use]
pub fn assign_tracking(
    candidates: &[Candidate],
    previous: Option<&Value>,
    timestamp: &str,
    wall_now: f64,
    cfg: &StormConfig,
) -> (Vec<CellFeature>, i64) {
    let prev_features = previous_features(previous);
    let current = parse_timestamp(timestamp);
    let previous_ts = previous
        .and_then(Value::as_object)
        .and_then(|p| p.get("timestamp"))
        .and_then(Value::as_str)
        .and_then(parse_timestamp);
    let elapsed_minutes = match (current, previous_ts) {
        (Some(cur), Some(prev)) => cur.minutes_since(prev),
        _ => 0.0,
    };
    let now_s = current.map_or(wall_now, Instant::epoch_seconds);

    let prev_ids: Vec<i64> = prev_features
        .iter()
        .filter_map(|p| {
            p.props
                .and_then(|props| props.get("cell_id"))
                .and_then(py_int)
        })
        .collect();
    let mut next_id = prev_ids.iter().max().map_or(1, |m| m + 1);
    let stored_counter = previous
        .and_then(Value::as_object)
        .and_then(|p| p.get("next_cell_id"))
        .and_then(py_int);
    if let Some(counter) = stored_counter {
        next_id = next_id.max(counter);
    }

    let matches = match_candidates(candidates, &prev_features, elapsed_minutes, cfg);

    let mut features = Vec::with_capacity(candidates.len());
    for (index, cand) in candidates.iter().enumerate() {
        let mut history: Vec<[f64; 3]> = Vec::new();
        let mut cell_id = None;
        if let Some(&prev_index) = matches.get(&index) {
            let old_props = prev_features[prev_index].props;
            if let Some(id) = old_props.and_then(|p| p.get("cell_id")).and_then(py_int) {
                cell_id = Some(id);
                history = retained_history(old_props, now_s, cfg);
            }
        }
        let cell_id = cell_id.unwrap_or_else(|| {
            let id = next_id;
            next_id += 1;
            id
        });
        history.push([
            round_digits(now_s, 1),
            round_digits(cand.lon, 5),
            round_digits(cand.lat, 5),
        ]);
        features.push(build_feature(cand, cell_id, history, cfg));
    }
    (features, next_id)
}

fn build_feature(
    cand: &Candidate,
    cell_id: i64,
    history: Vec<[f64; 3]>,
    cfg: &StormConfig,
) -> CellFeature {
    let span_min = (history[history.len() - 1][0] - history[0][0]) / 60.0;
    let mut east = 0.0_f64;
    let mut north = 0.0_f64;
    let mut confidence = 0.0_f64;
    if history.len() >= 2 && span_min >= cfg.min_track_span_min {
        let (e, n, rms_km) = fit_motion(&history);
        east = e;
        north = n;
        confidence = (span_min / 15.0).min(1.0) / (1.0 + rms_km / 3.0);
    }
    let mut speed = east.hypot(north);
    if speed > cfg.max_track_speed_kmh {
        // Faster than any storm: an association error, not motion.
        east = 0.0;
        north = 0.0;
        speed = 0.0;
        confidence = 0.0;
    }
    let bearing = if speed != 0.0 {
        (degrees(east.atan2(north)) + 360.0) % 360.0
    } else {
        0.0
    };
    let vector = TrackingVector {
        east_kmh: round_digits(east, 3),
        north_kmh: round_digits(north, 3),
        speed_kmh: round_digits(speed, 3),
        bearing_deg: round_digits(bearing, 2),
    };
    let predicted_bboxes = [0_i64, 5, 10]
        .into_iter()
        .map(|lead| PredictedBbox {
            lead_minutes: lead,
            bbox: translate_bbox(cand.bbox, cand.lat, east, north, lead),
        })
        .collect();
    CellFeature {
        kind: "Feature".to_owned(),
        geometry: PointGeometry {
            kind: "Point".to_owned(),
            coordinates: [cand.lon, cand.lat],
        },
        properties: CellProperties {
            peak_dbz: cand.peak_dbz,
            area_km2: cand.area_km2,
            pixel_count: cand.pixel_count,
            bbox: cand.bbox,
            cell_id,
            track_history: history,
            tracking_vector: vector,
            tracking_confidence: round_digits(confidence, 2),
            predicted_bboxes,
        },
    }
}

/// Map each matched current index to its previous index (`_attach_tracking` steps 1 to 5).
fn match_candidates(
    candidates: &[Candidate],
    prev: &[PrevFeature<'_>],
    elapsed_minutes: f64,
    cfg: &StormConfig,
) -> HashMap<usize, usize> {
    let mut matches = HashMap::new();
    // Backfilled or out-of-order frames must not produce backwards vectors.
    if candidates.is_empty()
        || prev.is_empty()
        || !(elapsed_minutes > 0.0 && elapsed_minutes <= 15.0)
    {
        return matches;
    }
    let predicted: Vec<Option<(f64, f64)>> = prev
        .iter()
        .map(|p| predicted_position(p, elapsed_minutes))
        .collect();
    let dist: Vec<Vec<f64>> = candidates
        .iter()
        .map(|c| {
            predicted
                .iter()
                .map(|pred| pred.map_or(NO_PREDICTION_KM, |p| haversine_km((c.lon, c.lat), p)))
                .collect()
        })
        .collect();
    let base_gate = cfg
        .match_slack_km
        .max(cfg.max_track_speed_kmh * elapsed_minutes / 60.0 * 0.5);
    for (row, col) in optimal_pairs(&dist) {
        // Big cells' centroids shift as parts cross the threshold: allow half their width.
        let area = candidates[row].area_km2.max(0.0);
        if dist[row][col] <= base_gate.max(0.5 * area.sqrt()) {
            matches.insert(row, col);
        }
    }
    matches
}

/// Globally optimal assignment of `min(rows, cols)` pairs, minimising total distance.
fn optimal_pairs(dist: &[Vec<f64>]) -> Vec<(usize, usize)> {
    let n_cur = dist.len();
    let n_prev = dist.first().map_or(0, Vec::len);
    if n_cur == 0 || n_prev == 0 {
        return Vec::new();
    }
    let cost = |d: f64| (d * COST_SCALE).round() as i64;
    if n_cur <= n_prev {
        let m = Matrix::from_fn(n_cur, n_prev, |(r, c)| cost(dist[r][c]));
        let (_, assign) = kuhn_munkres_min(&m);
        (0..n_cur).map(|r| (r, assign[r])).collect()
    } else {
        let m = Matrix::from_fn(n_prev, n_cur, |(r, c)| cost(dist[c][r]));
        let (_, assign) = kuhn_munkres_min(&m);
        (0..n_prev).map(|r| (assign[r], r)).collect()
    }
}

/// Position of a previous cell advanced to the current time (`_advance` of `old` coords).
fn predicted_position(prev: &PrevFeature<'_>, elapsed_minutes: f64) -> Option<(f64, f64)> {
    let coords = prev.geometry?.get("coordinates")?.as_array()?;
    if coords.len() != 2 {
        return None;
    }
    let lon = py_float(&coords[0])?;
    let lat = py_float(&coords[1])?;
    let (east, north) = previous_motion(prev.props);
    Some(advance(lon, lat, east, north, elapsed_minutes))
}

/// `_previous_motion`: `(east, north)` from `tracking_vector`, or `(0, 0)` when unusable.
fn previous_motion(props: Option<&Map<String, Value>>) -> (f64, f64) {
    let Some(vector) = props
        .and_then(|p| p.get("tracking_vector"))
        .and_then(Value::as_object)
    else {
        return (0.0, 0.0);
    };
    let read = |key: &str| match vector.get(key) {
        None => Some(0.0),
        Some(v) => py_float(v),
    };
    match (read("east_kmh"), read("north_kmh")) {
        (Some(e), Some(n)) => (e, n),
        _ => (0.0, 0.0),
    }
}

/// Previous history samples that are still inside the retention window.
fn retained_history(
    props: Option<&Map<String, Value>>,
    now_s: f64,
    cfg: &StormConfig,
) -> Vec<[f64; 3]> {
    let limit = cfg.track_history_min * 60.0;
    props
        .and_then(|p| p.get("track_history"))
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|entry| {
            let items = entry.as_array().filter(|a| a.len() == 3)?;
            let t = py_float(&items[0])?;
            let lon = py_float(&items[1])?;
            let lat = py_float(&items[2])?;
            (now_s - t <= limit).then_some([t, lon, lat])
        })
        .collect()
}

/// Least-squares motion over `[epoch_s, lon, lat]` samples (`storms._fit_motion`).
///
/// Returns `(east_kmh, north_kmh, rms_km)`. The sums use numpy's association so
/// the results agree with Python to the last bit in practice.
#[must_use]
pub fn fit_motion(history: &[[f64; 3]]) -> (f64, f64, f64) {
    if history.len() < 2 {
        return (0.0, 0.0, 0.0);
    }
    let t: Vec<f64> = history.iter().map(|h| h[0] / 3600.0).collect();
    let lon: Vec<f64> = history.iter().map(|h| h[1]).collect();
    let lat: Vec<f64> = history.iter().map(|h| h[2]).collect();
    let last = history.len() - 1;
    let cos_last = crate::numeric::radians(lat[last]).cos();
    let x: Vec<f64> = lon
        .iter()
        .map(|l| (l - lon[last]) * 111.320 * cos_last)
        .collect();
    let y: Vec<f64> = lat.iter().map(|l| (l - lat[last]) * 110.574).collect();

    let t_mean = np_mean_f64(&t);
    let tc: Vec<f64> = t.iter().map(|v| v - t_mean).collect();
    let tc_sq: Vec<f64> = tc.iter().map(|v| v * v).collect();
    let denom = np_sum_f64(&tc_sq);
    if denom <= 0.0 {
        return (0.0, 0.0, 0.0);
    }
    let x_mean = np_mean_f64(&x);
    let y_mean = np_mean_f64(&y);
    let xc: Vec<f64> = x.iter().map(|v| v - x_mean).collect();
    let yc: Vec<f64> = y.iter().map(|v| v - y_mean).collect();
    let tx: Vec<f64> = tc.iter().zip(&xc).map(|(a, b)| a * b).collect();
    let ty: Vec<f64> = tc.iter().zip(&yc).map(|(a, b)| a * b).collect();
    let ve = np_sum_f64(&tx) / denom;
    let vn = np_sum_f64(&ty) / denom;

    let resid_sq: Vec<f64> = (0..history.len())
        .map(|i| {
            let rx = x[i] - (x_mean + ve * tc[i]);
            let ry = y[i] - (y_mean + vn * tc[i]);
            let r = rx.hypot(ry);
            r * r
        })
        .collect();
    let rms = (np_sum_f64(&resid_sq) / history.len() as f64).sqrt();
    (ve, vn, rms)
}

/// Previous features as `(geometry, properties)` objects. Non-object entries are dropped.
fn previous_features(previous: Option<&Value>) -> Vec<PrevFeature<'_>> {
    previous
        .and_then(Value::as_object)
        .and_then(|p| p.get("features"))
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(Value::as_object)
        .map(|feature| PrevFeature {
            geometry: feature.get("geometry").and_then(Value::as_object),
            props: feature.get("properties").and_then(Value::as_object),
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn cfg() -> StormConfig {
        StormConfig::default()
    }

    #[test]
    fn fit_motion_recovers_constant_velocity() {
        // 60 km/h east, sampled every 2 minutes, no noise.
        let km_per_deg = 111.320 * 42.0_f64.to_radians().cos();
        let hist: Vec<[f64; 3]> = (0..8)
            .map(|i| {
                let t = 1_783_684_800.0 + 120.0 * i as f64;
                let lon = -85.0 + (60.0 * (2.0 * i as f64) / 60.0) / km_per_deg;
                [t, lon, 42.0]
            })
            .collect();
        let (e, n, rms) = fit_motion(&hist);
        assert!((e - 60.0).abs() < 1e-6, "east {e}");
        assert!(n.abs() < 1e-9);
        assert!(rms < 1e-6);
    }

    #[test]
    fn optimal_pairs_handles_rectangular_both_ways() {
        // Two rows, three columns: greedy would take (0,0)=1 then (1,1)=5; optimum is (0,1)+(1,0)=2+2.
        let wide = vec![vec![1.0, 2.0, 9.0], vec![2.0, 5.0, 9.0]];
        let mut pairs = optimal_pairs(&wide);
        pairs.sort_unstable();
        assert_eq!(pairs, vec![(0, 1), (1, 0)]);
        // Three rows, two columns (transposed branch): optimum is row 1 -> col 0 and row 2 -> col 1.
        let tall = vec![vec![2.0, 2.0], vec![1.0, 9.0], vec![9.0, 1.0]];
        let mut pairs = optimal_pairs(&tall);
        pairs.sort_unstable();
        assert_eq!(pairs, vec![(1, 0), (2, 1)]);
    }

    #[test]
    fn previous_motion_defaults_and_failures() {
        let p = json!({"tracking_vector": {"east_kmh": 4.0}});
        assert_eq!(previous_motion(p.as_object()), (4.0, 0.0));
        let bad = json!({"tracking_vector": {"east_kmh": null}});
        assert_eq!(previous_motion(bad.as_object()), (0.0, 0.0));
        assert_eq!(previous_motion(None), (0.0, 0.0));
    }

    #[test]
    fn mixed_timezones_do_not_panic_and_match_utc() {
        let cand = Candidate {
            lon: -85.0,
            lat: 42.0,
            peak_dbz: 50.0,
            area_km2: 10.0,
            pixel_count: 9,
            bbox: [-85.1, 41.9, -84.9, 42.1],
        };
        let prev = json!({"timestamp": "2026-07-10T12:00:00", "features": [
            {"geometry": {"coordinates": [-85.0, 42.0]}, "properties": {"cell_id": 3}}
        ]});
        let (features, next) =
            assign_tracking(&[cand], Some(&prev), "2026-07-10T12:02:00Z", 0.0, &cfg());
        assert_eq!(features[0].properties.cell_id, 3);
        assert_eq!(next, 4);
    }

    #[test]
    fn invalid_previous_cell_id_gets_a_fresh_id_instead_of_crashing() {
        let cand = Candidate {
            lon: -85.0,
            lat: 42.0,
            peak_dbz: 50.0,
            area_km2: 10.0,
            pixel_count: 9,
            bbox: [-85.1, 41.9, -84.9, 42.1],
        };
        let prev = json!({"timestamp": "2026-07-10T12:00:00+00:00", "features": [
            {"geometry": {"coordinates": [-85.0, 42.0]}, "properties": {"cell_id": null}}
        ]});
        let (features, _) = assign_tracking(
            &[cand],
            Some(&prev),
            "2026-07-10T12:02:00+00:00",
            0.0,
            &cfg(),
        );
        assert_eq!(features[0].properties.cell_id, 1);
    }
}
