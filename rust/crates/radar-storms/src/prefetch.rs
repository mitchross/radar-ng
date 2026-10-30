//! Three-bbox storm tile prefetch plan (spec-shared section 7, spec-api 5.9).
//!
//! The plan picks the best storm for a point, then for leads 0, 5 and 10 minutes
//! finds the manifest frame to show and lists the tiles that exist on disk.
//!
//! **D1 fix.** Tile paths come from the manifest frame's `path` (for example
//! `runs/<anchor>/<valid>` for nowcast), not from its timestamp. The Python code
//! built paths from the timestamp, so leads 5 and 10 silently had no tiles.
//! The `style_url` query still carries `timestamp`, as the API contract requires.

use std::path::Path;

use serde::Serialize;
use serde_json::{Map, Value};

use crate::config::StormConfig;
use crate::error::StormsError;
use crate::geo::{distance_latlon_km, lat_lon_to_tile};
use crate::numeric::{py_float, py_int};
use crate::time::{Instant, parse_timestamp};

/// Characters that stay unescaped in tile URL segments (spec-shared 7.6).
pub const TILE_URL_SAFE: &str = ":+-.T_";

/// Minimum requested zoom (`MIN_ZOOM`).
pub const MIN_ZOOM: i64 = 4;

/// Maximum zoom per layer (`MAX_ZOOM`).
#[must_use]
pub fn max_zoom(layer: &str) -> Option<i64> {
    match layer {
        "radar" => Some(7),
        "nowcast" => Some(6),
        _ => None,
    }
}

/// Parameters of one plan request.
#[derive(Debug, Clone, Copy)]
pub struct PrefetchRequest<'a> {
    /// Latitude of the user (degrees).
    pub lat: f64,
    /// Longitude of the user (degrees).
    pub lon: f64,
    /// Requested zoom (validated by the API to 4..=8).
    pub zoom: i64,
    /// Palette name (`classic`, `vivid`, `muted`).
    pub palette: &'a str,
    /// Public base URL with or without a trailing slash.
    pub base_url: &'a str,
}

/// One of the three prefetch entries.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct PlanBbox {
    /// Lead in minutes.
    pub lead_minutes: i64,
    /// Predicted bbox `[west, south, east, north]`.
    pub bbox: [f64; 4],
    /// `radar` or `nowcast`, or `None` when no frame matches the lead.
    pub layer: Option<String>,
    /// Frame timestamp, or `None`.
    pub timestamp: Option<String>,
    /// Zoom after clamping to the layer's coverage.
    pub zoom: i64,
    /// Style URL for the frame, or `None`.
    pub style_url: Option<String>,
    /// Tile URLs that exist on disk.
    pub tile_urls: Vec<String>,
}

/// The plan (`build_storm_prefetch_plan` return value).
///
/// With no storm selected only `plan_id`, `storm_cell_id`, `bboxes` and
/// `tile_urls` are present, as in the Python shape.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct PrefetchPlan {
    /// `"<anchor>:<cell_id>:<palette>:<requested_zoom>"`, or `None`.
    pub plan_id: Option<String>,
    /// Selected cell id, or `None`.
    pub storm_cell_id: Option<i64>,
    /// Copied from `storms.generated_at` (present only with a selected storm).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub generated_at: Option<Value>,
    /// Copied from the selected cell (present only with a selected storm).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tracking_vector: Option<Value>,
    /// Up to three entries, one per lead.
    pub bboxes: Vec<PlanBbox>,
    /// Order-preserving union of the per-bbox tile URLs.
    pub tile_urls: Vec<String>,
}

impl PrefetchPlan {
    fn empty() -> Self {
        Self {
            plan_id: None,
            storm_cell_id: None,
            generated_at: None,
            tracking_vector: None,
            bboxes: Vec::new(),
            tile_urls: Vec::new(),
        }
    }
}

/// Where a lead's frame came from: layer, timestamp and the tile path.
#[derive(Debug, Clone, PartialEq)]
pub struct FrameRef {
    /// `radar` or `nowcast`.
    pub layer: String,
    /// Frame timestamp (used in the style URL).
    pub timestamp: String,
    /// Tile path from the manifest frame (D1). Equals `timestamp` for legacy frames.
    pub path: String,
}

