use axum::{
    Router,
    body::Body,
    http::{Method, Request, StatusCode},
};
use http_body_util::BodyExt;
use object_store::memory::InMemory;
use radar_api::{AppState, Config, router};
use radar_level2::{catalogue::Catalogue, memstore::MemoryStore, service::Level2Service};
use serde_json::{Value, json};
use std::sync::Arc;
use tower::ServiceExt;

fn app(enabled: bool) -> (Router, Arc<MemoryStore>) {
    let dir = tempfile::tempdir().unwrap().keep();
    let mut config = Config::defaults_for(dir.join("state"), dir.join("grid"), dir.join("tiles"));
    config.rate_limit_burst = 100_000.;
    config.gibs_wmts_base = String::new();
    let state = AppState::new(config).unwrap();
    let mem = Arc::new(MemoryStore::default());
    if enabled {
        let svc = Level2Service::new(
            mem.clone(),
            mem.clone(),
            Arc::new(InMemory::new()),
            Arc::new(Catalogue::snapshot().unwrap()),
            vec!["KGRR".into()],
            "level2/test".into(),
            Arc::new(chrono::Utc::now),
        )
        .unwrap();
        state.set_level2(svc);
    }
    (router(state), mem)
}

async fn call(
    app: &Router,
    method: Method,
    uri: &str,
    ctype: Option<&str>,
    body: Vec<u8>,
    inm: Option<&str>,
) -> (StatusCode, axum::http::HeaderMap, Vec<u8>) {
    let mut req = Request::builder().method(method).uri(uri);
    if let Some(c) = ctype {
        req = req.header("content-type", c);
    }
    if let Some(i) = inm {
        req = req.header("if-none-match", i);
    }
    let resp = app
        .clone()
        .oneshot(req.body(Body::from(body)).unwrap())
        .await
        .unwrap();
    let (status, headers) = (resp.status(), resp.headers().clone());
    (
        status,
        headers,
        resp.into_body()
            .collect()
            .await
            .unwrap()
            .to_bytes()
            .to_vec(),
    )
}

fn view(tiles: Value, zoom: u32) -> Vec<u8> {
    serde_json::to_vec(&json!({"centre": {"lat": 42.89389, "lon": -85.54489}, "zoom": zoom, "tiles": tiles, "palette": "classic"})).unwrap()
}

#[tokio::test]
async fn every_level2_route_is_404_while_the_gate_is_off() {
    let (app, _) = app(false);
    let id = "0".repeat(32);
    for (m, uri) in [
        (Method::GET, "/api/level2/sites".to_string()),
        (Method::POST, "/api/level2/views".to_string()),
        (Method::GET, format!("/api/level2/views/{id}")),
        (
            Method::GET,
            format!("/api/level2/views/{id}/0123456789abcdef/manifest.json"),
        ),
        (
            Method::GET,
            format!("/level2/tiles/KGRR/{}/v/classic/9/1/1.png", "a".repeat(64)),
        ),
    ] {
        let (status, ..) = call(
            &app,
            m,
            &uri,
            Some("application/json"),
            view(json!([[1, 1]]), 9),
            None,
        )
        .await;
        assert_eq!(status, StatusCode::NOT_FOUND, "{uri}");
    }
}

#[tokio::test]
async fn sites_is_cacheable_and_revalidates() {
    let (app, _) = app(true);
    let (status, headers, body) =
        call(&app, Method::GET, "/api/level2/sites", None, vec![], None).await;
    assert_eq!(status, StatusCode::OK);
    let v: Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(v["schema_version"], 1);
    assert_eq!(v["enabled_regions"], json!(["KGRR"]));
    let etag = headers["etag"].to_str().unwrap().to_owned();
    assert!(
        headers["cache-control"]
            .to_str()
            .unwrap()
            .contains("max-age=15")
    );
    let (status, _, body) = call(
        &app,
        Method::GET,
        "/api/level2/sites",
        None,
        vec![],
        Some(&etag),
    )
    .await;
    assert_eq!((status, body.len()), (StatusCode::NOT_MODIFIED, 0));
}

