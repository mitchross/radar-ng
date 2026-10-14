//! Integration tests against a live Temporal server.
//!
//! Every test returns early unless `TEMPORAL_TEST_ADDRESS` is set (for example `localhost:7233`). Run them
//! against a disposable server, because the seeding test creates and updates real schedules:
//!
//! ```text
//! temporal server start-dev
//! TEMPORAL_TEST_ADDRESS=localhost:7233 cargo test -p radar-worker --test integration
//! ```
//!
//! The namespace defaults to `default` and can be changed with `TEMPORAL_TEST_NAMESPACE`.

use std::time::Duration;

use radar_alerts::alerts::WatchSignaler;
use radar_types::schedules::all_schedules;
use radar_worker::health::check_once;
use radar_worker::seed::{MAX_ATTEMPTS, Outcome, seed_with_retry};
use radar_worker::temporal_io::TemporalWatchSignaler;
use temporalio_client::{Client, ClientOptions, Connection, ConnectionOptions, Url};

/// The server address, or `None` when the tests should be skipped.
fn test_address() -> Option<String> {
    std::env::var("TEMPORAL_TEST_ADDRESS")
        .ok()
        .filter(|value| !value.trim().is_empty())
}

fn test_namespace() -> String {
    std::env::var("TEMPORAL_TEST_NAMESPACE").unwrap_or_else(|_| "default".to_owned())
}

/// Connect to the test server. Plaintext, because the dev server does not serve TLS.
async fn connect(address: &str) -> anyhow::Result<Client> {
    let url = Url::parse(&format!("http://{address}"))?;
    let connection = Connection::connect(ConnectionOptions::new(url).build()).await?;
    Ok(Client::new(
        connection,
        ClientOptions::new(test_namespace()).build(),
    )?)
}

/// Skip message for a test that needs the server.
macro_rules! require_server {
    () => {
        match test_address() {
            Some(address) => address,
            None => {
                eprintln!("skipped: TEMPORAL_TEST_ADDRESS is not set");
                return;
            }
        }
    };
}

/// Spec 11 invariants 1-18: the seeder creates every declared schedule, then a second pass leaves them all
/// unchanged, so compare-before-update sends no update RPC.
#[tokio::test]
async fn seed_creates_then_leaves_definitions_unchanged() {
    let address = require_server!();
    let client = connect(&address).await.expect("connect to the test server");
    let namespace = test_namespace();
    let expected = all_schedules().len();

    let first = seed_with_retry(
        &client,
        &namespace,
        false,
        MAX_ATTEMPTS,
        Duration::from_millis(200),
        Duration::from_secs(2),
    )
    .await
    .expect("first seed pass");
    assert_eq!(
        first.len(),
        expected,
        "every declared schedule is reconciled"
    );

    let second = seed_with_retry(
        &client,
        &namespace,
        false,
        MAX_ATTEMPTS,
        Duration::from_millis(200),
        Duration::from_secs(2),
    )
    .await
    .expect("second seed pass");
    assert_eq!(second.len(), expected);
    assert!(
        second
            .values()
            .all(|outcome| *outcome == Outcome::Unchanged),
        "second pass is a no-op: {second:?}"
    );
}

/// Spec 9.1: the health probe reports SERVING against a live server.
#[tokio::test]
async fn health_probe_reports_serving() {
    let address = require_server!();
    let client = connect(&address).await.expect("connect to the test server");
    assert!(
        check_once(&client).await,
        "health check reported not serving"
    );
}

/// The visibility query used by PollAlerts runs against a live server. An empty result is fine.
#[tokio::test]
async fn running_watch_listing_queries_visibility() {
    let address = require_server!();
    let client = connect(&address).await.expect("connect to the test server");
    let signaler = TemporalWatchSignaler::new(client);
    let watches = signaler
        .list_running_watches()
        .await
        .expect("list running watches");
    assert!(watches.iter().all(|id| !id.is_empty()));
}

/// A signal to a watch that does not exist must surface as an error, not as success. PollAlerts counts
/// that failure as a missed match.
#[tokio::test]
async fn signal_to_missing_watch_is_an_error() {
    let address = require_server!();
    let client = connect(&address).await.expect("connect to the test server");
    let signaler = TemporalWatchSignaler::new(client);
    let result = signaler
        .signal_alert_match("watch:synthetic-missing-watch", "synthetic-alert")
        .await;
    assert!(result.is_err(), "signalling a missing watch should fail");
}
