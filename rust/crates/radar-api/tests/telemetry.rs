//! HTTP tests for the `/api/telemetry/v1/*` relay against a local mock collector.

use std::sync::{Arc, Mutex};

use axum::Router;
use axum::body::{Body, Bytes};
use axum::http::{HeaderMap, Request, StatusCode, header};
use axum::routing::post;
use http_body_util::BodyExt;
use radar_api::{AppState, Config, router};
use tempfile::TempDir;
use tower::ServiceExt;

type Seen = Arc<Mutex<Vec<(String, HeaderMap, Bytes)>>>;

fn app(dir: &TempDir, tweak: impl FnOnce(&mut Config)) -> Router {
    let root = dir.path();
    let mut cfg = Config::defaults_for(root.join("state"), root.join("grid"), root.join("tiles"));
    cfg.rate_limit_rps = 100_000.0;
    cfg.rate_limit_burst = 100_000.0;
    cfg.telemetry_rate_per_min = 6_000_000.0;
    cfg.telemetry_rate_burst = 100_000.0;
    tweak(&mut cfg);
    router(AppState::new(cfg).expect("state"))
}

/// A mock collector that records each request and answers 200 `{}`.
async fn collector(status: StatusCode) -> (String, Seen) {
    let seen: Seen = Arc::default();
    let record = |path: &'static str, seen: Seen| {
        move |headers: HeaderMap, body: Bytes| async move {
            seen.lock().unwrap().push((path.to_string(), headers, body));
            (status, [(header::CONTENT_TYPE, "application/json")], "{}")
        }
    };
    let mock = Router::new()
        .route("/v1/traces", post(record("/v1/traces", seen.clone())))
        .route("/v1/logs", post(record("/v1/logs", seen.clone())));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind");
    let addr = listener.local_addr().expect("addr");
    tokio::spawn(async move {
        let _ = axum::serve(listener, mock).await;
    });
    (format!("http://{addr}"), seen)
}

fn otlp(path: &str, auth: Option<&str>, body: impl Into<Body>) -> Request<Body> {
    let mut b = Request::builder()
        .method("POST")
        .uri(path)
        .header(header::CONTENT_TYPE, "application/x-protobuf");
    if let Some(a) = auth {
        b = b.header(header::AUTHORIZATION, a);
    }
    b.body(body.into()).expect("request")
}

async fn status(app: &Router, req: Request<Body>) -> (StatusCode, Vec<u8>) {
    let resp = app.clone().oneshot(req).await.expect("infallible");
    let code = resp.status();
    let body = resp.into_body().collect().await.expect("body").to_bytes();
    (code, body.to_vec())
}

