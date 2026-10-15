//! GOES GeoColor satellite imagery from NASA GIBS: the `/tiles/satellite/{time}/{z}/{x}/{y}.png` proxy
//! and the `satellite` manifest layer.
//!
//! - Two satellites. GOES-East (75.2° W) covers the map from the 106.1° W seam eastward and GOES-West
//!   (137.0° W) covers the rest, so each place is seen from the satellite with the lower view angle.
//!   A meridian is a pixel column in Web Mercator, so only the tiles that straddle a seam are decoded
//!   and spliced. All other tiles pass through byte for byte.
//! - Frame times are the 10-minute steps that GIBS DescribeDomains lists for both layers in the last
//!   [`FRAME_WINDOW_S`]. The list is cached for [`DOMAIN_REFRESH`] and refreshed in the background.
//!   Only the first manifest build after start waits for it (at most [`COLD_WAIT`]).
//! - The proxy accepts only a canonical 10-minute time inside [`ACCEPT_WINDOW_S`], `z` 0 to 7 and an
//!   in-range `x`/`y`, so it cannot fetch arbitrary GIBS URLs. A tile URL names one frame, so a 200 is
//!   immutable. Errors are `no-store` (the static-lane override).

use std::collections::BTreeSet;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use axum::body::Bytes;
use axum::extract::{Path, State};
use axum::http::{HeaderValue, StatusCode, header};
use axum::response::{IntoResponse, Response};
use chrono::{DateTime, NaiveDateTime, TimeDelta, Timelike, Utc};
use serde_json::{Value, json};

use crate::respond;
use crate::state::AppState;
use crate::upstream;

/// GIBS layer east of the seams (GOES-East).
pub const EAST_LAYER: &str = "GOES-East_ABI_GeoColor_v0_NRT";
/// GIBS layer west of the seams (GOES-West).
pub const WEST_LAYER: &str = "GOES-West_ABI_GeoColor_v0_NRT";
const MATRIX_SET: &str = "GoogleMapsCompatible_Level7";
/// Deepest native zoom of [`MATRIX_SET`]. Clients overzoom past it.
pub const MAX_ZOOM: u8 = 7;
const TILE_PX: usize = 256;
/// GOES-East is the source for longitudes in `[SEAM_WEST_LON, SEAM_EAST_LON)`: the midpoint between the
/// two sub-satellite points (75.2° W, 137.0° W) and its antipode.
const SEAM_WEST_LON: f64 = -106.1;
const SEAM_EAST_LON: f64 = 73.9;
/// GIBS publishes GeoColor every 10 minutes.
const STEP_MIN: u32 = 10;
/// Frames listed in the manifest: the MRMS retention (4 h) plus the ~40 min GIBS latency.
pub const FRAME_WINDOW_S: i64 = 4 * 3600 + 30 * 60;
/// Oldest time the proxy accepts, so a client holding an older manifest still gets its tiles.
pub const ACCEPT_WINDOW_S: i64 = 6 * 3600;
const DOMAIN_REFRESH: Duration = Duration::from_secs(120);
const DOMAIN_RETRY: Duration = Duration::from_secs(60);
const COLD_WAIT: Duration = Duration::from_secs(3);
const DOMAIN_TIMEOUT: Duration = Duration::from_secs(10);
const TILE_TIMEOUT: Duration = Duration::from_secs(20);
const TILE_CC: &str = "public, max-age=86400, immutable";
/// Guards the domain parser against a malformed interval that would expand without end.
const MAX_DOMAIN_TIMES: usize = 2000;

/// The cached frame list and the bookkeeping for its refresh.
#[derive(Default)]
pub struct DomainCache {
    times: std::sync::Mutex<DomainState>,
    refreshing: AtomicBool,
}

#[derive(Default)]
struct DomainState {
    times: Option<Arc<Vec<DateTime<Utc>>>>,
    fetched: Option<Instant>,
    attempted: Option<Instant>,
}

/// Key of a cached tile: frame (epoch minutes), z, x, y.
pub type TileKey = (i64, u8, u32, u32);

/// Why a tile could not be served.
#[derive(Debug, Clone, Copy)]
pub enum TileError {
    /// GIBS answered with this status (a 404 is a time it does not have).
    Upstream(StatusCode),
    /// Transport failure, or a PNG that would not decode.
    BadGateway,
}

