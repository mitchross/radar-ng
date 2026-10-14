//! NHC tropical cyclones: `tropical_fetch_and_publish` (spec-ingest-aux section 7).
//!
//! One poll: GET the NHC `CurrentStorms.json` feed, fetch the per-basin GIS layers (best effort), build one
//! GeoJSON FeatureCollection, and replace `STATE_DIR/tropical.json` atomically.
//!
//! Deliberate deviations (README): the output temp file is unique and fsynced (D-10); a zero latitude,
//! longitude, wind or pressure is kept (D-11); a non-finite or unparseable number is treated as missing
//! (Python would write NaN or crash); the file is written 0644 as `Path.write_text` does.

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use futures_util::StreamExt;
use radar_core::time::round_half_even;
use radar_types::payloads::tropical::TropicalResult;
use serde_json::{Map, Value, json};
use tracing::{info, warn};

/// File name of the published GeoJSON, in `STATE_DIR`.
pub const OUT_FILE: &str = "tropical.json";
/// Knots to miles per hour (`KT_TO_MPH`).
pub const KT_TO_MPH: f64 = 1.15078;
/// GIS layer suffixes per kind, in Python order.
pub const GIS_LAYERS: [(&str, &str); 3] = [
    ("cone", "Forecast Cone"),
    ("track", "Forecast Track"),
    ("points", "Forecast Points"),
];
/// Per-bin GIS data: basin bin → kind (`cone`, `track`, `points`) → GeoJSON FeatureCollection.
pub type GisMap = BTreeMap<String, BTreeMap<String, Value>>;

/// Configuration for `tropical_fetch_and_publish`.
#[derive(Debug, Clone)]
pub struct TropicalConfig {
    /// `STATE_DIR`.
    pub state_dir: PathBuf,
    /// `TROPICAL_FEED_URL`.
    pub feed_url: String,
    /// `TROPICAL_GIS_URL` (the MapServer root, without `?f=json`).
    pub gis_url: String,
    /// Feed timeout (20 s).
    pub feed_timeout: Duration,
    /// GIS index timeout (15 s).
    pub index_timeout: Duration,
    /// GIS layer query timeout (15 s).
    pub query_timeout: Duration,
    /// Parallel GIS queries (8).
    pub gis_concurrency: usize,
}

impl TropicalConfig {
    /// Reads `STATE_DIR`, `TROPICAL_FEED_URL` and `TROPICAL_GIS_URL` with the code defaults.
    pub fn from_env() -> Self {
        Self {
            state_dir: radar_core::env::path_var("STATE_DIR", "/data/state"),
            feed_url: radar_core::env::var("TROPICAL_FEED_URL")
                .unwrap_or_else(|| "https://www.nhc.noaa.gov/CurrentStorms.json".to_string()),
            gis_url: radar_core::env::var("TROPICAL_GIS_URL").unwrap_or_else(|| {
                "https://mapservices.weather.noaa.gov/tropical/rest/services/tropical/NHC_tropical_weather/MapServer"
                    .to_string()
            }),
            feed_timeout: Duration::from_secs(20),
            index_timeout: Duration::from_secs(15),
            query_timeout: Duration::from_secs(15),
            gis_concurrency: 8,
        }
    }

    /// Path of `tropical.json`.
    pub fn out_path(&self) -> PathBuf {
        self.state_dir.join(OUT_FILE)
    }
}

/// Errors from the tropical activity.
#[derive(Debug, thiserror::Error)]
pub enum TropicalError {
    /// The feed request failed before a status was received.
    #[error("transport error for {url}: {source}")]
    Transport {
        /// The URL.
        url: String,
        /// Underlying error.
        #[source]
        source: reqwest::Error,
    },
    /// The feed returned a non-2xx status (redirects included: they are not followed).
    #[error("HTTP {status} from {url}")]
    Status {
        /// Status code.
        status: u16,
        /// The URL.
        url: String,
    },
    /// The feed body is not JSON.
    #[error("feed is not valid JSON: {0}")]
    Json(#[from] serde_json::Error),
    /// Writing the output file failed.
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),
}

