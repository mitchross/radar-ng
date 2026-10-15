//! Basemap and static lanes that Caddy served before the port (spec 2.1 C1 to C6, 3.4, 3.5, 5.13).
//!
//! - `/tiles/*` radar tiles, from `TILE_DIR` (`ServeDir`, gzip precompressed, `immutable`). A tile
//!   missing from a complete pyramid is a cacheable empty 204, because empty tiles are never written.
//!   Packed buckets (`/tiles/packed/<layer>/<bucket>/...`) count as complete once their bucket file exists.
//! - `/basemap/styles/*`, `/basemap/fonts/*`: static files under `BASEMAP_ROOT`.
//! - `/basemap/imagery/{z}/{x}/{y}.jpg`: USGS imagery proxy with a moka cache (Caddy did not cache).
//! - `/basemap/tiles/{z}/{x}/{y}.mvt`: vector tiles read straight from the `.pmtiles` archive.
//!   This replaces the go-pmtiles sidecar.
//! - `/api/basemap/style/{name}`: style JSON with its origin-relative URLs made absolute.
//!
//! Every static-lane response that is an error (status 400 or higher) is rewritten by
//! [`static_error_override`], as Caddy's `handle_errors` did: `Cache-Control: no-store` and a text
//! body `"{code} {reason}"`. The imagery `respond 404` is not an error in Caddy, so it is marked
//! to skip the rewrite.

use std::convert::Infallible;
use std::future::Future;
use std::io;
use std::path::{Path as FsPath, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

use axum::Router;
use axum::body::Bytes;
use axum::extract::{Path, Request, State};
use axum::http::{HeaderMap, HeaderName, HeaderValue, StatusCode, header};
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};
use axum::routing::get;
use pmtiles::{AsyncBackend, AsyncPmTilesReader, BackendResponse, PmtError, PmtResult, TileCoord};
use serde_json::{Value, json};
use tower::{ServiceBuilder, ServiceExt};
use tower_http::services::ServeDir;
use tower_http::set_header::SetResponseHeaderLayer;

use crate::prefetch::request_base;
use crate::respond;
use crate::state::{AppState, CachedImagery};
use crate::upstream;

/// Marker for responses that Caddy does not treat as errors (the imagery `respond 404`).
#[derive(Debug, Clone, Copy)]
pub struct NoErrorOverride;

/// Upstream response headers dropped before the imagery response is sent (Caddy `header_down -X`).
const DROPPED_UPSTREAM_HEADERS: [&str; 4] = ["cache-control", "set-cookie", "x-cache", "via"];

/// Only these upstream headers are forwarded to the client (content metadata).
const FORWARDED_UPSTREAM_HEADERS: [&str; 3] = ["content-type", "etag", "last-modified"];

const IMAGERY_CC: &str = "public, max-age=604800";
const TILE_CC: &str = "public, max-age=86400, immutable";

/// Files at a pyramid's root that say every tile is in place: the `radar_tiler::marker` completion
/// markers, and the bucket file of a packed bucket (`radar_tiler::packed::BUCKET_FILE`).
const PYRAMID_COMPLETE_FILES: [&str; 3] = [
    ".render-complete-v2.json",
    ".render-complete.json",
    ".packed-bucket-v1.json",
];
const USGS_TILE_PATH: &str = "/arcgis/rest/services/USGSImageryOnly/MapServer/tile";

/// A 404 with an empty body and no headers (Caddy's `respond 404` in C5).
/// The imagery 404: no body and no `Cache-Control` (spec 5.13). Shared with the fallback.
pub(crate) fn bare_404() -> Response {
    let mut resp = StatusCode::NOT_FOUND.into_response();
    resp.extensions_mut().insert(NoErrorOverride);
    resp
}

/// Error override for the static lane (spec 3.5). Passes through responses marked [`NoErrorOverride`].
pub async fn static_error_override(req: Request, next: Next) -> Response {
    let resp = next.run(req).await;
    let status = resp.status();
    if (status.is_client_error() || status.is_server_error())
        && resp.extensions().get::<NoErrorOverride>().is_none()
    {
        return respond::caddy_error(status);
    }
    resp
}

