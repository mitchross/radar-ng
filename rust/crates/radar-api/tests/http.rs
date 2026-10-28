//! HTTP-level tests against the real router (spec 15). Each test builds its own state in a temp
//! directory and sends requests with `oneshot`. Upstreams (Open-Meteo, NWS, Photon, USGS, the LLM,
//! the NWS alert detail) are local axum servers on 127.0.0.1.
//!
//! Requests carry no socket peer, so every request shares one rate-limit bucket ("unknown").

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

use axum::Json;
use axum::Router;
use axum::body::Body;
use axum::extract::{Path as AxPath, Query};
use axum::http::{HeaderMap, Request, StatusCode, header};
use axum::routing::get;
use base64::Engine;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use chrono::{SecondsFormat, Utc};
use futures::future::join_all;
use hmac::{Hmac, Mac};
use http_body_util::BodyExt;
use pmtiles::{Compression, PmTilesWriter, TileCoord, TileType};
use radar_api::{AppState, Config, router};
use serde_json::{Value, json};
use sha2::Sha256;
use tempfile::TempDir;
use tower::ServiceExt;

/// A temp directory with the usual layout, plus the config and app built on it.
struct Env {
    _dir: TempDir,
    root: PathBuf,
}

impl Env {
    fn new() -> Self {
        let dir = tempfile::tempdir().expect("tempdir");
        let root = dir.path().to_path_buf();
        for sub in [
            "state",
            "grid",
            "tiles",
            "basemap/styles",
            "basemap/fonts",
            "styles",
        ] {
            std::fs::create_dir_all(root.join(sub)).expect("subdir");
        }
        Self { _dir: dir, root }
    }

    fn path(&self, sub: &str) -> PathBuf {
        self.root.join(sub)
    }

    fn config(&self, tweak: impl FnOnce(&mut Config)) -> Config {
        let mut cfg =
            Config::defaults_for(self.path("state"), self.path("grid"), self.path("tiles"));
        cfg.style_dir = self.path("styles");
        cfg.basemap_root = self.path("basemap");
        cfg.pmtiles_path = self.path("basemap.pmtiles");
        cfg.rate_limit_burst = 100_000.0;
        cfg.rate_limit_rps = 100_000.0;
        cfg.imagery_cache_max_bytes = 1024 * 1024;
        tweak(&mut cfg);
        cfg
    }

    fn app(&self, tweak: impl FnOnce(&mut Config)) -> Router {
        let state = AppState::new(self.config(tweak)).expect("state");
        router(state)
    }
}

async fn send(app: &Router, req: Request<Body>) -> (StatusCode, HeaderMap, Vec<u8>) {
    let resp = app.clone().oneshot(req).await.expect("infallible");
    let status = resp.status();
    let headers = resp.headers().clone();
    let body = resp
        .into_body()
        .collect()
        .await
        .expect("body")
        .to_bytes()
        .to_vec();
    (status, headers, body)
}

fn get_req(uri: &str) -> Request<Body> {
    Request::builder()
        .uri(uri)
        .body(Body::empty())
        .expect("request")
}

async fn get_json(app: &Router, uri: &str) -> (StatusCode, HeaderMap, Value) {
    let (status, headers, body) = send(app, get_req(uri)).await;
    let value = serde_json::from_slice(&body).unwrap_or(Value::Null);
    (status, headers, value)
}

fn header_str<'a>(headers: &'a HeaderMap, name: &str) -> &'a str {
    headers
        .get(name)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("")
}

/// Spawn a local upstream and return its base URL.
async fn mock(app: Router) -> String {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind");
    let addr = listener.local_addr().expect("addr");
    tokio::spawn(async move {
        let _ = axum::serve(listener, app).await;
    });
    format!("http://{addr}")
}

fn counter() -> Arc<AtomicUsize> {
    Arc::new(AtomicUsize::new(0))
}

/// Compact token in the issuer's format: base64url(json) "." base64url(HMAC-SHA256(key, part)).
fn token(key: &[u8], payload: &str) -> String {
    let part = URL_SAFE_NO_PAD.encode(payload.as_bytes());
    let mut mac = Hmac::<Sha256>::new_from_slice(key).expect("key");
    mac.update(part.as_bytes());
    let sig = URL_SAFE_NO_PAD.encode(mac.finalize().into_bytes());
    format!("{part}.{sig}")
}

fn write_json(path: &Path, value: &Value) {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).expect("parent");
    }
    std::fs::write(path, serde_json::to_vec(value).expect("json")).expect("write");
}

/// A manifest with one radar frame at `now` and an optional nowcast run.
fn manifest(radar_latest: Option<&str>, nowcast: Option<(&str, &str)>) -> Value {
    let mut layers = serde_json::Map::new();
    if let Some(ts) = radar_latest {
        layers.insert(
            "radar".into(),
            json!({
                "timestamps": [ts], "frames": [{"timestamp": ts, "path": ts, "lead_minutes": 0,
                    "kind": "observation", "spatial_resolution_km": 1.0}],
                "latest": ts, "palettes": ["classic", "vivid", "muted"], "max_zoom": 7,
                "kind": "observation", "spatial_resolution_km": 1.0,
            }),
        );
    }
    if let Some((run, valid)) = nowcast {
        layers.insert(
            "nowcast".into(),
            json!({
                "timestamps": [valid], "frames": [{"timestamp": valid,
                    "path": format!("runs/{run}/{valid}"), "lead_minutes": 5,
                    "kind": "nowcast", "spatial_resolution_km": 1.0, "grid_key": format!("runs/{run}/{valid}")}],
                "latest": valid, "palettes": ["classic", "vivid", "muted"], "max_zoom": 6,
                "kind": "nowcast", "spatial_resolution_km": 1.0, "horizon_minutes": 60, "step_minutes": 5,
                "method": "advect", "run_id": run,
            }),
        );
    }
    json!({
        "schema_version": 2,
        "layers": layers,
        "tile_url_template": "/tiles/{layer}/{palette}/{path}/{z}/{x}/{y}.png",
        "tile_url_template_legacy": "/tiles/{layer}/{timestamp}/{z}/{x}/{y}.png",
        "updated_at": "2026-10-13T12:00:00+00:00",
    })
}

fn now_iso() -> String {
    Utc::now().to_rfc3339_opts(SecondsFormat::Secs, false)
}

fn open_meteo_body() -> Value {
    json!({
        "utc_offset_seconds": -14400,
        "current": {"temperature_2m": 61.4, "apparent_temperature": 60.2, "weather_code": 61,
                    "wind_speed_10m": 8.2, "wind_gusts_10m": 15.1, "wind_direction_10m": 0,
                    "relative_humidity_2m": 70, "dew_point_2m": 51.5,
                    "surface_pressure": 1013.2, "precipitation": 0},
        "hourly": {"time": ["2026-10-13T08:00", "2026-10-13T09:00", "2026-10-13T10:00", "2026-10-13T11:00"],
                   "temperature_2m": [50, 52, 55, 58], "weather_code": [0, 1, 61, 3],
                   "precipitation": [0.0, 0.01, 0.02, 0.0]},
        "daily": {"temperature_2m_max": [66.2], "temperature_2m_min": [52.0], "precipitation_sum": [0.3]},
    })
}

// ---------------------------------------------------------------------------------------------
// Request controls, health and metrics
// ---------------------------------------------------------------------------------------------

#[tokio::test]
async fn livez_is_plain_and_exempt_from_the_rate_limit() {
    let env = Env::new();
    let app = env.app(|c| {
        c.rate_limit_burst = 2.0;
        c.rate_limit_rps = 0.0;
    });
    let (s1, _, _) = get_json(&app, "/api/metrics").await;
    let (s2, _, _) = get_json(&app, "/api/metrics").await;
    assert_eq!((s1, s2), (StatusCode::OK, StatusCode::OK));
    let (s3, h3, body3) = get_json(&app, "/api/metrics").await;
    assert_eq!(s3, StatusCode::TOO_MANY_REQUESTS);
    assert_eq!(header_str(&h3, "retry-after"), "1");
    assert_eq!(body3, json!({"error": "rate_limited"}));
    for _ in 0..3 {
        let (status, headers, body) = get_json(&app, "/api/livez").await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(body, json!({"status": "ok"}));
        assert!(headers.get(header::CACHE_CONTROL).is_none());
    }
}

#[tokio::test]
async fn metrics_text_has_the_counter_block_and_route_labels() {
    let env = Env::new();
    let app = env.app(|_| {});
    let _ = get_json(&app, "/api/livez").await;
    let (status, headers, body) = send(&app, get_req("/api/metrics")).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        header_str(&headers, "content-type"),
        "text/plain; charset=utf-8"
    );
    let text = String::from_utf8(body).expect("utf8");
    assert!(text.starts_with(
        "# TYPE radar_ng_forecast_requests_total counter\nradar_ng_forecast_requests_total 0\n"
    ));
    assert!(text.contains(
        "radar_ng_http_requests_total{method=\"GET\",path=\"/api/livez\",status=\"200\"} 1"
    ));
    assert!(text.ends_with('\n'));
}

fn write_skill_log(env: &Env, anchors: usize) {
    let scored = now_iso();
    let entries: Vec<Value> = (0..anchors)
        .map(|i| {
            json!({"anchor": format!("a{i}"), "lead_minutes": 30, "scored_at": scored,
                        "thresholds": {"rain": {"hits": 1, "misses": 0, "false_alarms": 0}}})
        })
        .collect();
    write_json(
        &env.path("state/nowcast-skill.json"),
        &json!({"updated_at": scored, "window_hours": 24, "entries": entries}),
    );
}

#[tokio::test]
async fn metrics_reuse_the_skill_snapshot_until_the_log_changes() {
    let env = Env::new();
    write_skill_log(&env, 2);
    let app = env.app(|_| {});
    let runs = |text: &str| {
        text.lines()
            .find_map(|l| l.strip_prefix("radar_ng_nowcast_skill_runs "))
            .map(str::to_string)
    };
    let (_, _, body) = send(&app, get_req("/api/metrics")).await;
    assert_eq!(
        runs(&String::from_utf8(body).expect("utf8")).as_deref(),
        Some("2")
    );
    // A rewrite (new size and inode) is picked up on the next scrape, with no TTL wait.
    write_skill_log(&env, 3);
    let (_, _, body) = send(&app, get_req("/api/metrics")).await;
    assert_eq!(
        runs(&String::from_utf8(body).expect("utf8")).as_deref(),
        Some("3")
    );
    let (_, _, skill) = get_json(&app, "/api/nowcast/skill").await;
    assert_eq!(skill["runs_scored"], json!(3));
}

#[tokio::test]
async fn manifest_is_etagged_and_304s_on_a_match() {
    let env = Env::new();
    write_json(&env.path("state/manifest.json"), &manifest(None, None));
    let app = env.app(|_| {});
    let (status, headers, _) = send(&app, get_req("/api/manifest.json")).await;
    assert_eq!(status, StatusCode::OK);
    let etag = header_str(&headers, "etag").to_string();
    assert!(
        etag.starts_with('"') && etag.ends_with('"') && etag.len() == 26,
        "{etag}"
    );
    assert_eq!(header_str(&headers, "cache-control"), "public, max-age=15");
    let req = Request::builder()
        .uri("/api/manifest.json")
        .header(header::IF_NONE_MATCH, format!("W/{etag}"))
        .body(Body::empty())
        .expect("req");
    let (status304, _, body304) = send(&app, req).await;
    assert_eq!(status304, StatusCode::NOT_MODIFIED);
    assert!(body304.is_empty());
}