/// Build the prefetch plan for a point.
///
/// `storms` is the parsed `storms.json`. `manifest` must already be validated (see
/// [`read_manifest_value`]). `tile_dir` is the tile root, used for existence checks only.
///
/// # Errors
/// Returns [`StormsError::Manifest`] when a storm is selected but the manifest is
/// not a usable manifest. With no storm selected, the manifest is not read.
pub fn build_storm_prefetch_plan(
    storms: &Value,
    manifest: &Value,
    tile_dir: &Path,
    req: &PrefetchRequest<'_>,
    cfg: &StormConfig,
) -> Result<PrefetchPlan, StormsError> {
    let Some(selected) = select_storm(storms, req.lat, req.lon, cfg) else {
        return Ok(PrefetchPlan::empty());
    };
    let Some(anchor) = storms
        .get("timestamp")
        .and_then(Value::as_str)
        .and_then(parse_timestamp)
    else {
        return Ok(PrefetchPlan::empty());
    };
    let layers = manifest_layers(manifest)?;

    let props = selected
        .get("properties")
        .and_then(Value::as_object)
        .ok_or_else(|| manifest_err("selected storm has no properties"))?;
    let cell_id = props.get("cell_id").and_then(py_int).unwrap_or(0);
    let base_url = req.base_url.trim_end_matches('/');
    let requested_zoom = requested_zoom(req.zoom);

    let mut bboxes = Vec::new();
    let mut all_urls: Vec<String> = Vec::new();
    let predictions = props
        .get("predicted_bboxes")
        .and_then(Value::as_array)
        .map_or(&[][..], Vec::as_slice);
    for prediction in predictions.iter().take(3) {
        let Some(pred) = prediction.as_object() else {
            continue;
        };
        let (Some(lead), Some(bbox)) = (
            pred.get("lead_minutes").and_then(py_int),
            pred.get("bbox").and_then(read_bbox),
        ) else {
            continue;
        };
        match frame_for_lead(layers, anchor, lead) {
            None => bboxes.push(PlanBbox {
                lead_minutes: lead,
                bbox,
                layer: None,
                timestamp: None,
                zoom: requested_zoom,
                style_url: None,
                tile_urls: Vec::new(),
            }),
            Some(frame) => {
                let layer_max = max_zoom(&frame.layer).unwrap_or(requested_zoom);
                let frame_zoom = requested_zoom.min(layer_max);
                let urls =
                    existing_tile_urls(tile_dir, base_url, req.palette, &frame, bbox, frame_zoom);
                let query = encode_query(&[
                    ("layer", frame.layer.clone()),
                    ("palette", req.palette.to_owned()),
                    ("timestamp", frame.timestamp.clone()),
                    ("zoom", frame_zoom.to_string()),
                ]);
                all_urls.extend(urls.iter().cloned());
                bboxes.push(PlanBbox {
                    lead_minutes: lead,
                    bbox,
                    style_url: Some(format!("{base_url}/api/storm-prefetch/style.json?{query}")),
                    layer: Some(frame.layer),
                    timestamp: Some(frame.timestamp),
                    zoom: frame_zoom,
                    tile_urls: urls,
                });
            }
        }
    }

    let anchor_text = storms
        .get("timestamp")
        .and_then(Value::as_str)
        .unwrap_or_default();
    let mut unique: Vec<String> = Vec::new();
    for url in all_urls {
        if !unique.contains(&url) {
            unique.push(url);
        }
    }
    Ok(PrefetchPlan {
        plan_id: Some(format!(
            "{anchor_text}:{cell_id}:{}:{requested_zoom}",
            req.palette
        )),
        storm_cell_id: Some(cell_id),
        generated_at: Some(storms.get("generated_at").cloned().unwrap_or(Value::Null)),
        tracking_vector: Some(props.get("tracking_vector").cloned().unwrap_or(Value::Null)),
        bboxes,
        tile_urls: unique,
    })
}

/// `max(4, int(zoom))`.
fn requested_zoom(zoom: i64) -> i64 {
    zoom.max(MIN_ZOOM)
}