/// An empty tile: 204 with the tile cache headers, so the CDN and browsers keep it like any tile.
fn empty_tile() -> Response {
    (
        StatusCode::NO_CONTENT,
        [
            (header::CACHE_CONTROL, TILE_CC),
            (header::ACCESS_CONTROL_ALLOW_ORIGIN, "*"),
        ],
    )
        .into_response()
}

/// The pyramid root of a `/{pyramid...}/{z}/{x}/{y}.png` tile path (relative to `TILE_DIR`, percent-
/// decoded), or `None` for any other shape or an unsafe segment.
fn pyramid_root(rel: &str) -> Option<PathBuf> {
    let decoded = percent_encoding::percent_decode_str(rel)
        .decode_utf8()
        .ok()?;
    let segments: Vec<&str> = decoded.trim_start_matches('/').split('/').collect();
    let [pyramid @ .., z, x, y] = segments.as_slice() else {
        return None;
    };
    let numeric = |s: &str| !s.is_empty() && s.bytes().all(|b| b.is_ascii_digit());
    let y = y.strip_suffix(".png")?;
    if pyramid.is_empty() || ![*z, *x, y].into_iter().all(numeric) {
        return None;
    }
    let unsafe_segment =
        |s: &&str| s.is_empty() || *s == "." || *s == ".." || s.contains(['\\', '\0']);
    if pyramid.iter().any(unsafe_segment) {
        return None;
    }
    Some(pyramid.iter().collect())
}

async fn in_complete_pyramid(root: &FsPath, rel: &str) -> bool {
    let Some(pyramid) = pyramid_root(rel) else {
        return false;
    };
    for marker in PYRAMID_COMPLETE_FILES {
        if tokio::fs::try_exists(root.join(&pyramid).join(marker))
            .await
            .unwrap_or(false)
        {
            return true;
        }
    }
    false
}

/// The static lane router: radar tiles, basemap styles and fonts, imagery, and vector tiles.
pub fn static_router(tile_dir: PathBuf, basemap_root: PathBuf) -> Router<AppState> {
    let with_headers = |cc: &'static str| {
        ServiceBuilder::new()
            .layer(SetResponseHeaderLayer::overriding(
                header::CACHE_CONTROL,
                HeaderValue::from_static(cc),
            ))
            .layer(SetResponseHeaderLayer::overriding(
                header::ACCESS_CONTROL_ALLOW_ORIGIN,
                HeaderValue::from_static("*"),
            ))
    };
    let serve_tiles = with_headers(TILE_CC).service(ServeDir::new(&tile_dir).precompressed_gzip());
    let tile_root = Arc::new(tile_dir);
    let tiles = tower::service_fn(move |req: Request| {
        let serve = serve_tiles.clone();
        let root = Arc::clone(&tile_root);
        async move {
            let rel = req.uri().path().to_owned();
            let resp = serve.oneshot(req).await?.into_response();
            if resp.status() == StatusCode::NOT_FOUND && in_complete_pyramid(&root, &rel).await {
                return Ok::<_, Infallible>(empty_tile());
            }
            Ok(resp)
        }
    });
    let styles = with_headers("public, max-age=3600")
        .service(ServeDir::new(basemap_root.join("styles")).precompressed_gzip());
    let fonts = with_headers("public, max-age=604800")
        .service(ServeDir::new(basemap_root.join("fonts")).precompressed_gzip());
    Router::new()
        .nest_service("/tiles", tiles)
        .nest_service("/basemap/styles", styles)
        .nest_service("/basemap/fonts", fonts)
        // Paths that do not fit this pattern fall through to `edge::unmatched`, which returns the
        // same bare 404 for the `/basemap/imagery/` prefix (axum rejects a catch-all here).
        .route("/basemap/imagery/{z}/{x}/{yjpg}", get(get_imagery))
        .route("/basemap/tiles/{z}/{x}/{ymvt}", get(get_vector_tile))
}