/// Storms in the feed: `activeStorms`, or `storms` when that is empty or absent (Python `or` chain).
pub fn storms_of(feed: &Value) -> &[Value] {
    for key in ["activeStorms", "storms"] {
        if let Some(list) = feed
            .get(key)
            .and_then(Value::as_array)
            .filter(|l| !l.is_empty())
        {
            return list;
        }
    }
    &[]
}

/// Basin bins to look up on the GIS server: `binNumber` of each `activeStorms` entry, non-empty only.
pub fn gis_bins(feed: &Value) -> Vec<String> {
    feed.get("activeStorms")
        .and_then(Value::as_array)
        .map(|storms| {
            storms
                .iter()
                .filter_map(|s| s.get("binNumber").and_then(Value::as_str))
                .filter(|b| !b.is_empty())
                .map(str::to_string)
                .collect::<BTreeSet<_>>()
                .into_iter()
                .collect()
        })
        .unwrap_or_default()
}

/// Python `_num`: a finite float from a JSON number or numeric string. Anything else is `None`.
pub fn num(value: Option<&Value>) -> Option<f64> {
    let parsed = match value? {
        Value::Number(n) => n.as_f64(),
        Value::String(s) => s.trim().parse::<f64>().ok(),
        _ => None,
    };
    parsed.filter(|f| f.is_finite())
}

/// Python `a or b or ...` on JSON values, with one deliberate change (D-11): a numeric `0` is a value,
/// not a miss. Null and the empty string fall through, as before.
pub fn first_present<'a>(candidates: &[Option<&'a Value>]) -> Option<&'a Value> {
    candidates
        .iter()
        .flatten()
        .copied()
        .find(|v| !v.is_null() && v.as_str() != Some(""))
}

/// Python truthiness for the GeoJSON checks (`if geometry`, `if fc`).
fn truthy(value: &Value) -> bool {
    match value {
        Value::Null => false,
        Value::Bool(b) => *b,
        Value::Number(n) => n.as_f64().is_some_and(|f| f != 0.0),
        Value::String(s) => !s.is_empty(),
        Value::Array(a) => !a.is_empty(),
        Value::Object(o) => !o.is_empty(),
    }
}

/// Saffir-Simpson category from knots: `None` below 64 kt, otherwise 1 to 5 (verified table, spec 7.3).
pub fn saffir_simpson(kt: Option<f64>) -> Option<i64> {
    let kt = kt?;
    if kt < 64.0 {
        return None;
    }
    Some(
        1 + [83.0, 96.0, 113.0, 137.0]
            .iter()
            .filter(|t| kt >= **t)
            .count() as i64,
    )
}

/// `round(kt * 1.15078)` with Python's round-half-even.
pub fn knots_to_mph(kt: f64) -> i64 {
    round_half_even(kt * KT_TO_MPH) as i64
}

/// The GeoJSON document and its counts.
#[derive(Debug, Clone, PartialEq)]
pub struct BuiltGeoJson {
    /// The FeatureCollection, with `generated_at` and `storm_count`.
    pub document: Value,
    /// Storms in the feed (`len(storms)`, not features).
    pub storm_count: usize,
    /// Features emitted.
    pub feature_count: usize,
}

