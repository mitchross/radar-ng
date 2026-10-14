//! Strike buffer, seeding from the previous `lightning.json`, and the atomic GeoJSON writer (spec 6.8 to 6.10).

use std::collections::VecDeque;
use std::io;
use std::path::Path;

use radar_core::fsutil::atomic_write;
use radar_core::pycompat::{Json, Style, dumps};
use radar_storms::numeric::{py_float, py_int};
use serde_json::Value;

use crate::now_epoch;

/// One lightning strike. `t` is seconds since the epoch, as a float.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Strike {
    /// Event time in seconds (`time / 1e9` from the feed).
    pub t: f64,
    /// Latitude, degrees.
    pub lat: f64,
    /// Longitude, degrees.
    pub lon: f64,
    /// Polarity (`pol`, default 0).
    pub pol: i64,
    /// Maximum detection steepness (`mds`, default 0).
    pub mds: i64,
}

/// Bounded FIFO of strikes. Appending past the cap drops the oldest, as `deque(maxlen=...)` does.
#[derive(Debug, Clone)]
pub struct StrikeBuffer {
    strikes: VecDeque<Strike>,
    cap: usize,
}

impl StrikeBuffer {
    /// Empty buffer with capacity `cap`.
    #[must_use]
    pub fn new(cap: usize) -> Self {
        Self {
            strikes: VecDeque::with_capacity(cap.min(8192)),
            cap,
        }
    }

    /// Append a strike, dropping the oldest when full.
    pub fn push(&mut self, strike: Strike) {
        if self.cap == 0 {
            return;
        }
        if self.strikes.len() == self.cap {
            self.strikes.pop_front();
        }
        self.strikes.push_back(strike);
    }

    /// Drop strikes from the front while `t < cutoff`. Only the front is inspected, as Python's `_prune` does.
    pub fn prune_before(&mut self, cutoff: f64) {
        while self.strikes.front().is_some_and(|s| s.t < cutoff) {
            self.strikes.pop_front();
        }
    }

    /// Number of strikes held.
    #[must_use]
    pub fn len(&self) -> usize {
        self.strikes.len()
    }

    /// True when empty.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.strikes.is_empty()
    }

    /// Copy of the current contents, oldest first.
    #[must_use]
    pub fn snapshot(&self) -> Vec<Strike> {
        self.strikes.iter().copied().collect()
    }
}

/// Strikes still inside the retention window of the last published file (spec 6.10, Python
/// `_load_existing_strikes`).
///
/// A missing file, invalid JSON, or a non-object top level gives an empty list. Features that cannot be read are
/// skipped one by one. Non-finite times are skipped too. The result is sorted by time and keeps the newest
/// `max_strikes`.
#[must_use]
pub fn load_existing_strikes(
    path: &Path,
    retention_min: i64,
    now: f64,
    max_strikes: usize,
) -> Vec<Strike> {
    let Ok(bytes) = std::fs::read(path) else {
        return Vec::new();
    };
    let Ok(body) = serde_json::from_slice::<Value>(&bytes) else {
        return Vec::new();
    };
    let Some(object) = body.as_object() else {
        return Vec::new();
    };
    let cutoff = now - retention_min as f64 * 60.0;
    let features: &[Value] = match object.get("features") {
        Some(Value::Array(items)) => items,
        _ => &[],
    };
    let mut strikes: Vec<Strike> = features
        .iter()
        .filter_map(seed_from_feature)
        .filter(|s| s.t >= cutoff)
        .collect();
    strikes.sort_by(|a, b| a.t.total_cmp(&b.t));
    let start = strikes.len().saturating_sub(max_strikes);
    strikes.split_off(start)
}

fn seed_from_feature(feature: &Value) -> Option<Strike> {
    let feature = feature.as_object()?;
    let props = feature.get("properties")?.as_object()?;
    let geometry = feature.get("geometry")?.as_object()?;
    let coordinates = geometry.get("coordinates")?.as_array()?;
    if coordinates.len() < 2 {
        return None;
    }
    let lon = py_float(&coordinates[0])?;
    let lat = py_float(&coordinates[1])?;
    let t = py_float(props.get("time")?)?;
    if !t.is_finite() {
        return None;
    }
    // Python: int(props.get("polarity", 0)). An explicit null raises and the feature is skipped.
    let pol = match props.get("polarity") {
        None => 0,
        Some(v) => py_int(v)?,
    };
    let mds = match props.get("mds") {
        None => 0,
        Some(v) => py_int(v)?,
    };
    Some(Strike {
        t,
        lat,
        lon,
        pol,
        mds,
    })
}

/// The `lightning.json` document for `strikes` at time `now` (spec 6.9). Written with Python's default
/// `json.dumps` separators, so the bytes match the Python writer.
#[must_use]
pub fn render_geojson(strikes: &[Strike], now: f64, retention_min: i64) -> String {
    let features: Vec<Json> = strikes
        .iter()
        .map(|s| {
            obj(vec![
                ("type", Json::Str("Feature".into())),
                (
                    "geometry",
                    obj(vec![
                        ("type", Json::Str("Point".into())),
                        (
                            "coordinates",
                            Json::Array(vec![Json::Float(s.lon), Json::Float(s.lat)]),
                        ),
                    ]),
                ),
                (
                    "properties",
                    obj(vec![
                        ("time", Json::Float(s.t)),
                        ("age_s", Json::Int((now - s.t) as i64)),
                        ("polarity", Json::Int(s.pol)),
                        ("mds", Json::Int(s.mds)),
                    ]),
                ),
            ])
        })
        .collect();
    let body = obj(vec![
        ("type", Json::Str("FeatureCollection".into())),
        ("features", Json::Array(features)),
        ("generated_at", Json::Float(now)),
        ("retention_min", Json::Int(retention_min)),
    ]);
    dumps(&body, Style::Default, false)
}