/// The Caddy imagery regexp `^/basemap/imagery/([0-9]|1[0-6])/([0-9]+)/([0-9]+)\.jpg$` on the
/// three captured parts. `z` is one digit or 10 to 16 (no leading zeros).
#[must_use]
pub fn imagery_parts(z: &str, x: &str, yjpg: &str) -> Option<(u8, String, String)> {
    let y = yjpg.strip_suffix(".jpg")?;
    let digits = |s: &str| !s.is_empty() && s.bytes().all(|b| b.is_ascii_digit());
    if !digits(z) || (z.len() > 1 && z.starts_with('0')) {
        return None;
    }
    let zoom: u8 = z.parse().ok().filter(|v| *v <= 16)?;
    if !digits(x) || !digits(y) {
        return None;
    }
    Some((zoom, x.to_string(), y.to_string()))
}

fn copy_upstream_headers(upstream_headers: &HeaderMap, out: &mut HeaderMap) {
    for name in FORWARDED_UPSTREAM_HEADERS {
        if DROPPED_UPSTREAM_HEADERS.contains(&name) {
            continue;
        }
        if let Some(value) = upstream_headers.get(name)
            && let Ok(header_name) = HeaderName::from_bytes(name.as_bytes())
        {
            out.insert(header_name, value.clone());
        }
    }
}

/// The upstream headers the imagery proxy keeps (`ETag`, `Last-Modified`, content type), as a
/// map that is stored with the cached tile.
fn forwarded_headers(upstream: &HeaderMap) -> HeaderMap {
    let mut out = HeaderMap::new();
    copy_upstream_headers(upstream, &mut out);
    out
}

/// The imagery 200: the tile bytes, the forwarded upstream headers, the fixed cache policy and CORS.
fn image_response(bytes: Bytes, forwarded: &HeaderMap) -> Response {
    let mut headers = HeaderMap::new();
    headers.insert(header::CONTENT_TYPE, HeaderValue::from_static("image/jpeg"));
    copy_upstream_headers(forwarded, &mut headers);
    headers.insert(header::CACHE_CONTROL, HeaderValue::from_static(IMAGERY_CC));
    headers.insert(
        header::ACCESS_CONTROL_ALLOW_ORIGIN,
        HeaderValue::from_static("*"),
    );
    (StatusCode::OK, headers, bytes).into_response()
}

/// `GET /basemap/imagery/{z}/{x}/{y}.jpg`: the USGS ArcGIS tile, rewritten to z/y/x order.
pub async fn get_imagery(
    State(state): State<AppState>,
    Path((z, x, yjpg)): Path<(String, String, String)>,
) -> Response {
    let Some((zoom, x, y)) = imagery_parts(&z, &x, &yjpg) else {
        return bare_404();
    };
    let cache_key = match (x.parse::<u64>(), y.parse::<u64>()) {
        (Ok(xn), Ok(yn)) => Some((zoom, xn, yn)),
        _ => None,
    };
    if let Some(key) = cache_key
        && let Some(hit) = state.imagery_cache.get(&key)
    {
        return image_response(hit.body, &hit.headers);
    }
    let url = format!(
        "{}{}/{zoom}/{y}/{x}",
        state.config.usgs_imagery_base, USGS_TILE_PATH
    );
    let fetched = match upstream::get(&state.http, &url, &[], &[], Duration::from_secs(30)).await {
        Ok(f) => f,
        Err(_) => return respond::caddy_error(StatusCode::BAD_GATEWAY),
    };
    if !fetched.is_success() {
        return respond::caddy_error(fetched.status);
    }
    let forwarded = forwarded_headers(&fetched.headers);
    if let Some(key) = cache_key {
        state.imagery_cache.insert(
            key,
            CachedImagery {
                body: fetched.body.clone(),
                headers: forwarded.clone(),
            },
        );
    }
    image_response(fetched.body, &forwarded)
}

/// A positioned-read backend for the basemap archive (F6).
///
/// The archive is never memory-mapped. A mapping of a file that is later rewritten or truncated in place
/// raises SIGBUS and kills the whole API process. Here a read past the current end of the file returns fewer
/// bytes or an I/O error, and the reader turns that into a 500 for that one request.
pub struct FileBackend {
    file: Arc<std::fs::File>,
    /// Length at open. Reads are clamped to it, so one open reader sees one consistent size.
    len: u64,
}

