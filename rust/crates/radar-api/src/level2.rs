//! Level II viewport routes (design level2-close-up section 4). Handlers only translate the
//! `radar_level2::service` results to HTTP; nothing here decodes or renders.
//!
//! - `GET /api/level2/sites`, `POST /api/level2/views`, `GET /api/level2/views/{id}`
//! - `GET /api/level2/views/{id}/{revision}/manifest.json`
//! - `GET /level2/tiles/{station}/{sweep}/{version}/{palette}/{z}/{x}/{y}.png`
//!
//! All of it stays off (404) until `LEVEL2_VIEWPORT_ENABLED=true` and the service connects.

use std::sync::Arc;
use std::time::Instant;

use axum::Router;
use axum::extract::{Path as UrlPath, Request, State};
use axum::http::{HeaderMap, HeaderValue, StatusCode, header};
use axum::response::{IntoResponse, Response};
use axum::routing::get;
use bytes::Bytes;
use radar_level2::service::{Body, Level2Service, ManifestReply, Submit, TileReply};
use radar_level2::viewport::RawView;
use serde_json::json;

use crate::manifest::if_none_match_matches;
use crate::respond;
use crate::state::AppState;

const MAX_POST_BYTES: usize = 4 * 1024;
const NO_STORE: &str = "no-store";

pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/api/level2/sites", get(sites))
        .route("/api/level2/views", axum::routing::post(submit))
        .route("/api/level2/views/{id}", get(status))
        .route(
            "/api/level2/views/{id}/{revision}/manifest.json",
            get(manifest),
        )
        .route(
            "/level2/tiles/{station}/{sweep}/{version}/{palette}/{z}/{x}/{y}",
            get(tile),
        )
}

fn service(state: &AppState) -> Option<Arc<Level2Service>> {
    state.level2.get().cloned()
}

fn not_found() -> Response {
    respond::detail(StatusCode::NOT_FOUND, "Not Found")
}

fn error(status: StatusCode, code: &str) -> Response {
    respond::json(status, &json!({ "error": code }), Some(NO_STORE))
}

fn conditional(headers: &HeaderMap, body: &Body, cc: &str) -> Response {
    let mut out = HeaderMap::new();
    if let Ok(v) = HeaderValue::from_str(&body.etag) {
        out.insert(header::ETAG, v);
    }
    respond::insert_cc(&mut out, cc);
    let inm = headers
        .get(header::IF_NONE_MATCH)
        .and_then(|v| v.to_str().ok());
    if if_none_match_matches(inm, &body.etag) {
        return (StatusCode::NOT_MODIFIED, out).into_response();
    }
    out.insert(
        header::CONTENT_TYPE,
        HeaderValue::from_static("application/json"),
    );
    (StatusCode::OK, out, Bytes::from(body.bytes.clone())).into_response()
}

async fn sites(State(state): State<AppState>, headers: HeaderMap) -> Response {
    let Some(svc) = service(&state) else {
        return not_found();
    };
    match svc.sites().await {
        Ok(body) => conditional(&headers, &body, "public, max-age=15"),
        Err(_) => error(StatusCode::SERVICE_UNAVAILABLE, "level2_unavailable"),
    }
}

