//! Shared helpers for the integration tests: a scripted local HTTP server and a recording heartbeat.
#![allow(dead_code)]

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use axum::Router;
use axum::extract::State;
use axum::http::{StatusCode, Uri};
use axum::response::IntoResponse;
use radar_aux::Beat;
use serde_json::Value;

/// One scripted response, keyed by request path (query ignored).
#[derive(Clone)]
pub struct Route {
    pub status: u16,
    pub body: Vec<u8>,
    pub delay: Duration,
}

#[derive(Default)]
struct Inner {
    routes: Mutex<HashMap<String, Route>>,
    hits: Mutex<HashMap<String, usize>>,
}

/// A local HTTP server with scripted responses. Unknown paths answer 404.
#[derive(Clone)]
pub struct FakeHttp {
    pub base: String,
    inner: Arc<Inner>,
}

impl FakeHttp {
    /// Starts the server on an ephemeral 127.0.0.1 port.
    pub async fn start() -> Self {
        let inner = Arc::new(Inner::default());
        let app = Router::new().fallback(handle).with_state(inner.clone());
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind");
        let addr = listener.local_addr().expect("addr");
        tokio::spawn(async move {
            let _ = axum::serve(listener, app).await;
        });
        Self {
            base: format!("http://{addr}"),
            inner,
        }
    }

    /// Scripts a response for `path` (no query, leading slash).
    pub fn set(&self, path: &str, status: u16, body: impl Into<Vec<u8>>) {
        self.set_delayed(path, status, body, Duration::ZERO);
    }

    /// As [`FakeHttp::set`], but waits `delay` before answering.
    pub fn set_delayed(&self, path: &str, status: u16, body: impl Into<Vec<u8>>, delay: Duration) {
        self.inner.routes.lock().unwrap().insert(
            path.to_string(),
            Route {
                status,
                body: body.into(),
                delay,
            },
        );
    }

    /// Removes the scripted response for `path`, so it answers 404 again.
    pub fn clear(&self, path: &str) {
        self.inner.routes.lock().unwrap().remove(path);
    }

    /// Number of requests received for `path`.
    pub fn hits(&self, path: &str) -> usize {
        self.inner
            .hits
            .lock()
            .unwrap()
            .get(path)
            .copied()
            .unwrap_or(0)
    }
}

async fn handle(State(inner): State<Arc<Inner>>, uri: Uri) -> impl IntoResponse {
    let path = uri.path().to_string();
    *inner.hits.lock().unwrap().entry(path.clone()).or_default() += 1;
    let route = inner.routes.lock().unwrap().get(&path).cloned();
    match route {
        Some(route) => {
            if !route.delay.is_zero() {
                tokio::time::sleep(route.delay).await;
            }
            (StatusCode::from_u16(route.status).unwrap(), route.body).into_response()
        }
        None => (StatusCode::NOT_FOUND, Vec::<u8>::new()).into_response(),
    }
}

/// A reqwest client that ignores proxy variables (the test server is on 127.0.0.1).
pub fn client() -> reqwest::Client {
    reqwest::Client::builder()
        .no_proxy()
        .build()
        .expect("client")
}

/// A heartbeat that records every payload.
pub fn recording_beat() -> (Beat, Arc<Mutex<Vec<Value>>>) {
    let log: Arc<Mutex<Vec<Value>>> = Arc::new(Mutex::new(Vec::new()));
    let sink = log.clone();
    let beat: Beat = Arc::new(move |v: Value| sink.lock().unwrap().push(v));
    (beat, log)
}

/// Counts the files under `dir` whose name ends with `suffix` (recursive; 0 when missing).
pub fn count_files(dir: &std::path::Path, suffix: &str) -> usize {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return 0;
    };
    entries
        .flatten()
        .map(|e| {
            let p = e.path();
            if p.is_dir() {
                count_files(&p, suffix)
            } else if p.to_string_lossy().ends_with(suffix) {
                1
            } else {
                0
            }
        })
        .sum()
}

/// Reads a fixture from `testdata/`.
pub fn testdata(name: &str) -> Vec<u8> {
    std::fs::read(
        std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("testdata")
            .join(name),
    )
    .unwrap_or_else(|e| panic!("reading testdata/{name}: {e}"))
}
