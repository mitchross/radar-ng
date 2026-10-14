//! One helper for outbound GETs: a timeout, optional headers, the status, and the body bytes.
//!
//! Callers decide the failure mapping (502 `upstream_unavailable`, a soft `available:false`, a
//! `null` place, and so on). Python does the same through `httpx` plus `raise_for_status()`.

use std::time::Duration;

use bytes::Bytes;
use reqwest::StatusCode;
use reqwest::header::HeaderMap;
use serde_json::Value;

/// The status and body of a completed upstream exchange.
#[derive(Debug)]
pub struct Fetched {
    /// HTTP status from upstream.
    pub status: StatusCode,
    /// Raw body.
    pub body: Bytes,
    /// Response headers from upstream (used where a caller forwards some of them).
    pub headers: HeaderMap,
}

impl Fetched {
    /// True for 2xx.
    #[must_use]
    pub fn is_success(&self) -> bool {
        self.status.is_success()
    }

    /// The body parsed as JSON. `None` when it is not JSON (Python's `resp.json()` raising).
    #[must_use]
    pub fn json(&self) -> Option<Value> {
        serde_json::from_slice(&self.body).ok()
    }
}

/// GET `url` with `query` (in order) and extra `headers`, under `timeout`.
///
/// # Errors
/// Returns the transport error (connect, timeout, body read). A non-2xx status is **not** an error
/// here; check [`Fetched::is_success`].
pub async fn get(
    http: &reqwest::Client,
    url: &str,
    query: &[(&str, String)],
    headers: &[(&str, &str)],
    timeout: Duration,
) -> Result<Fetched, reqwest::Error> {
    let mut req = http.get(url).query(query).timeout(timeout);
    for (name, value) in headers {
        req = req.header(*name, *value);
    }
    let resp = req.send().await?;
    let status = resp.status();
    let headers = resp.headers().clone();
    let body = resp.bytes().await?;
    Ok(Fetched {
        status,
        body,
        headers,
    })
}