#[tokio::test]
async fn health_is_503_with_reasons_on_an_empty_state_directory() {
    let env = Env::new();
    let app = env.app(|_| {});
    let (status, headers, body) = get_json(&app, "/api/health").await;
    assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE);
    assert!(headers.get(header::CACHE_CONTROL).is_none());
    assert_eq!(body["reasons"], json!(["no_mrms_tiles"]));
    assert_eq!(body["mrms_age_s"], Value::Null);
    assert_eq!(
        body["nowcast"],
        json!({"status": "unknown", "reason": "no_status"})
    );
    assert_eq!(body["nowcast_skill"]["available"], json!(false));
    assert!(body["checked_at"].is_string());
}

#[tokio::test]
async fn health_is_200_for_a_fresh_radar_frame() {
    let env = Env::new();
    write_json(
        &env.path("state/manifest.json"),
        &manifest(Some(&now_iso()), None),
    );
    let app = env.app(|_| {});
    let (status, _, body) = get_json(&app, "/api/health").await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["reasons"], json!([]));
    assert_eq!(body["mrms_age_s"], json!(0));
    assert!(body["tiles_disk"]["total_bytes"].as_u64().unwrap_or(0) > 0);
}

// ---------------------------------------------------------------------------------------------
// Static lane, imagery and vector tiles
// ---------------------------------------------------------------------------------------------

#[tokio::test]
async fn tiles_are_immutable_and_errors_are_overridden_to_no_store_text() {
    let env = Env::new();
    let tile = env.path("tiles/radar/classic/2026-10-13T12:00:00+00:00/6/16/23.png");
    std::fs::create_dir_all(tile.parent().expect("parent")).expect("dirs");
    std::fs::write(&tile, b"png").expect("tile");
    let app = env.app(|_| {});
    let uri = "/tiles/radar/classic/2026-10-13T12:00:00+00:00/6/16/23.png";
    let (status, headers, body) = send(&app, get_req(uri)).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body, b"png");
    assert_eq!(
        header_str(&headers, "cache-control"),
        "public, max-age=86400, immutable"
    );
    assert_eq!(header_str(&headers, "access-control-allow-origin"), "*");

    let (miss, miss_headers, miss_body) =
        send(&app, get_req("/tiles/radar/classic/none/6/1/1.png")).await;
    assert_eq!(miss, StatusCode::NOT_FOUND);
    assert_eq!(header_str(&miss_headers, "cache-control"), "no-store");
    assert_eq!(
        header_str(&miss_headers, "content-type"),
        "text/plain; charset=utf-8"
    );
    assert_eq!(miss_body, b"404 Not Found");
}

#[tokio::test]
async fn missing_tile_in_a_complete_pyramid_is_a_cacheable_204() {
    let env = Env::new();
    let root = env.path("tiles/radar/classic/2026-10-13T12:00:00+00:00");
    std::fs::create_dir_all(root.join("6/16")).expect("dirs");
    std::fs::write(root.join("6/16/23.png"), b"png").expect("tile");
    std::fs::write(root.join(".render-complete-v2.json"), b"{}").expect("marker");
    let app = env.app(|_| {});

    // Clients percent-encode the timestamp segment.
    for uri in [
        "/tiles/radar/classic/2026-10-13T12:00:00+00:00/6/17/23.png",
        "/tiles/radar/classic/2026-10-13T12%3A00%3A00%2B00%3A00/6/17/23.png",
    ] {
        let (status, headers, body) = send(&app, get_req(uri)).await;
        assert_eq!(status, StatusCode::NO_CONTENT, "{uri}");
        assert!(body.is_empty());
        assert_eq!(
            header_str(&headers, "cache-control"),
            "public, max-age=86400, immutable"
        );
        assert_eq!(header_str(&headers, "access-control-allow-origin"), "*");
    }

    // No marker yet (still publishing) or not a tile path: the no-store 404 stays.
    let pending = env.path("tiles/radar/classic/2026-10-13T12:02:00+00:00/6/16");
    std::fs::create_dir_all(&pending).expect("dirs");
    for uri in [
        "/tiles/radar/classic/2026-10-13T12:02:00+00:00/6/17/23.png",
        "/tiles/radar/classic/2026-10-13T12:00:00+00:00/6/17/x.png",
        "/tiles/radar/classic/2026-10-13T12:00:00+00:00/../2026-10-13T12:00:00+00:00/6/17/23.png",
    ] {
        let (status, headers, _) = send(&app, get_req(uri)).await;
        assert_eq!(status, StatusCode::NOT_FOUND, "{uri}");
        assert_eq!(header_str(&headers, "cache-control"), "no-store", "{uri}");
    }
}

#[tokio::test]
async fn packed_bucket_tiles_are_immutable_and_missing_ones_are_204_once_the_bucket_file_exists() {
    let env = Env::new();
    let bucket = env.path("tiles/packed/nowcast/2026-10-15T18:30:00+00:00");
    std::fs::create_dir_all(bucket.join("5/7/11")).expect("dirs");
    std::fs::write(bucket.join("5/7/11.png"), b"png").expect("tile");
    let app = env.app(|_| {});

    // Present tile: served with the tile cache headers, with or without percent-encoding or `?v=`.
    for uri in [
        "/tiles/packed/nowcast/2026-10-15T18:30:00+00:00/5/7/11.png?v=3",
        "/tiles/packed/nowcast/2026-10-15T18%3A30%3A00%2B00%3A00/5/7/11.png?v=3",
    ] {
        let (status, headers, body) = send(&app, get_req(uri)).await;
        assert_eq!(status, StatusCode::OK, "{uri}");
        assert_eq!(body, b"png");
        assert_eq!(
            header_str(&headers, "cache-control"),
            "public, max-age=86400, immutable"
        );
        assert_eq!(header_str(&headers, "access-control-allow-origin"), "*");
    }

    // Missing tile before the bucket file is written: the no-store 404.
    let missing = "/tiles/packed/nowcast/2026-10-15T18%3A30%3A00%2B00%3A00/5/8/11.png?v=3";
    let (status, headers, _) = send(&app, get_req(missing)).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    assert_eq!(header_str(&headers, "cache-control"), "no-store");

    // With the bucket file, a missing tile is a cacheable empty 204.
    std::fs::write(
        bucket.join(".packed-bucket-v1.json"),
        br#"{"v":3,"slots":[]}"#,
    )
    .expect("bucket file");
    let (status, headers, body) = send(&app, get_req(missing)).await;
    assert_eq!(status, StatusCode::NO_CONTENT);
    assert!(body.is_empty());
    assert_eq!(
        header_str(&headers, "cache-control"),
        "public, max-age=86400, immutable"
    );
    // A bucket that does not exist stays a 404.
    let (status, _, _) = send(
        &app,
        get_req("/tiles/packed/nowcast/2026-10-15T18:35:00+00:00/5/8/11.png"),
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn imagery_outside_the_regexp_is_a_bare_404_without_headers() {
    let env = Env::new();
    let app = env.app(|_| {});
    for uri in [
        "/basemap/imagery/3/x/1.jpg",
        "/basemap/imagery/17/1/1.jpg",
        "/basemap/imagery/3/1/1.png",
    ] {
        let (status, headers, body) = send(&app, get_req(uri)).await;
        assert_eq!(status, StatusCode::NOT_FOUND, "{uri}");
        assert!(headers.get(header::CACHE_CONTROL).is_none(), "{uri}");
        assert!(body.is_empty(), "{uri}");
    }
}

#[tokio::test]
async fn imagery_proxy_rewrites_to_zyx_and_caches_bytes() {
    let hits = counter();
    let usgs_hits = hits.clone();
    let usgs = Router::new().route(
        "/arcgis/rest/services/USGSImageryOnly/MapServer/tile/{z}/{y}/{x}",
        get(move |AxPath((z, y, x)): AxPath<(String, String, String)>| {
            let hits = usgs_hits.clone();
            async move {
                hits.fetch_add(1, Ordering::SeqCst);
                if x == "99" {
                    return (StatusCode::NOT_FOUND, "nope").into_response_boxed();
                }
                let body = format!("JPEG-{z}-{y}-{x}");
                ([(header::CONTENT_TYPE, "image/jpeg")], body).into_response_boxed()
            }
        }),
    );
    let base = mock(usgs).await;
    let env = Env::new();
    let app = env.app(|c| c.usgs_imagery_base = base.clone());

    let (status, headers, body) = send(&app, get_req("/basemap/imagery/3/2/1.jpg")).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body, b"JPEG-3-1-2", "ArcGIS order is z/y/x");
    assert_eq!(
        header_str(&headers, "cache-control"),
        "public, max-age=604800"
    );
    assert_eq!(header_str(&headers, "access-control-allow-origin"), "*");
    let (again, _, again_body) = send(&app, get_req("/basemap/imagery/3/2/1.jpg")).await;
    assert_eq!(
        (again, again_body),
        (StatusCode::OK, b"JPEG-3-1-2".to_vec())
    );
    assert_eq!(
        hits.load(Ordering::SeqCst),
        1,
        "second request is served from the cache"
    );

    let (missing, missing_headers, missing_body) =
        send(&app, get_req("/basemap/imagery/3/99/1.jpg")).await;
    assert_eq!(missing, StatusCode::NOT_FOUND);
    assert_eq!(header_str(&missing_headers, "cache-control"), "no-store");
    assert_eq!(missing_body, b"404 Not Found");
}

/// Small helper so handlers with different body types return one type.
trait IntoBoxed {
    fn into_response_boxed(self) -> axum::response::Response;
}

impl<T: axum::response::IntoResponse> IntoBoxed for T {
    fn into_response_boxed(self) -> axum::response::Response {
        self.into_response()
    }
}

#[tokio::test]
async fn vector_tiles_come_from_the_pmtiles_archive() {
    let env = Env::new();
    let archive = env.path("basemap.pmtiles");
    {
        let file = std::fs::File::create(&archive).expect("archive");
        let mut writer = PmTilesWriter::new(TileType::Mvt)
            .tile_compression(Compression::None)
            .create(file)
            .expect("writer");
        writer
            .add_tile(TileCoord::new(0, 0, 0).expect("coord"), b"mvt-bytes")
            .expect("add");
        writer.finalize().expect("finalize");
    }
    let app = env.app(|_| {});
    let (status, headers, body) = send(&app, get_req("/basemap/tiles/0/0/0.mvt")).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body, b"mvt-bytes");
    assert_eq!(
        header_str(&headers, "content-type"),
        "application/x-protobuf"
    );
    assert_eq!(
        header_str(&headers, "cache-control"),
        "public, max-age=86400"
    );
    assert_eq!(header_str(&headers, "access-control-allow-origin"), "*");
    assert!(headers.get(header::CONTENT_ENCODING).is_none());

    let (empty, _, empty_body) = send(&app, get_req("/basemap/tiles/1/0/0.mvt")).await;
    assert_eq!(empty, StatusCode::NO_CONTENT);
    assert!(empty_body.is_empty());
}