impl FileBackend {
    /// Open `path` for positioned reads.
    ///
    /// # Errors
    /// Fails when the file cannot be opened or stat'ed.
    pub fn open(path: &FsPath) -> io::Result<Self> {
        let file = std::fs::File::open(path)?;
        let len = file.metadata()?.len();
        Ok(Self {
            file: Arc::new(file),
            len,
        })
    }

    /// The identity of the open file (from the descriptor, not the path).
    fn identity(&self) -> io::Result<FileIdentity> {
        Ok(FileIdentity::of(&self.file.metadata()?))
    }
}

/// Read up to `length` bytes at `offset`, clamped to the snapshot length `len`. An offset past `len` is an error,
/// as in the mmap backend.
fn read_at(
    file: &std::fs::File,
    len: u64,
    offset: usize,
    length: usize,
) -> io::Result<bytes::Bytes> {
    use std::os::unix::fs::FileExt;
    let offset = offset as u64;
    if offset > len {
        return Err(io::Error::from(io::ErrorKind::UnexpectedEof));
    }
    let want = usize::try_from((length as u64).min(len - offset)).unwrap_or(usize::MAX);
    let mut buf = vec![0u8; want];
    let mut filled = 0usize;
    while filled < want {
        match file.read_at(&mut buf[filled..], offset + filled as u64) {
            Ok(0) => break,
            Ok(n) => filled += n,
            Err(err) if err.kind() == io::ErrorKind::Interrupted => {}
            Err(err) => return Err(err),
        }
    }
    buf.truncate(filled);
    Ok(bytes::Bytes::from(buf))
}

impl AsyncBackend for FileBackend {
    fn read(
        &self,
        offset: usize,
        length: usize,
    ) -> impl Future<Output = PmtResult<BackendResponse>> + Send {
        let file = Arc::clone(&self.file);
        let len = self.len;
        async move {
            // A blocking pool thread does the pread, so a slow disk does not park a runtime worker (F5).
            match tokio::task::spawn_blocking(move || read_at(&file, len, offset, length)).await {
                Ok(Ok(bytes)) => Ok(BackendResponse::new(bytes)),
                Ok(Err(err)) => Err(PmtError::Reading(err)),
                Err(join) => Err(PmtError::Reading(io::Error::other(join))),
            }
        }
    }
}

/// Inode, size and mtime of the archive file. A replacement by rename changes the inode. An in-place rewrite
/// changes the size or mtime.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileIdentity {
    dev: u64,
    ino: u64,
    len: u64,
    mtime: i64,
    mtime_nsec: i64,
}

impl FileIdentity {
    fn of(meta: &std::fs::Metadata) -> Self {
        use std::os::unix::fs::MetadataExt;
        Self {
            dev: meta.dev(),
            ino: meta.ino(),
            len: meta.len(),
            mtime: meta.mtime(),
            mtime_nsec: meta.mtime_nsec(),
        }
    }
}

/// The archive reader and the identity of the file it was opened from (F6).
pub struct OpenArchive {
    reader: Arc<AsyncPmTilesReader<FileBackend>>,
    identity: FileIdentity,
    /// When the identity was last checked.
    checked: Instant,
}

/// Open the archive from scratch: the file, its identity from the same descriptor, and the header and root
/// directory (blocking reads run on the blocking pool).
async fn open_fresh(path: PathBuf) -> Result<OpenArchive, String> {
    let opened = tokio::task::spawn_blocking(move || -> io::Result<(FileBackend, FileIdentity)> {
        let backend = FileBackend::open(&path)?;
        let identity = backend.identity()?;
        Ok((backend, identity))
    })
    .await
    .map_err(|e| e.to_string())?;
    let (backend, identity) = opened.map_err(|e| e.to_string())?;
    let reader = AsyncPmTilesReader::try_from_source(backend)
        .await
        .map_err(|e| e.to_string())?;
    Ok(OpenArchive {
        reader: Arc::new(reader),
        identity,
        checked: Instant::now(),
    })
}

