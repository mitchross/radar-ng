//! AQM activities end to end against a local fake NAQFC server (`aqm_find_latest_run`, `aqm_render_chunk`,
//! `aqm_publish_run`, `aqm_mark_processed`, `aqm_cleanup`). Fixtures: `testdata/aqm_*_mini.grib2`.

mod common;

use std::path::PathBuf;
use std::time::{Duration, SystemTime};

use chrono::{DateTime, TimeZone, Utc};
use common::{FakeHttp, client, count_files, recording_beat, testdata};
use radar_aux::aqm::{self, AqmConfig, AqmError, find_latest_run_at};
use radar_core::manifest::read_manifest;
use radar_core::processed_set::ProcessedSet;
use radar_grib::naqfc::AqmSources;
use radar_types::payloads::airquality::AqmChunkResult;
use tempfile::TempDir;

const RUN: &str = "20260716_12";
const NOW: &str = "2026-07-16T13:00:00Z";
const PM25_PATH: &str = "/AQMv7/CS/20260716/12/aqm.t12z.ave_1hr_pm25_bc.20260716.227.grib2";
const O3_PATH: &str = "/AQMv7/CS/20260716/12/aqm.t12z.ave_1hr_o3_bc.20260716.227.grib2";
const NOMADS_PM25_PATH: &str = "/nomads/aqm.20260716/12/aqm.t12z.ave_1hr_pm25_bc.227.grib2";

struct Env {
    _root: TempDir,
    tile: PathBuf,
    state: PathBuf,
    grid: PathBuf,
    tmp: PathBuf,
    cfg: AqmConfig,
    http: FakeHttp,
    client: reqwest::Client,
}

async fn env() -> Env {
    let root = TempDir::new().unwrap();
    let tile = root.path().join("tiles");
    let state = root.path().join("state");
    let grid = root.path().join("grid");
    let tmp = root.path().join("tmp");
    for d in [&tile, &state, &grid, &tmp] {
        std::fs::create_dir_all(d).unwrap();
    }
    let http = FakeHttp::start().await;
    let mut cfg = AqmConfig::new(&tile, &state, &grid, &tmp);
    cfg.sources = AqmSources {
        aws_base: format!("{}/AQMv7/CS", http.base),
        nomads_base: format!("{}/nomads", http.base),
    };
    cfg.read_timeout = Duration::from_secs(10);
    cfg.heartbeat_every = Duration::from_millis(200);
    // Default: AWS serves both products, NOMADS has nothing.
    http.set(PM25_PATH, 200, testdata("aqm_pm25_mini.grib2"));
    http.set(O3_PATH, 200, testdata("aqm_o3_mini.grib2"));
    Env {
        _root: root,
        tile,
        state,
        grid,
        tmp,
        cfg,
        http,
        client: client(),
    }
}

fn at(s: &str) -> DateTime<Utc> {
    s.parse().unwrap()
}

fn chunk(layer: &str, start: i64, ts: Vec<String>) -> AqmChunkResult {
    AqmChunkResult {
        layer: layer.to_string(),
        start_msg: start,
        rendered_timestamps: ts,
    }
}

/// Renders the three chunks of one layer, returning the results in order.
async fn render_layer(e: &Env, layer: &str) -> Vec<AqmChunkResult> {
    let mut out = Vec::new();
    for start in [0, 24, 48] {
        let (beat, _) = recording_beat();
        out.push(
            aqm::aqm_render_chunk(&e.cfg, &e.client, RUN, layer, start, beat)
                .await
                .expect("chunk"),
        );
    }
    out
}

fn all_timestamps(results: &[AqmChunkResult]) -> Vec<String> {
    results
        .iter()
        .flat_map(|r| r.rendered_timestamps.clone())
        .collect()
}

// ---- discovery -------------------------------------------------------------

#[tokio::test]
async fn discovery_finds_the_run_and_reports_processed_state() {
    let e = env().await;
    let r = find_latest_run_at(&e.cfg, &e.client, at(NOW))
        .await
        .unwrap();
    assert_eq!(r.run_id.as_deref(), Some(RUN));
    assert!(!r.already_processed);

    aqm::aqm_mark_processed(&e.cfg, RUN).await.unwrap();
    let r = find_latest_run_at(&e.cfg, &e.client, at(NOW))
        .await
        .unwrap();
    assert!(r.already_processed);
}