#[tokio::test]
async fn style_rewrites_relative_urls_and_rejects_bad_names() {
    let env = Env::new();
    write_json(
        &env.path("styles/positron.json"),
        &json!({
            "version": 8,
            "sources": {"base": {"type": "vector", "tiles": ["/basemap/tiles/{z}/{x}/{y}.mvt"]}},
            "glyphs": "/basemap/fonts/{fontstack}/{range}.pbf",
            "sprite": "https://cdn.example/sprite",
        }),
    );
    let app = env.app(|_| {});
    let req = Request::builder()
        .uri("/api/basemap/style/positron")
        .header("x-forwarded-proto", "https")
        .header(header::HOST, "radar.example")
        .body(Body::empty())
        .expect("req");
    let (status, headers, body) = send(&app, req).await;
    assert_eq!(status, StatusCode::OK);
    assert!(headers.get(header::CACHE_CONTROL).is_none());
    assert!(headers.get(header::ACCESS_CONTROL_ALLOW_ORIGIN).is_none());
    let style: Value = serde_json::from_slice(&body).expect("json");
    assert_eq!(
        style["sources"]["base"]["tiles"][0],
        json!("https://radar.example/basemap/tiles/{z}/{x}/{y}.mvt")
    );
    assert_eq!(
        style["glyphs"],
        json!("https://radar.example/basemap/fonts/{fontstack}/{range}.pbf")
    );
    assert_eq!(style["sprite"], json!("https://cdn.example/sprite"));

    let (bad, _, bad_body) = get_json(&app, "/api/basemap/style/a..b").await;
    assert_eq!(bad, StatusCode::BAD_REQUEST);
    assert_eq!(bad_body, json!({"detail": "invalid style name"}));
    let (missing, _, missing_body) = get_json(&app, "/api/basemap/style/missing").await;
    assert_eq!(missing, StatusCode::NOT_FOUND);
    assert_eq!(missing_body, json!({"detail": "style not found"}));
    let (encoded, _, encoded_body) = get_json(&app, "/api/basemap/style/a%2Fb").await;
    assert_eq!(encoded, StatusCode::NOT_FOUND);
    assert_eq!(encoded_body, json!({"detail": "Not Found"}));
}

// ---------------------------------------------------------------------------------------------
// Storm prefetch
// ---------------------------------------------------------------------------------------------

#[tokio::test]
async fn storm_prefetch_validates_in_order_and_returns_the_empty_plan() {
    let env = Env::new();
    let app = env.app(|_| {});
    let (s1, _, b1) = get_json(&app, "/api/storm-prefetch?lat=91&lon=0").await;
    assert_eq!(s1, StatusCode::UNPROCESSABLE_ENTITY);
    assert_eq!(b1, json!({"detail": "lat/lon out of range"}));
    let (s2, _, b2) = get_json(&app, "/api/storm-prefetch?lat=42&lon=-85&zoom=9").await;
    assert_eq!(
        (s2, b2),
        (
            StatusCode::UNPROCESSABLE_ENTITY,
            json!({"detail": "zoom must be between 4 and 8"})
        )
    );
    let (s3, _, b3) = get_json(&app, "/api/storm-prefetch?lat=42&lon=-85&palette=neon").await;
    assert_eq!(
        (s3, b3),
        (
            StatusCode::UNPROCESSABLE_ENTITY,
            json!({"detail": "unsupported palette"})
        )
    );
    let (s4, h4, b4) = get_json(&app, "/api/storm-prefetch?lat=42&lon=-85").await;
    assert_eq!(s4, StatusCode::OK);
    assert_eq!(header_str(&h4, "cache-control"), "public, max-age=30");
    assert_eq!(
        b4,
        json!({"plan_id": null, "storm_cell_id": null, "bboxes": [], "tile_urls": []})
    );
}

#[tokio::test]
async fn style_json_uses_the_manifest_frame_path_for_run_scoped_nowcast() {
    // D1: the nowcast frame path includes the run, and the style must use it, not the bare timestamp.
    let env = Env::new();
    let valid = "2026-10-13T12:05:00+00:00";
    write_json(
        &env.path("state/manifest.json"),
        &manifest(None, Some(("2026-10-13T12:00:00+00:00", valid))),
    );
    let app = env.app(|_| {});
    let uri = format!(
        "/api/storm-prefetch/style.json?layer=nowcast&palette=vivid&timestamp={}&zoom=5",
        valid.replace('+', "%2B").replace(':', "%3A")
    );
    let (status, headers, body) = get_json(&app, &uri).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(header_str(&headers, "cache-control"), "public, max-age=30");
    let tile = body["sources"]["storm-prefetch"]["tiles"][0]
        .as_str()
        .expect("tile");
    assert!(tile.starts_with("http://"), "{tile}");
    assert!(
        tile.contains("/tiles/nowcast/vivid/runs/2026-10-13T12:00:00+00:00/"),
        "{tile}"
    );
    assert_eq!(body["sources"]["storm-prefetch"]["minzoom"], json!(5));
    assert_eq!(body["sources"]["storm-prefetch"]["maxzoom"], json!(5));

    let (coverage, _, cov_body) = get_json(
        &app,
        &format!(
            "/api/storm-prefetch/style.json?layer=nowcast&palette=vivid&timestamp={}&zoom=7",
            "x"
        ),
    )
    .await;
    assert_eq!(coverage, StatusCode::UNPROCESSABLE_ENTITY);
    assert_eq!(cov_body, json!({"detail": "zoom outside layer coverage"}));
    let (ts, _, ts_body) = get_json(
        &app,
        "/api/storm-prefetch/style.json?layer=radar&palette=classic&timestamp=a%20b&zoom=5",
    )
    .await;
    assert_eq!(ts, StatusCode::UNPROCESSABLE_ENTITY);
    assert_eq!(ts_body, json!({"detail": "invalid timestamp"}));
}

// ---------------------------------------------------------------------------------------------
// Forecast, alerts, geocoding
// ---------------------------------------------------------------------------------------------

#[tokio::test]
async fn forecast_is_cached_per_cell_and_upstream_failures_are_502_without_cache_control() {
    let calls = counter();
    let om_calls = calls.clone();
    let open_meteo = Router::new()
        .route(
            "/v1/forecast",
            get(move || {
                let calls = om_calls.clone();
                async move {
                    calls.fetch_add(1, Ordering::SeqCst);
                    Json(open_meteo_body())
                }
            }),
        )
        .route("/v1/html", get(|| async { "<html>not json</html>" }));
    let base = mock(open_meteo).await;
    let env = Env::new();
    let app = env.app(|c| c.open_meteo_base = format!("{base}/v1/forecast"));

    let (status, headers, body) = get_json(&app, "/api/forecast/42.96/-85.67").await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body, open_meteo_body());
    assert_eq!(header_str(&headers, "cache-control"), "public, max-age=900");
    // Both points round (one decimal) to the cell 43.0,-85.7.
    let _ = get_json(&app, "/api/forecast/42.99/-85.72").await;
    assert_eq!(
        calls.load(Ordering::SeqCst),
        1,
        "both points round to the same cell"
    );

    let bad_app = env.app(|c| c.open_meteo_base = format!("{base}/v1/html"));
    let (bad, bad_headers, bad_body) = get_json(&bad_app, "/api/forecast/42.96/-85.67").await;
    assert_eq!(bad, StatusCode::BAD_GATEWAY);
    assert_eq!(bad_body, json!({"error": "upstream_unavailable"}));
    assert!(bad_headers.get(header::CACHE_CONTROL).is_none());

    let (range, _, range_body) = get_json(&app, "/api/forecast/91/0").await;
    assert_eq!(range, StatusCode::UNPROCESSABLE_ENTITY);
    assert_eq!(range_body, json!({"detail": "lat/lon out of range"}));
}

/// A fresh stack's Open-Meteo answers all-null until the first model sync; that body must not be
/// cached, or the forecast stays empty for `FORECAST_TTL_S` after the data lands.
#[tokio::test]
async fn forecast_without_data_is_served_no_store_and_not_cached() {
    let calls = counter();
    let om_calls = calls.clone();
    let open_meteo = Router::new().route(
        "/v1/forecast",
        get(move || {
            let calls = om_calls.clone();
            async move {
                let n = calls.fetch_add(1, Ordering::SeqCst);
                let mut body = open_meteo_body();
                if n == 0 {
                    body["current"]["temperature_2m"] = Value::Null;
                }
                Json(body)
            }
        }),
    );
    let base = mock(open_meteo).await;
    let env = Env::new();
    let app = env.app(|c| c.open_meteo_base = format!("{base}/v1/forecast"));

    let (status, headers, body) = get_json(&app, "/api/forecast/42.96/-85.67").await;
    assert_eq!(status, StatusCode::OK);
    let mut empty = open_meteo_body();
    empty["current"]["temperature_2m"] = Value::Null;
    assert_eq!(body, empty);
    assert_eq!(header_str(&headers, "cache-control"), "no-store");

    let (_, headers, body) = get_json(&app, "/api/forecast/42.96/-85.67").await;
    assert_eq!(body, open_meteo_body());
    assert_eq!(header_str(&headers, "cache-control"), "public, max-age=900");
    let _ = get_json(&app, "/api/forecast/42.96/-85.67").await;
    assert_eq!(
        calls.load(Ordering::SeqCst),
        2,
        "only the data-less answer is refetched"
    );
}

#[tokio::test]
async fn partial_forecast_is_refetched_until_current_fields_are_complete() {
    let fields = [
        "temperature_2m",
        "relative_humidity_2m",
        "apparent_temperature",
        "weather_code",
        "wind_speed_10m",
        "wind_direction_10m",
        "wind_gusts_10m",
        "dew_point_2m",
        "surface_pressure",
        "precipitation",
    ];
    for field in fields {
        for missing in [false, true] {
            let mut partial = open_meteo_body();
            if missing {
                partial["current"]
                    .as_object_mut()
                    .expect("current")
                    .remove(field);
            } else {
                partial["current"][field] = Value::Null;
            }
            let first = partial.clone();
            let calls = counter();
            let om_calls = calls.clone();
            let base = mock(Router::new().route(
                "/v1/forecast",
                get(move || {
                    let calls = om_calls.clone();
                    let first = first.clone();
                    async move {
                        Json(if calls.fetch_add(1, Ordering::SeqCst) == 0 {
                            first
                        } else {
                            open_meteo_body()
                        })
                    }
                }),
            ))
            .await;
            let env = Env::new();
            let app = env.app(|c| c.open_meteo_base = format!("{base}/v1/forecast"));
            let uri = "/api/forecast/42.96/-85.67";
            let (status, headers, body) = get_json(&app, uri).await;
            assert_eq!(status, StatusCode::OK);
            assert_eq!(body, partial);
            assert_eq!(
                header_str(&headers, "cache-control"),
                "no-store",
                "{field}, missing={missing}"
            );
            for _ in 0..2 {
                let (status, headers, body) = get_json(&app, uri).await;
                assert_eq!(status, StatusCode::OK);
                assert_eq!(body, open_meteo_body());
                assert_eq!(header_str(&headers, "cache-control"), "public, max-age=900");
            }
            assert_eq!(
                calls.load(Ordering::SeqCst),
                2,
                "partial bypasses cache; complete answer is cached"
            );
            let (_, _, metrics) = send(&app, get_req("/api/metrics")).await;
            let metrics = String::from_utf8(metrics).expect("utf8");
            assert!(
                metrics.contains("radar_ng_forecast_cache_hits_total 1\n"),
                "{metrics}"
            );
            assert!(
                metrics.contains("radar_ng_forecast_upstream_errors_total 0\n"),
                "{metrics}"
            );
        }
    }
}