/// The basemap archive reader. Opened on first use. Its file identity is checked at most every
/// `BASEMAP_PMTILES_RECHECK_S` seconds. A replaced or rewritten archive is reopened at that check, so a rename
/// over the file is picked up without a restart (F6). A failed reopen keeps the reader that is already open.
async fn open_archive(state: &AppState) -> Option<Arc<AsyncPmTilesReader<FileBackend>>> {
    let mut slot = state.pmtiles.lock().await;
    let path = state.config.pmtiles_path.clone();
    let recheck = Duration::from_secs(state.config.pmtiles_recheck_s);
    if let Some(open) = slot.as_mut() {
        if open.checked.elapsed() < recheck {
            return Some(open.reader.clone());
        }
        let probe = path.clone();
        let now = tokio::task::spawn_blocking(move || {
            std::fs::metadata(probe).ok().map(|m| FileIdentity::of(&m))
        })
        .await
        .ok()
        .flatten();
        if now.as_ref() == Some(&open.identity) {
            open.checked = Instant::now();
            return Some(open.reader.clone());
        }
        tracing::info!(path = %path.display(), "basemap archive changed on disk; reopening");
    }
    match open_fresh(path.clone()).await {
        Ok(fresh) => {
            let reader = fresh.reader.clone();
            *slot = Some(fresh);
            Some(reader)
        }
        Err(err) => {
            tracing::error!(path = %path.display(), "basemap archive unavailable: {err}");
            // Keep serving the reader that is open. Its file descriptor still reads the old bytes.
            slot.as_mut().map(|open| {
                open.checked = Instant::now();
                open.reader.clone()
            })
        }
    }
}

/// `GET /basemap/tiles/{z}/{x}/{y}.mvt`: 200 with the stored bytes, 204 for a missing tile.
pub async fn get_vector_tile(
    State(state): State<AppState>,
    Path((z, x, ymvt)): Path<(String, String, String)>,
) -> Response {
    let Some(y) = ymvt.strip_suffix(".mvt") else {
        return respond::caddy_error(StatusCode::NOT_FOUND);
    };
    let (Ok(zoom), Ok(xn), Ok(yn)) = (z.parse::<u8>(), x.parse::<u32>(), y.parse::<u32>()) else {
        return respond::caddy_error(StatusCode::NOT_FOUND);
    };
    let Ok(coord) = TileCoord::new(zoom, xn, yn) else {
        return respond::caddy_error(StatusCode::NOT_FOUND);
    };
    let Some(reader) = open_archive(&state).await else {
        if !state.config.basemap_tiles_upstream.is_empty() {
            return proxy_vector_tile(&state, zoom, xn, yn).await;
        }
        return respond::caddy_error(StatusCode::SERVICE_UNAVAILABLE);
    };
    let encoding = reader.get_header().tile_compression.content_encoding();
    match reader.get_tile(coord).await {
        Ok(Some(data)) => {
            let mut headers = HeaderMap::new();
            headers.insert(
                header::CONTENT_TYPE,
                HeaderValue::from_static("application/x-protobuf"),
            );
            if let Some(enc) = encoding {
                headers.insert(header::CONTENT_ENCODING, HeaderValue::from_static(enc));
            }
            headers.insert(
                header::CACHE_CONTROL,
                HeaderValue::from_static("public, max-age=86400"),
            );
            headers.insert(
                header::ACCESS_CONTROL_ALLOW_ORIGIN,
                HeaderValue::from_static("*"),
            );
            (StatusCode::OK, headers, data).into_response()
        }
        Ok(None) => {
            let mut headers = HeaderMap::new();
            headers.insert(
                header::CACHE_CONTROL,
                HeaderValue::from_static("public, max-age=86400"),
            );
            headers.insert(
                header::ACCESS_CONTROL_ALLOW_ORIGIN,
                HeaderValue::from_static("*"),
            );
            (StatusCode::NO_CONTENT, headers).into_response()
        }
        Err(err) => {
            tracing::error!("basemap tile read failed: {err}");
            respond::caddy_error(StatusCode::INTERNAL_SERVER_ERROR)
        }
    }
}

