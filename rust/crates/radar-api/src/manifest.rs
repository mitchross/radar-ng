//! `GET /api/manifest.json` and the manifest snapshot shared by nowcast, health, metrics and storm
//! prefetch (spec 5.1).
//!
//! - The file is read with `radar_core::manifest::read_manifest` on the blocking pool.
//! - A read failure serves the last good copy (no TTL), or `503 {"detail":"manifest unavailable"}`
//!   when there is none. Defect D5 is handled on the reader side: a malformed file is an error, so
//!   it is never silently treated as an empty manifest.
//! - The snapshot is re-read at most every 15 s. The 200 body is the Starlette-compact encoding of
//!   the normalized manifest, and its ETag is `"<blake2b-96 hex>"` over those exact bytes.
//! - `If-None-Match` with a matching candidate (after trimming, with a leading `W/` removed, or `*`)
//!   answers 304 with an empty body, `ETag` and `Cache-Control`.

use std::sync::Arc;
use std::time::{Duration, Instant};

use axum::extract::State;
use axum::http::{HeaderMap, HeaderValue, StatusCode, header};
use axum::response::{IntoResponse, Response};
use blake2::Blake2b;
use blake2::digest::Digest;
use blake2::digest::consts::U12;
use bytes::Bytes;
use serde_json::Value;

use crate::respond;
use crate::state::{AppState, ManifestSnapshot};

/// TTL of the manifest snapshot (`_MANIFEST_TTL_S`).
pub const MANIFEST_TTL: Duration = Duration::from_secs(15);

/// `'"' + blake2b(encoded, digest_size=12).hexdigest() + '"'`.
#[must_use]
pub fn etag_of(encoded: &[u8]) -> String {
    let digest = Blake2b::<U12>::digest(encoded);
    format!("\"{}\"", hex::encode(digest))
}

/// True when `If-None-Match` matches `etag` (spec 5.1, the 304 rule).
#[must_use]
pub fn if_none_match_matches(header_value: Option<&str>, etag: &str) -> bool {
    let Some(raw) = header_value else {
        return false;
    };
    raw.split(',').map(str::trim).any(|candidate| {
        candidate == "*" || candidate.strip_prefix("W/").unwrap_or(candidate) == etag
    })
}

/// Read the manifest from disk now and remember it as the last good copy on success.
///
/// # Errors
/// Returns the 503 response when the read fails and no last-good copy exists.
pub async fn build_manifest(state: &AppState) -> Result<Arc<Value>, Response> {
    let dir = state.config.state_dir.clone();
    let read = tokio::task::spawn_blocking(move || {
        radar_core::manifest::read_manifest(&dir).map(|m| serde_json::to_value(&m))
    })
    .await;
    match read {
        Ok(Ok(Ok(value))) => {
            let value = Arc::new(value);
            *lock(&state.manifest_last_good) = Some(value.clone());
            Ok(value)
        }
        Ok(Ok(Err(err))) => fallback_or_503(state, &format!("manifest read failed: {err}")),
        Ok(Err(err)) => fallback_or_503(state, &format!("manifest read failed: {err}")),
        Err(join) => fallback_or_503(state, &format!("manifest read task failed: {join}")),
    }
}

#[allow(
    clippy::result_large_err,
    reason = "the Err is the ready-made HTTP response; boxing it would add a deref at about 50 call sites for no gain on this cold path"
)]
fn fallback_or_503(state: &AppState, reason: &str) -> Result<Arc<Value>, Response> {
    if let Some(last) = lock(&state.manifest_last_good).clone() {
        tracing::warn!("{reason}; serving last good copy");
        return Ok(last);
    }
    tracing::warn!("{reason}; no last good copy");
    Err(respond::detail(
        StatusCode::SERVICE_UNAVAILABLE,
        "manifest unavailable",
    ))
}

/// The TTL-cached snapshot (value, bytes, ETag). Re-reads at most once per [`MANIFEST_TTL`].
///
/// # Errors
/// Returns the 503 response when no manifest can be read.
pub async fn cached_manifest(state: &AppState) -> Result<Arc<ManifestSnapshot>, Response> {
    if let Some((expires, snap)) = lock(&state.manifest).clone()
        && Instant::now() < expires
    {
        return Ok(snap);
    }
    let value = build_manifest(state).await?;
    let encoded = respond::encode(&value);
    let etag = etag_of(&encoded);
    let snap = Arc::new(ManifestSnapshot {
        value,
        encoded,
        etag,
    });
    *lock(&state.manifest) = Some((Instant::now() + MANIFEST_TTL, snap.clone()));
    Ok(snap)
}

/// `GET /api/manifest.json`.
pub async fn get_manifest(State(state): State<AppState>, headers: HeaderMap) -> Response {
    state
        .metrics
        .counters
        .manifest_requests_total
        .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let snap = match cached_manifest(&state).await {
        Ok(s) => s,
        Err(resp) => return resp,
    };
    let inm = headers
        .get(header::IF_NONE_MATCH)
        .and_then(|v| v.to_str().ok());
    let cc = respond::public_max_age(15);
    if if_none_match_matches(inm, &snap.etag) {
        let mut h = HeaderMap::new();
        h.insert(
            header::ETAG,
            HeaderValue::from_str(&snap.etag).unwrap_or(HeaderValue::from_static("\"\"")),
        );
        respond::insert_cc(&mut h, &cc);
        return (StatusCode::NOT_MODIFIED, h).into_response();
    }
    let mut h = HeaderMap::new();
    h.insert(
        header::CONTENT_TYPE,
        HeaderValue::from_static("application/json"),
    );
    h.insert(
        header::ETAG,
        HeaderValue::from_str(&snap.etag).unwrap_or(HeaderValue::from_static("\"\"")),
    );
    respond::insert_cc(&mut h, &cc);
    let body: Bytes = snap.encoded.clone();
    (StatusCode::OK, h, body).into_response()
}

fn lock<T>(m: &std::sync::Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    m.lock().unwrap_or_else(std::sync::PoisonError::into_inner)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn etag_is_quoted_24_hex() {
        let e = etag_of(b"{}");
        assert!(e.starts_with('"') && e.ends_with('"'));
        assert_eq!(e.len(), 26);
    }

    #[test]
    fn if_none_match_rules() {
        let etag = "\"abc123\"";
        assert!(if_none_match_matches(Some(etag), etag));
        assert!(if_none_match_matches(Some("W/\"abc123\""), etag));
        assert!(if_none_match_matches(Some("\"stale\", \"abc123\""), etag));
        assert!(if_none_match_matches(Some("*"), etag));
        assert!(!if_none_match_matches(Some("\"stale\""), etag));
        assert!(!if_none_match_matches(None, etag));
    }
}
