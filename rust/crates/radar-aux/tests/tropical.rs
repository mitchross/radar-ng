//! `tropical_fetch_and_publish` against a local fake NHC feed and ArcGIS server.
//! Fixtures: `testdata/tropical/` (the Python `test_activities.py` FEED, CONE, TRACK and POINTS).

mod common;

use std::path::Path;
use std::time::Duration;

use common::{FakeHttp, client, testdata};
use radar_aux::tropical::{TropicalConfig, TropicalError, tropical_fetch_and_publish};
use serde_json::Value;
use tempfile::TempDir;

const FEED: &str = "/feed";
const GIS: &str = "/gis/MapServer";
const CONE_QUERY: &str = "/gis/MapServer/3/query";
const TRACK_QUERY: &str = "/gis/MapServer/4/query";
const POINTS_QUERY: &str = "/gis/MapServer/5/query";

fn tropical_fixture(name: &str) -> Vec<u8> {
    testdata(&format!("tropical/{name}"))
}

struct Env {
    _dir: TempDir,
    state: std::path::PathBuf,
    http: FakeHttp,
    client: reqwest::Client,
    cfg: TropicalConfig,
}

async fn env() -> Env {
    let dir = TempDir::new().unwrap();
    let state = dir.path().join("state");
    std::fs::create_dir_all(&state).unwrap();
    let http = FakeHttp::start().await;
    http.set(FEED, 200, tropical_fixture("feed.json"));
    http.set(GIS, 200, tropical_fixture("gis_index.json"));
    http.set(CONE_QUERY, 200, tropical_fixture("gis_cone.json"));
    http.set(TRACK_QUERY, 200, tropical_fixture("gis_track.json"));
    http.set(POINTS_QUERY, 200, tropical_fixture("gis_points.json"));
    let cfg = TropicalConfig {
        state_dir: state.clone(),
        feed_url: format!("{}{FEED}", http.base),
        gis_url: format!("{}{GIS}", http.base),
        feed_timeout: Duration::from_secs(5),
        index_timeout: Duration::from_secs(5),
        query_timeout: Duration::from_secs(5),
        gis_concurrency: 8,
    };
    Env {
        _dir: dir,
        state,
        client: client(),
        http,
        cfg,
    }
}

fn read_output(state: &Path) -> Value {
    serde_json::from_slice(&std::fs::read(state.join("tropical.json")).expect("tropical.json"))
        .unwrap()
}

fn features_of_kind<'a>(doc: &'a Value, kind: &str) -> Vec<&'a Value> {
    doc["features"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|f| f["properties"]["kind"] == kind)
        .collect()
}

#[tokio::test]
async fn publishes_position_cone_track_and_forecast_points() {
    let e = env().await;
    let result = tropical_fetch_and_publish(&e.cfg, &e.client).await.unwrap();
    assert_eq!(result.storm_count, 1);
    assert_eq!(result.feature_count, 4);

    let doc = read_output(&e.state);
    assert_eq!(doc["features"].as_array().unwrap().len(), 4);

    let pos = &features_of_kind(&doc, "position")[0]["properties"];
    assert_eq!(pos["name"], "Polo");
    assert_eq!(pos["wind_kt"], 100.0);
    assert_eq!(pos["wind_mph"], 115, "100 kt * 1.15078 rounded half-even");
    assert_eq!(pos["category"], 3);
    assert_eq!(pos["pressure_mb"], 963.0);
    assert_eq!(pos["movement_mph"], 13.0);
    assert_eq!(pos["movement_dir_deg"], 35.0);

    assert_eq!(features_of_kind(&doc, "cone").len(), 1);
    assert_eq!(features_of_kind(&doc, "track").len(), 1);
    let pt = &features_of_kind(&doc, "forecast_point")[0]["properties"];
    assert_eq!(pt["name"], "Polo");
    assert_eq!(pt["tau_h"], 12.0);
    assert_eq!(pt["wind_mph"], 86, "75 kt * 1.15078 rounded");
    assert_eq!(pt["category"], 1);
    assert_eq!(pt["storm_type"], "H");
}