fn lock(m: &std::sync::Mutex<DomainState>) -> std::sync::MutexGuard<'_, DomainState> {
    m.lock().unwrap_or_else(std::sync::PoisonError::into_inner)
}

/// A time path segment in the one canonical form `YYYY-MM-DDTHH:M0:00Z`.
#[must_use]
pub fn parse_time(segment: &str) -> Option<DateTime<Utc>> {
    let naive = NaiveDateTime::parse_from_str(segment, "%Y-%m-%dT%H:%M:%SZ").ok()?;
    let at = naive.and_utc();
    (format_time(at) == segment && at.second() == 0 && at.minute() % STEP_MIN == 0).then_some(at)
}

/// The GIBS time form, also the frame `path` in the manifest.
#[must_use]
pub fn format_time(at: DateTime<Utc>) -> String {
    at.format("%Y-%m-%dT%H:%M:%SZ").to_string()
}

/// `z`, `x` and `y` (with its `.png`): decimal, no leading zeros, `z` at most [`MAX_ZOOM`], `x`/`y` on the grid.
#[must_use]
pub fn tile_parts(z: &str, x: &str, ypng: &str) -> Option<(u8, u32, u32)> {
    let y = ypng.strip_suffix(".png")?;
    let number = |s: &str| -> Option<u32> {
        let canonical = !s.is_empty()
            && s.len() <= 3
            && s.bytes().all(|b| b.is_ascii_digit())
            && (s == "0" || !s.starts_with('0'));
        canonical.then(|| s.parse().ok()).flatten()
    };
    let zoom = u8::try_from(number(z)?).ok().filter(|v| *v <= MAX_ZOOM)?;
    let (xn, yn) = (number(x)?, number(y)?);
    let side = 1u32 << zoom;
    (xn < side && yn < side).then_some((zoom, xn, yn))
}

/// True when `at` is a frame the proxy serves at `now`: not in the future and inside [`ACCEPT_WINDOW_S`].
#[must_use]
pub fn time_accepted(at: DateTime<Utc>, now: DateTime<Utc>) -> bool {
    at <= now && now - at <= TimeDelta::seconds(ACCEPT_WINDOW_S)
}

/// Which satellite paints which pixel columns of tile `(z, x)`.
#[derive(Debug, PartialEq, Eq)]
pub enum Split {
    /// Every column from GOES-East.
    East,
    /// Every column from GOES-West.
    West,
    /// Columns `start..end` from GOES-East, the rest from GOES-West.
    Columns(usize, usize),
}

/// The column split for tile `(z, x)`.
#[must_use]
pub fn split(z: u8, x: u32) -> Split {
    let tiles = f64::from(1u32 << z);
    let column = |lon: f64| {
        let px = ((lon + 180.0) / 360.0 * tiles - f64::from(x)) * TILE_PX as f64;
        px.round().clamp(0.0, TILE_PX as f64) as usize
    };
    match (column(SEAM_WEST_LON), column(SEAM_EAST_LON)) {
        (0, TILE_PX) => Split::East,
        (start, end) if start == end => Split::West,
        (start, end) => Split::Columns(start, end),
    }
}

/// The times in a DescribeDomains `<Domain>` value: comma-separated single times or
/// `start/end/PTnM` intervals. Unknown items are skipped.
#[must_use]
pub fn parse_domain(xml: &str) -> BTreeSet<DateTime<Utc>> {
    let mut out = BTreeSet::new();
    let Some(body) = xml
        .split_once("<Domain>")
        .and_then(|(_, rest)| rest.split_once("</Domain>"))
        .map(|(body, _)| body)
    else {
        return out;
    };
    let iso = |s: &str| {
        DateTime::parse_from_rfc3339(s.trim())
            .ok()
            .map(|t| t.to_utc())
    };
    for item in body.split(',') {
        let parts: Vec<&str> = item.trim().split('/').collect();
        match parts.as_slice() {
            [one] => out.extend(iso(one)),
            [start, end, period] => {
                let (Some(mut at), Some(end)) = (iso(start), iso(end)) else {
                    continue;
                };
                let Some(step) = period_minutes(period) else {
                    continue;
                };
                while at <= end && out.len() < MAX_DOMAIN_TIMES {
                    out.insert(at);
                    at += TimeDelta::minutes(step);
                }
            }
            _ => {}
        }
    }
    out
}