/// `POST /api/level2/views`: tile demand. 202 while warming, 200 for ready or a fallback reason.
async fn submit(State(state): State<AppState>, req: Request) -> Response {
    let Some(svc) = service(&state) else {
        return not_found();
    };
    let client = crate::edge::client_key(&state, &req);
    let admitted = state
        .level2_limiter
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .admit(&client, Instant::now(), false);
    if !admitted {
        let mut resp = error(StatusCode::TOO_MANY_REQUESTS, "rate_limited");
        resp.headers_mut()
            .insert(header::RETRY_AFTER, HeaderValue::from_static("2"));
        return resp;
    }
    let json_body = req
        .headers()
        .get(header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .is_some_and(|v| v.starts_with("application/json"));
    if !json_body {
        return error(StatusCode::UNSUPPORTED_MEDIA_TYPE, "json_required");
    }
    let Ok(bytes) = axum::body::to_bytes(req.into_body(), MAX_POST_BYTES).await else {
        return error(StatusCode::PAYLOAD_TOO_LARGE, "body_too_large");
    };
    let Ok(raw) = serde_json::from_slice::<RawView>(&bytes) else {
        return error(StatusCode::UNPROCESSABLE_ENTITY, "invalid_view");
    };
    match svc.submit(raw).await {
        Ok(Submit::Invalid(reason)) => respond::json(
            StatusCode::UNPROCESSABLE_ENTITY,
            &json!({ "error": "invalid_view", "reason": reason }),
            Some(NO_STORE),
        ),
        Ok(Submit::Reply { status, body }) => {
            let code = StatusCode::from_u16(status).unwrap_or(StatusCode::OK);
            let location = serde_json::from_slice::<serde_json::Value>(&body.bytes)
                .ok()
                .and_then(|v| {
                    v["view_id"]
                        .as_str()
                        .map(|id| format!("/api/level2/views/{id}"))
                });
            let mut resp = respond::json_bytes(code, Bytes::from(body.bytes), Some(NO_STORE));
            if let Some(v) = location.and_then(|l| HeaderValue::from_str(&l).ok()) {
                resp.headers_mut().insert(header::LOCATION, v);
            }
            resp
        }
        Err(_) => error(StatusCode::SERVICE_UNAVAILABLE, "level2_unavailable"),
    }
}

async fn status(
    State(state): State<AppState>,
    UrlPath(id): UrlPath<String>,
    headers: HeaderMap,
) -> Response {
    let Some(svc) = service(&state) else {
        return not_found();
    };
    match svc.status(&id).await {
        Ok(Some(body)) => conditional(&headers, &body, "no-store"),
        Ok(None) => not_found(),
        Err(_) => error(StatusCode::SERVICE_UNAVAILABLE, "level2_unavailable"),
    }
}

async fn manifest(
    State(state): State<AppState>,
    UrlPath((id, revision)): UrlPath<(String, String)>,
    headers: HeaderMap,
) -> Response {
    let Some(svc) = service(&state) else {
        return not_found();
    };
    match svc.manifest(&id, &revision).await {
        ManifestReply::Ok(rev) => {
            let remaining = (rev.expires_at - svc.now()).num_seconds().clamp(0, 15);
            let body = Body {
                bytes: rev.envelope.clone().into_bytes(),
                etag: rev.etag.clone(),
            };
            conditional(&headers, &body, &format!("public, max-age={remaining}"))
        }
        ManifestReply::NotFound => not_found(),
        ManifestReply::Gone => error(StatusCode::GONE, "expired"),
        ManifestReply::Unavailable => error(StatusCode::SERVICE_UNAVAILABLE, "level2_unavailable"),
    }
}

type TileParts = (String, String, String, String, String, String, String);

/// An immutable PNG. Missing is 404 or 410, a transient failure is 503; never a transparent 200.
async fn tile(
    State(state): State<AppState>,
    UrlPath((station, sweep, version, palette, z, x, ypng)): UrlPath<TileParts>,
    headers: HeaderMap,
) -> Response {
    let Some(svc) = service(&state) else {
        return not_found();
    };
    let num = |s: &str| {
        (!s.is_empty() && s.len() <= 8)
            .then(|| s.parse::<u32>().ok())
            .flatten()
    };
    let (Some(z), Some(x), Some(y)) = (num(&z), num(&x), ypng.strip_suffix(".png").and_then(num))
    else {
        return not_found();
    };
    match svc
        .tile(&station, &sweep, &version, &palette, z, x, y)
        .await
    {
        TileReply::Ok { bytes, sha256 } => {
            let etag = format!("\"{sha256}\"");
            let mut out = HeaderMap::new();
            if let Ok(v) = HeaderValue::from_str(&etag) {
                out.insert(header::ETAG, v);
            }
            respond::insert_cc(&mut out, "public, max-age=60");
            let inm = headers
                .get(header::IF_NONE_MATCH)
                .and_then(|v| v.to_str().ok());
            if if_none_match_matches(inm, &etag) {
                return (StatusCode::NOT_MODIFIED, out).into_response();
            }
            out.insert(header::CONTENT_TYPE, HeaderValue::from_static("image/png"));
            (StatusCode::OK, out, Bytes::from(bytes)).into_response()
        }
        TileReply::NotFound => respond_status(StatusCode::NOT_FOUND),
        TileReply::Gone => respond_status(StatusCode::GONE),
        TileReply::Unavailable(_) => {
            let mut resp = respond_status(StatusCode::SERVICE_UNAVAILABLE);
            resp.headers_mut()
                .insert(header::RETRY_AFTER, HeaderValue::from_static("2"));
            resp
        }
    }
}

fn respond_status(status: StatusCode) -> Response {
    let mut resp = status.into_response();
    resp.headers_mut()
        .insert(header::CACHE_CONTROL, HeaderValue::from_static(NO_STORE));
    resp
}