/// Proxy one vector tile to the go-pmtiles server, as Caddy did:
/// `/basemap/tiles/{z}/{x}/{y}.mvt` becomes `{upstream}/basemap/{z}/{x}/{y}.mvt` (archive `basemap`).
/// Status, body and the content headers pass through; cache and CORS headers are set like Caddy's.
async fn proxy_vector_tile(state: &AppState, z: u8, x: u32, y: u32) -> Response {
    let url = format!(
        "{}/basemap/{z}/{x}/{y}.mvt",
        state.config.basemap_tiles_upstream
    );
    // Tiles are stored gzip-encoded; pass the bytes through untouched instead of letting the shared
    // client decompress them (it advertises and decodes gzip).
    static PASSTHROUGH: std::sync::OnceLock<reqwest::Client> = std::sync::OnceLock::new();
    let client = PASSTHROUGH.get_or_init(|| {
        reqwest::Client::builder()
            .no_gzip()
            .build()
            .unwrap_or_else(|_| state.http.clone())
    });
    let fetched = match upstream::get(client, &url, &[], &[], Duration::from_secs(15)).await {
        Ok(f) => f,
        Err(err) => {
            tracing::warn!("basemap tile proxy to {url} failed: {err}");
            return respond::caddy_error(StatusCode::BAD_GATEWAY);
        }
    };
    if fetched.status.is_client_error() || fetched.status.is_server_error() {
        return respond::caddy_error(fetched.status);
    }
    let mut headers = HeaderMap::new();
    for name in [
        header::CONTENT_TYPE,
        header::CONTENT_ENCODING,
        header::ETAG,
        header::LAST_MODIFIED,
    ] {
        if let Some(v) = fetched.headers.get(&name) {
            headers.insert(name, v.clone());
        }
    }
    headers.insert(
        header::CACHE_CONTROL,
        HeaderValue::from_static("public, max-age=86400"),
    );
    headers.insert(
        header::ACCESS_CONTROL_ALLOW_ORIGIN,
        HeaderValue::from_static("*"),
    );
    (fetched.status, headers, fetched.body).into_response()
}

/// The origin for relative URLs in a style: `X-Forwarded-Proto` (or `http`) and `X-Forwarded-Host`
/// (or `Host`).
fn style_origin(headers: &HeaderMap) -> String {
    let base = request_base(headers);
    let host = headers
        .get("x-forwarded-host")
        .and_then(|v| v.to_str().ok())
        .filter(|h| !h.is_empty());
    match host {
        Some(host) => {
            let proto = base.split("://").next().unwrap_or("http");
            format!("{proto}://{host}")
        }
        None => base,
    }
}

/// Rewrite origin-relative `sources.*.tiles[i]`, `glyphs` and `sprite` to absolute URLs.
pub fn rewrite_origin(style: &mut Value, origin: &str) {
    if let Some(sources) = style.get_mut("sources").and_then(Value::as_object_mut) {
        for source in sources.values_mut() {
            let Some(tiles) = source.get_mut("tiles").and_then(Value::as_array_mut) else {
                continue;
            };
            for tile in tiles.iter_mut() {
                let rewritten = tile
                    .as_str()
                    .filter(|s| s.starts_with('/'))
                    .map(|s| format!("{origin}{s}"));
                if let Some(url) = rewritten {
                    *tile = json!(url);
                }
            }
        }
    }
    for key in ["glyphs", "sprite"] {
        let rewritten = style
            .get(key)
            .and_then(Value::as_str)
            .filter(|s| s.starts_with('/'))
            .map(|s| format!("{origin}{s}"));
        if let Some(url) = rewritten {
            style[key] = json!(url);
        }
    }
}