/// `PTnM` or `PTnH` as minutes.
fn period_minutes(period: &str) -> Option<i64> {
    let rest = period.trim().strip_prefix("PT")?;
    let (n, scale) = match rest.strip_suffix('M') {
        Some(n) => (n, 1),
        None => (rest.strip_suffix('H')?, 60),
    };
    n.parse::<i64>().ok().filter(|v| *v > 0).map(|v| v * scale)
}

/// The `satellite` manifest layer for `times` (sorted, unique). Same shape as a radar-core layer.
#[must_use]
pub fn manifest_layer(times: &[DateTime<Utc>]) -> Value {
    let stamp = |t: &DateTime<Utc>| t.format("%Y-%m-%dT%H:%M:%S+00:00").to_string();
    let frames: Vec<Value> = times
        .iter()
        .map(|t| {
            json!({
                "timestamp": stamp(t),
                "path": format_time(*t),
                "palettes": [],
                "kind": "observation",
                "source": "goes-geocolor",
                "max_zoom": MAX_ZOOM,
            })
        })
        .collect();
    json!({
        "frames": frames,
        "timestamps": times.iter().map(stamp).collect::<Vec<_>>(),
        "latest": times.last().map(stamp),
        "palettes": [],
        "title": "GOES GeoColor satellite",
        "kind": "satellite",
        "tile_url_template": "/tiles/satellite/{path}/{z}/{x}/{y}.png",
        "attribution": "NASA GIBS, NOAA GOES-East and GOES-West",
    })
}

/// The manifest with the `satellite` layer added when frame times are known. Without times (GIBS
/// unreachable, or `GIBS_WMTS_BASE` empty) the manifest is returned unchanged.
pub async fn with_layer(state: &AppState, manifest: Arc<Value>) -> Arc<Value> {
    let Some(times) = frame_times(state).await.filter(|t| !t.is_empty()) else {
        return manifest;
    };
    let mut value = (*manifest).clone();
    if let Some(layers) = value.get_mut("layers").and_then(Value::as_object_mut) {
        layers.insert("satellite".into(), manifest_layer(&times));
    }
    Arc::new(value)
}

/// The cached frame times. Starts a refresh when the list is stale; waits for it only when there is none.
async fn frame_times(state: &AppState) -> Option<Arc<Vec<DateTime<Utc>>>> {
    if state.config.gibs_wmts_base.is_empty() {
        return None;
    }
    let cache = &state.satellite_domain;
    let (current, due) = {
        let st = lock(&cache.times);
        let fresh = st.fetched.is_some_and(|t| t.elapsed() < DOMAIN_REFRESH);
        let backing_off = st.attempted.is_some_and(|t| t.elapsed() < DOMAIN_RETRY);
        (st.times.clone(), !fresh && !backing_off)
    };
    if !due || cache.refreshing.swap(true, Ordering::AcqRel) {
        return current;
    }
    lock(&cache.times).attempted = Some(Instant::now());
    let task = tokio::spawn(refresh_domain(state.clone()));
    if current.is_some() {
        return current;
    }
    let _ = tokio::time::timeout(COLD_WAIT, task).await;
    lock(&cache.times).times.clone()
}

async fn refresh_domain(state: AppState) {
    let now = Utc::now();
    let start = floor_to_step(now - TimeDelta::seconds(FRAME_WINDOW_S));
    // The end moves every minute, so GIBS's 30-minute cache on a fixed range never serves a stale list.
    let end = now.with_second(0).unwrap_or(now);
    let range = format!("{}--{}", format_time(start), format_time(end));
    let (east, west) = tokio::join!(
        fetch_domain(&state, EAST_LAYER, &range),
        fetch_domain(&state, WEST_LAYER, &range),
    );
    match (east, west) {
        (Some(east), Some(west)) => {
            let times: Vec<DateTime<Utc>> = east
                .intersection(&west)
                .filter(|t| **t >= start && **t <= now)
                .copied()
                .collect();
            let mut st = lock(&state.satellite_domain.times);
            st.times = Some(Arc::new(times));
            st.fetched = Some(Instant::now());
        }
        _ => tracing::warn!("GIBS DescribeDomains failed; keeping the previous satellite frames"),
    }
    state
        .satellite_domain
        .refreshing
        .store(false, Ordering::Release);
}