#[tokio::test]
async fn disabled_without_upstream_is_404() {
    let dir = tempfile::tempdir().unwrap();
    let app = app(&dir, |_| {});
    let (code, _) = status(&app, otlp("/api/telemetry/v1/traces", None, "x")).await;
    assert_eq!(code, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn missing_or_wrong_token_is_401_and_never_forwarded() {
    let (base, seen) = collector(StatusCode::OK).await;
    let dir = tempfile::tempdir().unwrap();
    let app = app(&dir, |c| {
        c.otlp_upstream = base;
        c.telemetry_token = "s3cret".into();
    });
    for auth in [None, Some("Bearer nope"), Some("s3cret")] {
        let (code, _) = status(&app, otlp("/api/telemetry/v1/logs", auth, "x")).await;
        assert_eq!(code, StatusCode::UNAUTHORIZED, "{auth:?}");
    }
    assert!(seen.lock().unwrap().is_empty());
}

#[tokio::test]
async fn oversize_body_is_413() {
    let (base, seen) = collector(StatusCode::OK).await;
    let dir = tempfile::tempdir().unwrap();
    let app = app(&dir, |c| c.otlp_upstream = base);
    let big = vec![0u8; 1024 * 1024 + 1];
    let (code, _) = status(&app, otlp("/api/telemetry/v1/traces", None, big.clone())).await;
    assert_eq!(code, StatusCode::PAYLOAD_TOO_LARGE);
    // A streamed body without Content-Length is cut off too.
    let stream = Body::from_stream(futures::stream::iter([Ok::<_, std::io::Error>(
        Bytes::from(big),
    )]));
    let (code, _) = status(&app, otlp("/api/telemetry/v1/traces", None, stream)).await;
    assert_eq!(code, StatusCode::PAYLOAD_TOO_LARGE);
    assert!(seen.lock().unwrap().is_empty());
}

#[tokio::test]
async fn non_otlp_content_type_is_415() {
    let (base, _) = collector(StatusCode::OK).await;
    let dir = tempfile::tempdir().unwrap();
    let app = app(&dir, |c| c.otlp_upstream = base);
    let req = Request::builder()
        .method("POST")
        .uri("/api/telemetry/v1/traces")
        .header(header::CONTENT_TYPE, "text/plain")
        .body(Body::from("x"))
        .unwrap();
    let (code, _) = status(&app, req).await;
    assert_eq!(code, StatusCode::UNSUPPORTED_MEDIA_TYPE);
}

#[tokio::test]
async fn forwards_body_and_headers_to_the_matching_signal() {
    let (base, seen) = collector(StatusCode::OK).await;
    let dir = tempfile::tempdir().unwrap();
    let app = app(&dir, |c| {
        c.otlp_upstream = base;
        c.telemetry_token = "s3cret".into();
    });
    let mut req = otlp("/api/telemetry/v1/traces", Some("Bearer s3cret"), "spans");
    req.headers_mut()
        .insert(header::CONTENT_ENCODING, "gzip".parse().unwrap());
    let (code, body) = status(&app, req).await;
    assert_eq!(code, StatusCode::OK);
    assert_eq!(body, b"{}");
    let (code, _) = status(
        &app,
        otlp("/api/telemetry/v1/logs", Some("Bearer s3cret"), "records"),
    )
    .await;
    assert_eq!(code, StatusCode::OK);

    let seen = seen.lock().unwrap();
    assert_eq!(seen.len(), 2);
    let (path, headers, body) = &seen[0];
    assert_eq!(path, "/v1/traces");
    assert_eq!(body.as_ref(), b"spans");
    assert_eq!(headers[header::CONTENT_TYPE], "application/x-protobuf");
    assert_eq!(headers[header::CONTENT_ENCODING], "gzip");
    assert!(headers.get(header::AUTHORIZATION).is_none());
    assert_eq!(seen[1].0, "/v1/logs");
    assert_eq!(seen[1].2.as_ref(), b"records");
}

#[tokio::test]
async fn upstream_status_passes_through_and_transport_errors_are_502() {
    let (base, _) = collector(StatusCode::SERVICE_UNAVAILABLE).await;
    let dir = tempfile::tempdir().unwrap();
    let app1 = app(&dir, |c| c.otlp_upstream = base);
    let (code, _) = status(&app1, otlp("/api/telemetry/v1/logs", None, "x")).await;
    assert_eq!(code, StatusCode::SERVICE_UNAVAILABLE);

    let closed = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let dead = format!("http://{}", closed.local_addr().unwrap());
    drop(closed);
    let app2 = app(&dir, |c| c.otlp_upstream = dead);
    let (code, _) = status(&app2, otlp("/api/telemetry/v1/logs", None, "x")).await;
    assert_eq!(code, StatusCode::BAD_GATEWAY);
}

#[tokio::test]
async fn upstream_redirects_are_not_followed() {
    let (target, seen) = collector(StatusCode::OK).await;
    let redirect = Router::new().fallback(move || {
        let to = format!("{target}/v1/traces");
        async move { (StatusCode::TEMPORARY_REDIRECT, [(header::LOCATION, to)]) }
    });
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    tokio::spawn(async move {
        let _ = axum::serve(listener, redirect).await;
    });
    let dir = tempfile::tempdir().unwrap();
    let app = app(&dir, |c| c.otlp_upstream = base);
    let (code, _) = status(&app, otlp("/api/telemetry/v1/traces", None, "x")).await;
    assert_eq!(code, StatusCode::TEMPORARY_REDIRECT);
    assert!(seen.lock().unwrap().is_empty());
}

#[tokio::test(start_paused = true)]
async fn stalled_upload_times_out_with_408() {
    let (base, seen) = collector(StatusCode::OK).await;
    let dir = tempfile::tempdir().unwrap();
    let app = app(&dir, |c| c.otlp_upstream = base);
    let stalled = Body::from_stream(futures::stream::pending::<Result<Bytes, std::io::Error>>());
    let (code, _) = status(&app, otlp("/api/telemetry/v1/traces", None, stalled)).await;
    assert_eq!(code, StatusCode::REQUEST_TIMEOUT);
    assert!(seen.lock().unwrap().is_empty());
}

#[tokio::test]
async fn per_client_limit_answers_429() {
    let (base, seen) = collector(StatusCode::OK).await;
    let dir = tempfile::tempdir().unwrap();
    let app = app(&dir, |c| {
        c.otlp_upstream = base;
        c.telemetry_rate_per_min = 0.0;
        c.telemetry_rate_burst = 2.0;
    });
    let mut codes = Vec::new();
    for _ in 0..3 {
        codes.push(
            status(&app, otlp("/api/telemetry/v1/traces", None, "x"))
                .await
                .0,
        );
    }
    assert_eq!(
        codes,
        [
            StatusCode::OK,
            StatusCode::OK,
            StatusCode::TOO_MANY_REQUESTS
        ]
    );
    assert_eq!(seen.lock().unwrap().len(), 2);
}

#[tokio::test]
async fn get_is_405() {
    let (base, _) = collector(StatusCode::OK).await;
    let dir = tempfile::tempdir().unwrap();
    let app = app(&dir, |c| c.otlp_upstream = base);
    let req = Request::builder()
        .uri("/api/telemetry/v1/traces")
        .body(Body::empty())
        .unwrap();
    assert_eq!(status(&app, req).await.0, StatusCode::METHOD_NOT_ALLOWED);
}
