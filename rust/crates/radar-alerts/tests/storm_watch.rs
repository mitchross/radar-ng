//! Storm-watch frame comparison and change detection against synthetic radar grids (spec-shared section 8,
//! spec-temporal 8.5 and 8.6, Python `storm_watch_activities.py`).

use std::path::Path;

use radar_alerts::storm_watch::{compare_radar_frames, detect_storm_change};
use radar_storms::watch::WatchConfig;
use radar_types::payloads::watch_storm::{CompareFramesInput, DetectChangeInput};

const STAMP: &str = "2026-10-14T00:00:00Z";

/// Write one frame in the layout the radar pipeline produces: `radar/<stamp>.meta.json` with the bounds, and a
/// little-endian f32 `<stamp>.bin` with row 0 at `lat_max`.
fn write_frame(grid_dir: &Path, stamp: &str, height: usize, width: usize, values: &[f32]) {
    assert_eq!(values.len(), height * width);
    let radar = grid_dir.join("radar");
    std::fs::create_dir_all(&radar).unwrap();
    let meta = serde_json::json!({
        "height": height,
        "width": width,
        "lat_min": 30.0,
        "lat_max": 40.0,
        "lon_min": -100.0,
        "lon_max": -90.0,
    });
    std::fs::write(radar.join(format!("{stamp}.meta.json")), meta.to_string()).unwrap();
    let bytes: Vec<u8> = values.iter().flat_map(|v| v.to_le_bytes()).collect();
    std::fs::write(radar.join(format!("{stamp}.bin")), bytes).unwrap();
}

/// A 5 by 5 grid with a 45 dBZ core at (lat 35, lng -95) and a weaker 30 dBZ cell beside it. Everything else is
/// the -9999 fill value, which must be ignored.
fn storm_frame() -> Vec<f32> {
    let mut values = vec![-9999.0_f32; 25];
    values[2 * 5 + 2] = 45.0;
    values[5 + 1] = 30.0;
    values
}

fn compare_input(prev_timestamp: Option<&str>, prev_max_dbz: Option<f64>) -> CompareFramesInput {
    CompareFramesInput {
        lat: 35.0,
        lng: -95.0,
        prev_timestamp: prev_timestamp.map(str::to_owned),
        prev_max_dbz,
    }
}

#[tokio::test]
async fn compare_samples_the_latest_frame_around_the_point() {
    let dir = tempfile::tempdir().unwrap();
    write_frame(dir.path(), STAMP, 5, 5, &storm_frame());

    let first = compare_radar_frames(dir.path(), &compare_input(None, None))
        .await
        .unwrap();
    assert!(first.sampled);
    assert_eq!(first.curr_timestamp, STAMP);
    assert_eq!(first.curr_max_dbz, 45.0);
    assert!(!first.has_prev);
    assert_eq!(first.max_dbz_delta, 0.0);

    // The same frame as the previous poll is not a new sample.
    let same = compare_radar_frames(dir.path(), &compare_input(Some(STAMP), Some(45.0)))
        .await
        .unwrap();
    assert!(!same.sampled);

    // A different previous frame gives a delta.
    let changed = compare_radar_frames(dir.path(), &compare_input(Some("older"), Some(30.0)))
        .await
        .unwrap();
    assert!(changed.sampled && changed.has_prev);
    assert_eq!(changed.prev_max_dbz, 30.0);
    assert_eq!(changed.max_dbz_delta, 15.0);
}

#[tokio::test]
async fn compare_reports_nothing_without_data_or_outside_the_grid() {
    let empty = tempfile::tempdir().unwrap();
    let none = compare_radar_frames(empty.path(), &compare_input(None, None))
        .await
        .unwrap();
    assert!(!none.sampled);

    let dir = tempfile::tempdir().unwrap();
    write_frame(dir.path(), STAMP, 5, 5, &storm_frame());
    let outside = CompareFramesInput {
        lat: 50.0,
        lng: -95.0,
        prev_timestamp: None,
        prev_max_dbz: None,
    };
    let result = compare_radar_frames(dir.path(), &outside).await.unwrap();
    assert!(!result.sampled, "a point outside the grid has no data");
}

#[tokio::test]
async fn compare_ignores_fill_values_in_the_window() {
    let dir = tempfile::tempdir().unwrap();
    // Every cell is fill except one moderate value, so the max is that value and not the fill.
    let mut values = vec![-9999.0_f32; 25];
    values[2 * 5 + 2] = 22.0;
    write_frame(dir.path(), STAMP, 5, 5, &values);
    let result = compare_radar_frames(dir.path(), &compare_input(None, None))
        .await
        .unwrap();
    assert!(result.sampled);
    assert_eq!(result.curr_max_dbz, 22.0);
}

#[tokio::test]
async fn detect_change_follows_the_threshold_table() {
    let cfg = WatchConfig::default();
    let case = |curr: f64, prev: f64, delta: f64, has_prev: bool| DetectChangeInput {
        has_prev,
        curr_max_dbz: curr,
        prev_max_dbz: prev,
        max_dbz_delta: delta,
    };

    let severe = detect_storm_change(&case(65.0, 40.0, 25.0, true), &cfg).await;
    assert_eq!(
        (severe.kind.as_str(), severe.summary.as_str()),
        ("severe", "reflectivity 65 dBZ — possible hail/severe")
    );

    // Severe takes precedence over intensifying when both apply.
    let both = detect_storm_change(&case(62.0, 45.0, 17.0, true), &cfg).await;
    assert_eq!(both.kind, "severe");

    let rising = detect_storm_change(&case(45.0, 30.0, 15.0, true), &cfg).await;
    assert_eq!(
        (rising.kind.as_str(), rising.summary.as_str()),
        ("intensifying", "+15 dBZ since last frame (30→45)")
    );

    let falling = detect_storm_change(&case(30.0, 45.0, -15.0, true), &cfg).await;
    assert_eq!(
        (falling.kind.as_str(), falling.summary.as_str()),
        ("dissipating", "-15 dBZ since last frame (45→30)")
    );

    // A change below both deltas is not an event.
    let small = detect_storm_change(&case(35.0, 40.0, -5.0, true), &cfg).await;
    assert_eq!((small.kind.as_str(), small.summary.as_str()), ("", ""));

    // No previous frame: nothing to classify.
    let first = detect_storm_change(&case(65.0, 0.0, 0.0, false), &cfg).await;
    assert_eq!((first.kind.as_str(), first.summary.as_str()), ("", ""));
}

#[tokio::test]
async fn detect_change_respects_configured_thresholds() {
    let cfg = WatchConfig {
        hail_dbz_threshold: 50.0,
        intensify_dbz_delta: 5.0,
        dissipate_dbz_delta: 5.0,
    };
    let input = DetectChangeInput {
        has_prev: true,
        curr_max_dbz: 52.0,
        prev_max_dbz: 40.0,
        max_dbz_delta: 12.0,
    };
    assert_eq!(detect_storm_change(&input, &cfg).await.kind, "severe");
    let milder = DetectChangeInput {
        has_prev: true,
        curr_max_dbz: 44.0,
        prev_max_dbz: 40.0,
        max_dbz_delta: 4.0,
    };
    assert_eq!(detect_storm_change(&milder, &cfg).await.kind, "");
}