fn floor_to_step(at: DateTime<Utc>) -> DateTime<Utc> {
    let minute = at.minute() - at.minute() % STEP_MIN;
    at.with_minute(minute)
        .and_then(|t| t.with_second(0))
        .and_then(|t| t.with_nanosecond(0))
        .unwrap_or(at)
}

async fn fetch_domain(
    state: &AppState,
    layer: &str,
    range: &str,
) -> Option<BTreeSet<DateTime<Utc>>> {
    let url = format!(
        "{}/1.0.0/{layer}/default/{MATRIX_SET}/all/{range}.xml",
        state.config.gibs_wmts_base
    );
    let fetched = upstream::get(&state.http, &url, &[], &[], DOMAIN_TIMEOUT)
        .await
        .ok()?;
    if !fetched.is_success() {
        tracing::warn!(status = %fetched.status, "GIBS DescribeDomains for {layer}");
        return None;
    }
    Some(parse_domain(&String::from_utf8_lossy(&fetched.body)))
}

/// `GET /tiles/satellite/{time}/{z}/{x}/{y}.png`.
pub async fn get_tile(
    State(state): State<AppState>,
    Path((time, z, x, ypng)): Path<(String, String, String, String)>,
) -> Response {
    let (Some(at), Some((z, x, y))) = (parse_time(&time), tile_parts(&z, &x, &ypng)) else {
        return respond::caddy_error(StatusCode::NOT_FOUND);
    };
    if state.config.gibs_wmts_base.is_empty() || !time_accepted(at, Utc::now()) {
        return respond::caddy_error(StatusCode::NOT_FOUND);
    }
    let key: TileKey = (at.timestamp() / 60, z, x, y);
    let loaded = state
        .satellite_tiles
        .try_get_with(key, load_tile(&state, &time, z, x, y))
        .await;
    match loaded {
        Ok(bytes) => (
            StatusCode::OK,
            [
                (header::CONTENT_TYPE, HeaderValue::from_static("image/png")),
                (header::CACHE_CONTROL, HeaderValue::from_static(TILE_CC)),
                (
                    header::ACCESS_CONTROL_ALLOW_ORIGIN,
                    HeaderValue::from_static("*"),
                ),
            ],
            bytes,
        )
            .into_response(),
        Err(err) => match *err {
            TileError::Upstream(status) if status == StatusCode::NOT_FOUND => {
                respond::caddy_error(StatusCode::NOT_FOUND)
            }
            _ => respond::caddy_error(StatusCode::BAD_GATEWAY),
        },
    }
}

async fn load_tile(
    state: &AppState,
    time: &str,
    z: u8,
    x: u32,
    y: u32,
) -> Result<Bytes, TileError> {
    match split(z, x) {
        Split::East => fetch_tile(state, EAST_LAYER, time, z, x, y).await,
        Split::West => fetch_tile(state, WEST_LAYER, time, z, x, y).await,
        Split::Columns(start, end) => {
            let (east, west) = tokio::join!(
                fetch_tile(state, EAST_LAYER, time, z, x, y),
                fetch_tile(state, WEST_LAYER, time, z, x, y),
            );
            let (east, west) = (east?, west?);
            tokio::task::spawn_blocking(move || splice(&east, &west, start, end))
                .await
                .map_err(|_| TileError::BadGateway)?
        }
    }
}

async fn fetch_tile(
    state: &AppState,
    layer: &str,
    time: &str,
    z: u8,
    x: u32,
    y: u32,
) -> Result<Bytes, TileError> {
    // WMTS order: TileMatrix/TileRow/TileCol, so y before x.
    let url = format!(
        "{}/{layer}/default/{time}/{MATRIX_SET}/{z}/{y}/{x}.png",
        state.config.gibs_wmts_base
    );
    let fetched = upstream::get(&state.http, &url, &[], &[], TILE_TIMEOUT)
        .await
        .map_err(|err| {
            tracing::warn!("GIBS tile {url}: {err}");
            TileError::BadGateway
        })?;
    if !fetched.is_success() {
        return Err(TileError::Upstream(fetched.status));
    }
    Ok(fetched.body)
}