#[tokio::test]
async fn concurrent_forecast_misses_for_one_cell_make_one_upstream_call() {
    let calls = counter();
    let om_calls = calls.clone();
    let open_meteo = Router::new().route(
        "/v1/forecast",
        get(move || {
            let calls = om_calls.clone();
            async move {
                calls.fetch_add(1, Ordering::SeqCst);
                tokio::time::sleep(Duration::from_millis(200)).await;
                Json(open_meteo_body())
            }
        }),
    );
    let base = mock(open_meteo).await;
    let env = Env::new();
    let app = env.app(|c| c.open_meteo_base = format!("{base}/v1/forecast"));
    // 42.96 and 42.99 round to the same cell.
    let requests = (0..32).map(|i| {
        let uri = if i % 2 == 0 {
            "/api/forecast/42.96/-85.67"
        } else {
            "/api/forecast/42.99/-85.72"
        };
        get_json(&app, uri)
    });
    let responses = join_all(requests).await;
    assert!(
        responses
            .iter()
            .all(|(s, _, b)| *s == StatusCode::OK && *b == open_meteo_body())
    );
    assert_eq!(calls.load(Ordering::SeqCst), 1);
    let (_, _, metrics) = send(&app, get_req("/api/metrics")).await;
    let text = String::from_utf8(metrics).expect("utf8");
    assert!(
        text.contains("radar_ng_forecast_cache_hits_total 31\n"),
        "{text}"
    );
}

fn get_with_encoding(uri: &str, accept: &str) -> Request<Body> {
    Request::builder()
        .uri(uri)
        .header(header::ACCEPT_ENCODING, accept)
        .body(Body::empty())
        .expect("request")
}

#[tokio::test]
async fn api_json_is_compressed_per_accept_encoding_and_identical_once_decoded() {
    let mut big = open_meteo_body();
    big["hourly"]["temperature_2m"] = json!(vec![61.5; 2000]);
    let body = big.clone();
    let open_meteo = Router::new().route(
        "/v1/forecast",
        get(move || {
            let body = body.clone();
            async move { Json(body) }
        }),
    );
    let base = mock(open_meteo).await;
    let env = Env::new();
    let app = env.app(|c| c.open_meteo_base = format!("{base}/v1/forecast"));
    let uri = "/api/forecast/42.96/-85.67";

    let (_, plain_headers, plain) = send(&app, get_req(uri)).await;
    assert!(plain_headers.get(header::CONTENT_ENCODING).is_none());
    assert_eq!(serde_json::from_slice::<Value>(&plain).expect("json"), big);

    let (status, headers, gz) = send(&app, get_with_encoding(uri, "gzip")).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(header_str(&headers, "content-encoding"), "gzip");
    assert_eq!(header_str(&headers, "vary"), "accept-encoding");
    assert_eq!(header_str(&headers, "cache-control"), "public, max-age=900");
    assert!(gz.len() < plain.len() / 10);
    let mut decoded = Vec::new();
    std::io::Read::read_to_end(&mut flate2::read::GzDecoder::new(&gz[..]), &mut decoded)
        .expect("gunzip");
    assert_eq!(decoded, plain, "byte-identical once decoded");

    let (_, br_headers, _) = send(&app, get_with_encoding(uri, "br;q=1.0, gzip;q=0.5")).await;
    assert_eq!(header_str(&br_headers, "content-encoding"), "br");

    // Small bodies are not worth compressing.
    let (_, small_headers, _) = send(&app, get_with_encoding("/api/livez", "gzip, br")).await;
    assert!(small_headers.get(header::CONTENT_ENCODING).is_none());
    assert!(small_headers.get(header::VARY).is_none());
}

#[tokio::test]
async fn tiles_are_never_recompressed() {
    let env = Env::new();
    let dir = env.path("tiles/radar/classic/2026-10-13T12:00:00+00:00/6/16");
    std::fs::create_dir_all(&dir).expect("dirs");
    let png = vec![7u8; 4096];
    std::fs::write(dir.join("23.png"), &png).expect("tile");
    let mvt = vec![9u8; 4096];
    std::fs::write(dir.join("24.mvt"), &mvt).expect("tile");
    std::fs::write(dir.join("24.mvt.gz"), b"precompressed").expect("gz tile");
    let app = env.app(|_| {});
    let base = "/tiles/radar/classic/2026-10-13T12:00:00+00:00/6/16";

    let (status, headers, body) = send(
        &app,
        get_with_encoding(&format!("{base}/23.png"), "gzip, br"),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert!(headers.get(header::CONTENT_ENCODING).is_none());
    assert!(headers.get(header::VARY).is_none());
    assert_eq!(body, png);

    // ServeDir's own precompressed .gz is passed through once, not compressed again.
    let (_, gz_headers, gz_body) = send(
        &app,
        get_with_encoding(&format!("{base}/24.mvt"), "gzip, br"),
    )
    .await;
    assert_eq!(header_str(&gz_headers, "content-encoding"), "gzip");
    assert_eq!(gz_body, b"precompressed");
    let (_, raw_headers, raw_body) =
        send(&app, get_with_encoding(&format!("{base}/24.mvt"), "br")).await;
    assert!(raw_headers.get(header::CONTENT_ENCODING).is_none());
    assert_eq!(raw_body, mvt);
}

/// An NWS mock: the national feed, `?point=` answers that echo the point, and `/points` that maps
/// one point to zones. Counts feed, point and `/points` calls.
async fn nws_mock(delay: Duration) -> (String, [Arc<AtomicUsize>; 3]) {
    let calls = [counter(), counter(), counter()];
    let (feed_calls, point_calls, zone_calls) =
        (calls[0].clone(), calls[1].clone(), calls[2].clone());
    let nws = Router::new()
        .route(
            "/alerts/active",
            get(move |Query(q): Query<HashMap<String, String>>| {
                let (feed_calls, point_calls) = (feed_calls.clone(), point_calls.clone());
                async move {
                    tokio::time::sleep(delay).await;
                    if let Some(point) = q.get("point") {
                        point_calls.fetch_add(1, Ordering::SeqCst);
                        return Json(json!({"type": "FeatureCollection", "features": [], "echo": point}));
                    }
                    feed_calls.fetch_add(1, Ordering::SeqCst);
                    Json(json!({
                        "@context": ["ctx"], "type": "FeatureCollection",
                        "features": [
                            {"id": "poly", "type": "Feature",
                             "geometry": {"type": "Polygon", "coordinates": [[[-86.0, 42.0], [-85.0, 42.0], [-85.0, 43.0], [-86.0, 43.0], [-86.0, 42.0]]]},
                             "properties": {"id": "poly", "status": "Actual", "event": "Severe Thunderstorm Warning",
                                            "description": "long", "affectedZones": ["zones/county/MIC081"]}},
                            {"id": "zone", "type": "Feature", "geometry": null,
                             "properties": {"id": "zone", "status": "Actual", "affectedZones": ["zones/forecast/MIZ057"]}},
                            {"id": "elsewhere", "type": "Feature", "geometry": null,
                             "properties": {"id": "elsewhere", "status": "Actual", "affectedZones": ["zones/forecast/ILZ078"]}},
                        ],
                        "title": "Current watches, warnings, and advisories", "updated": "2026-10-15T22:00:00+00:00",
                    }))
                }
            }),
        )
        .route(
            "/points/{point}",
            get(move |AxPath(point): AxPath<String>| {
                let zone_calls = zone_calls.clone();
                async move {
                    zone_calls.fetch_add(1, Ordering::SeqCst);
                    if point.starts_with("42.") || point.starts_with("43.") {
                        return Json(json!({"properties": {
                            "county": "zones/county/MIC081", "forecastZone": "zones/forecast/MIZ057",
                            "fireWeatherZone": "zones/fire/MIZ057"}}))
                        .into_response_boxed();
                    }
                    (StatusCode::NOT_FOUND, "Data Unavailable For Requested Point").into_response_boxed()
                }
            }),
        )
        .route("/broken/alerts/active", get(|| async { StatusCode::INTERNAL_SERVER_ERROR }));
    (mock(nws).await, calls)
}

fn feature_ids(body: &Value) -> Vec<&str> {
    body["features"]
        .as_array()
        .expect("features")
        .iter()
        .map(|f| f["id"].as_str().expect("id"))
        .collect()
}

#[tokio::test]
async fn alerts_match_points_from_one_shared_feed_and_fail_closed() {
    let (base, [feed_calls, point_calls, zone_calls]) = nws_mock(Duration::ZERO).await;
    let env = Env::new();
    let app = env.app(|c| {
        c.nws_point_alerts_url = format!("{base}/alerts/active");
        c.nws_points_url = format!("{base}/points");
    });

    // Inside the polygon and in the zone.
    let (status, headers, body) = get_json(&app, "/api/alerts?lat=42.9631&lon=-85.6684").await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(feature_ids(&body), vec!["poly", "zone"]);
    assert_eq!(body["@context"], json!(["ctx"]));
    assert_eq!(
        body["title"],
        json!("Current watches, warnings, and advisories for 42.963 N, 85.668 W")
    );
    assert_eq!(header_str(&headers, "cache-control"), "public, max-age=60");
    // Same zones, outside the polygon: the polygon warning is left out, as NWS's ?point= does.
    let (_, _, outside) = get_json(&app, "/api/alerts?lat=43.5&lon=-85.5").await;
    assert_eq!(feature_ids(&outside), vec!["zone"]);
    assert_eq!(
        feed_calls.load(Ordering::SeqCst),
        1,
        "one feed for every point"
    );
    assert_eq!(
        zone_calls.load(Ordering::SeqCst),
        2,
        "one zone lookup per point"
    );
    let _ = get_json(&app, "/api/alerts?lat=42.9631&lon=-85.6684").await;
    assert_eq!(zone_calls.load(Ordering::SeqCst), 2, "zones are cached");

    // No zones here (/points 404): NWS's own ?point= answer, with the 3-dp key.
    let (status, _, body) = get_json(&app, "/api/alerts?lat=25.0&lon=-80.1234").await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["echo"], json!("25.0,-80.123"));
    assert_eq!(point_calls.load(Ordering::SeqCst), 1);

    // The map comes from the same feed: polygons only, nine properties.
    let (map, _, map_body) = get_json(&app, "/api/alerts/map").await;
    assert_eq!(map, StatusCode::OK);
    let features = map_body["features"].as_array().expect("features");
    assert_eq!(features.len(), 1, "zone-only alerts are dropped");
    let props = features[0]["properties"].as_object().expect("props");
    assert_eq!(props.len(), 9);
    assert!(!props.contains_key("description"));
    assert_eq!(feed_calls.load(Ordering::SeqCst), 1);

    let broken_app = env.app(|c| {
        c.nws_point_alerts_url = format!("{base}/broken/alerts/active");
        c.nws_points_url = format!("{base}/points");
    });
    let (bad, bad_headers, bad_body) = get_json(&broken_app, "/api/alerts?lat=42&lon=-85").await;
    assert_eq!(bad, StatusCode::BAD_GATEWAY);
    assert_eq!(header_str(&bad_headers, "cache-control"), "no-store");
    assert_eq!(bad_body, json!({"error": "upstream_unavailable"}));
}

