//! NWS active alerts against a local HTTP server, and the watch-signal fan-out through a fake `WatchSignaler`
//! (spec-ingest-aux section 10).

use std::collections::{HashMap, HashSet};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use axum::Json;
use axum::Router;
use axum::extract::State;
use axum::http::{HeaderMap, StatusCode};
use axum::routing::get;
use radar_alerts::alerts::{
    AlertsConfig, SNAPSHOT_FILE, WatchCenter, WatchSignaler, fetch_nws_active_alerts,
    load_active_snapshot, load_seen, mark_alerts_seen, signal_matching_storm_watches,
};
use radar_types::payloads::alerts::SignalWatchesInput;
use serde_json::{Value, json};

#[derive(Default)]
struct Feed {
    hits: usize,
    user_agent: Option<String>,
    accept: Option<String>,
    body: Value,
    status: u16,
}

type SharedFeed = Arc<Mutex<Feed>>;

async fn alerts(State(feed): State<SharedFeed>, headers: HeaderMap) -> (StatusCode, Json<Value>) {
    let mut feed = feed.lock().unwrap();
    feed.hits += 1;
    feed.user_agent = headers
        .get("user-agent")
        .and_then(|v| v.to_str().ok())
        .map(str::to_owned);
    feed.accept = headers
        .get("accept")
        .and_then(|v| v.to_str().ok())
        .map(str::to_owned);
    (
        StatusCode::from_u16(feed.status).unwrap(),
        Json(feed.body.clone()),
    )
}

async fn spawn_feed(body: Value, status: u16) -> (String, SharedFeed) {
    let feed: SharedFeed = Arc::new(Mutex::new(Feed {
        body,
        status,
        ..Feed::default()
    }));
    let app = Router::new()
        .route("/alerts/active", get(alerts))
        .with_state(Arc::clone(&feed));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    (format!("http://{addr}/alerts/active?status=actual"), feed)
}

fn feed_body() -> Value {
    json!({"features": [
        // Properties id wins over feature id.
        {"id": "urn:feature-a", "properties": {"id": "urn:a"}, "geometry": {"type": "Polygon", "coordinates": []}},
        // Feature id used when properties carry none.
        {"id": "urn:b", "properties": {}},
        // Not an object: skipped, but still counted in alert_count.
        "junk",
        // Falsy properties id and null feature id: skipped.
        {"id": null, "properties": {"id": 0}},
        // Duplicate of urn:a: reported twice, as in Python.
        {"properties": {"id": "urn:a"}},
    ]})
}

fn config(dir: &std::path::Path, url: &str) -> AlertsConfig {
    let mut cfg = AlertsConfig::new(dir);
    cfg.alerts_url = url.to_owned();
    cfg.user_agent = "radar-ng-test (ops@example.com)".to_owned();
    cfg
}

