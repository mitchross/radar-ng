//! Test helpers: a fake S3 bucket (ListObjectsV2, HEAD, GET, ranged GET) served over HTTP on localhost,
//! and fixture paths. Each test starts its own server, so tests run in parallel.

#![allow(dead_code)]

use std::collections::HashMap;
use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use axum::body::Body;
use axum::extract::{Request, State};
use axum::http::{Method, StatusCode, header};
use axum::response::Response;

/// MRMS key for the fixture, with a valid time of 2026-09-30T00:00:00Z.
pub const MRMS_PREFIX: &str = "CONUS/MergedBaseReflectivityQC_00.50";
/// MRMS object key of the fixture.
pub const MRMS_KEY: &str = "CONUS/MergedBaseReflectivityQC_00.50/20260930/MRMS_MergedBaseReflectivityQC_00.50_20260930-000000.grib2.gz";
/// Valid time of the MRMS fixture.
pub const MRMS_VALID: &str = "2026-09-30T00:00:00+00:00";

/// Mutable bucket state, shared with the server task.
#[derive(Default)]
pub struct Bucket {
    /// Objects by key.
    pub objects: HashMap<String, Vec<u8>>,
    /// Forced status codes by key (applies to GET and HEAD).
    pub overrides: HashMap<String, u16>,
    /// Keys returned by listings whose prefix matches (product directory match, any date).
    pub list_keys: Vec<String>,
    /// Answer 200 to Range requests with the full body (a server that ignores Range).
    pub ignore_range: bool,
    /// Answer 200 to every HEAD request (finder tests).
    pub head_all: bool,
    /// Every request as `METHOD path`, in arrival order.
    pub requests: Vec<String>,
}

/// A running fake bucket.
pub struct FakeS3 {
    /// Base URL, e.g. `http://127.0.0.1:41234`.
    pub base: String,
    /// Shared state to set up and inspect the bucket.
    pub bucket: Arc<Mutex<Bucket>>,
}

/// Start the server on an ephemeral port.
pub async fn start() -> FakeS3 {
    let bucket = Arc::new(Mutex::new(Bucket::default()));
    let app = axum::Router::new()
        .fallback(handle)
        .with_state(bucket.clone());
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr: SocketAddr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    FakeS3 {
        base: format!("http://{addr}"),
        bucket,
    }
}

impl FakeS3 {
    /// Add an object.
    pub fn put(&self, key: &str, body: Vec<u8>) {
        self.bucket
            .lock()
            .unwrap()
            .objects
            .insert(key.to_string(), body);
    }

    /// Requests seen so far.
    pub fn requests(&self) -> Vec<String> {
        self.bucket.lock().unwrap().requests.clone()
    }
}

fn percent_decode(s: &str) -> String {
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        match bytes[i] {
            b'+' => {
                out.push(b' ');
                i += 1;
            }
            b'%' if i + 2 < bytes.len() => {
                let hex = std::str::from_utf8(&bytes[i + 1..i + 3]).unwrap_or("00");
                out.push(u8::from_str_radix(hex, 16).unwrap_or(b'?'));
                i += 3;
            }
            b => {
                out.push(b);
                i += 1;
            }
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

fn query_param(query: &str, name: &str) -> Option<String> {
    query.split('&').find_map(|pair| {
        let (k, v) = pair.split_once('=')?;
        (k == name).then(|| percent_decode(v))
    })
}

fn product_dir(prefix: &str) -> String {
    // "CONUS/Product/20260930" -> "CONUS/Product"
    match prefix.rsplit_once('/') {
        Some((head, tail)) if tail.chars().all(|c| c.is_ascii_digit()) => head.to_string(),
        _ => prefix.to_string(),
    }
}

fn list_xml(keys: &[String]) -> String {
    let mut s = String::from(
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<ListBucketResult xmlns=\"http://s3.amazonaws.com/doc/2006-03-01/\">\
         <IsTruncated>false</IsTruncated>",
    );
    for k in keys {
        s.push_str(&format!("<Contents><Key>{k}</Key></Contents>"));
    }
    s.push_str("</ListBucketResult>");
    s
}

fn plain(status: u16, body: Vec<u8>) -> Response {
    Response::builder()
        .status(status)
        .body(Body::from(body))
        .unwrap()
}

async fn handle(State(bucket): State<Arc<Mutex<Bucket>>>, req: Request) -> Response {
    let method = req.method().clone();
    let uri = req.uri().clone();
    let range = req
        .headers()
        .get(header::RANGE)
        .and_then(|v| v.to_str().ok())
        .map(str::to_string);
    let path = percent_decode(uri.path());
    let key = path.trim_start_matches('/').to_string();
    let mut b = bucket.lock().unwrap();
    b.requests.push(format!("{method} {path}"));

    if method == Method::GET && key.is_empty() {
        let prefix = query_param(uri.query().unwrap_or(""), "prefix").unwrap_or_default();
        let dir = product_dir(&prefix);
        let keys: Vec<String> = b
            .list_keys
            .iter()
            .filter(|k| k.starts_with(&dir))
            .cloned()
            .collect();
        return Response::builder()
            .status(200)
            .header(header::CONTENT_TYPE, "application/xml")
            .body(Body::from(list_xml(&keys)))
            .unwrap();
    }

    if let Some(code) = b.overrides.get(&key).copied() {
        return plain(code, Vec::new());
    }
    if method == Method::HEAD {
        let exists = b.head_all || b.objects.contains_key(&key);
        return plain(if exists { 200 } else { 404 }, Vec::new());
    }
    let Some(body) = b.objects.get(&key).cloned() else {
        return plain(404, Vec::new());
    };
    if let (Some(spec), false) = (range, b.ignore_range) {
        let spec = spec.trim_start_matches("bytes=");
        let (start, end) = spec.split_once('-').unwrap();
        let start: usize = start.parse().unwrap();
        let end: usize = if end.is_empty() {
            body.len() - 1
        } else {
            end.parse().unwrap()
        };
        let slice = body[start..=end.min(body.len() - 1)].to_vec();
        return Response::builder()
            .status(StatusCode::PARTIAL_CONTENT)
            .header(
                header::CONTENT_RANGE,
                format!("bytes {start}-{}/{}", start + slice.len() - 1, body.len()),
            )
            .body(Body::from(slice))
            .unwrap();
    }
    plain(200, body)
}

/// Path of a crate-owned fixture.
pub fn fixture(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("testdata")
        .join(name)
}

/// The MRMS gzipped fixture bytes.
pub fn mrms_gz() -> Vec<u8> {
    std::fs::read(fixture("mrms_regular_ll_png.grib2.gz")).unwrap()
}

/// The HRRR mini fixture bytes.
pub fn hrrr_grib() -> Vec<u8> {
    std::fs::read(fixture("hrrr_lambert_mini.grib2")).unwrap()
}

/// The HRRR mini fixture index.
pub fn hrrr_idx() -> Vec<u8> {
    std::fs::read(fixture("hrrr_lambert_mini.grib2.idx")).unwrap()
}
