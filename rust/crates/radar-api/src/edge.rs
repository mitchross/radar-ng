//! The API route table and the global middleware (spec 1.4, 2, 4, 5.11, 10.2, 13 D13, D22).
//!
//! The middleware runs for every request, including 404s and 405s:
//! 1. Resolve the client IP (strict `X-Forwarded-For` walk from the socket peer).
//! 2. Token bucket per client, for `/api/*` and `/v1/*` only (`/api/livez` exempt). Static lanes
//!    are never limited, as in the Python stack where Caddy served them. A limited request answers
//!    429 and consumes no token.
//! 3. The handler runs. Its metrics are recorded under the matched route template, or `unmatched`.
//! 4. A JSON access log line for every request except `/api/livez` and `/api/health`.
//!
//! Method and path fallbacks: a HEAD on a `/api` route answers 405, and every 405 under `/api` or
//! `/v1` carries FastAPI's `{"detail":"Method Not Allowed"}` body. An unknown `/api` or `/v1` path
//! answers 404 `{"detail":"Not Found"}`. Any other unknown path answers the Caddy-style text 404
//! (D13: Caddy answered 200 empty).

use std::net::SocketAddr;
use std::sync::{Mutex, MutexGuard, PoisonError};
use std::time::Instant;

use axum::Router;
use axum::extract::{ConnectInfo, MatchedPath, Request, State};
use axum::http::{HeaderValue, Method, StatusCode, header};
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};
use axum::routing::get;

use crate::alerts;
use crate::briefing;
use crate::client_ip;
use crate::files;
use crate::forecast;
use crate::grid;
use crate::health;
use crate::manifest;
use crate::nowcast;
use crate::outlooks;
use crate::palettes;
use crate::places;
use crate::prefetch;
use crate::respond;
use crate::state::AppState;
use crate::wind;

/// Route templates, used for the redirect check and to recognize API paths.
pub const ROUTE_TEMPLATES: &[&str] = &[
    "/api/manifest.json",
    "/api/palettes",
    "/api/forecast/{lat}/{lon}",
    "/api/inspect/{layer}/{timestamp}/{lat}/{lon}",
    "/api/nowcast/skill",
    "/api/nowcast/{lat}/{lon}",
    "/api/wind-field/{timestamp}",
    "/api/lightning",
    "/api/storms",
    "/api/storm-prefetch",
    "/api/storm-prefetch/style.json",
    "/api/tropical",
    "/api/livez",
    "/api/health",
    "/api/metrics",
    "/api/alerts",
    "/api/alerts/map",
    "/api/outlooks",
    "/api/geocode",
    "/api/reverse-geocode",
    "/api/briefing",
    "/api/alerts/explain",
    "/v1/push-tokens",
    "/v1/push-tokens/{token}",
    "/v1/watches",
    "/v1/watches/{user_id}/{storm_cell_id}",
];

/// The `/api` routes (the `/v1` routes live in `workflows`).
pub fn api_router() -> Router<AppState> {
    Router::new()
        .route("/api/manifest.json", get(manifest::get_manifest))
        .route("/api/palettes", get(palettes::get_palettes))
        .route("/api/forecast/{lat}/{lon}", get(forecast::get_forecast))
        .route(
            "/api/inspect/{layer}/{timestamp}/{lat}/{lon}",
            get(grid::inspect_point),
        )
        .route("/api/nowcast/skill", get(nowcast::get_nowcast_skill))
        .route("/api/nowcast/{lat}/{lon}", get(nowcast::get_nowcast_point))
        .route("/api/wind-field/{timestamp}", get(wind::get_wind_field))
        .route("/api/lightning", get(files::get_lightning))
        .route("/api/storms", get(files::get_storms))
        .route("/api/storm-prefetch", get(prefetch::get_storm_prefetch))
        .route(
            "/api/storm-prefetch/style.json",
            get(prefetch::get_storm_prefetch_style),
        )
        .route("/api/tropical", get(files::get_tropical))
        .route("/api/livez", get(health::get_livez))
        .route("/api/health", get(health::get_health))
        .route("/api/metrics", get(health::get_metrics))
        .route("/api/alerts", get(alerts::get_alerts))
        .route("/api/alerts/map", get(alerts::get_alerts_map))
        .route("/api/outlooks", get(outlooks::get_outlooks))
        .route("/api/alerts/explain", get(briefing::explain_alert))
        .route("/api/geocode", get(places::get_geocode))
        .route("/api/reverse-geocode", get(places::get_reverse_geocode))
        .route("/api/briefing", get(briefing::get_briefing))
}

fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(PoisonError::into_inner)
}

/// Does `path` match a route template? Segments match literally, or as any non-empty value for `{name}`.
#[must_use]
pub fn template_matches(template: &str, path: &str) -> bool {
    let t: Vec<&str> = template.split('/').collect();
    let p: Vec<&str> = path.split('/').collect();
    if t.len() != p.len() {
        return false;
    }
    t.iter().zip(&p).all(|(tp, pp)| {
        if tp.starts_with('{') && tp.ends_with('}') {
            !pp.is_empty()
        } else {
            tp == pp
        }
    })
}