/// Python `_build_geojson`: the feed plus optional GIS layers into one FeatureCollection.
///
/// Per storm, features are appended in this order: position, cones, tracks, forecast points. A storm
/// whose bin has no GIS layers gets only its position. `generated_at` is passed in (`time.time()` in the
/// Python).
pub fn build_geojson(feed: &Value, gis: &GisMap, generated_at: f64) -> BuiltGeoJson {
    let storms = storms_of(feed);
    let mut features: Vec<Value> = Vec::new();
    for storm in storms {
        let sid = first_present(&[storm.get("id"), storm.get("stormId")])
            .cloned()
            .unwrap_or_else(|| json!(""));
        let name = first_present(&[storm.get("name"), storm.get("storm_name")])
            .cloned()
            .unwrap_or_else(|| json!("Unknown"));
        let classification = first_present(&[storm.get("classification")])
            .cloned()
            .unwrap_or_else(|| json!(""));
        let basin = first_present(&[storm.get("binNumber"), storm.get("basin")])
            .cloned()
            .unwrap_or_else(|| json!(""));
        let lat = num(first_present(&[
            storm.get("latitudeNumeric"),
            storm.get("lat"),
        ]));
        let lon = num(first_present(&[
            storm.get("longitudeNumeric"),
            storm.get("lon"),
        ]));
        let wind_kt = num(first_present(&[
            storm.get("intensity"),
            storm.get("windSpeed"),
        ]));
        let pressure = num(first_present(&[
            storm.get("pressure"),
            storm.get("minPressure"),
        ]));
        let base = json!({ "storm_id": sid.clone(), "name": name.clone() });

        if let (Some(lat), Some(lon)) = (lat, lon) {
            let mut props = Map::new();
            props.insert("kind".into(), json!("position"));
            merge(&mut props, &base);
            props.insert("classification".into(), classification.clone());
            props.insert("basin".into(), basin.clone());
            props.insert("wind_kt".into(), json!(wind_kt));
            props.insert("wind_mph".into(), json!(wind_kt.map(knots_to_mph)));
            props.insert("category".into(), json!(saffir_simpson(wind_kt)));
            props.insert("pressure_mb".into(), json!(pressure));
            props.insert(
                "movement_dir_deg".into(),
                json!(num(storm.get("movementDir"))),
            );
            props.insert(
                "movement_mph".into(),
                json!(num(storm.get("movementSpeed"))),
            );
            props.insert(
                "updated_at".into(),
                storm.get("lastUpdate").cloned().unwrap_or(Value::Null),
            );
            props.insert(
                "advisory_url".into(),
                storm
                    .get("publicAdvisory")
                    .and_then(|a| a.get("url"))
                    .cloned()
                    .unwrap_or(Value::Null),
            );
            features.push(json!({
                "type": "Feature",
                "geometry": { "type": "Point", "coordinates": [lon, lat] },
                "properties": Value::Object(props),
            }));
        }

        let layers = basin.as_str().and_then(|b| gis.get(b));
        for (kind, out_kind) in [("cone", "cone"), ("track", "track")] {
            for f in gis_features(layers, kind) {
                features.push(json!({
                    "type": "Feature",
                    "geometry": f["geometry"].clone(),
                    "properties": merged_props(&[("kind", json!(out_kind))], &base),
                }));
            }
        }
        for f in gis_features(layers, "points") {
            let props = f
                .get("properties")
                .filter(|p| p.is_object())
                .cloned()
                .unwrap_or_else(|| json!({}));
            let kt = num(props.get("maxwind"));
            let mut out = Map::new();
            out.insert("kind".into(), json!("forecast_point"));
            merge(&mut out, &base);
            out.insert("tau_h".into(), json!(num(props.get("tau"))));
            out.insert(
                "label".into(),
                first_present(&[props.get("datelbl"), props.get("fldatelbl")])
                    .cloned()
                    .unwrap_or(Value::Null),
            );
            out.insert(
                "storm_type".into(),
                props.get("dvlbl").cloned().unwrap_or(Value::Null),
            );
            out.insert("wind_kt".into(), json!(kt));
            out.insert("wind_mph".into(), json!(kt.map(knots_to_mph)));
            out.insert("category".into(), json!(saffir_simpson(kt)));
            features.push(json!({
                "type": "Feature",
                "geometry": f["geometry"].clone(),
                "properties": Value::Object(out),
            }));
        }
    }
    let feature_count = features.len();
    let document = json!({
        "type": "FeatureCollection",
        "features": features,
        "generated_at": generated_at,
        "storm_count": storms.len(),
    });
    BuiltGeoJson {
        document,
        storm_count: storms.len(),
        feature_count,
    }
}

/// Features of one GIS kind for a bin, keeping only those with truthy geometry (Python `_gis_features`).
fn gis_features<'a>(layers: Option<&'a BTreeMap<String, Value>>, kind: &str) -> Vec<&'a Value> {
    layers
        .and_then(|l| l.get(kind))
        .and_then(|fc| fc.get("features"))
        .and_then(Value::as_array)
        .map(|fs| {
            fs.iter()
                .filter(|f| f.get("geometry").is_some_and(truthy))
                .collect()
        })
        .unwrap_or_default()
}

