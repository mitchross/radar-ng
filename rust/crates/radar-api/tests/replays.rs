use axum::{
    Router,
    body::Body,
    http::{HeaderMap, Request, StatusCode},
};
use http_body_util::BodyExt;
use radar_api::{AppState, Config, router};
use serde_json::{Value, json};
use std::{fs, path::PathBuf};
use tower::ServiceExt;

const FIXTURE: &str = include_str!("../../../../web/src/shared/replayApi.fixture.json");
const WARNINGS: &str = include_str!("../../../../web/src/shared/replayWarnings.fixture.json");

struct Env {
    _dir: tempfile::TempDir,
    root: PathBuf,
    revision: PathBuf,
    fixture: Value,
    app: Router,
}

impl Env {
    fn new() -> Self {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().to_path_buf();
        let fixture: Value = serde_json::from_str(FIXTURE).unwrap();
        let revision = root.join("tiles/replays/rolling-fork-2023/contract-1");
        fs::create_dir_all(&revision).unwrap();
        for (name, value) in [
            ("revision.json", &fixture["revision"]),
            ("manifest.json", &fixture["manifest"]),
        ] {
            fs::write(revision.join(name), serde_json::to_vec(value).unwrap()).unwrap();
        }
        let warnings: Value = serde_json::from_str(WARNINGS).unwrap();
        fs::write(
            revision.join("warnings.geojson"),
            serde_json::to_vec(&warnings["history"]).unwrap(),
        )
        .unwrap();
        let mut config =
            Config::defaults_for(root.join("state"), root.join("grid"), root.join("tiles"));
        config.rate_limit_burst = 100_000.;
        config.gibs_wmts_base = String::new();
        let app = router(AppState::new(config).unwrap());
        Self {
            _dir: dir,
            root,
            revision,
            fixture,
            app,
        }
    }

    async fn get(&self, url: &str, etag: Option<&str>) -> (StatusCode, HeaderMap, Vec<u8>) {
        let mut request = Request::builder().uri(url);
        if let Some(etag) = etag {
            request = request.header("if-none-match", etag);
        }
        let response = self
            .app
            .clone()
            .oneshot(request.body(Body::empty()).unwrap())
            .await
            .unwrap();
        let status = response.status();
        let headers = response.headers().clone();
        let body = response
            .into_body()
            .collect()
            .await
            .unwrap()
            .to_bytes()
            .to_vec();
        (status, headers, body)
    }

    fn url(&self, file: &str) -> String {
        format!("/api/replays/rolling-fork-2023/contract-1/{file}")
    }

    fn write(&self, file: &str, value: &Value) {
        fs::write(self.revision.join(file), serde_json::to_vec(value).unwrap()).unwrap();
    }
}

#[tokio::test]
async fn shared_contract_and_conditional_get() {
    let env = Env::new();
    for (url, expected, content_type) in [
        (
            "/api/replays".to_string(),
            env.fixture["catalogue"].clone(),
            "application/json",
        ),
        (
            env.url("manifest.json"),
            env.fixture["manifest"].clone(),
            "application/json",
        ),
        (
            env.url("warnings.geojson"),
            serde_json::from_str::<Value>(WARNINGS).unwrap()["history"].clone(),
            "application/geo+json",
        ),
    ] {
        let (status, headers, body) = env.get(&url, None).await;
        assert_eq!(
            status,
            StatusCode::OK,
            "{url}: {}",
            String::from_utf8_lossy(&body)
        );
        assert_eq!(headers["content-type"], content_type);
        assert_eq!(serde_json::from_slice::<Value>(&body).unwrap(), expected);
        assert!(!headers["etag"].is_empty());
        for candidate in [
            headers["etag"].to_str().unwrap().to_string(),
            format!("\"old\", W/{}", headers["etag"].to_str().unwrap()),
            "*".into(),
        ] {
            let (status, cached, body) = env.get(&url, Some(&candidate)).await;
            assert_eq!(status, StatusCode::NOT_MODIFIED);
            assert!(body.is_empty());
            assert_eq!(cached["etag"], headers["etag"]);
            assert_eq!(cached["cache-control"], headers["cache-control"]);
        }
        assert_eq!(env.get(&url, Some("\"stale\"")).await.0, StatusCode::OK);
    }
}