#[tokio::test]
async fn fetch_sends_headers_diffs_and_writes_snapshot() {
    let (url, feed) = spawn_feed(feed_body(), 200).await;
    let dir = tempfile::tempdir().unwrap();
    let cfg = config(dir.path(), &url);

    let result = fetch_nws_active_alerts(&cfg).await.unwrap();

    {
        let feed = feed.lock().unwrap();
        assert_eq!(feed.hits, 1);
        assert_eq!(
            feed.user_agent.as_deref(),
            Some("radar-ng-test (ops@example.com)")
        );
        assert_eq!(feed.accept.as_deref(), Some("*/*"));
    }
    assert_eq!(result.alert_count, 5);
    assert_eq!(result.new_alert_ids, vec!["urn:a", "urn:b", "urn:a"]);
    assert_eq!(result.new_alerts.len(), 3);
    assert_eq!(result.new_alerts[1].alert_id, "urn:b");

    let snapshot = load_active_snapshot(dir.path()).unwrap();
    assert_eq!(
        snapshot.into_iter().collect::<Vec<_>>(),
        vec!["urn:a", "urn:b"]
    );
    let raw = std::fs::read_to_string(dir.path().join(SNAPSHOT_FILE)).unwrap();
    assert_eq!(raw, r#"["urn:a", "urn:b"]"#);
    // Fetching does not commit the seen set.
    assert!(load_seen(dir.path()).is_empty());
}

#[tokio::test]
async fn mark_seen_is_at_least_once_and_idempotent() {
    let (url, _feed) = spawn_feed(feed_body(), 200).await;
    let dir = tempfile::tempdir().unwrap();
    let cfg = config(dir.path(), &url);
    fetch_nws_active_alerts(&cfg).await.unwrap();

    mark_alerts_seen(&cfg, &["urn:a".to_owned()]).await.unwrap();
    assert_eq!(
        load_seen(dir.path()).into_iter().collect::<Vec<_>>(),
        vec!["urn:a"]
    );

    // The next poll reports only the alert that is not seen yet.
    let second = fetch_nws_active_alerts(&cfg).await.unwrap();
    assert_eq!(second.new_alert_ids, vec!["urn:b"]);

    // Committing the same IDs again, or an empty list, leaves the file byte-identical.
    mark_alerts_seen(&cfg, &["urn:b".to_owned(), "urn:b".to_owned()])
        .await
        .unwrap();
    let first = std::fs::read(dir.path().join("alerts_seen.json")).unwrap();
    mark_alerts_seen(&cfg, &["urn:b".to_owned(), "urn:b".to_owned()])
        .await
        .unwrap();
    mark_alerts_seen(&cfg, &[]).await.unwrap();
    let again = std::fs::read(dir.path().join("alerts_seen.json")).unwrap();
    assert_eq!(first, again);

    let third = fetch_nws_active_alerts(&cfg).await.unwrap();
    assert!(third.new_alert_ids.is_empty());
    assert_eq!(third.alert_count, 5);
}

#[tokio::test]
async fn upstream_error_status_fails_and_writes_nothing() {
    let (url, _feed) = spawn_feed(json!({"title": "unavailable"}), 500).await;
    let dir = tempfile::tempdir().unwrap();
    let err = fetch_nws_active_alerts(&config(dir.path(), &url))
        .await
        .unwrap_err();
    assert!(format!("{err:#}").contains("error status"), "got {err:#}");
    assert!(load_active_snapshot(dir.path()).is_none());
}

#[tokio::test]
async fn non_list_features_fail_the_fetch() {
    let (url, _feed) = spawn_feed(json!({"features": 5}), 200).await;
    let dir = tempfile::tempdir().unwrap();
    assert!(
        fetch_nws_active_alerts(&config(dir.path(), &url))
            .await
            .is_err()
    );
}

/// In-memory stand-in for the Temporal client. Each watch has a center (or none), and some watches fail on
/// their query or their signal.
#[derive(Default)]
struct FakeSignaler {
    running: Vec<String>,
    centers: HashMap<String, WatchCenter>,
    no_center: HashSet<String>,
    failing: HashSet<String>,
    list_error: bool,
    list_calls: AtomicUsize,
    signals: Mutex<Vec<(String, String)>>,
}

#[async_trait::async_trait]
impl WatchSignaler for FakeSignaler {
    async fn list_running_watches(&self) -> anyhow::Result<Vec<String>> {
        self.list_calls.fetch_add(1, Ordering::SeqCst);
        if self.list_error {
            anyhow::bail!("temporal unavailable");
        }
        Ok(self.running.clone())
    }

    async fn watch_center(&self, workflow_id: &str) -> anyhow::Result<Option<WatchCenter>> {
        if self.failing.contains(workflow_id) {
            anyhow::bail!("query failed for {workflow_id}");
        }
        if self.no_center.contains(workflow_id) {
            return Ok(None);
        }
        Ok(self.centers.get(workflow_id).copied())
    }

    async fn signal_alert_match(&self, workflow_id: &str, alert_id: &str) -> anyhow::Result<()> {
        if self.failing.contains(workflow_id) {
            anyhow::bail!("signal failed for {workflow_id}");
        }
        self.signals
            .lock()
            .unwrap()
            .push((workflow_id.to_owned(), alert_id.to_owned()));
        Ok(())
    }
}

fn square_input(alert_id: &str) -> SignalWatchesInput {
    serde_json::from_value(json!({
        "alert_id": alert_id,
        "geometry": {"type": "Polygon", "coordinates": [[[-98.0, 35.0], [-96.0, 35.0], [-96.0, 37.0], [-98.0, 37.0], [-98.0, 35.0]]]},
    }))
    .unwrap()
}

fn center(lat: f64, lng: f64) -> WatchCenter {
    WatchCenter { lat, lng }
}

#[tokio::test]
async fn signals_only_watches_inside_the_alert_polygon() {
    let mut fake = FakeSignaler {
        running: vec![
            "w-inside".into(),
            "w-outside".into(),
            "w-no-center".into(),
            "w-query-fails".into(),
            "w-signal-fails".into(),
            "w-inside-2".into(),
        ],
        ..FakeSignaler::default()
    };
    fake.centers.insert("w-inside".into(), center(36.0, -97.0));
    fake.centers.insert("w-outside".into(), center(40.0, -90.0));
    fake.centers
        .insert("w-signal-fails".into(), center(36.5, -97.5));
    fake.centers
        .insert("w-inside-2".into(), center(35.5, -96.5));
    fake.no_center.insert("w-no-center".into());
    fake.failing.insert("w-query-fails".into());
    fake.failing.insert("w-signal-fails".into());

    let result = signal_matching_storm_watches(&fake, &square_input("urn:a"))
        .await
        .unwrap();

    // The failed watches are logged and not counted, so the call still succeeds (spec Q-15, D7).
    assert_eq!(result.matched, 2);
    let signals = fake.signals.lock().unwrap().clone();
    assert_eq!(
        signals,
        vec![
            ("w-inside".to_owned(), "urn:a".to_owned()),
            ("w-inside-2".to_owned(), "urn:a".to_owned())
        ]
    );
}

#[tokio::test]
async fn failing_to_list_watches_fails_the_call() {
    let fake = FakeSignaler {
        list_error: true,
        running: vec!["w".into()],
        ..FakeSignaler::default()
    };
    assert!(
        signal_matching_storm_watches(&fake, &square_input("urn:a"))
            .await
            .is_err()
    );
    assert!(fake.signals.lock().unwrap().is_empty());
}

#[tokio::test]
async fn empty_geometry_skips_the_watch_list_entirely() {
    let fake = FakeSignaler {
        running: vec!["w".into()],
        ..FakeSignaler::default()
    };
    let input: SignalWatchesInput =
        serde_json::from_value(json!({"alert_id": "urn:x", "geometry": {}})).unwrap();
    let result = signal_matching_storm_watches(&fake, &input).await.unwrap();
    assert_eq!(result.matched, 0);
    assert_eq!(fake.list_calls.load(Ordering::SeqCst), 0);
}