/// `GET /api/basemap/style/{name}` (spec 5.13). No `Cache-Control` and no CORS header.
pub async fn get_basemap_style(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(name): Path<String>,
) -> Response {
    // Starlette decodes %2F before routing, so an encoded slash never reaches this handler.
    if name.contains('/') {
        return respond::detail(StatusCode::NOT_FOUND, "Not Found");
    }
    let alnum = !name.is_empty() && name.chars().all(char::is_alphanumeric);
    if !alnum
        && name != "positron"
        && name != "dark-matter"
        && (name.contains("..") || name.contains('\\'))
    {
        return respond::detail(StatusCode::BAD_REQUEST, "invalid style name");
    }
    let path: PathBuf = state.config.style_dir.join(format!("{name}.json"));
    let exists_path = path.clone();
    let exists = tokio::task::spawn_blocking(move || exists_path.is_file())
        .await
        .unwrap_or(false);
    if !exists {
        return respond::detail(StatusCode::NOT_FOUND, "style not found");
    }
    let read = tokio::task::spawn_blocking(move || read_style(&path)).await;
    let mut style = match read {
        Ok(Ok(style)) => style,
        _ => return respond::internal_error(),
    };
    rewrite_origin(&mut style, &style_origin(&headers));
    respond::json(StatusCode::OK, &style, None)
}

fn read_style(path: &FsPath) -> Result<Value, ()> {
    let text = std::fs::read_to_string(path).map_err(|_| ())?;
    serde_json::from_str(&text).map_err(|_| ())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn imagery_regexp_forms() {
        assert_eq!(
            imagery_parts("0", "1", "2.jpg"),
            Some((0, "1".into(), "2".into()))
        );
        assert_eq!(imagery_parts("16", "65535", "7.jpg").map(|p| p.0), Some(16));
        assert_eq!(imagery_parts("17", "1", "2.jpg"), None);
        assert_eq!(imagery_parts("01", "1", "2.jpg"), None);
        assert_eq!(imagery_parts("3", "x", "2.jpg"), None);
        assert_eq!(imagery_parts("3", "1", "2.png"), None);
    }

    #[test]
    fn style_origin_rewrites_only_relative_urls() {
        let mut style = json!({
            "sources": {"base": {"type": "vector", "tiles": ["/basemap/tiles/{z}/{x}/{y}.mvt", "https://other/x"]}},
            "glyphs": "/basemap/fonts/{fontstack}/{range}.pbf",
            "sprite": "https://cdn.example/sprite",
        });
        rewrite_origin(&mut style, "https://radar.example");
        assert_eq!(
            style["sources"]["base"]["tiles"][0],
            json!("https://radar.example/basemap/tiles/{z}/{x}/{y}.mvt")
        );
        assert_eq!(
            style["sources"]["base"]["tiles"][1],
            json!("https://other/x")
        );
        assert_eq!(
            style["glyphs"],
            json!("https://radar.example/basemap/fonts/{fontstack}/{range}.pbf")
        );
        assert_eq!(style["sprite"], json!("https://cdn.example/sprite"));
    }

    #[tokio::test]
    async fn file_backend_clamps_reads_to_the_snapshot_length() {
        use pmtiles::AsyncBackend;
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("a.bin");
        std::fs::write(&path, b"0123456789").expect("write");
        let backend = FileBackend::open(&path).expect("open");
        assert_eq!(
            backend.read(0, 4).await.expect("read").bytes.as_ref(),
            b"0123"
        );
        assert_eq!(
            backend
                .read(8, 10)
                .await
                .expect("short read")
                .bytes
                .as_ref(),
            b"89"
        );
        assert!(backend.read(10, 1).await.expect("at end").bytes.is_empty());
        assert!(
            backend.read(11, 1).await.is_err(),
            "past the end is an error, as with mmap"
        );
        // Growth after open is not visible: the snapshot length bounds every read.
        std::fs::write(&path, b"0123456789abcdef").expect("grow");
        assert_eq!(backend.read(10, 6).await.expect("read").bytes.len(), 0);
    }

    #[test]
    fn upstream_cache_headers_are_dropped() {
        let mut up = HeaderMap::new();
        up.insert(header::CACHE_CONTROL, HeaderValue::from_static("max-age=5"));
        up.insert(header::CONTENT_TYPE, HeaderValue::from_static("image/jpeg"));
        let mut out = HeaderMap::new();
        copy_upstream_headers(&up, &mut out);
        assert!(out.get(header::CACHE_CONTROL).is_none());
        assert_eq!(out[header::CONTENT_TYPE], "image/jpeg");
    }
}