fn merged_props(first: &[(&str, Value)], base: &Value) -> Value {
    let mut props = Map::new();
    for (k, v) in first {
        props.insert((*k).to_string(), v.clone());
    }
    merge(&mut props, base);
    Value::Object(props)
}

fn merge(into: &mut Map<String, Value>, from: &Value) {
    if let Some(o) = from.as_object() {
        for (k, v) in o {
            into.insert(k.clone(), v.clone());
        }
    }
}

/// `tropical_fetch_and_publish`: one poll. Replaces `tropical.json` atomically.
///
/// The feed is required: its failure fails the activity (the workflow retries). The GIS layers are best
/// effort: a GIS failure publishes positions without cones, tracks or forecast points.
pub async fn tropical_fetch_and_publish(
    cfg: &TropicalConfig,
    http: &reqwest::Client,
) -> Result<TropicalResult, TropicalError> {
    fs::create_dir_all(&cfg.state_dir)?;
    let feed = get_json(http, &cfg.feed_url, cfg.feed_timeout).await?;
    let bins = gis_bins(&feed);
    let gis = fetch_gis(http, cfg, &bins).await;
    let generated_at = unix_now();
    let built = build_geojson(&feed, &gis, generated_at);
    let bytes = serde_json::to_vec(&built.document)?;
    let out = cfg.out_path();
    let dir = cfg.state_dir.clone();
    tokio::task::spawn_blocking(move || write_public_atomic(&dir, &out, &bytes))
        .await
        .map_err(|e| TropicalError::Io(std::io::Error::other(e)))??;
    info!(
        storm_count = built.storm_count,
        features = built.feature_count,
        "updated"
    );
    Ok(TropicalResult {
        storm_count: built.storm_count as i64,
        feature_count: built.feature_count as i64,
    })
}