#[test]
fn grib_urls_are_aws_then_nomads() {
    let urls = AqmSources::default()
        .grib_urls(RUN, "ave_1hr_pm25_bc")
        .unwrap();
    assert_eq!(urls.len(), 2);
    assert!(
        urls[0].starts_with("https://noaa-nws-naqfc-pds.s3.amazonaws.com/"),
        "AWS first: {}",
        urls[0]
    );
    assert!(
        urls[1].starts_with("https://nomads.ncep.noaa.gov/"),
        "NOMADS second: {}",
        urls[1]
    );
}

#[tokio::test]
async fn discovery_probes_aws_before_nomads() {
    let e = env().await;
    e.http.clear(PM25_PATH);
    e.http.set(PM25_PATH, 404, Vec::new());
    e.http.set(NOMADS_PM25_PATH, 200, Vec::new());
    let r = find_latest_run_at(&e.cfg, &e.client, at(NOW))
        .await
        .unwrap();
    assert_eq!(r.run_id.as_deref(), Some(RUN));
    assert_eq!(e.http.hits(PM25_PATH), 1, "AWS probed first");
    assert_eq!(
        e.http.hits(NOMADS_PM25_PATH),
        1,
        "NOMADS probed after AWS 404"
    );
}

#[tokio::test]
async fn discovery_falls_through_403_and_500_to_nomads() {
    let e = env().await;
    e.http.clear(PM25_PATH);
    e.http.set(PM25_PATH, 403, Vec::new());
    e.http.set(NOMADS_PM25_PATH, 200, Vec::new());
    let r = find_latest_run_at(&e.cfg, &e.client, at(NOW))
        .await
        .unwrap();
    assert_eq!(r.run_id.as_deref(), Some(RUN));

    e.http.set(PM25_PATH, 500, Vec::new());
    e.http.clear(NOMADS_PM25_PATH);
    e.http.set(NOMADS_PM25_PATH, 500, Vec::new());
    let r = find_latest_run_at(&e.cfg, &e.client, at(NOW))
        .await
        .unwrap();
    // Every 12z and 06z probe on the last three days fails, so nothing is found.
    assert_eq!(r.run_id, None);
    assert!(!r.already_processed);
}

#[tokio::test]
async fn discovery_skips_cycles_later_than_now() {
    let e = env().await;
    // At 10:00 the 12z cycle has not started, so the 06z run of the same day is the latest.
    e.http.set(
        "/AQMv7/CS/20260716/06/aqm.t06z.ave_1hr_pm25_bc.20260716.227.grib2",
        200,
        Vec::new(),
    );
    let r = find_latest_run_at(&e.cfg, &e.client, at("2026-07-16T10:00:00Z"))
        .await
        .unwrap();
    assert_eq!(r.run_id.as_deref(), Some("20260716_06"));
}

// ---- chunk rendering -------------------------------------------------------

#[tokio::test]
async fn chunks_render_24_frames_each_and_download_once_per_run_and_layer() {
    let e = env().await;
    let results = render_layer(&e, "air-quality").await;
    assert_eq!(
        results
            .iter()
            .map(|r| r.rendered_timestamps.len())
            .collect::<Vec<_>>(),
        vec![24, 24, 24]
    );
    let ts = all_timestamps(&results);
    assert_eq!(
        ts.first().map(String::as_str),
        Some("2026-07-16T12:00:00+00:00")
    );
    assert_eq!(
        ts.last().map(String::as_str),
        Some("2026-07-19T11:00:00+00:00")
    );
    let unique: std::collections::BTreeSet<_> = ts.iter().collect();
    assert_eq!(unique.len(), 72, "one frame per forecast message");

    // One fetch for the whole run and layer, despite three chunk calls.
    assert_eq!(e.http.hits(PM25_PATH), 1);
    assert_eq!(
        e.http.hits(O3_PATH),
        0,
        "ozone is not fetched for the air-quality layer"
    );

    // Tiles and grid dumps exist for every frame.
    let frame_dir = e.tile.join("air-quality/classic/runs").join(RUN);
    assert_eq!(std::fs::read_dir(&frame_dir).unwrap().count(), 72);
    assert!(count_files(&e.grid.join("air-quality"), ".meta.json") >= 72);
}