/// `west` with columns `start..end` replaced by `east`'s, as an RGBA PNG.
///
/// # Errors
/// [`TileError::BadGateway`] when either input is not a 256 px PNG.
pub fn splice(east: &[u8], west: &[u8], start: usize, end: usize) -> Result<Bytes, TileError> {
    let east = decode_rgba(east).ok_or(TileError::BadGateway)?;
    let mut out = decode_rgba(west).ok_or(TileError::BadGateway)?;
    let row = TILE_PX * 4;
    for r in 0..TILE_PX {
        let span = r * row + start * 4..r * row + end * 4;
        out[span.clone()].copy_from_slice(&east[span]);
    }
    encode_rgba(&out).ok_or(TileError::BadGateway)
}

/// A 256×256 PNG as RGBA8, whatever its color type and depth.
fn decode_rgba(bytes: &[u8]) -> Option<Vec<u8>> {
    let mut decoder = png::Decoder::new(std::io::Cursor::new(bytes));
    decoder.set_transformations(png::Transformations::EXPAND | png::Transformations::STRIP_16);
    let mut reader = decoder.read_info().ok()?;
    let mut buf = vec![0; reader.output_buffer_size()?];
    let info = reader.next_frame(&mut buf).ok()?;
    if info.width as usize != TILE_PX || info.height as usize != TILE_PX {
        return None;
    }
    let px = &buf[..info.buffer_size()];
    let rgba = match info.color_type {
        png::ColorType::Rgba => px.to_vec(),
        png::ColorType::Rgb => px
            .chunks_exact(3)
            .flat_map(|p| [p[0], p[1], p[2], 255])
            .collect(),
        png::ColorType::GrayscaleAlpha => px
            .chunks_exact(2)
            .flat_map(|p| [p[0], p[0], p[0], p[1]])
            .collect(),
        png::ColorType::Grayscale => px.iter().flat_map(|&g| [g, g, g, 255]).collect(),
        png::ColorType::Indexed => return None,
    };
    (rgba.len() == TILE_PX * TILE_PX * 4).then_some(rgba)
}