/// Best-effort GIS fetch (Python `_fetch_gis`). Any failure of the index returns an empty map. A failed layer
/// contributes nothing.
pub async fn fetch_gis(http: &reqwest::Client, cfg: &TropicalConfig, bins: &[String]) -> GisMap {
    if bins.is_empty() {
        return GisMap::new();
    }
    let index_url = format!("{}?f=json", cfg.gis_url);
    let index = match get_json(http, &index_url, cfg.index_timeout).await {
        Ok(v) => v,
        Err(err) => {
            warn!(error = %err, "gis_index_failed");
            return GisMap::new();
        }
    };
    // Python: {layer["name"]: layer["id"]} raises KeyError on a layer without either key, which drops the
    // whole index. Keep that behaviour.
    let mut ids: BTreeMap<String, i64> = BTreeMap::new();
    for layer in index
        .get("layers")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
    {
        match (
            layer.get("name").and_then(Value::as_str),
            layer.get("id").and_then(Value::as_i64),
        ) {
            (Some(name), Some(id)) => {
                ids.insert(name.to_string(), id);
            }
            _ => {
                warn!("gis_index_failed: layer without name or id");
                return GisMap::new();
            }
        }
    }
    let mut jobs: Vec<(String, &'static str, i64)> = Vec::new();
    for bin in bins.iter().collect::<BTreeSet<_>>() {
        for (kind, suffix) in GIS_LAYERS {
            if let Some(id) = ids.get(&format!("{bin} {suffix}")) {
                jobs.push((bin.clone(), kind, *id));
            }
        }
    }
    let results: Vec<(String, &'static str, Option<Value>)> = futures_util::stream::iter(jobs)
        .map(|(bin, kind, id)| async move {
            let fc = query_layer(http, &cfg.gis_url, id, cfg.query_timeout).await;
            (bin, kind, fc)
        })
        .buffer_unordered(cfg.gis_concurrency.max(1))
        .collect()
        .await;
    let mut out = GisMap::new();
    for (bin, kind, fc) in results {
        if let Some(fc) = fc.filter(truthy) {
            out.entry(bin).or_default().insert(kind.to_string(), fc);
        }
    }
    out
}

async fn query_layer(
    http: &reqwest::Client,
    gis_url: &str,
    layer_id: i64,
    timeout: Duration,
) -> Option<Value> {
    let url = format!("{gis_url}/{layer_id}/query");
    let outcome = async {
        let resp = http
            .get(&url)
            .query(&[("where", "1=1"), ("outFields", "*"), ("f", "geojson")])
            .timeout(timeout)
            .send()
            .await
            .map_err(|e| e.to_string())?;
        if !resp.status().is_success() {
            return Err(format!("HTTP {}", resp.status().as_u16()));
        }
        resp.json::<Value>().await.map_err(|e| e.to_string())
    }
    .await;
    match outcome {
        Ok(v) => Some(v),
        Err(detail) => {
            warn!(layer = layer_id, error = %detail, "gis_layer_failed");
            None
        }
    }
}

async fn get_json(
    http: &reqwest::Client,
    url: &str,
    timeout: Duration,
) -> Result<Value, TropicalError> {
    let resp = http
        .get(url)
        .timeout(timeout)
        .send()
        .await
        .map_err(|source| TropicalError::Transport {
            url: url.to_string(),
            source,
        })?;
    let status = resp.status();
    if !status.is_success() {
        return Err(TropicalError::Status {
            status: status.as_u16(),
            url: url.to_string(),
        });
    }
    let body = resp
        .bytes()
        .await
        .map_err(|source| TropicalError::Transport {
            url: url.to_string(),
            source,
        })?;
    Ok(serde_json::from_slice(&body)?)
}

/// Writes `bytes` to `path` through a unique temp file in `dir`, fsynced, 0644, then renamed and the
/// directory fsynced (D-10 and D-13 for this file).
fn write_public_atomic(dir: &Path, path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    use std::os::unix::fs::PermissionsExt;
    let mut tmp = tempfile::Builder::new()
        .prefix(".tropical.")
        .suffix(".tmp")
        .tempfile_in(dir)?;
    tmp.write_all(bytes)?;
    tmp.flush()?;
    tmp.as_file().sync_all()?;
    fs::set_permissions(tmp.path(), fs::Permissions::from_mode(0o644))?;
    tmp.persist(path).map_err(|e| e.error)?;
    radar_core::fsutil::fsync_dir(dir)
}

fn unix_now() -> f64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs_f64())
        .unwrap_or(0.0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn saffir_simpson_table() {
        let kts = [
            None,
            Some(63.0),
            Some(64.0),
            Some(83.0),
            Some(96.0),
            Some(113.0),
            Some(137.0),
        ];
        let got: Vec<Option<i64>> = kts.iter().map(|k| saffir_simpson(*k)).collect();
        assert_eq!(
            got,
            vec![None, None, Some(1), Some(2), Some(3), Some(4), Some(5)]
        );
    }

    #[test]
    fn python_round_is_half_even() {
        assert_eq!(round_half_even(0.5), 0.0);
        assert_eq!(round_half_even(1.5), 2.0);
        assert_eq!(round_half_even(2.5), 2.0);
        assert_eq!(knots_to_mph(100.0), 115);
        assert_eq!(knots_to_mph(75.0), 86);
    }

    #[test]
    fn numbers_accept_numeric_strings_and_reject_non_finite() {
        assert_eq!(num(Some(&json!("100"))), Some(100.0));
        assert_eq!(num(Some(&json!(0))), Some(0.0));
        assert_eq!(num(Some(&json!("nan"))), None);
        assert_eq!(num(Some(&json!("x"))), None);
        assert_eq!(num(None), None);
    }

    #[test]
    fn first_present_skips_null_and_empty_but_keeps_zero() {
        let zero = json!(0);
        let empty = json!("");
        let other = json!("b");
        assert_eq!(first_present(&[Some(&zero), Some(&other)]), Some(&zero));
        assert_eq!(first_present(&[Some(&empty), Some(&other)]), Some(&other));
        assert_eq!(first_present(&[None, Some(&Value::Null)]), None);
    }
}