fn obj(pairs: Vec<(&str, Json)>) -> Json {
    Json::Object(pairs.into_iter().map(|(k, v)| (k.to_owned(), v)).collect())
}

/// Write `lightning.json` atomically (spec 6.9): a unique temp file in the same directory, then rename over the
/// target. Creates the directory first. Blocking, so call it through `spawn_blocking`.
pub fn write_geojson(path: &Path, strikes: &[Strike], retention_min: i64) -> io::Result<()> {
    if let Some(parent) = path.parent().filter(|p| !p.as_os_str().is_empty()) {
        std::fs::create_dir_all(parent)?;
    }
    let text = render_geojson(strikes, now_epoch(), retention_min);
    atomic_write(path, text.as_bytes(), ".lightning.json.", ".tmp")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn strike(t: f64) -> Strike {
        Strike {
            t,
            lat: 36.0,
            lon: -97.0,
            pol: 0,
            mds: 0,
        }
    }

    #[test]
    fn buffer_drops_oldest_at_cap() {
        let mut b = StrikeBuffer::new(3);
        for i in 0..5 {
            b.push(strike(f64::from(i)));
        }
        let ts: Vec<f64> = b.snapshot().iter().map(|s| s.t).collect();
        assert_eq!(ts, vec![2.0, 3.0, 4.0]);
    }

    #[test]
    fn prune_stops_at_first_fresh_strike() {
        let mut b = StrikeBuffer::new(10);
        for t in [1.0, 2.0, 50.0, 3.0] {
            b.push(strike(t));
        }
        b.prune_before(10.0);
        // 1.0 and 2.0 are popped from the front. The scan stops at 50.0, so the stale 3.0 behind it stays, as in
        // Python.
        let ts: Vec<f64> = b.snapshot().iter().map(|s| s.t).collect();
        assert_eq!(ts, vec![50.0, 3.0]);
    }

    #[test]
    fn seed_keeps_window_drops_malformed_and_sorts() {
        let now = 1_800_000_000.0;
        let cutoff = now - 900.0;
        let feature = |t: Value, coords: Value, props_extra: Value| {
            let mut props = serde_json::Map::new();
            props.insert("time".into(), t);
            if let Value::Object(extra) = props_extra {
                props.extend(extra);
            }
            serde_json::json!({"type": "Feature", "geometry": {"type": "Point", "coordinates": coords}, "properties": props})
        };
        let body = serde_json::json!({"features": [
            feature(serde_json::json!(now - 1200.0), serde_json::json!([-97.0, 36.0]), serde_json::json!({})),
            feature(serde_json::json!(now - 30.0), serde_json::json!([-98.0, 35.0]), serde_json::json!({"polarity": -1, "mds": 12})),
            feature(serde_json::json!(cutoff), serde_json::json!([-90.0, 30.0]), serde_json::json!({})),
            feature(serde_json::json!(now - 60.0), serde_json::json!([1.0]), serde_json::json!({})),
            feature(serde_json::json!(now - 60.0), serde_json::json!([1.0, 2.0]), serde_json::json!({"polarity": null})),
            feature(serde_json::json!(now - 45.0), serde_json::json!([-96.0, 33.0]), serde_json::json!({"polarity": "2", "mds": 1.9})),
            Value::String("not an object".into()),
        ]});
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("lightning.json");
        std::fs::write(&path, body.to_string()).unwrap();
        let seeded = load_existing_strikes(&path, 15, now, 5000);
        let ts: Vec<f64> = seeded.iter().map(|s| s.t).collect();
        assert_eq!(ts, vec![cutoff, now - 45.0, now - 30.0]);
        assert_eq!((seeded[1].pol, seeded[1].mds), (2, 1));
        assert_eq!((seeded[2].pol, seeded[2].mds), (-1, 12));
    }

    #[test]
    fn seed_tolerates_missing_invalid_and_array_files() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("lightning.json");
        assert!(load_existing_strikes(&path, 15, 0.0, 10).is_empty());
        std::fs::write(&path, "{not json").unwrap();
        assert!(load_existing_strikes(&path, 15, 0.0, 10).is_empty());
        std::fs::write(&path, "[1, 2, 3]").unwrap();
        assert!(load_existing_strikes(&path, 15, 0.0, 10).is_empty());
    }

    #[test]
    fn seed_keeps_newest_max_strikes() {
        let now = 1_800_000_000.0;
        let features: Vec<Value> = (0..10)
            .map(|i| serde_json::json!({"geometry": {"coordinates": [-97.0, 36.0]}, "properties": {"time": now - f64::from(i)}}))
            .collect();
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("lightning.json");
        std::fs::write(&path, serde_json::json!({"features": features}).to_string()).unwrap();
        let seeded = load_existing_strikes(&path, 15, now, 3);
        let ts: Vec<f64> = seeded.iter().map(|s| s.t).collect();
        assert_eq!(ts, vec![now - 2.0, now - 1.0, now]);
    }

    #[test]
    fn writer_round_trips_through_seed() {
        let now = 1_789_999_999.5;
        let strikes = vec![
            Strike {
                t: now - 12.7,
                lat: 36.123_456,
                lon: -97.5,
                pol: 1,
                mds: 4,
            },
            Strike {
                t: now - 1.25,
                lat: 40.0,
                lon: -75.0,
                pol: -1,
                mds: 0,
            },
        ];
        let text = render_geojson(&strikes, now, 15);
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("lightning.json");
        std::fs::write(&path, &text).unwrap();
        let back = load_existing_strikes(&path, 15, now, 100);
        assert_eq!(back, strikes);
    }
}