#[tokio::test]
async fn concurrent_alert_misses_make_one_upstream_call_per_key() {
    let (base, [feed_calls, point_calls, zone_calls]) = nws_mock(Duration::from_millis(200)).await;
    let env = Env::new();
    let app = env.app(|c| {
        c.nws_point_alerts_url = format!("{base}/alerts/active");
        c.nws_points_url = format!("{base}/points");
    });
    let covered = (0..16).map(|_| get_json(&app, "/api/alerts?lat=42.5&lon=-85.5"));
    let uncovered = (0..16).map(|_| get_json(&app, "/api/alerts?lat=25&lon=-80"));
    let (a, b) = tokio::join!(join_all(covered), join_all(uncovered));
    assert!(a.iter().chain(&b).all(|(s, _, _)| *s == StatusCode::OK));
    assert_eq!(feed_calls.load(Ordering::SeqCst), 1);
    assert_eq!(zone_calls.load(Ordering::SeqCst), 2);
    assert_eq!(point_calls.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn outlooks_proxy_spc_by_day_cache_and_fail_closed() {
    let hits = counter();
    let seen = hits.clone();
    let spc = Router::new().route(
        "/outlook/{file}",
        get(move |AxPath(file): AxPath<String>| {
            let seen = seen.clone();
            async move {
                seen.fetch_add(1, Ordering::SeqCst);
                if file != "day2otlk_cat.lyr.geojson" {
                    return StatusCode::NOT_FOUND.into_response_boxed();
                }
                Json(json!({"features": [
                    {"geometry": {"type": "Polygon", "coordinates": []},
                     "properties": {"LABEL": "ENH", "LABEL2": "Enhanced Risk", "fill": "#FFA366", "stroke": "#FF6600"}}
                ]}))
                .into_response_boxed()
            }
        }),
    );
    let base = mock(spc).await;
    let env = Env::new();
    let app = env.app(|c| c.spc_outlook_base = format!("{base}/outlook"));

    for _ in 0..2 {
        let (status, headers, body) = get_json(&app, "/api/outlooks?day=2").await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(header_str(&headers, "cache-control"), "public, max-age=600");
        assert_eq!(body["features"][0]["properties"]["label"], json!("ENH"));
    }
    assert_eq!(
        hits.load(Ordering::SeqCst),
        1,
        "second request is served from the cache"
    );

    let (bad, bad_headers, _) = get_json(&app, "/api/outlooks?day=1").await;
    assert_eq!(bad, StatusCode::BAD_GATEWAY);
    assert_eq!(header_str(&bad_headers, "cache-control"), "no-store");
    let (invalid, _, _) = get_json(&app, "/api/outlooks?day=4").await;
    assert_eq!(invalid, StatusCode::UNPROCESSABLE_ENTITY);
}

#[tokio::test]
async fn geocode_is_503_when_unconfigured_and_filters_photon_to_settlements() {
    let env = Env::new();
    let unconfigured = env.app(|_| {});
    let (status, headers, body) = get_json(&unconfigured, "/api/geocode?q=Grand").await;
    assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE);
    assert_eq!(header_str(&headers, "cache-control"), "no-store");
    assert_eq!(body, json!({"error": "geocoding_not_configured"}));

    let photon = Router::new().route(
        "/api",
        get(|| async {
            Json(json!({"features": [
                {"geometry": {"coordinates": [-85.66, 42.96]},
                 "properties": {"osm_key": "place", "osm_value": "city", "name": "Grand Rapids", "osm_id": 1,
                                "state": "Michigan", "country": "United States", "countrycode": "us"}},
                {"geometry": {"coordinates": [-85.6, 42.9]},
                 "properties": {"osm_key": "highway", "osm_value": "primary", "name": "Main", "osm_id": 2}},
                {"geometry": {"coordinates": [-85.1, 42.2]},
                 "properties": {"osm_key": "place", "osm_value": "town", "name": "Ada", "osm_id": "abc"}},
                {"geometry": {"coordinates": [-85.5, 42.3]},
                 "properties": {"osm_key": "place", "osm_value": "town", "name": "Ada", "osm_id": 3,
                                "countrycode": "us"}},
            ]}))
        }),
    );
    let base = mock(photon).await;
    let app = env.app(|c| c.photon_url = base.clone());
    let (ok, _, body) = get_json(&app, "/api/geocode?q=grand%20%20rapids&limit=8").await;
    assert_eq!(ok, StatusCode::OK);
    let ids: Vec<i64> = body["results"]
        .as_array()
        .expect("results")
        .iter()
        .map(|r| r["id"].as_i64().expect("id"))
        .collect();
    assert_eq!(
        ids,
        vec![1, 3],
        "street dropped; non-integer osm_id dropped (D8)"
    );
    assert_eq!(body["results"][1]["countryCode"], json!("US"));

    let (cut, _, cut_body) = get_json(&app, "/api/geocode?q=grand&limit=1").await;
    assert_eq!(cut, StatusCode::OK);
    assert_eq!(cut_body["results"].as_array().expect("results").len(), 1);

    let (invalid, _, _) = get_json(&app, "/api/geocode?q=g&limit=0").await;
    assert_eq!(invalid, StatusCode::UNPROCESSABLE_ENTITY);
}

#[tokio::test]
async fn reverse_geocode_checks_config_before_range() {
    let env = Env::new();
    let unconfigured = env.app(|_| {});
    let (status, _, body) = get_json(&unconfigured, "/api/reverse-geocode?lat=91&lon=0").await;
    assert_eq!(
        status,
        StatusCode::SERVICE_UNAVAILABLE,
        "config check comes before the range check"
    );
    assert_eq!(body, json!({"error": "geocoding_not_configured"}));

    let photon = Router::new().route(
        "/reverse",
        get(|| async {
            Json(json!({"features": [
                {"geometry": {"coordinates": [-85.6684, 42.96314]},
                 "properties": {"osm_key": "highway", "city": "Grand Rapids", "osm_id": 9, "state": "Michigan"}}
            ]}))
        }),
    );
    let base = mock(photon).await;
    let app = env.app(|c| c.photon_url = base.clone());
    let (ok, headers, body) =
        get_json(&app, "/api/reverse-geocode?lat=42.96314&lon=-85.66842").await;
    assert_eq!(ok, StatusCode::OK);
    assert_eq!(
        header_str(&headers, "cache-control"),
        "public, max-age=3600"
    );
    assert_eq!(body["place"]["name"], json!("Grand Rapids"));
    assert_eq!(body["place"]["latitude"], json!(42.963));
    assert_eq!(body["place"]["longitude"], json!(-85.668));
}

// ---------------------------------------------------------------------------------------------
// Briefing and explainer (LLM mock)
// ---------------------------------------------------------------------------------------------

#[tokio::test]
async fn briefing_and_explainer_use_the_llm_in_json_mode() {
    let llm_calls = counter();
    let llm_seen = llm_calls.clone();
    let alert_hits = counter();
    let alert_seen = alert_hits.clone();
    let upstream = Router::new()
        .route("/v1/forecast", get(|| async { Json(open_meteo_body()) }))
        .route(
            "/v1/chat/completions",
            axum::routing::post(move |Json(req): Json<Value>| {
                let llm_seen = llm_seen.clone();
                async move {
                    llm_seen.fetch_add(1, Ordering::SeqCst);
                    if req["response_format"]["type"] != json!("json_object") {
                        return (StatusCode::BAD_REQUEST, Json(json!({}))).into_response_boxed();
                    }
                    let user = req["messages"][1]["content"].as_str().unwrap_or("");
                    let content = if user.contains("\"what\"") || user.contains("instruction") {
                        "<think>scratch</think>{\"what\": \"Flooding in Kent County.\", \"do\": \"Avoid low roads.\"}"
                    } else {
                        "<think>scratch</think> {\"headline\": \"Rain soon\", \"body\": \"Light rain by 2 PM, gone by 3:30.\"}"
                    };
                    Json(json!({"choices": [{"message": {"content": content}}]})).into_response_boxed()
                }
            }),
        )
        .route(
            "/alerts/{id}",
            get(move |AxPath(_id): AxPath<String>| {
                let alert_seen = alert_seen.clone();
                async move {
                    alert_seen.fetch_add(1, Ordering::SeqCst);
                    Json(json!({"properties": {"event": "Flood Warning", "areaDesc": "Kent County",
                        "headline": "Flood", "description": "River rising.", "instruction": "Move to high ground.",
                        "ends": "2026-10-14T00:00:00+00:00"}}))
                }
            }),
        );
    let base = mock(upstream).await;
    let env = Env::new();
    let app = env.app(|c| {
        c.open_meteo_base = format!("{base}/v1/forecast");
        c.llm_base_url = format!("{base}/v1");
        c.llm_model = "test-model".into();
        c.nws_alert_detail_base = format!("{base}/alerts");
    });

    let (status, headers, body) = get_json(
        &app,
        "/api/briefing?lat=42.9&lon=-85.6&place=Grand%20Rapids",
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["available"], json!(true));
    assert_eq!(body["headline"], json!("Rain soon"));
    assert_eq!(body["body"], json!("Light rain by 2 PM, gone by 3:30."));
    assert_eq!(body["model"], json!("test-model"));
    assert_eq!(header_str(&headers, "cache-control"), "public, max-age=120");
    assert_eq!(llm_calls.load(Ordering::SeqCst), 1);

    let (cached, _, _) = get_json(
        &app,
        "/api/briefing?lat=42.9&lon=-85.6&place=Grand%20Rapids",
    )
    .await;
    assert_eq!(cached, StatusCode::OK);
    assert_eq!(
        llm_calls.load(Ordering::SeqCst),
        1,
        "the second request is a cache hit"
    );

    let id = "urn:oid:2.49.0.1.840.0.abcdef";
    let (explain, eh, ebody) = get_json(&app, &format!("/api/alerts/explain?id={id}")).await;
    assert_eq!(explain, StatusCode::OK, "{ebody}");
    assert_eq!(ebody["what"], json!("Flooding in Kent County."));
    assert_eq!(ebody["do"], json!("Avoid low roads."));
    assert_eq!(header_str(&eh, "cache-control"), "public, max-age=600");
    assert_eq!(alert_hits.load(Ordering::SeqCst), 1);

    let (bad_id, _, bad_body) =
        get_json(&app, "/api/alerts/explain?id=https://evil.example/x").await;
    assert_eq!(bad_id, StatusCode::OK);
    assert_eq!(bad_body, json!({"available": false, "reason": "bad_id"}));
    let (short, _, _) = get_json(&app, "/api/alerts/explain?id=urn:oid").await;
    assert_eq!(short, StatusCode::UNPROCESSABLE_ENTITY);
}

#[tokio::test]
async fn briefing_reports_not_configured_without_an_llm() {
    let env = Env::new();
    let app = env.app(|_| {});
    let (status, headers, body) = get_json(&app, "/api/briefing?lat=42.9&lon=-85.6").await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(header_str(&headers, "cache-control"), "no-store");
    assert_eq!(
        body,
        json!({"available": false, "reason": "not_configured"})
    );
}

// ---------------------------------------------------------------------------------------------
// Grids, nowcast and wind
// ---------------------------------------------------------------------------------------------

fn write_f32_grid(dir: &Path, name: &str, values: &[f32], height: usize, width: usize) {
    std::fs::create_dir_all(dir).expect("dir");
    let mut bytes = Vec::new();
    for v in values {
        bytes.extend_from_slice(&v.to_le_bytes());
    }
    std::fs::write(dir.join(format!("{name}.bin")), bytes).expect("bin");
    write_json(
        &dir.join(format!("{name}.meta.json")),
        &json!({"height": height, "width": width, "lat_min": 40.0, "lat_max": 42.0,
                "lon_min": -86.0, "lon_max": -83.0, "unit": "dBZ", "fill": null, "stride": 1,
                "data_file": format!("{name}.bin")}),
    );
}

#[tokio::test]
async fn inspector_samples_a_grid_and_soft_fails() {
    let env = Env::new();
    let values: Vec<f32> = (0..3)
        .flat_map(|r| (0..4).map(move |c| (r * 10 + c) as f32))
        .collect();
    write_f32_grid(&env.path("grid/x"), "g", &values, 3, 4);
    let app = env.app(|_| {});
    let (status, _, body) = get_json(&app, "/api/inspect/x/g/42.0/-86.0").await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["value"], json!(0.0));
    assert_eq!(body["unit"], json!("dBZ"));
    let (far, _, far_body) = get_json(&app, "/api/inspect/x/g/44.0/-85.0").await;
    assert_eq!(
        (far, far_body["reason"].clone()),
        (StatusCode::OK, json!("out_of_bounds"))
    );
    let (missing, _, missing_body) = get_json(&app, "/api/inspect/x/nope/42.0/-86.0").await;
    assert_eq!(
        (missing, missing_body["reason"].clone()),
        (StatusCode::OK, json!("grid_missing"))
    );
}