/// Pick the storm to prefetch (`_select_storm`). Returns the feature object.
///
/// The score is distance to the nearest predicted-bbox centre, minus an intensity
/// and size bonus capped at 35 km. The lowest score wins; ties keep list order.
/// The selected feature's point must be within `prefetch_max_distance_km`.
fn select_storm<'a>(
    storms: &'a Value,
    lat: f64,
    lon: f64,
    cfg: &StormConfig,
) -> Option<&'a Map<String, Value>> {
    let features = storms.get("features").and_then(Value::as_array)?;
    let mut ranked: Vec<(f64, &Map<String, Value>)> = Vec::new();
    for feature in features {
        let Some(feature) = feature.as_object() else {
            continue;
        };
        let Some(props) = feature.get("properties").and_then(Value::as_object) else {
            continue;
        };
        if props.get("cell_id").and_then(py_int).is_none() {
            continue;
        }
        let Some(predictions) = props.get("predicted_bboxes").and_then(Value::as_array) else {
            continue;
        };
        // Any malformed prediction drops the whole feature, as the Python try block does.
        let Some(distances) = predictions
            .iter()
            .map(|p| {
                let [west, south, east, north] = p.get("bbox").and_then(read_bbox)?;
                Some(distance_latlon_km(
                    lat,
                    lon,
                    (south + north) / 2.0,
                    (west + east) / 2.0,
                ))
            })
            .collect::<Option<Vec<f64>>>()
        else {
            continue;
        };
        let Some(distance) = distances.iter().copied().reduce(f64::min) else {
            continue;
        };
        let peak = match props.get("peak_dbz") {
            None => 40.0,
            Some(v) => match py_float(v) {
                Some(f) => f,
                None => continue,
            },
        };
        let area = match props.get("area_km2") {
            None => 1.0,
            Some(v) => match py_float(v) {
                Some(f) => f,
                None => continue,
            },
        };
        let severity = (peak - 40.0).max(0.0);
        let area = area.max(1.0);
        let score = distance - (severity * 0.8 + area.log10() * 3.0).min(35.0);
        ranked.push((score, feature));
    }
    // Stable sort: the first of equal scores wins, as Python's sort does.
    ranked.sort_by(|a, b| a.0.partial_cmp(&b.0).unwrap_or(std::cmp::Ordering::Equal));
    let (_, selected) = *ranked.first()?;
    let point = selected
        .get("geometry")
        .and_then(Value::as_object)?
        .get("coordinates")?;
    let coords = point.as_array()?;
    if coords.len() != 2 {
        return None;
    }
    let sel_lon = py_float(&coords[0])?;
    let sel_lat = py_float(&coords[1])?;
    if distance_latlon_km(lat, lon, sel_lat, sel_lon) > cfg.prefetch_max_distance_km {
        return None;
    }
    Some(selected)
}

/// Frame for a lead (`_frame_for_lead`), including the manifest path (D1).
fn frame_for_lead(layers: &Map<String, Value>, anchor: Instant, lead: i64) -> Option<FrameRef> {
    if lead == 0 {
        let timestamp = nearest_timestamp(layer_timestamps(layers, "radar"), anchor, true)?;
        return Some(frame_ref(layers, "radar", timestamp));
    }
    let target = anchor.plus_minutes(lead);
    let timestamp = nearest_timestamp(layer_timestamps(layers, "nowcast"), target, false)?;
    let parsed = parse_timestamp(&timestamp)?;
    if (parsed.0 - target.0).abs() > 8 * 60 * 1_000_000 {
        return None;
    }
    Some(frame_ref(layers, "nowcast", timestamp))
}

fn frame_ref(layers: &Map<String, Value>, layer: &str, timestamp: String) -> FrameRef {
    let path = layers
        .get(layer)
        .and_then(Value::as_object)
        .and_then(|l| l.get("frames"))
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(Value::as_object)
        .find(|f| f.get("timestamp").and_then(Value::as_str) == Some(timestamp.as_str()))
        .and_then(|f| f.get("path").and_then(Value::as_str))
        .filter(|p| !p.is_empty())
        .map_or_else(|| timestamp.clone(), str::to_owned);
    FrameRef {
        layer: layer.to_owned(),
        timestamp,
        path,
    }
}

fn layer_timestamps(layers: &Map<String, Value>, layer: &str) -> Vec<String> {
    layers
        .get(layer)
        .and_then(Value::as_object)
        .and_then(|l| l.get("timestamps"))
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|v| v.as_str().map(str::to_owned))
        .collect()
}

/// Nearest parseable timestamp to `target`. With `prefer_past`, only those not after the target.
/// Ties keep list order (first wins).
fn nearest_timestamp(
    candidates: Vec<String>,
    target: Instant,
    prefer_past: bool,
) -> Option<String> {
    let mut best: Option<(i64, String)> = None;
    for value in candidates {
        let Some(parsed) = parse_timestamp(&value) else {
            continue;
        };
        if prefer_past && parsed > target {
            continue;
        }
        let diff = (parsed.0 - target.0).abs();
        if best.as_ref().is_none_or(|(d, _)| diff < *d) {
            best = Some((diff, value));
        }
    }
    best.map(|(_, v)| v)
}

