//! One helper for outbound GETs: a timeout, optional headers, the status, and the body bytes.
//!
//! Callers decide the failure mapping (502 `upstream_unavailable`, a soft `available:false`, a
//! `null` place, and so on).

use std::fmt;
use std::time::Duration;

use bytes::{Bytes, BytesMut};
use reqwest::StatusCode;
use reqwest::header::HeaderMap;
use serde_json::Value;

/// Upper bound on any single upstream response body read by [`get`] (16 MiB).
pub const MAX_BODY_BYTES: usize = 16 * 1024 * 1024;

/// Failure returned by [`get`].
#[derive(Debug)]
pub enum UpstreamError {
    /// Transport, timeout, or body-stream failure from `reqwest`.
    Request(reqwest::Error),
    /// Upstream response exceeded [`MAX_BODY_BYTES`].
    TooLarge { limit: usize },
}

impl fmt::Display for UpstreamError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Request(err) => err.fmt(f),
            Self::TooLarge { limit } => {
                write!(f, "upstream response exceeded {limit} byte limit")
            }
        }
    }
}

impl std::error::Error for UpstreamError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Request(err) => Some(err),
            Self::TooLarge { .. } => None,
        }
    }
}

impl From<reqwest::Error> for UpstreamError {
    fn from(err: reqwest::Error) -> Self {
        Self::Request(err)
    }
}

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

    /// The body parsed as JSON. `None` when it is not JSON (a body that fails to parse).
    #[must_use]
    pub fn json(&self) -> Option<Value> {
        serde_json::from_slice(&self.body).ok()
    }
}

/// GET `url` with `query` (in order) and extra `headers`, under `timeout`, capped at [`MAX_BODY_BYTES`].
///
/// # Errors
/// Returns the transport error (connect, timeout, body read) or [`UpstreamError::TooLarge`].
/// A non-2xx status is **not** an error here; check [`Fetched::is_success`].
pub async fn get(
    http: &reqwest::Client,
    url: &str,
    query: &[(&str, String)],
    headers: &[(&str, &str)],
    timeout: Duration,
) -> Result<Fetched, UpstreamError> {
    get_limited(http, url, query, headers, timeout, MAX_BODY_BYTES).await
}

async fn get_limited(
    http: &reqwest::Client,
    url: &str,
    query: &[(&str, String)],
    headers: &[(&str, &str)],
    timeout: Duration,
    max_bytes: usize,
) -> Result<Fetched, UpstreamError> {
    let mut req = http.get(url).query(query).timeout(timeout);
    for (name, value) in headers {
        req = req.header(*name, *value);
    }
    let mut resp = req.send().await?;
    let status = resp.status();
    let headers = resp.headers().clone();
    if resp
        .content_length()
        .is_some_and(|len| len > max_bytes as u64)
    {
        return Err(UpstreamError::TooLarge { limit: max_bytes });
    }
    let mut buf = BytesMut::new();
    while let Some(chunk) = resp.chunk().await? {
        if buf.len().saturating_add(chunk.len()) > max_bytes {
            return Err(UpstreamError::TooLarge { limit: max_bytes });
        }
        buf.extend_from_slice(&chunk);
    }
    Ok(Fetched {
        status,
        body: buf.freeze(),
        headers,
    })
}