#[tokio::test]
async fn output_file_is_world_readable_and_no_temp_file_is_left() {
    use std::os::unix::fs::PermissionsExt;
    let e = env().await;
    tropical_fetch_and_publish(&e.cfg, &e.client).await.unwrap();
    let mode = std::fs::metadata(e.state.join("tropical.json"))
        .unwrap()
        .permissions()
        .mode()
        & 0o777;
    assert_eq!(mode, 0o644);
    let stray: Vec<_> = std::fs::read_dir(&e.state)
        .unwrap()
        .flatten()
        .map(|d| d.file_name().to_string_lossy().into_owned())
        .filter(|n| n.starts_with(".tropical."))
        .collect();
    assert!(stray.is_empty(), "temp files left behind: {stray:?}");
}

#[tokio::test]
async fn each_gis_layer_is_queried_once_per_bin() {
    let e = env().await;
    tropical_fetch_and_publish(&e.cfg, &e.client).await.unwrap();
    assert_eq!(e.http.hits(CONE_QUERY), 1);
    assert_eq!(e.http.hits(TRACK_QUERY), 1);
    assert_eq!(e.http.hits(POINTS_QUERY), 1);
}

#[tokio::test]
async fn gis_index_failure_still_publishes_positions() {
    let e = env().await;
    e.http.set(GIS, 500, Vec::new());
    let result = tropical_fetch_and_publish(&e.cfg, &e.client).await.unwrap();
    assert_eq!(result.feature_count, 1);
    let doc = read_output(&e.state);
    assert_eq!(features_of_kind(&doc, "position").len(), 1);
    assert!(features_of_kind(&doc, "cone").is_empty());
}

#[tokio::test]
async fn one_failed_gis_layer_does_not_drop_the_others() {
    let e = env().await;
    e.http.set(CONE_QUERY, 500, Vec::new());
    tropical_fetch_and_publish(&e.cfg, &e.client).await.unwrap();
    let doc = read_output(&e.state);
    assert!(features_of_kind(&doc, "cone").is_empty());
    assert_eq!(features_of_kind(&doc, "track").len(), 1);
    assert_eq!(features_of_kind(&doc, "forecast_point").len(), 1);
}

#[tokio::test]
async fn feed_failure_fails_the_activity_and_keeps_the_previous_file() {
    let e = env().await;
    tropical_fetch_and_publish(&e.cfg, &e.client).await.unwrap();
    let before = std::fs::read(e.state.join("tropical.json")).unwrap();

    e.http.set(FEED, 500, Vec::new());
    let err = tropical_fetch_and_publish(&e.cfg, &e.client)
        .await
        .unwrap_err();
    assert!(
        matches!(err, TropicalError::Status { status: 500, .. }),
        "got {err:?}"
    );
    assert_eq!(
        std::fs::read(e.state.join("tropical.json")).unwrap(),
        before,
        "file untouched"
    );
}

#[tokio::test]
async fn latitude_zero_is_a_real_position() {
    let e = env().await;
    let mut feed: Value = serde_json::from_slice(&tropical_fixture("feed.json")).unwrap();
    feed["activeStorms"][0]["latitudeNumeric"] = Value::from(0.0);
    feed["activeStorms"][0]["longitudeNumeric"] = Value::from(-30.0);
    e.http.set(FEED, 200, serde_json::to_vec(&feed).unwrap());
    tropical_fetch_and_publish(&e.cfg, &e.client).await.unwrap();
    let doc = read_output(&e.state);
    let pos = features_of_kind(&doc, "position");
    assert_eq!(
        pos.len(),
        1,
        "a latitude of 0 must not be treated as missing"
    );
    assert_eq!(pos[0]["geometry"]["coordinates"][1], 0.0);
}

#[tokio::test]
async fn unparseable_latitude_skips_the_position_but_counts_the_storm() {
    let e = env().await;
    let mut feed: Value = serde_json::from_slice(&tropical_fixture("feed.json")).unwrap();
    feed["activeStorms"][0]["latitudeNumeric"] = Value::from("not-a-number");
    e.http.set(FEED, 200, serde_json::to_vec(&feed).unwrap());
    let result = tropical_fetch_and_publish(&e.cfg, &e.client).await.unwrap();
    assert_eq!(result.storm_count, 1);
    let doc = read_output(&e.state);
    assert!(features_of_kind(&doc, "position").is_empty());
}
