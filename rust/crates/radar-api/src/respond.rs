//! Response builders that reproduce the Python error and body shapes.
//!
//! - `{"detail": "..."}` for `HTTPException` (FastAPI).
//! - `{"detail": [ {type, loc, msg, input} ]}` with status 422 for request validation.
//! - `{"error": "<code>"}` for the custom failures (forecast, alerts, geocode).
//! - Starlette JSON encoding: compact separators, UTF-8 (not ASCII-escaped), and non-finite floats
//!   written as `null` (defect D6 fix; `serde_json` already maps NaN to `null`).

use axum::http::{HeaderMap, HeaderValue, StatusCode, header};
use axum::response::{IntoResponse, Response};
use bytes::Bytes;
use serde_json::{Value, json};

/// Starlette's compact JSON bytes for a value (`json.dumps(..., separators=(",", ":"), ensure_ascii=False)`).
#[must_use]
pub fn encode(value: &Value) -> Bytes {
    // Serializing a `Value` cannot fail (keys are strings, NaN is already `null`).
    Bytes::from(serde_json::to_vec(value).unwrap_or_else(|_| b"null".to_vec()))
}

/// A `200`-style JSON response. `cache_control` adds the header when present.
#[must_use]
pub fn json(status: StatusCode, value: &Value, cache_control: Option<&str>) -> Response {
    json_bytes(status, encode(value), cache_control)
}

/// A JSON response from pre-encoded bytes (used for the manifest, which is encoded once per TTL).
#[must_use]
pub fn json_bytes(status: StatusCode, body: Bytes, cache_control: Option<&str>) -> Response {
    let mut headers = HeaderMap::new();
    headers.insert(
        header::CONTENT_TYPE,
        HeaderValue::from_static("application/json"),
    );
    if let Some(cc) = cache_control {
        insert_cc(&mut headers, cc);
    }
    (status, headers, body).into_response()
}

/// Set `Cache-Control` from a runtime string.
pub fn insert_cc(headers: &mut HeaderMap, cc: &str) {
    if let Ok(v) = HeaderValue::from_str(cc) {
        headers.insert(header::CACHE_CONTROL, v);
    }
}

/// `public, max-age=N`.
#[must_use]
pub fn public_max_age(seconds: u64) -> String {
    format!("public, max-age={seconds}")
}

/// `HTTPException(status, detail)`: `{"detail": msg}`.
#[must_use]
pub fn detail(status: StatusCode, msg: &str) -> Response {
    json(status, &json!({ "detail": msg }), None)
}

/// `JSONResponse({"error": code}, status, headers)`, with `no-store` when `no_store` is set.
#[must_use]
pub fn error_code(status: StatusCode, code: &str, no_store: bool) -> Response {
    json(
        status,
        &json!({ "error": code }),
        if no_store { Some("no-store") } else { None },
    )
}

/// The 502 `{"error": "upstream_unavailable"}` with `Cache-Control: no-store` (alerts, geocoding).
#[must_use]
pub fn upstream_unavailable_no_store() -> Response {
    error_code(StatusCode::BAD_GATEWAY, "upstream_unavailable", true)
}

/// The 502 `{"error": "upstream_unavailable"}` with no `Cache-Control` (forecast).
#[must_use]
pub fn upstream_unavailable() -> Response {
    error_code(StatusCode::BAD_GATEWAY, "upstream_unavailable", false)
}

/// FastAPI's 422 body: `{"detail": [issues...]}`.
#[must_use]
pub fn validation(issues: Vec<Value>) -> Response {
    json(
        StatusCode::UNPROCESSABLE_ENTITY,
        &json!({ "detail": issues }),
        None,
    )
}

/// Starlette's plain 500 (`Internal Server Error`, text/plain).
#[must_use]
pub fn internal_error() -> Response {
    (
        StatusCode::INTERNAL_SERVER_ERROR,
        [(header::CONTENT_TYPE, "text/plain; charset=utf-8")],
        "Internal Server Error",
    )
        .into_response()
}

/// Caddy's error override (spec 3.5): `Cache-Control: no-store`, text/plain body `"{code} {reason}"`.
#[must_use]
pub fn caddy_error(status: StatusCode) -> Response {
    let reason = status.canonical_reason().unwrap_or("");
    let body = format!("{} {}", status.as_u16(), reason);
    let mut headers = HeaderMap::new();
    headers.insert(
        header::CONTENT_TYPE,
        HeaderValue::from_static("text/plain; charset=utf-8"),
    );
    headers.insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
    (status, headers, body).into_response()
}

/// One FastAPI validation issue. `loc` is e.g. `["query", "lat"]`, `input` is the raw value.
#[must_use]
pub fn issue(kind: &str, loc: &[&str], msg: &str, input: Value) -> Value {
    json!({ "type": kind, "loc": loc, "msg": msg, "input": input })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn encode_is_compact_and_unescaped() {
        let v = json!({"a": 1, "b": "é"});
        assert_eq!(&encode(&v)[..], "{\"a\":1,\"b\":\"é\"}".as_bytes());
    }

    #[test]
    fn non_finite_floats_become_null() {
        let v = json!({"x": f64::NAN, "y": f64::INFINITY});
        assert_eq!(&encode(&v)[..], b"{\"x\":null,\"y\":null}");
    }

    #[test]
    fn caddy_error_text_matches_go_status_text() {
        let r = caddy_error(StatusCode::NOT_FOUND);
        assert_eq!(r.status(), StatusCode::NOT_FOUND);
        assert_eq!(r.headers()["cache-control"], "no-store");
    }
}