#[tokio::test]
async fn all_nan_message_is_not_rendered_or_published() {
    let e = env().await;
    // The blank fixture has 24 messages; message 5 (valid 17:00) is entirely missing.
    e.http.clear(PM25_PATH);
    e.http
        .set(PM25_PATH, 200, testdata("aqm_pm25_blank24.grib2"));
    let (beat, _) = recording_beat();
    let r = aqm::aqm_render_chunk(&e.cfg, &e.client, RUN, "air-quality", 0, beat)
        .await
        .unwrap();
    assert_eq!(r.rendered_timestamps.len(), 23);
    assert!(
        !r.rendered_timestamps
            .contains(&"2026-07-16T17:00:00+00:00".to_string())
    );
    assert!(
        !e.tile
            .join("air-quality/classic/runs")
            .join(RUN)
            .join("2026-07-16T17:00:00+00:00")
            .exists()
    );
}

#[tokio::test]
async fn chunk_starting_past_the_last_message_renders_nothing() {
    let e = env().await;
    let (beat, _) = recording_beat();
    let r = aqm::aqm_render_chunk(&e.cfg, &e.client, RUN, "air-quality", 72, beat)
        .await
        .unwrap();
    assert!(r.rendered_timestamps.is_empty());
}

#[tokio::test]
async fn file_with_too_few_messages_is_a_missing_message_error() {
    let e = env().await;
    // The two-message flipped fixture stands in for a truncated NAQFC file.
    e.http
        .set(PM25_PATH, 200, testdata("aqm_pm25_flipped.grib2"));
    let (beat, _) = recording_beat();
    let err = aqm::aqm_render_chunk(&e.cfg, &e.client, RUN, "air-quality", 0, beat)
        .await
        .unwrap_err();
    assert!(
        matches!(err, AqmError::MissingMessage { .. }),
        "got {err:?}"
    );
    assert!(
        count_files(&e.tile, ".png") == 0,
        "a failed chunk writes no tiles"
    );
}

#[tokio::test]
async fn both_palettes_render_when_configured() {
    let mut e = env().await;
    e.cfg.palette_names = vec!["classic".into(), "vivid".into()];
    let (beat, _) = recording_beat();
    let r = aqm::aqm_render_chunk(&e.cfg, &e.client, RUN, "air-quality", 0, beat)
        .await
        .unwrap();
    assert_eq!(r.rendered_timestamps.len(), 24);
    for palette in ["classic", "vivid"] {
        let dir = e
            .tile
            .join("air-quality")
            .join(palette)
            .join("runs")
            .join(RUN);
        assert_eq!(
            std::fs::read_dir(&dir).unwrap().count(),
            24,
            "{palette} frames"
        );
    }
}

#[tokio::test]
async fn slow_download_emits_heartbeats_with_phase_details() {
    let e = env().await;
    e.http.clear(PM25_PATH);
    e.http.set_delayed(
        PM25_PATH,
        200,
        testdata("aqm_pm25_mini.grib2"),
        Duration::from_millis(700),
    );
    let (beat, log) = recording_beat();
    aqm::aqm_render_chunk(&e.cfg, &e.client, RUN, "air-quality", 0, beat)
        .await
        .unwrap();
    let beats = log.lock().unwrap().clone();
    assert!(
        beats.len() >= 2,
        "expected heartbeats while waiting, got {}",
        beats.len()
    );
    assert!(
        beats
            .iter()
            .any(|b| b["phase"] == "download" && b["layer"] == "air-quality")
    );
    assert!(beats.iter().any(|b| b["phase"] == "render"));
}

#[tokio::test]
async fn failed_download_leaves_no_partial_file_and_a_retry_succeeds() {
    let e = env().await;
    e.http.clear(PM25_PATH);
    e.http.set(PM25_PATH, 500, Vec::new());
    e.http.set(NOMADS_PM25_PATH, 500, Vec::new());
    let (beat, _) = recording_beat();
    let err = aqm::aqm_render_chunk(&e.cfg, &e.client, RUN, "air-quality", 0, beat)
        .await
        .unwrap_err();
    assert!(
        matches!(err, AqmError::Status { status: 500, .. }),
        "got {err:?}"
    );

    let cache = e.tmp.join("cache").join(RUN);
    assert!(!cache.join("ave_1hr_pm25_bc.grib2").exists());
    assert!(
        !cache.join("ave_1hr_pm25_bc.grib2.part").exists(),
        "partial file must be removed"
    );

    e.http.set(PM25_PATH, 200, testdata("aqm_pm25_mini.grib2"));
    let (beat, _) = recording_beat();
    let ok = aqm::aqm_render_chunk(&e.cfg, &e.client, RUN, "air-quality", 0, beat)
        .await
        .unwrap();
    assert_eq!(ok.rendered_timestamps.len(), 24);
}