#[tokio::test]
async fn wind_field_pairs_u_and_v_and_reports_missing_grids() {
    let env = Env::new();
    let ts = "2026-10-13T12:00:00+00:00";
    write_f32_grid(&env.path("grid/wind_u"), ts, &[1.0, 2.0, 3.0, 4.0], 2, 2);
    write_f32_grid(&env.path("grid/wind_v"), ts, &[4.0, 3.0, 2.0, 1.0], 2, 2);
    let app = env.app(|_| {});
    let (status, headers, body) = get_json(&app, &format!("/api/wind-field/{ts}")).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["ok"], json!(true));
    assert_eq!(body["u"].as_array().expect("u").len(), 4);
    assert_eq!(header_str(&headers, "cache-control"), "public, max-age=300");
    let (latest, _, latest_body) = get_json(&app, "/api/wind-field/latest").await;
    assert_eq!(latest, StatusCode::OK);
    assert_eq!(latest_body["timestamp"], json!(ts));

    let empty = env.app(|c| c.grid_dir = env.path("nothing-here"));
    let (none, none_headers, none_body) = get_json(&empty, "/api/wind-field/latest").await;
    assert_eq!(none, StatusCode::OK);
    assert_eq!(
        none_body,
        json!({"ok": false, "reason": "grid_missing", "timestamp": "latest"})
    );
    assert!(none_headers.get(header::CACHE_CONTROL).is_none());
}

#[tokio::test]
async fn nowcast_point_reports_the_status_file_when_no_frames_exist() {
    let env = Env::new();
    write_json(&env.path("state/manifest.json"), &manifest(None, None));
    write_json(
        &env.path("state/nowcast-status.json"),
        &json!({"status": "unavailable", "reason": "no_frames", "detail": "warming up"}),
    );
    let app = env.app(|_| {});
    let (status, headers, body) = get_json(&app, "/api/nowcast/42.9/-85.6").await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["status"], json!("unavailable"));
    assert_eq!(body["reason"], json!("no_frames"));
    assert_eq!(header_str(&headers, "cache-control"), "public, max-age=15");
    let (range, _, range_body) = get_json(&app, "/api/nowcast/91/0").await;
    assert_eq!(range, StatusCode::UNPROCESSABLE_ENTITY);
    assert_eq!(range_body, json!({"detail": "lat/lon out of range"}));
}

#[tokio::test]
async fn expired_nowcast_is_unavailable_even_when_its_point_is_cached() {
    let env = Env::new();
    let run = "2020-01-01T00:00:00+00:00";
    let valid = "2020-01-01T01:00:00+00:00";
    write_json(
        &env.path("state/manifest.json"),
        &manifest(Some(&now_iso()), Some((run, valid))),
    );
    write_json(
        &env.path("state/nowcast-status.json"),
        &json!({"status": "ok", "updated_at": run}),
    );
    let state = AppState::new(env.config(|_| {})).unwrap();
    state.nowcast_cache.insert(
        (run.into(), "42.9".into(), "-85.6".into()),
        serde_json::to_vec(&json!({"status": "ok", "points": [{"precipitation_mm_h": 0.0}]}))
            .expect("json")
            .into(),
    );
    let app = router(state);
    let (status, _, body) = get_json(&app, "/api/nowcast/42.9/-85.6").await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["status"], "unavailable");
    assert_eq!(body["reason"], "nowcast_expired");
    assert_eq!(body["points"], json!([]));
    let (status, _, body) = get_json(&app, "/api/health").await;
    assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE);
    assert_eq!(body["reasons"], json!(["nowcast_expired"]));
}

#[tokio::test]
async fn unexpired_nowcast_keeps_its_cached_point_and_healthy_status() {
    let env = Env::new();
    let run = now_iso();
    let valid = (Utc::now() + chrono::Duration::minutes(30)).to_rfc3339();
    write_json(
        &env.path("state/manifest.json"),
        &manifest(Some(&run), Some((&run, &valid))),
    );
    let state = AppState::new(env.config(|_| {})).unwrap();
    state.nowcast_cache.insert(
        (run, "42.9".into(), "-85.6".into()),
        serde_json::to_vec(&json!({"status": "ok", "points": [{"precipitation_mm_h": 1.0}]}))
            .expect("json")
            .into(),
    );
    let app = router(state);
    let (_, _, body) = get_json(&app, "/api/nowcast/42.9/-85.6").await;
    assert_eq!(body["status"], "ok");
    assert_eq!(body["points"][0]["precipitation_mm_h"], 1.0);
    let (status, _, _) = get_json(&app, "/api/health").await;
    assert_eq!(status, StatusCode::OK);
}

// ---------------------------------------------------------------------------------------------
// Workflow routes
// ---------------------------------------------------------------------------------------------

const WF_KEY: &[u8] = b"workflow-test-key";
const FAR_FUTURE: &str = "4102444800";