/// Existing tile URLs for one bbox (`_tiles_for_bbox` plus the disk check).
fn existing_tile_urls(
    tile_dir: &Path,
    base_url: &str,
    palette: &str,
    frame: &FrameRef,
    bbox: [f64; 4],
    zoom: i64,
) -> Vec<String> {
    let [west, south, east, north] = bbox;
    let zoom_u = u32::try_from(zoom.max(0)).unwrap_or(0);
    let (x_min, y_min) = lat_lon_to_tile(north, west, zoom_u);
    let (x_max, y_max) = lat_lon_to_tile(south, east, zoom_u);
    let mut urls = Vec::new();
    for x in x_min.min(x_max)..=x_min.max(x_max) {
        for y in y_min.min(y_max)..=y_min.max(y_max) {
            let raw = format!(
                "{}/{palette}/{}/{zoom}/{x}/{y}.png",
                frame.layer, frame.path
            );
            let disk = tile_dir.join(&raw);
            if disk.is_file() {
                urls.push(format!(
                    "{base_url}{}",
                    tile_url_path(&frame.layer, palette, &frame.path, zoom, x, y)
                ));
            }
        }
    }
    urls
}

/// URL path `/tiles/<layer>/<palette>/<path>/<z>/<x>/<y>.png`. Each segment of `path`
/// is quoted separately, so the `/` separators in `runs/<anchor>/<valid>` survive.
#[must_use]
pub fn tile_url_path(layer: &str, palette: &str, path: &str, zoom: i64, x: i64, y: i64) -> String {
    let mut out = String::from("/tiles");
    for seg in [layer, palette] {
        out.push('/');
        out.push_str(&quote(seg, TILE_URL_SAFE));
    }
    for seg in path.split('/') {
        out.push('/');
        out.push_str(&quote(seg, TILE_URL_SAFE));
    }
    out.push_str(&format!("/{zoom}/{x}/{y}.png"));
    out
}

/// Percent-encode like Python `urllib.parse.quote(s, safe=...)`.
#[must_use]
pub fn quote(s: &str, safe: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for &b in s.as_bytes() {
        if b.is_ascii_alphanumeric() || b"_.-~".contains(&b) || safe.as_bytes().contains(&b) {
            out.push(char::from(b));
        } else {
            out.push_str(&format!("%{b:02X}"));
        }
    }
    out
}

/// `urlencode` with `quote_plus` semantics for the given pairs, in order.
#[must_use]
pub fn encode_query(pairs: &[(&str, String)]) -> String {
    pairs
        .iter()
        .map(|(k, v)| format!("{}={}", quote(k, ""), quote(v, "").replace("%20", "+")))
        .collect::<Vec<_>>()
        .join("&")
}

fn read_bbox(v: &Value) -> Option<[f64; 4]> {
    let items = v.as_array().filter(|a| a.len() == 4)?;
    Some([
        py_float(&items[0])?,
        py_float(&items[1])?,
        py_float(&items[2])?,
        py_float(&items[3])?,
    ])
}

fn manifest_layers(manifest: &Value) -> Result<&Map<String, Value>, StormsError> {
    manifest
        .as_object()
        .and_then(|m| m.get("layers"))
        .and_then(Value::as_object)
        .ok_or_else(|| manifest_err("manifest has no layers object"))
}

fn manifest_err(reason: &str) -> StormsError {
    StormsError::Manifest {
        path: std::path::PathBuf::from("manifest.json"),
        reason: reason.to_owned(),
    }
}

/// Read and validate `state_dir/manifest.json` for prefetch (spec-shared 3.4 and D7).
///
/// This is radar-core's `read_manifest`, as a JSON value. A missing file is an empty
/// manifest. A file that does not parse or is structurally invalid is an error, so
/// callers never plan from a half-read or wiped manifest.
///
/// # Errors
/// Returns [`StormsError::Manifest`] for unusable content and [`StormsError::Io`]
/// for read failures other than "not found".
pub fn read_manifest_value(state_dir: &Path) -> Result<Value, StormsError> {
    let manifest = radar_core::manifest::read_manifest(state_dir).map_err(|err| match err {
        radar_core::manifest::ManifestError::Io(io) => StormsError::Io(io),
        other => StormsError::Manifest {
            path: radar_core::manifest::manifest_path(state_dir),
            reason: other.to_string(),
        },
    })?;
    Ok(serde_json::to_value(&manifest)?)
}

/// Build the plan from the state directory: validate the manifest, then plan.
///
/// # Errors
/// See [`read_manifest_value`] and [`build_storm_prefetch_plan`].
pub fn build_storm_prefetch_plan_from_state(
    storms: &Value,
    state_dir: &Path,
    tile_dir: &Path,
    req: &PrefetchRequest<'_>,
    cfg: &StormConfig,
) -> Result<PrefetchPlan, StormsError> {
    let manifest = read_manifest_value(state_dir)?;
    build_storm_prefetch_plan(storms, &manifest, tile_dir, req, cfg)
}
