//! Same-origin OTLP/HTTP relay for the phone app; the token is a revocable abuse filter, not identity.

use std::time::{Duration, Instant};

use axum::Router;
use axum::extract::{Request, State};
use axum::http::{HeaderMap, HeaderValue, StatusCode, header};
use axum::response::{IntoResponse, Response};
use axum::routing::post;
use bytes::{Bytes, BytesMut};
use serde_json::json;
use sha2::{Digest, Sha256};
use subtle::ConstantTimeEq;

use crate::respond;
use crate::state::AppState;

/// Largest request body forwarded upstream.
pub const MAX_BODY_BYTES: usize = 1024 * 1024;
/// Largest upstream response body passed back to the client.
const MAX_REPLY_BYTES: usize = 64 * 1024;
/// Upstream request timeout, reply body included.
const UPSTREAM_TIMEOUT: Duration = Duration::from_secs(5);
/// Deadline for reading the client's body, so stalled uploads free their buffer.
const BODY_TIMEOUT: Duration = Duration::from_secs(10);
/// Relays in flight at once (bounds buffered bodies to about 32 MiB).
pub const MAX_IN_FLIGHT: usize = 32;
const NO_STORE: &str = "no-store";

/// The two relay routes (both 404 while `OTLP_UPSTREAM` is unset).
pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/api/telemetry/v1/traces", post(traces))
        .route("/api/telemetry/v1/logs", post(logs))
}

async fn traces(State(state): State<AppState>, req: Request) -> Response {
    relay(&state, "traces", req).await
}

async fn logs(State(state): State<AppState>, req: Request) -> Response {
    relay(&state, "logs", req).await
}

fn error(status: StatusCode, code: &str) -> Response {
    respond::json(status, &json!({ "error": code }), Some(NO_STORE))
}

/// Constant-time token check; hashing first keeps the token length out of the timing too.
fn token_ok(headers: &HeaderMap, expected: &str) -> bool {
    let presented = headers
        .get(header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.strip_prefix("Bearer "))
        .map(str::trim);
    let Some(presented) = presented else {
        return false;
    };
    let a = Sha256::digest(presented.as_bytes());
    let b = Sha256::digest(expected.as_bytes());
    bool::from(a.ct_eq(&b))
}

fn otlp_content_type(headers: &HeaderMap) -> Option<HeaderValue> {
    let value = headers.get(header::CONTENT_TYPE)?;
    let essence = value.to_str().ok()?.split(';').next()?.trim();
    let ok = essence.eq_ignore_ascii_case("application/x-protobuf")
        || essence.eq_ignore_ascii_case("application/json");
    ok.then(|| value.clone())
}

/// `Ok(None)` for no encoding, `Ok(Some(gzip))`, `Err` for anything else.
fn otlp_content_encoding(headers: &HeaderMap) -> Result<Option<HeaderValue>, ()> {
    let Some(value) = headers.get(header::CONTENT_ENCODING) else {
        return Ok(None);
    };
    match value.to_str().map(str::trim) {
        Ok(v) if v.eq_ignore_ascii_case("identity") => Ok(None),
        Ok(v) if v.eq_ignore_ascii_case("gzip") => Ok(Some(value.clone())),
        _ => Err(()),
    }
}