fn is_api_path(path: &str) -> bool {
    path.starts_with("/api/") || path.starts_with("/v1/")
}

/// The client key: the strict client IP from the socket peer and `X-Forwarded-For` (D22: the
/// `X-Client-IP` header is not trusted).
fn client_key(state: &AppState, req: &Request) -> String {
    let peer = req
        .extensions()
        .get::<ConnectInfo<SocketAddr>>()
        .map(|c| c.0.ip());
    let xff: Vec<String> = req
        .headers()
        .get_all("x-forwarded-for")
        .iter()
        .filter_map(|v| v.to_str().ok())
        .flat_map(|s| {
            s.split(',')
                .map(|part| part.trim().to_string())
                .collect::<Vec<_>>()
        })
        .collect();
    client_ip::resolve(peer, &xff, &state.trusted)
}

/// The global middleware: rate limit, metrics, the 405 rewrite and the access log.
pub async fn edge(State(state): State<AppState>, req: Request, next: Next) -> Response {
    let started = Instant::now();
    let method = req.method().clone();
    let path = req.uri().path().to_string();
    let query = req.uri().query().map(str::to_string);
    let user_agent = req
        .headers()
        .get(header::USER_AGENT)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("")
        .to_string();
    let route = req
        .extensions()
        .get::<MatchedPath>()
        .map_or_else(|| "unmatched".to_string(), |m| m.as_str().to_string());
    let client = client_key(&state, &req);
    // Only the API lanes are limited (F1). Static lanes (tiles, basemap styles, fonts, imagery and
    // vector tiles) were served by Caddy and never reached FastAPI's limiter, and `/api/livez` is
    // exempt so the liveness probe cannot be throttled into a restart.
    let exempt = path == "/api/livez" || !is_api_path(&path);

    let admitted = lock(&state.limiter).admit(&client, Instant::now(), exempt);
    let resp = if !admitted {
        state
            .metrics
            .counters
            .rate_limited_requests_total
            .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let mut resp = respond::json(
            StatusCode::TOO_MANY_REQUESTS,
            &serde_json::json!({ "error": "rate_limited" }),
            None,
        );
        resp.headers_mut()
            .insert(header::RETRY_AFTER, HeaderValue::from_static("1"));
        resp
    } else if method == Method::HEAD && path.starts_with("/api/") {
        respond::detail(StatusCode::METHOD_NOT_ALLOWED, "Method Not Allowed")
    } else {
        let resp = next.run(req).await;
        rewrite_405(resp, &path)
    };

    let elapsed = started.elapsed().as_secs_f64();
    let status = resp.status().as_u16();
    state
        .metrics
        .record(method.as_str(), &route, status, elapsed);
    let skip_log = path == "/api/livez" || path == "/api/health";
    if !skip_log {
        let uri = match &query {
            Some(q) => format!("{path}?{q}"),
            None => path.clone(),
        };
        tracing::info!(
            method = %method,
            uri = %uri,
            status = status,
            duration = elapsed,
            user_agent = %user_agent,
            "request"
        );
    }
    resp
}

/// FastAPI's 405 body for API paths. The `Allow` header is kept from the router.
fn rewrite_405(resp: Response, path: &str) -> Response {
    if resp.status() != StatusCode::METHOD_NOT_ALLOWED || !is_api_path(path) {
        return resp;
    }
    let allow = resp
        .headers()
        .get(header::ALLOW)
        .cloned()
        .unwrap_or_else(|| HeaderValue::from_static("GET"));
    let mut out = respond::detail(StatusCode::METHOD_NOT_ALLOWED, "Method Not Allowed");
    out.headers_mut().insert(header::ALLOW, allow);
    out
}

/// Unmatched paths: Starlette's redirect for a trailing slash, then 404.
pub async fn unmatched(req: Request) -> Response {
    let path = req.uri().path().to_string();
    if path.len() > 1 && path.ends_with('/') {
        let trimmed = path.trim_end_matches('/');
        if ROUTE_TEMPLATES.iter().any(|t| template_matches(t, trimmed)) {
            let location = match req.uri().query() {
                Some(q) => format!("{trimmed}?{q}"),
                None => trimmed.to_string(),
            };
            let mut resp = StatusCode::TEMPORARY_REDIRECT.into_response();
            if let Ok(value) = HeaderValue::from_str(&location) {
                resp.headers_mut().insert(header::LOCATION, value);
            }
            return resp;
        }
    }
    if is_api_path(&path) {
        return respond::detail(StatusCode::NOT_FOUND, "Not Found");
    }
    if path.starts_with("/basemap/imagery/") {
        return crate::basemap::bare_404();
    }
    respond::caddy_error(StatusCode::NOT_FOUND)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn templates_match_by_segment() {
        assert!(template_matches(
            "/api/forecast/{lat}/{lon}",
            "/api/forecast/42.9/-85.6"
        ));
        assert!(!template_matches(
            "/api/forecast/{lat}/{lon}",
            "/api/forecast/42.9"
        ));
        assert!(!template_matches("/api/livez", "/api/health"));
        assert!(template_matches("/api/manifest.json", "/api/manifest.json"));
    }
}