#[tokio::test]
async fn both_sources_failing_reports_the_last_error() {
    let e = env().await;
    e.http.clear(PM25_PATH);
    e.http.set(PM25_PATH, 403, Vec::new());
    e.http.set(NOMADS_PM25_PATH, 503, Vec::new());
    let (beat, _) = recording_beat();
    let err = aqm::aqm_render_chunk(&e.cfg, &e.client, RUN, "air-quality", 0, beat)
        .await
        .unwrap_err();
    assert!(
        matches!(err, AqmError::Status { status: 503, .. }),
        "got {err:?}"
    );
}

// ---- publication -----------------------------------------------------------

#[tokio::test]
async fn full_layer_publishes_72_frames_with_first_and_last_invariants() {
    let e = env().await;
    let results = render_layer(&e, "air-quality").await;
    let published = aqm::aqm_publish_run(&e.cfg, RUN, &results).await.unwrap();
    assert_eq!(published, vec!["air-quality".to_string()]);

    let m = read_manifest(&e.state).unwrap();
    let layer = &m.layers["air-quality"];
    assert_eq!(layer.timestamps.len(), 72);
    assert_eq!(
        layer.timestamps.first().map(String::as_str),
        Some("2026-07-16T12:00:00+00:00")
    );
    assert_eq!(
        layer.timestamps.last().map(String::as_str),
        Some("2026-07-19T11:00:00+00:00")
    );
    assert_eq!(
        layer.latest.as_deref(),
        layer.timestamps.last().map(String::as_str)
    );
    assert_eq!(layer.extra["run_id"], RUN);
    assert_eq!(layer.extra["complete"], true);
    assert_eq!(layer.palettes, vec!["classic".to_string()]);
    let first = &layer.frames[0];
    assert_eq!(first.path, "runs/20260716_12/2026-07-16T12:00:00+00:00");
    assert_eq!(
        first.extra["lead_minutes"], 0,
        "first frame is run + 0 h for a 4.0 product"
    );
    assert_eq!(first.extra["run_id"], RUN);
    assert_eq!(first.extra["source"], "aqm");
    assert_eq!(layer.frames.last().unwrap().extra["lead_minutes"], 71 * 60);
    assert!(
        !m.layers.contains_key("ozone"),
        "no ozone key when ozone was not rendered"
    );
}

#[tokio::test]
async fn two_palettes_are_recorded_on_the_layer() {
    let mut e = env().await;
    e.cfg.palette_names = vec!["classic".into(), "vivid".into()];
    let mut results = Vec::new();
    for start in [0, 24, 48] {
        let (beat, _) = recording_beat();
        results.push(
            aqm::aqm_render_chunk(&e.cfg, &e.client, RUN, "air-quality", start, beat)
                .await
                .unwrap(),
        );
    }
    aqm::aqm_publish_run(&e.cfg, RUN, &results).await.unwrap();
    let m = read_manifest(&e.state).unwrap();
    assert_eq!(
        m.layers["air-quality"].palettes,
        vec!["classic".to_string(), "vivid".to_string()]
    );
}

#[tokio::test]
async fn partial_layer_is_refused_and_leaves_the_manifest_alone() {
    let e = env().await;
    let fifty: Vec<String> = (0..50)
        .map(|i| format!("2026-07-16T{:02}:00:00+00:00", i % 24))
        .collect();
    let partial = vec![chunk("air-quality", 0, fifty)];
    let published = aqm::aqm_publish_run(&e.cfg, RUN, &partial).await.unwrap();
    assert!(published.is_empty());
    assert!(read_manifest(&e.state).unwrap().layers.is_empty());
}