async fn relay(state: &AppState, signal: &str, req: Request) -> Response {
    let upstream = &state.config.otlp_upstream;
    if upstream.is_empty() {
        return respond::detail(StatusCode::NOT_FOUND, "Not Found");
    }
    let client = crate::edge::client_key(state, &req);
    let admitted = state
        .telemetry_limiter
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .admit(&client, Instant::now(), false);
    if !admitted {
        let mut resp = error(StatusCode::TOO_MANY_REQUESTS, "rate_limited");
        resp.headers_mut()
            .insert(header::RETRY_AFTER, HeaderValue::from_static("5"));
        return resp;
    }
    let token = &state.config.telemetry_token;
    if !token.is_empty() && !token_ok(req.headers(), token) {
        return error(StatusCode::UNAUTHORIZED, "unauthorized");
    }
    let Some(content_type) = otlp_content_type(req.headers()) else {
        return error(
            StatusCode::UNSUPPORTED_MEDIA_TYPE,
            "otlp_content_type_required",
        );
    };
    let Ok(content_encoding) = otlp_content_encoding(req.headers()) else {
        return error(
            StatusCode::UNSUPPORTED_MEDIA_TYPE,
            "unsupported_content_encoding",
        );
    };
    let declared_too_big = req
        .headers()
        .get(header::CONTENT_LENGTH)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.parse::<u64>().ok())
        .is_some_and(|len| len > MAX_BODY_BYTES as u64);
    if declared_too_big {
        return error(StatusCode::PAYLOAD_TOO_LARGE, "body_too_large");
    }
    let Ok(_slot) = state.telemetry_slots.try_acquire() else {
        return error(StatusCode::SERVICE_UNAVAILABLE, "busy");
    };
    let body = match tokio::time::timeout(
        BODY_TIMEOUT,
        axum::body::to_bytes(req.into_body(), MAX_BODY_BYTES),
    )
    .await
    {
        Ok(Ok(body)) => body,
        Ok(Err(_)) => return error(StatusCode::PAYLOAD_TOO_LARGE, "body_too_large"),
        Err(_) => return error(StatusCode::REQUEST_TIMEOUT, "body_timeout"),
    };

    let mut out = state
        .telemetry_http
        .post(format!("{upstream}/v1/{signal}"))
        .timeout(UPSTREAM_TIMEOUT)
        .header(header::CONTENT_TYPE, content_type)
        .body(body);
    if let Some(encoding) = content_encoding {
        out = out.header(header::CONTENT_ENCODING, encoding);
    }
    let resp = match out.send().await {
        Ok(resp) => resp,
        Err(e) => {
            tracing::warn!(signal, "telemetry upstream failed: {e}");
            return error(StatusCode::BAD_GATEWAY, "upstream_unavailable");
        }
    };
    let status = StatusCode::from_u16(resp.status().as_u16()).unwrap_or(StatusCode::BAD_GATEWAY);
    let reply_type = resp.headers().get(header::CONTENT_TYPE).cloned();
    let Some(reply) = read_capped(resp).await else {
        return error(StatusCode::BAD_GATEWAY, "upstream_unavailable");
    };
    let mut headers = HeaderMap::new();
    if let Some(ct) = reply_type {
        headers.insert(header::CONTENT_TYPE, ct);
    }
    respond::insert_cc(&mut headers, NO_STORE);
    (status, headers, reply).into_response()
}

/// The upstream reply body, or `None` when it is too large or the read fails.
async fn read_capped(mut resp: reqwest::Response) -> Option<Bytes> {
    let mut buf = BytesMut::new();
    while let Some(chunk) = resp.chunk().await.ok()? {
        if buf.len() + chunk.len() > MAX_REPLY_BYTES {
            return None;
        }
        buf.extend_from_slice(&chunk);
    }
    Some(buf.freeze())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn auth(value: &str) -> HeaderMap {
        let mut h = HeaderMap::new();
        h.insert(header::AUTHORIZATION, HeaderValue::from_str(value).unwrap());
        h
    }

    #[test]
    fn token_must_match_exactly() {
        assert!(token_ok(&auth("Bearer s3cret"), "s3cret"));
        assert!(!token_ok(&auth("Bearer s3cret2"), "s3cret"));
        assert!(!token_ok(&auth("Basic s3cret"), "s3cret"));
        assert!(!token_ok(&HeaderMap::new(), "s3cret"));
    }

    #[test]
    fn only_otlp_content_types_and_encodings_pass() {
        let mut h = HeaderMap::new();
        assert!(otlp_content_type(&h).is_none());
        h.insert(header::CONTENT_TYPE, HeaderValue::from_static("text/plain"));
        assert!(otlp_content_type(&h).is_none());
        h.insert(
            header::CONTENT_TYPE,
            HeaderValue::from_static("application/json; charset=utf-8"),
        );
        assert!(otlp_content_type(&h).is_some());
        assert_eq!(otlp_content_encoding(&h), Ok(None));
        h.insert(header::CONTENT_ENCODING, HeaderValue::from_static("gzip"));
        assert!(matches!(otlp_content_encoding(&h), Ok(Some(_))));
        h.insert(header::CONTENT_ENCODING, HeaderValue::from_static("br"));
        assert_eq!(otlp_content_encoding(&h), Err(()));
    }
}