#[tokio::test]
async fn submit_rejects_bad_bodies_without_touching_the_store() {
    let (app, mem) = app(true);
    let json_ct = Some("application/json");
    let (s, ..) = call(
        &app,
        Method::POST,
        "/api/level2/views",
        Some("text/plain"),
        view(json!([[1, 1]]), 9),
        None,
    )
    .await;
    assert_eq!(s, StatusCode::UNSUPPORTED_MEDIA_TYPE);
    let (s, ..) = call(
        &app,
        Method::POST,
        "/api/level2/views",
        json_ct,
        vec![b'x'; 5000],
        None,
    )
    .await;
    assert_eq!(s, StatusCode::PAYLOAD_TOO_LARGE);
    let (s, ..) = call(
        &app,
        Method::POST,
        "/api/level2/views",
        json_ct,
        b"{\"zoom\":9}".to_vec(),
        None,
    )
    .await;
    assert_eq!(s, StatusCode::UNPROCESSABLE_ENTITY);
    let (s, _, body) = call(
        &app,
        Method::POST,
        "/api/level2/views",
        json_ct,
        view(json!([[1, 1]]), 4),
        None,
    )
    .await;
    assert_eq!(s, StatusCode::UNPROCESSABLE_ENTITY);
    assert_eq!(
        serde_json::from_slice::<Value>(&body).unwrap()["reason"],
        "invalid_zoom"
    );
    assert_eq!(mem.pin_count() + mem.revision_count(), 0);
}

#[tokio::test]
async fn submit_status_and_unknown_ids() {
    let (app, _) = app(true);
    let (x, y) = radar_tiler::geometry::lat_lon_to_tile(42.89389, -85.54489, 9);
    let (s, headers, body) = call(
        &app,
        Method::POST,
        "/api/level2/views",
        Some("application/json"),
        view(json!([[x, y]]), 9),
        None,
    )
    .await;
    assert_eq!(headers["cache-control"], "no-store");
    let v: Value = serde_json::from_slice(&body).unwrap();
    // Nothing published yet: 202 pending with a deadline, never a hang.
    assert_eq!(
        (s, v["state"].as_str()),
        (StatusCode::ACCEPTED, Some("pending")),
        "{v}"
    );
    assert_eq!(v["poll_after_s"], 5);
    assert!(
        headers["location"]
            .to_str()
            .unwrap()
            .starts_with("/api/level2/views/")
    );
    let id = v["view_id"].as_str().unwrap();
    let (s, _, body) = call(
        &app,
        Method::GET,
        &format!("/api/level2/views/{id}"),
        None,
        vec![],
        None,
    )
    .await;
    assert_eq!(s, StatusCode::OK);
    assert_eq!(
        serde_json::from_slice::<Value>(&body).unwrap()["state"],
        "pending"
    );
    let (s, ..) = call(
        &app,
        Method::GET,
        &format!("/api/level2/views/{}", "f".repeat(32)),
        None,
        vec![],
        None,
    )
    .await;
    assert_eq!(s, StatusCode::NOT_FOUND);
    let (s, ..) = call(
        &app,
        Method::GET,
        "/api/level2/views/..%2Fetc",
        None,
        vec![],
        None,
    )
    .await;
    assert_eq!(s, StatusCode::NOT_FOUND);
    let (s, ..) = call(
        &app,
        Method::GET,
        &format!("/api/level2/views/{id}/0123456789abcdef/manifest.json"),
        None,
        vec![],
        None,
    )
    .await;
    assert_eq!(s, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn tiles_never_return_a_transparent_200_for_missing_or_malformed_paths() {
    let (app, _) = app(true);
    let sweep = "a".repeat(64);
    let ok = format!("/level2/tiles/KGRR/{sweep}/level2-polar-v1-test0000/classic/9/1/1.png");
    let (s, h, _) = call(&app, Method::GET, &ok, None, vec![], None).await;
    assert_eq!(s, StatusCode::NOT_FOUND);
    assert_eq!(h["cache-control"], "no-store");
    for bad in [
        format!("/level2/tiles/KGRR/{sweep}/v/classic/9/1/1"),
        format!("/level2/tiles/KGRR/{sweep}/v/classic/9/1/x.png"),
        format!("/level2/tiles/kgrr/{sweep}/v/classic/9/1/1.png"),
        format!("/level2/tiles/KGRR/{sweep}/v/neon/9/1/1.png"),
        format!("/level2/tiles/KGRR/{sweep}/v/classic/99/1/1.png"),
    ] {
        let (s, ..) = call(&app, Method::GET, &bad, None, vec![], None).await;
        assert_eq!(s, StatusCode::NOT_FOUND, "{bad}");
    }
}