#[tokio::test]
async fn private_and_malformed_publications_fail_closed() {
    let env = Env::new();
    for (field, value) in [
        ("public_ready", json!(false)),
        ("scope", json!("pilot")),
        ("coverage_reviewed", json!(false)),
        ("schema_version", json!(9)),
        ("case", json!("greenfield-2024")),
        ("revision", json!("other")),
        ("start", json!("2026-01-01T00:00:00Z")),
        ("sources", json!([])),
        ("warnings", json!({"status":"missing"})),
    ] {
        let mut metadata = env.fixture["revision"].clone();
        metadata[field] = value;
        env.write("revision.json", &metadata);
        assert_eq!(
            env.get(&env.url("manifest.json"), Some("*")).await.0,
            StatusCode::NOT_FOUND,
            "{field}"
        );
        let (_, _, body) = env.get("/api/replays", None).await;
        assert_eq!(
            serde_json::from_slice::<Value>(&body).unwrap()["cases"],
            json!([])
        );
    }
    env.write("revision.json", &env.fixture["revision"]);
    for file in ["manifest.json", "warnings.geojson", "revision.json"] {
        let old = fs::read(env.revision.join(file)).unwrap();
        fs::write(env.revision.join(file), b"{").unwrap();
        assert_eq!(
            env.get(&env.url("manifest.json"), None).await.0,
            StatusCode::NOT_FOUND,
            "{file}"
        );
        fs::write(env.revision.join(file), old).unwrap();
    }
}

#[tokio::test]
async fn unknown_and_malicious_paths_never_reach_live_or_private_files() {
    let env = Env::new();
    fs::create_dir_all(env.root.join("state")).unwrap();
    fs::write(
        env.root.join("state/manifest.json"),
        br#"{"schema_version":2,"layers":{}}"#,
    )
    .unwrap();
    for path in [
        "/api/replays/unknown/contract-1/manifest.json",
        "/api/replays/rolling-fork-2023/unknown/manifest.json",
        "/api/replays/rolling-fork-2023/.staging-contract-1/manifest.json",
        "/api/replays/rolling-fork-2023/%2e%2e/manifest.json",
        "/api/replays/rolling-fork-2023/x%2f..%2fcontract-1/manifest.json",
        "/api/replays/rolling-fork-2023/x%5ccontract-1/manifest.json",
        "/tiles/replays/rolling-fork-2023/contract-1/revision.json",
        "/tiles/%72eplays/rolling-fork-2023/contract-1/manifest.json",
        "/tiles/./replays/rolling-fork-2023/contract-1/warnings.geojson",
        "/replays/rolling-fork-2023/contract-1/revision.json",
    ] {
        let (status, _, _) = env.get(path, Some("*")).await;
        assert_eq!(status, StatusCode::NOT_FOUND, "{path}");
    }
    assert_eq!(env.get("/api/manifest.json", None).await.0, StatusCode::OK);
}

#[tokio::test]
async fn tiles_are_immutable_and_only_served_for_ready_revisions() {
    let env = Env::new();
    let frame = "2023-03-24T23:00:00Z";
    let dir = env
        .revision
        .join("tiles/radar/classic")
        .join(frame)
        .join("4/3");
    fs::create_dir_all(&dir).unwrap();
    fs::write(dir.join("6.png"), b"png-bytes").unwrap();
    let base = "/replays/rolling-fork-2023/contract-1/tiles/radar/classic";
    let (status, headers, body) = env.get(&format!("{base}/{frame}/4/3/6.png"), None).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body, b"png-bytes");
    assert_eq!(headers["content-type"], "image/png");
    assert!(
        headers["cache-control"]
            .to_str()
            .unwrap()
            .contains("immutable")
    );
    for path in [
        format!("{base}/{frame}/4/3/7.png"),
        format!("{base}/{frame}/9/3/6.png"),
        format!("{base}/{frame}/4/3/6.jpg"),
        format!("{base}/../{frame}/4/3/6.png"),
        "/replays/rolling-fork-2023/contract-1/tiles/radar/vivid/x/4/3/6.png".into(),
        "/replays/rolling-fork-2023/contract-1/tiles/other/classic/x/4/3/6.png".into(),
        format!("/replays/unknown/contract-1/tiles/radar/classic/{frame}/4/3/6.png"),
        format!("/replays/rolling-fork-2023/other/tiles/radar/classic/{frame}/4/3/6.png"),
    ] {
        assert_eq!(
            env.get(&path, None).await.0,
            StatusCode::NOT_FOUND,
            "{path}"
        );
    }
}

#[tokio::test]
async fn tile_route_refuses_a_revision_that_is_not_ready() {
    let env = Env::new();
    let mut metadata = env.fixture["revision"].clone();
    metadata["public_ready"] = json!(false);
    env.write("revision.json", &metadata);
    let dir = env.revision.join("tiles/radar/classic/t/4/3");
    fs::create_dir_all(&dir).unwrap();
    fs::write(dir.join("6.png"), b"x").unwrap();
    let url = "/replays/rolling-fork-2023/contract-1/tiles/radar/classic/t/4/3/6.png";
    assert_eq!(env.get(url, None).await.0, StatusCode::NOT_FOUND);
}