fn encode_rgba(rgba: &[u8]) -> Option<Bytes> {
    let mut out = Vec::with_capacity(rgba.len() / 2);
    let mut encoder = png::Encoder::new(&mut out, TILE_PX as u32, TILE_PX as u32);
    encoder.set_color(png::ColorType::Rgba);
    encoder.set_depth(png::BitDepth::Eight);
    encoder.set_compression(png::Compression::Fast);
    let mut writer = encoder.write_header().ok()?;
    writer.write_image_data(rgba).ok()?;
    writer.finish().ok()?;
    Some(Bytes::from(out))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn utc(s: &str) -> DateTime<Utc> {
        DateTime::parse_from_rfc3339(s).expect("time").to_utc()
    }

    #[test]
    fn time_segment_is_strict_and_on_the_ten_minute_grid() {
        assert_eq!(
            parse_time("2026-10-15T22:10:00Z"),
            Some(utc("2026-10-15T22:10:00Z"))
        );
        for bad in [
            "2026-10-15T22:13:00Z",
            "2026-10-15T22:10:30Z",
            "2026-10-15T22:10:00.000Z",
            "2026-10-15T22:10:00+00:00",
            "2026-10-15",
            "2026-10-15T22:10Z",
            "../../x",
            "2026-13-15T22:10:00Z",
        ] {
            assert_eq!(parse_time(bad), None, "{bad}");
        }
    }

    #[test]
    fn tile_parts_bound_the_grid() {
        assert_eq!(tile_parts("0", "0", "0.png"), Some((0, 0, 0)));
        assert_eq!(tile_parts("7", "127", "127.png"), Some((7, 127, 127)));
        assert_eq!(tile_parts("6", "16", "23.png"), Some((6, 16, 23)));
        for (z, x, y) in [
            ("8", "1", "1.png"),
            ("7", "128", "1.png"),
            ("3", "1", "8.png"),
            ("03", "1", "1.png"),
            ("3", "01", "1.png"),
            ("3", "1", "1.jpg"),
            ("3", "-1", "1.png"),
            ("", "1", "1.png"),
        ] {
            assert_eq!(tile_parts(z, x, y), None, "{z}/{x}/{y}");
        }
    }

    #[test]
    fn accepted_times_are_past_and_recent() {
        let now = utc("2026-10-15T23:00:00Z");
        assert!(time_accepted(utc("2026-10-15T22:10:00Z"), now));
        assert!(time_accepted(utc("2026-10-15T17:00:00Z"), now));
        assert!(!time_accepted(utc("2026-10-15T16:50:00Z"), now));
        assert!(!time_accepted(utc("2026-10-15T23:10:00Z"), now));
    }

    #[test]
    fn seam_tiles_split_on_the_right_column() {
        // z0: one tile; East spans 106.1° W to 73.9° E.
        assert_eq!(split(0, 0), Split::Columns(53, 181));
        // z3, x=1 spans 135° W to 90° W: West of the seam, then East.
        assert_eq!(split(3, 1), Split::Columns(164, 256));
        assert_eq!(split(3, 0), Split::West);
        assert_eq!(split(3, 2), Split::East);
        // Grand Rapids at z6 (x=16) is GOES-East; Los Angeles (x=10) is GOES-West.
        assert_eq!(split(6, 16), Split::East);
        assert_eq!(split(6, 10), Split::West);
        // The antipodal seam at 73.9° E.
        assert_eq!(split(3, 5), Split::Columns(0, 164));
        assert_eq!(split(3, 7), Split::West);
    }

    #[test]
    fn domain_lists_expand_and_skip_gaps() {
        let xml = "<Domains><DimensionDomain><ows:Identifier>time</ows:Identifier>\
            <Domain>2026-10-12T15:30:00Z/2026-10-12T15:50:00Z/PT10M,2026-10-12T16:10:00Z/2026-10-12T16:20:00Z/PT10M,2026-10-12T17:00:00Z</Domain>\
            <Size>3</Size></DimensionDomain></Domains>";
        let times: Vec<String> = parse_domain(xml).into_iter().map(format_time).collect();
        assert_eq!(
            times,
            [
                "2026-10-12T15:30:00Z",
                "2026-10-12T15:40:00Z",
                "2026-10-12T15:50:00Z",
                "2026-10-12T16:10:00Z",
                "2026-10-12T16:20:00Z",
                "2026-10-12T17:00:00Z",
            ]
        );
        assert!(parse_domain("<Domains/>").is_empty());
        assert!(
            parse_domain("<Domain>2026-10-12T15:30:00Z/2026-10-13T15:30:00Z/P1D</Domain>")
                .is_empty()
        );
    }

    #[test]
    fn manifest_layer_matches_the_layer_shape() {
        let layer = manifest_layer(&[utc("2026-10-15T22:00:00Z"), utc("2026-10-15T22:10:00Z")]);
        assert_eq!(layer["latest"], json!("2026-10-15T22:10:00+00:00"));
        assert_eq!(
            layer["timestamps"],
            json!(["2026-10-15T22:00:00+00:00", "2026-10-15T22:10:00+00:00"])
        );
        assert_eq!(layer["frames"][1]["path"], json!("2026-10-15T22:10:00Z"));
        assert_eq!(layer["frames"][1]["max_zoom"], json!(7));
        assert_eq!(layer["frames"][1]["kind"], json!("observation"));
    }

    fn solid_png(rgba: [u8; 4]) -> Vec<u8> {
        let px: Vec<u8> = std::iter::repeat_n(rgba, TILE_PX * TILE_PX)
            .flatten()
            .collect();
        encode_rgba(&px).expect("encode").to_vec()
    }

    #[test]
    fn splice_takes_east_columns_from_east() {
        let east = solid_png([255, 0, 0, 255]);
        let west = solid_png([0, 0, 255, 255]);
        let out = splice(&east, &west, 100, 256).expect("splice");
        let rgba = decode_rgba(&out).expect("decode");
        let at = |r: usize, c: usize| &rgba[(r * TILE_PX + c) * 4..(r * TILE_PX + c) * 4 + 4];
        assert_eq!(at(0, 99), [0, 0, 255, 255]);
        assert_eq!(at(0, 100), [255, 0, 0, 255]);
        assert_eq!(at(255, 255), [255, 0, 0, 255]);
        assert!(splice(b"not a png", &west, 0, 1).is_err());
    }
}