fn bearer(sub: &str) -> String {
    format!(
        "Bearer {}",
        token(WF_KEY, &format!(r#"{{"sub":"{sub}","exp":{FAR_FUTURE}}}"#))
    )
}

fn request_with(method: &str, uri: &str, auth: Option<&str>, body: Option<Value>) -> Request<Body> {
    let mut builder = Request::builder().method(method).uri(uri);
    if let Some(a) = auth {
        builder = builder.header(header::AUTHORIZATION, a);
    }
    let body = match body {
        Some(v) => {
            builder = builder.header(header::CONTENT_TYPE, "application/json");
            Body::from(serde_json::to_vec(&v).expect("json"))
        }
        None => Body::empty(),
    };
    builder.body(body).expect("request")
}

#[tokio::test]
async fn workflow_routes_authenticate_before_anything_else() {
    let env = Env::new();
    let keyed = env.app(|c| c.workflow_signing_key = WF_KEY.to_vec());
    let (none, _, none_body) = send(
        &keyed,
        request_with("POST", "/v1/push-tokens", None, Some(json!({}))),
    )
    .await;
    assert_eq!(none, StatusCode::UNAUTHORIZED);
    assert_eq!(
        serde_json::from_slice::<Value>(&none_body).expect("json"),
        json!({"detail": "missing bearer token"})
    );

    let unkeyed = env.app(|_| {});
    let (unset, _, unset_body) = send(
        &unkeyed,
        request_with(
            "POST",
            "/v1/push-tokens",
            Some(&bearer("u")),
            Some(json!({})),
        ),
    )
    .await;
    assert_eq!(unset, StatusCode::SERVICE_UNAVAILABLE);
    assert_eq!(
        serde_json::from_slice::<Value>(&unset_body).expect("json"),
        json!({"detail": "workflow API authentication is not configured"})
    );

    let disabled = env.app(|c| {
        c.workflow_signing_key = WF_KEY.to_vec();
        c.workflow_routes_enabled = false;
    });
    let (gone, _, gone_body) = get_json(&disabled, "/v1/watches/u/7").await;
    assert_eq!(gone, StatusCode::NOT_FOUND);
    assert_eq!(gone_body, json!({"detail": "Not Found"}));
}

#[tokio::test]
async fn push_tokens_register_owner_checked_and_delete_is_always_204() {
    let env = Env::new();
    let app = env.app(|c| c.workflow_signing_key = WF_KEY.to_vec());
    let body = json!({"user_id": "user-1", "token": "0123456789abcdef", "platform": "ios"});
    let (ok, _, ok_body) = send(
        &app,
        request_with(
            "POST",
            "/v1/push-tokens",
            Some(&bearer("user-1")),
            Some(body.clone()),
        ),
    )
    .await;
    assert_eq!(ok, StatusCode::OK);
    assert_eq!(
        serde_json::from_slice::<Value>(&ok_body).expect("json"),
        json!({"registered": true})
    );
    let (again, _, _) = send(
        &app,
        request_with(
            "POST",
            "/v1/push-tokens",
            Some(&bearer("user-1")),
            Some(body),
        ),
    )
    .await;
    assert_eq!(
        again,
        StatusCode::OK,
        "re-registering overwrites and succeeds"
    );

    let (other, _, other_body) = send(
        &app,
        request_with(
            "POST",
            "/v1/push-tokens",
            Some(&bearer("user-2")),
            Some(json!({"user_id": "user-1", "token": "0123456789abcdef", "platform": "ios"})),
        ),
    )
    .await;
    assert_eq!(other, StatusCode::NOT_FOUND);
    assert_eq!(
        serde_json::from_slice::<Value>(&other_body).expect("json"),
        json!({"detail": "not found"})
    );

    let (bad, _, bad_body) = send(
        &app,
        request_with(
            "POST",
            "/v1/push-tokens",
            Some(&bearer("user-1")),
            Some(json!({"user_id": "user-1", "token": "short", "platform": "web"})),
        ),
    )
    .await;
    assert_eq!(bad, StatusCode::UNPROCESSABLE_ENTITY);
    let issues: Value = serde_json::from_slice(&bad_body).expect("json");
    let locs: Vec<Value> = issues["detail"]
        .as_array()
        .expect("issues")
        .iter()
        .map(|i| i["loc"].clone())
        .collect();
    assert!(locs.contains(&json!(["body", "token"])));
    assert!(locs.contains(&json!(["body", "platform"])));

    let (deleted, dh, dbody) = send(
        &app,
        request_with(
            "DELETE",
            "/v1/push-tokens/0123456789abcdef",
            Some(&bearer("user-1")),
            None,
        ),
    )
    .await;
    assert_eq!(deleted, StatusCode::NO_CONTENT);
    assert!(dbody.is_empty());
    assert_eq!(header_str(&dh, "content-type"), "application/json");
    let (unknown, _, _) = send(
        &app,
        request_with(
            "DELETE",
            "/v1/push-tokens/notatokenatall00",
            Some(&bearer("user-1")),
            None,
        ),
    )
    .await;
    assert_eq!(unknown, StatusCode::NO_CONTENT);
}

#[tokio::test]
async fn watch_start_maps_an_unreachable_temporal_to_503() {
    let env = Env::new();
    let app = env.app(|c| {
        c.workflow_signing_key = WF_KEY.to_vec();
        c.temporal_address = "127.0.0.1:1".into();
    });
    let body = json!({"storm_cell_id": "7", "user_id": "user-1", "lat": "42.5", "lng": -85.0});
    let request = request_with("POST", "/v1/watches", Some(&bearer("user-1")), Some(body));
    let (status, _, resp_body) = tokio::time::timeout(Duration::from_secs(60), send(&app, request))
        .await
        .expect("the connect attempt must give up");
    assert_eq!(
        status,
        StatusCode::SERVICE_UNAVAILABLE,
        "D3: connect failure is 503, not 500"
    );
    assert_eq!(
        serde_json::from_slice::<Value>(&resp_body).expect("json"),
        json!({"detail": "temporal unavailable"})
    );
}

#[tokio::test]
async fn validation_happens_before_the_owner_check_and_before_temporal() {
    let env = Env::new();
    let app = env.app(|c| {
        c.workflow_signing_key = WF_KEY.to_vec();
        c.temporal_address = "127.0.0.1:1".into();
    });
    let body = json!({"storm_cell_id": "7", "user_id": "user-1", "lat": 123.0, "lng": -85.0});
    let (status, _, resp_body) = send(
        &app,
        request_with("POST", "/v1/watches", Some(&bearer("user-1")), Some(body)),
    )
    .await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
    let issues: Value = serde_json::from_slice(&resp_body).expect("json");
    assert_eq!(issues["detail"][0]["loc"], json!(["body", "lat"]));
}

#[tokio::test]
async fn unmatched_head_redirect_and_fallback_shapes() {
    let env = Env::new();
    let app = env.app(|_| {});
    // HEAD responses carry no body by HTTP rules, so only the status is checked.
    let (head, _, hb) = send(&app, request_with("HEAD", "/api/livez", None, None)).await;
    assert_eq!(head, StatusCode::METHOD_NOT_ALLOWED);
    assert!(hb.is_empty());

    let (post, ph, _) = send(
        &app,
        request_with("POST", "/api/manifest.json", None, Some(json!({}))),
    )
    .await;
    assert_eq!(post, StatusCode::METHOD_NOT_ALLOWED);
    // Spec 2 (Starlette parity): GET-only routes advertise `Allow: GET`.
    assert_eq!(header_str(&ph, "allow"), "GET");

    let (redirect, rh, _) = send(&app, get_req("/api/manifest.json/?v=2")).await;
    assert_eq!(redirect, StatusCode::TEMPORARY_REDIRECT);
    assert_eq!(header_str(&rh, "location"), "/api/manifest.json?v=2");

    let (api_missing, _, api_body) = get_json(&app, "/api/nope").await;
    assert_eq!(api_missing, StatusCode::NOT_FOUND);
    assert_eq!(api_body, json!({"detail": "Not Found"}));

    let (plain, ph2, pb) = send(&app, get_req("/nope")).await;
    assert_eq!(plain, StatusCode::NOT_FOUND);
    assert_eq!(header_str(&ph2, "cache-control"), "no-store");
    assert_eq!(pb, b"404 Not Found");
}

// ---------------------------------------------------------------------------------------------
// Review regressions (F1, F7, F10)
// ---------------------------------------------------------------------------------------------

/// F1: the rate limiter applies to `/api/*` and `/v1/*` only. Static lanes are never throttled.
#[tokio::test]
async fn rate_limit_applies_to_api_lanes_but_not_static_tiles() {
    let env = Env::new();
    let tile = env.path("tiles/radar/classic/2026-10-13T12:00:00+00:00/6/16/23.png");
    std::fs::create_dir_all(tile.parent().expect("parent")).expect("dirs");
    std::fs::write(&tile, b"png").expect("tile");
    // The production defaults (burst 60, 20 rps), not the test-only unlimited values.
    let app = env.app(|c| {
        c.rate_limit_burst = 60.0;
        c.rate_limit_rps = 20.0;
    });
    let uri = "/tiles/radar/classic/2026-10-13T12:00:00+00:00/6/16/23.png";
    for i in 0..200 {
        let (status, _, _) = send(&app, get_req(uri)).await;
        assert_eq!(
            status,
            StatusCode::OK,
            "tile request {i} must not be rate limited"
        );
    }
    let (basemap_status, _, _) = send(&app, get_req("/basemap/styles/nope.json")).await;
    assert_ne!(
        basemap_status,
        StatusCode::TOO_MANY_REQUESTS,
        "basemap lane is not limited"
    );

    // The API lane is still limited: 70 quick requests exhaust the 60-token burst.
    let mut limited = 0;
    for _ in 0..70 {
        let (status, _, _) = get_json(&app, "/api/metrics").await;
        if status == StatusCode::TOO_MANY_REQUESTS {
            limited += 1;
        }
    }
    assert!(limited >= 1, "the /api lane must still be rate limited");
}

/// F7: `radar_ng_forecast_requests_total` counts each `/api/forecast` request exactly once,
/// including requests that fail validation (spec: counted before validation).
#[tokio::test]
async fn forecast_requests_are_counted_once_including_422s() {
    let open_meteo = Router::new().route("/v1/forecast", get(|| async { Json(open_meteo_body()) }));
    let base = mock(open_meteo).await;
    let env = Env::new();
    let app = env.app(|c| c.open_meteo_base = format!("{base}/v1/forecast"));

    let (ok, _, _) = get_json(&app, "/api/forecast/42.96/-85.67").await;
    assert_eq!(ok, StatusCode::OK);
    let (cached, _, _) = get_json(&app, "/api/forecast/42.99/-85.72").await;
    assert_eq!(cached, StatusCode::OK);
    let (range, _, _) = get_json(&app, "/api/forecast/91/0").await;
    assert_eq!(range, StatusCode::UNPROCESSABLE_ENTITY);
    let (bad_param, _, _) = get_json(&app, "/api/forecast/abc/0").await;
    assert_eq!(bad_param, StatusCode::UNPROCESSABLE_ENTITY);

    let (_, _, metrics) = send(&app, get_req("/api/metrics")).await;
    let text = String::from_utf8(metrics).expect("utf8");
    assert!(
        text.contains("\nradar_ng_forecast_requests_total 4\n"),
        "four forecast requests, each counted once: {text}"
    );
}

/// F10: the imagery proxy forwards upstream `ETag` and `Last-Modified`, on misses and on cache hits.
#[tokio::test]
async fn imagery_forwards_upstream_etag_and_last_modified_on_hits_and_misses() {
    let usgs = Router::new().route(
        "/arcgis/rest/services/USGSImageryOnly/MapServer/tile/{z}/{y}/{x}",
        get(|| async {
            (
                [
                    (header::CONTENT_TYPE, "image/jpeg"),
                    (header::ETAG, "\"tile-etag\""),
                    (header::LAST_MODIFIED, "Tue, 01 Sep 2026 00:00:00 GMT"),
                    (header::CACHE_CONTROL, "max-age=5"),
                ],
                b"JPEG".to_vec(),
            )
                .into_response_boxed()
        }),
    );
    let base = mock(usgs).await;
    let env = Env::new();
    let app = env.app(|c| c.usgs_imagery_base = base.clone());

    for label in ["miss", "hit"] {
        let (status, headers, body) = send(&app, get_req("/basemap/imagery/3/2/1.jpg")).await;
        assert_eq!(status, StatusCode::OK, "{label}");
        assert_eq!(body, b"JPEG", "{label}");
        assert_eq!(header_str(&headers, "etag"), "\"tile-etag\"", "{label}");
        assert_eq!(
            header_str(&headers, "last-modified"),
            "Tue, 01 Sep 2026 00:00:00 GMT",
            "{label}"
        );
        assert_eq!(
            header_str(&headers, "cache-control"),
            "public, max-age=604800",
            "{label}"
        );
    }
}

/// F5 regression: a blocking read of `nowcast-status.json` (here a FIFO that blocks until a writer opens it)
/// must not park the runtime worker. `/api/livez` must answer while `/api/health` waits. The unblocking
/// happens on a plain thread after 2 s and records whether liveness had already answered. The test does not
/// use runtime timers, because a parked single worker would stop them too and the test would hang.
#[tokio::test(flavor = "multi_thread", worker_threads = 1)]
async fn health_file_read_does_not_park_the_runtime_worker() {
    use std::io::Write;
    use std::sync::Arc;
    use std::sync::atomic::{AtomicBool, Ordering as AtomicOrdering};

    let env = Env::new();
    let status = env.path("state/nowcast-status.json");
    let made = std::process::Command::new("mkfifo")
        .arg(&status)
        .status()
        .expect("mkfifo");
    assert!(made.success(), "mkfifo must work on the test host");
    let app = env.app(|_| {});

    let health_app = app.clone();
    let blocked_health =
        tokio::spawn(async move { send(&health_app, get_req("/api/health")).await });
    // A std sleep: the runtime timer would stall too if the worker were parked.
    std::thread::sleep(Duration::from_millis(200));

    let livez_done = Arc::new(AtomicBool::new(false));
    let live_flag = livez_done.clone();
    let live_app = app.clone();
    let live = tokio::spawn(async move {
        let outcome = send(&live_app, get_req("/api/livez")).await;
        live_flag.store(true, AtomicOrdering::SeqCst);
        outcome
    });

    let writer_flag = livez_done.clone();
    let writer_path = status.clone();
    let writer = std::thread::spawn(move || {
        std::thread::sleep(Duration::from_secs(2));
        let answered_first = writer_flag.load(AtomicOrdering::SeqCst);
        let mut file = std::fs::OpenOptions::new()
            .write(true)
            .open(writer_path)
            .expect("open fifo for write");
        file.write_all(b"{}").expect("write fifo");
        answered_first
    });
    let answered_first = writer.join().expect("writer thread");

    let (live_status, _, _) = live.await.expect("livez task");
    let (health_status, _, _) = blocked_health.await.expect("health task");
    assert!(
        answered_first,
        "/api/livez must answer while /api/health waits on a blocking read"
    );
    assert_eq!(live_status, StatusCode::OK);
    assert_eq!(
        health_status,
        StatusCode::SERVICE_UNAVAILABLE,
        "an empty state directory is degraded"
    );
}

/// Write a one-tile PMTiles archive whose tile 0/0/0 is `payload`.
fn write_one_tile_archive(path: &Path, payload: &[u8]) {
    let file = std::fs::File::create(path).expect("archive");
    let mut writer = PmTilesWriter::new(TileType::Mvt)
        .tile_compression(Compression::None)
        .create(file)
        .expect("writer");
    writer
        .add_tile(TileCoord::new(0, 0, 0).expect("coord"), payload)
        .expect("add");
    writer.finalize().expect("finalize");
}

/// F6 regression: a rename-over replacement of the archive is served after the recheck. The old code mapped
/// the first file for the life of the process and kept serving it.
#[tokio::test]
async fn replaced_archive_is_reopened_after_a_rename() {
    let env = Env::new();
    let archive = env.path("basemap.pmtiles");
    write_one_tile_archive(&archive, b"old-archive");
    let app = env.app(|c| c.pmtiles_recheck_s = 0);

    let (first, _, first_body) = send(&app, get_req("/basemap/tiles/0/0/0.mvt")).await;
    assert_eq!(
        (first, first_body.as_slice()),
        (StatusCode::OK, b"old-archive".as_slice())
    );

    // The bootstrap writes a temp file and renames it over the served path.
    let partial = env.path("basemap.pmtiles.partial");
    write_one_tile_archive(&partial, b"new-archive");
    std::fs::rename(&partial, &archive).expect("rename over");

    let (second, _, second_body) = send(&app, get_req("/basemap/tiles/0/0/0.mvt")).await;
    assert_eq!(
        (second, second_body.as_slice()),
        (StatusCode::OK, b"new-archive".as_slice())
    );
}

/// F6 regression: truncating the archive in place must not kill the process (SIGBUS under the old mmap). The
/// affected tile answers 500 and liveness keeps answering.
#[tokio::test]
async fn truncating_the_archive_in_place_is_a_500_not_a_crash() {
    let env = Env::new();
    let archive = env.path("basemap.pmtiles");
    write_one_tile_archive(&archive, b"tile-bytes-that-will-vanish");
    let app = env.app(|c| c.pmtiles_recheck_s = 0);
    let (ok, _, _) = send(&app, get_req("/basemap/tiles/0/0/0.mvt")).await;
    assert_eq!(ok, StatusCode::OK);

    std::fs::OpenOptions::new()
        .write(true)
        .truncate(true)
        .open(&archive)
        .expect("truncate in place");

    let (status, _, _) = send(&app, get_req("/basemap/tiles/0/0/0.mvt")).await;
    assert_eq!(
        status,
        StatusCode::INTERNAL_SERVER_ERROR,
        "a read past the new end is an error, not a signal"
    );
    let (live, _, _) = get_json(&app, "/api/livez").await;
    assert_eq!(live, StatusCode::OK);
}

/// F9 regression: with a read-only STATE_DIR, `POST /v1/push-tokens` answers 503 with a clear detail, not a 500.
/// `/proc/self` is never writable (`access(W_OK)` is EACCES on Linux), so it stands in for a read-only mount.
#[tokio::test]
async fn push_token_registration_is_503_when_the_state_directory_is_read_only() {
    let env = Env::new();
    let app = env.app(|c| {
        c.state_dir = PathBuf::from("/proc/self");
        c.workflow_signing_key = WF_KEY.to_vec();
    });
    let body = json!({"user_id": "user-1", "token": "0123456789abcdef", "platform": "ios"});
    let (status, _, resp_body) = send(
        &app,
        request_with(
            "POST",
            "/v1/push-tokens",
            Some(&bearer("user-1")),
            Some(body),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE);
    assert_eq!(
        serde_json::from_slice::<Value>(&resp_body).expect("json"),
        json!({"detail": "push token store is read-only"})
    );
}

/// Drop-in for the current cluster: with no local archive, `/basemap/tiles/*` proxies to the go-pmtiles
/// service with Caddy's rewrite (`/basemap/{z}/{x}/{y}.mvt`) and headers.
#[tokio::test]
async fn basemap_tiles_proxy_to_go_pmtiles_when_no_local_archive() {
    let pmtiles = Router::new().route(
        "/basemap/{z}/{x}/{ymvt}",
        get(
            |AxPath((z, x, ymvt)): AxPath<(String, String, String)>| async move {
                if (z.as_str(), x.as_str(), ymvt.as_str()) == ("4", "3", "5.mvt") {
                    (
                        [
                            (header::CONTENT_TYPE, "application/x-protobuf"),
                            (header::CONTENT_ENCODING, "gzip"),
                        ],
                        b"MVT".to_vec(),
                    )
                        .into_response_boxed()
                } else {
                    StatusCode::NO_CONTENT.into_response_boxed()
                }
            },
        ),
    );
    let base = mock(pmtiles).await;
    let env = Env::new();
    let app = env.app(|c| {
        c.pmtiles_path = std::path::PathBuf::from("/nonexistent/basemap.pmtiles");
        c.basemap_tiles_upstream = base.clone();
    });

    let (status, headers, body) = send(&app, get_req("/basemap/tiles/4/3/5.mvt")).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body, b"MVT");
    assert_eq!(header_str(&headers, "content-encoding"), "gzip");
    assert_eq!(
        header_str(&headers, "content-type"),
        "application/x-protobuf"
    );
    assert_eq!(
        header_str(&headers, "cache-control"),
        "public, max-age=86400"
    );
    assert_eq!(header_str(&headers, "access-control-allow-origin"), "*");

    let (status, _, _) = send(&app, get_req("/basemap/tiles/4/3/6.mvt")).await;
    assert_eq!(status, StatusCode::NO_CONTENT);

    // Fallback disabled and no archive: 503 as before.
    let app = env.app(|c| {
        c.pmtiles_path = std::path::PathBuf::from("/nonexistent/basemap.pmtiles");
        c.basemap_tiles_upstream = String::new();
    });
    let (status, _, _) = send(&app, get_req("/basemap/tiles/4/3/5.mvt")).await;
    assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE);
}

// ---------------------------------------------------------------------------------------------
// Satellite (NASA GIBS)
// ---------------------------------------------------------------------------------------------

/// A 256 px PNG of one RGBA color, like a GIBS GeoColor tile.
fn solid_tile(rgba: [u8; 4]) -> Vec<u8> {
    let mut out = Vec::new();
    let mut encoder = png::Encoder::new(&mut out, 256, 256);
    encoder.set_color(png::ColorType::Rgba);
    encoder.set_depth(png::BitDepth::Eight);
    let mut writer = encoder.write_header().expect("header");
    let px: Vec<u8> = std::iter::repeat_n(rgba, 256 * 256).flatten().collect();
    writer.write_image_data(&px).expect("data");
    writer.finish().expect("finish");
    out
}

fn decode_tile(bytes: &[u8]) -> Vec<u8> {
    let mut reader = png::Decoder::new(std::io::Cursor::new(bytes))
        .read_info()
        .expect("png");
    let mut buf = vec![0; reader.output_buffer_size().expect("size")];
    let info = reader.next_frame(&mut buf).expect("frame");
    buf.truncate(info.buffer_size());
    buf
}

/// The 10-minute GIBS step `back` steps before now, in the tile path form.
fn gibs_step(back: i64) -> String {
    let floored = Utc::now().timestamp() / 600 * 600 - back * 600;
    chrono::DateTime::from_timestamp(floored, 0)
        .expect("time")
        .format("%Y-%m-%dT%H:%M:%SZ")
        .to_string()
}

type Seen = Arc<std::sync::Mutex<Vec<String>>>;

/// A mock GIBS. DescribeDomains lists `times` for East and `times[1..]` for West. Tiles are solid red
/// (East) or blue (West), except `missing`, which is a 404. Tile requests are recorded in `seen`.
async fn mock_gibs(times: Vec<String>, missing: String, seen: Seen) -> String {
    let domain = move |layer: &str| {
        let list = if layer.starts_with("GOES-East") {
            times.join(",")
        } else {
            times[1..].join(",")
        };
        format!("<Domains><DimensionDomain><Domain>{list}</Domain></DimensionDomain></Domains>")
    };
    type TilePath = (String, String, String, String, String, String);
    let gibs = Router::new()
        .route(
            "/1.0.0/{layer}/default/{set}/all/{range}",
            get(
                move |AxPath((layer, _, _)): AxPath<(String, String, String)>| {
                    let body = domain(&layer);
                    async move { ([(header::CONTENT_TYPE, "text/xml")], body) }
                },
            ),
        )
        .route(
            "/{layer}/default/{time}/{set}/{z}/{row}/{colpng}",
            get(
                move |AxPath((layer, time, set, z, row, col)): AxPath<TilePath>| {
                    seen.lock()
                        .expect("seen")
                        .push(format!("{layer}/{time}/{set}/{z}/{row}/{col}"));
                    let missing = time == missing;
                    async move {
                        if missing {
                            return (StatusCode::NOT_FOUND, "no such time").into_response_boxed();
                        }
                        let color = if layer.starts_with("GOES-East") {
                            [255, 0, 0, 255]
                        } else {
                            [0, 0, 255, 255]
                        };
                        ([(header::CONTENT_TYPE, "image/png")], solid_tile(color))
                            .into_response_boxed()
                    }
                },
            ),
        );
    mock(gibs).await
}

/// The manifest lists the frames both GIBS layers have. Tiles go out in WMTS row/col order with
/// immutable caching, seam tiles are spliced, and invalid paths never reach GIBS.
#[tokio::test]
async fn satellite_layer_lists_shared_frames_and_proxies_tiles() {
    let times = vec![gibs_step(3), gibs_step(2), gibs_step(1)];
    let missing = gibs_step(5);
    let seen: Seen = Arc::default();
    let base = mock_gibs(times.clone(), missing.clone(), seen.clone()).await;
    let env = Env::new();
    write_json(&env.path("state/manifest.json"), &manifest(None, None));
    let app = env.app(|c| {
        c.gibs_wmts_base = base.clone();
        c.satellite_cache_max_bytes = 4 * 1024 * 1024;
    });
    let fetches = || seen.lock().expect("seen").len();

    let (status, _, body) = get_json(&app, "/api/manifest.json").await;
    assert_eq!(status, StatusCode::OK);
    let layer = &body["layers"]["satellite"];
    let paths: Vec<&str> = layer["frames"]
        .as_array()
        .expect("frames")
        .iter()
        .map(|f| f["path"].as_str().expect("path"))
        .collect();
    assert_eq!(paths, [times[1].as_str(), times[2].as_str()], "East ∩ West");
    assert_eq!(layer["timestamps"].as_array().map(Vec::len), Some(2));
    assert_eq!(layer["frames"][0]["max_zoom"], json!(7));

    // Grand Rapids at z6 is GOES-East only. WMTS wants the row (y) before the column (x).
    let uri = format!("/tiles/satellite/{}/6/16/23.png", times[2]);
    let (status, headers, body) = send(&app, get_req(&uri)).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(header_str(&headers, "content-type"), "image/png");
    assert_eq!(
        header_str(&headers, "cache-control"),
        "public, max-age=86400, immutable"
    );
    assert_eq!(header_str(&headers, "access-control-allow-origin"), "*");
    assert_eq!(&decode_tile(&body)[..4], [255, 0, 0, 255]);
    assert_eq!(
        seen.lock().expect("seen").last().cloned(),
        Some(format!(
            "GOES-East_ABI_GeoColor_v0_NRT/{}/GoogleMapsCompatible_Level7/6/23/16.png",
            times[2]
        ))
    );
    let (again, _, _) = send(&app, get_req(&uri)).await;
    assert_eq!(again, StatusCode::OK);
    assert_eq!(fetches(), 1, "the second request is a cache hit");

    // z0 crosses both seams: West, East from 106.1° W to 73.9° E, then West again.
    let world = format!("/tiles/satellite/{}/0/0/0.png", times[2]);
    let (status, _, body) = send(&app, get_req(&world)).await;
    assert_eq!(status, StatusCode::OK);
    let rgba = decode_tile(&body);
    let px = |c: usize| rgba[c * 4..c * 4 + 4].to_vec();
    let (east, west) = (vec![255, 0, 0, 255], vec![0, 0, 255, 255]);
    assert_eq!(
        [px(52), px(53), px(180), px(181)],
        [west.clone(), east.clone(), east, west]
    );

    // A time GIBS does not have is a no-store 404.
    let (status, headers, _) = send(
        &app,
        get_req(&format!("/tiles/satellite/{missing}/3/2/3.png")),
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    assert_eq!(header_str(&headers, "cache-control"), "no-store");

    let before = fetches();
    let future = gibs_step(-2);
    let stale = gibs_step(40);
    for bad in [
        format!("/tiles/satellite/{}/8/1/1.png", times[2]),
        format!("/tiles/satellite/{}/3/8/1.png", times[2]),
        format!("/tiles/satellite/{}/3/01/1.png", times[2]),
        format!("/tiles/satellite/{}/3/1/1.jpg", times[2]),
        format!("/tiles/satellite/{future}/3/1/1.png"),
        format!("/tiles/satellite/{stale}/3/1/1.png"),
        "/tiles/satellite/2026-10-15T22:13:00Z/3/1/1.png".into(),
        "/tiles/satellite/2026-10-15/3/1/1.png".into(),
        "/tiles/satellite/x/3/1".into(),
    ] {
        let (status, headers, _) = send(&app, get_req(&bad)).await;
        assert_eq!(status, StatusCode::NOT_FOUND, "{bad}");
        assert_eq!(header_str(&headers, "cache-control"), "no-store", "{bad}");
    }
    assert_eq!(fetches(), before, "invalid paths never reach GIBS");

    // `GIBS_WMTS_BASE` empty: no layer and no proxy.
    let off = env.app(|c| c.gibs_wmts_base = String::new());
    let (_, _, body) = get_json(&off, "/api/manifest.json").await;
    assert!(body["layers"].get("satellite").is_none());
    let (status, _, _) = send(&off, get_req(&uri)).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}