#[tokio::test]
async fn two_chunks_summing_to_72_publish_and_an_incomplete_layer_is_dropped() {
    let e = env().await;
    let all = all_timestamps(&render_layer(&e, "air-quality").await);
    let results = vec![
        chunk("air-quality", 0, all[0..36].to_vec()),
        chunk("air-quality", 36, all[36..72].to_vec()),
        chunk("ozone", 0, all[0..36].to_vec()), // incomplete ozone: only one chunk of 36
    ];
    let published = aqm::aqm_publish_run(&e.cfg, RUN, &results).await.unwrap();
    assert_eq!(published, vec!["air-quality".to_string()]);
    let m = read_manifest(&e.state).unwrap();
    assert_eq!(m.layers["air-quality"].timestamps.len(), 72);
    assert!(
        !m.layers.contains_key("ozone"),
        "no ozone key when ozone is incomplete"
    );
}

#[tokio::test]
async fn ozone_only_publish_is_allowed() {
    let e = env().await;
    let results = render_layer(&e, "ozone").await;
    let published = aqm::aqm_publish_run(&e.cfg, RUN, &results).await.unwrap();
    assert_eq!(published, vec!["ozone".to_string()]);
    let m = read_manifest(&e.state).unwrap();
    assert!(!m.layers.contains_key("air-quality"));
    assert_eq!(m.layers["ozone"].timestamps.len(), 72);
}

#[tokio::test]
async fn corrupt_manifest_fails_the_publish() {
    let e = env().await;
    let results = render_layer(&e, "air-quality").await;
    std::fs::write(e.state.join("manifest.json"), b"{ not json").unwrap();
    assert!(aqm::aqm_publish_run(&e.cfg, RUN, &results).await.is_err());
}

#[tokio::test]
async fn bad_run_id_is_rejected_before_publishing() {
    let e = env().await;
    let results = render_layer(&e, "air-quality").await;
    let err = aqm::aqm_publish_run(&e.cfg, "2026-07-16", &results)
        .await
        .unwrap_err();
    assert!(matches!(err, AqmError::Invalid(_)), "got {err:?}");
}

// ---- processed set and cleanup --------------------------------------------

#[tokio::test]
async fn processed_set_keeps_the_newest_100_runs() {
    let e = env().await;
    for i in 0..101 {
        aqm::aqm_mark_processed(&e.cfg, &format!("run{i:03}"))
            .await
            .unwrap();
    }
    let set = ProcessedSet::open(e.cfg.processed_path(), 100).unwrap();
    assert!(!set.contains("run000"), "oldest entry dropped");
    assert!(set.contains("run100"), "newest entry kept");
    assert!(set.contains("run001"));
}

#[tokio::test]
async fn cleanup_removes_expired_runs_keeps_current_and_recent_and_prunes_cache() {
    let e = env().await;
    // Current run of the layer, in the manifest, is old but must survive.
    let mut meta = serde_json::Map::new();
    meta.insert(
        "run_id".into(),
        serde_json::Value::String("20260716_12".into()),
    );
    // A layer with no frames is not kept in the manifest, so give it one (the tile dirs need not exist).
    radar_core::manifest::replace_layer(
        &e.state,
        "air-quality",
        &["2026-07-16T12:00:00+00:00".to_string()],
        &radar_core::manifest::ReplaceOptions {
            palettes: vec![],
            frames: vec![],
            layer_metadata: Some(meta),
        },
    )
    .unwrap();

    let runs = e.tile.join("air-quality/classic/runs");
    for run in [
        "20260101_00",
        "20260710_12",
        "20260716_12",
        "20260719_00",
        "scratch",
    ] {
        std::fs::create_dir_all(runs.join(run)).unwrap();
    }
    for run in ["20260710_12", "20260716_12"] {
        std::fs::create_dir_all(e.tmp.join("cache").join(run)).unwrap();
    }

    let now = SystemTime::from(Utc.with_ymd_and_hms(2026, 7, 20, 0, 0, 0).unwrap());
    let result = aqm::aqm_cleanup_at(&e.cfg, 48, now).await.unwrap();

    assert!(
        !runs.join("20260101_00").exists(),
        "old non-current run removed"
    );
    assert!(
        !runs.join("20260710_12").exists(),
        "old non-current run removed"
    );
    assert!(
        runs.join("20260716_12").exists(),
        "current manifest run kept"
    );
    assert!(runs.join("20260719_00").exists(), "recent run kept");
    assert!(runs.join("scratch").exists(), "unparseable name left alone");
    assert_eq!(result.tile_dirs_removed, 2);

    assert!(
        !e.tmp.join("cache/20260710_12").exists(),
        "older cache entry pruned"
    );
    assert!(
        e.tmp.join("cache/20260716_12").exists(),
        "newest cache entry kept"
    );
}
