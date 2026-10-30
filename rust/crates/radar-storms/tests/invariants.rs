//! Invariant tests named after the checklist in spec-shared section 13 (STM, DRV)
//! and the PLAN.md defect fixes (D1, D2, D3, D5, D6, D7). These assert the
//! behaviour directly, so they stay meaningful even if the golden fixtures change.

use std::fs;

use radar_storms::detect::detect_storms_with_clock;
use radar_storms::prefetch::{PrefetchRequest, build_storm_prefetch_plan, read_manifest_value};
use radar_storms::watch::{
    FrameDiff, FrameSample, WatchConfig, compare_radar_frames, detect_change, latest_radar_meta,
    sample_window,
};
use radar_storms::{Grid, StormConfig, StormsError};
use serde_json::{Value, json};

/// numpy `linspace(start, stop, n)`: `start + i * step`, last element exactly `stop`.
fn linspace(start: f64, stop: f64, n: usize) -> Vec<f64> {
    let step = (stop - start) / (n as f64 - 1.0);
    let mut v: Vec<f64> = (0..n).map(|i| i as f64 * step + start).collect();
    v[n - 1] = stop;
    v
}

fn rects(h: usize, w: usize, cells: &[(usize, usize, usize, usize, f32)]) -> Vec<f32> {
    let mut data = vec![0.0_f32; h * w];
    for &(r0, r1, c0, c1, v) in cells {
        for r in r0..r1 {
            for c in c0..c1 {
                data[r * w + c] = v;
            }
        }
    }
    data
}

/// Same placement as `test_storms._cell_frame`, as one 5x5 (or `size`) block.
fn cell_frame(lon_center: f64, size: usize) -> Vec<f32> {
    let lat_center = 42.0_f64;
    let r = ((lat_center - 41.0) / 0.01).round_ties_even() as usize;
    let c = ((lon_center + 86.0) / 0.01).round_ties_even() as usize;
    rects(200, 300, &[(r, r + size, c, c + size, 55.0)])
}

fn cell_axes() -> (Vec<f64>, Vec<f64>) {
    let lats: Vec<f64> = (0..200).map(|i| 41.0 + 0.01 * f64::from(i)).collect();
    let lons: Vec<f64> = (0..300).map(|i| -86.0 + 0.01 * f64::from(i)).collect();
    (lats, lons)
}

fn detect(
    data: &[f32],
    h: usize,
    w: usize,
    lats: &[f64],
    lons: &[f64],
    ts: &str,
    prev: Option<&Value>,
) -> Value {
    let grid = Grid::new(data, h, w, lats, lons).unwrap();
    let doc = detect_storms_with_clock(&grid, ts, prev, &StormConfig::default(), 0.0);
    serde_json::to_value(doc).unwrap()
}

fn first_props(doc: &Value) -> &Value {
    &doc["features"][0]["properties"]
}

#[test]
fn stm01_tracking_keeps_cell_id_and_predicts_three_bboxes() {
    let lats = linspace(43.0, 42.81, 20);
    let lons = linspace(-85.0, -84.81, 20);
    let first = detect(
        &rects(20, 20, &[(7, 10, 7, 10, 55.0)]),
        20,
        20,
        &lats,
        &lons,
        "2026-07-10T12:00:00+00:00",
        None,
    );
    let second = detect(
        &rects(20, 20, &[(7, 10, 8, 11, 55.0)]),
        20,
        20,
        &lats,
        &lons,
        "2026-07-10T12:05:00+00:00",
        Some(&first),
    );
    let p = first_props(&second);
    assert_eq!(p["cell_id"], first_props(&first)["cell_id"]);
    assert!(p["tracking_vector"]["east_kmh"].as_f64().unwrap() > 0.0);
    let leads: Vec<i64> = p["predicted_bboxes"]
        .as_array()
        .unwrap()
        .iter()
        .map(|b| b["lead_minutes"].as_i64().unwrap())
        .collect();
    assert_eq!(leads, vec![0, 5, 10]);
    assert!(
        p["predicted_bboxes"][2]["bbox"][0].as_f64().unwrap()
            > p["predicted_bboxes"][0]["bbox"][0].as_f64().unwrap()
    );
    assert!(second.get("tracking_vectors").is_none());
}

#[test]
fn stm05_motion_is_fitted_over_history_not_one_noisy_step() {
    let km_per_deg = 111.320 * 42.0_f64.to_radians().cos();
    let wobble = [0.0, 0.01, -0.01, 0.01, 0.0, -0.01, 0.01, 0.0];
    let (lats, lons) = cell_axes();
    let mut prev: Option<Value> = None;
    for (step, jitter) in wobble.iter().enumerate() {
        let lon = -85.5 + (60.0 * step as f64 * 2.0 / 60.0) / km_per_deg + jitter;
        let ts = format!("2026-07-10T12:{:02}:00+00:00", step * 2);
        let doc = detect(
            &cell_frame(lon, 5),
            200,
            300,
            &lats,
            &lons,
            &ts,
            prev.as_ref(),
        );
        prev = Some(doc);
    }
    let p = first_props(prev.as_ref().unwrap()).clone();
    assert_eq!(p["track_history"].as_array().unwrap().len(), wobble.len());
    let speed = p["tracking_vector"]["speed_kmh"].as_f64().unwrap();
    let bearing = p["tracking_vector"]["bearing_deg"].as_f64().unwrap();
    assert!(45.0 < speed && speed < 75.0, "speed {speed}");
    assert!(75.0 < bearing && bearing < 105.0, "bearing {bearing}");
    assert!(p["tracking_confidence"].as_f64().unwrap() > 0.5);
}

#[test]
fn stm06_gap_under_five_minutes_reports_no_motion() {
    let (lats, lons) = cell_axes();
    let first = detect(
        &cell_frame(-85.5, 5),
        200,
        300,
        &lats,
        &lons,
        "2026-07-10T12:00:00+00:00",
        None,
    );
    let second = detect(
        &cell_frame(-85.48, 5),
        200,
        300,
        &lats,
        &lons,
        "2026-07-10T12:02:00+00:00",
        Some(&first),
    );
    let p = first_props(&second);
    assert_eq!(p["cell_id"], first_props(&first)["cell_id"]);
    assert_eq!(p["tracking_vector"]["speed_kmh"], json!(0.0));
    assert_eq!(p["tracking_confidence"], json!(0.0));
}

#[test]
fn stm07_distant_cell_is_not_associated() {
    let (lats, lons) = cell_axes();
    let first = detect(
        &cell_frame(-85.5, 5),
        200,
        300,
        &lats,
        &lons,
        "2026-07-10T12:00:00+00:00",
        None,
    );
    let second = detect(
        &cell_frame(-85.3, 5),
        200,
        300,
        &lats,
        &lons,
        "2026-07-10T12:02:00+00:00",
        Some(&first),
    );
    assert_ne!(
        first_props(&second)["cell_id"],
        first_props(&first)["cell_id"]
    );
}

#[test]
fn stm08_area_uses_the_source_grid_cell_size() {
    let (lats, lons) = cell_axes();
    let doc = detect(
        &cell_frame(-85.5, 10),
        200,
        300,
        &lats,
        &lons,
        "2026-07-10T12:00:00+00:00",
        None,
    );
    let area = first_props(&doc)["area_km2"].as_f64().unwrap();
    let expected = 100.0 * (0.01 * 110.574) * (0.01 * 111.320 * 42.05_f64.to_radians().cos());
    assert!(
        (area - expected).abs() / expected < 0.05,
        "{area} vs {expected}"
    );
}

#[test]
fn drv06_matched_keep_id_and_new_cells_take_max_plus_one() {
    let lats = linspace(43.0, 42.6, 40);
    let lons = linspace(-85.0, -84.6, 40);
    let f0 = rects(
        40,
        40,
        &[
            (2, 5, 2, 5, 60.0),
            (10, 13, 10, 13, 60.0),
            (30, 33, 30, 33, 60.0),
        ],
    );
    let d0 = detect(&f0, 40, 40, &lats, &lons, "2026-07-10T12:00:00+00:00", None);
    // Frame 1 keeps two cells and drops the one at (30,30): its id must not come back.
    let f1 = rects(
        40,
        40,
        &[
            (2, 5, 2, 5, 60.0),
            (10, 13, 10, 13, 60.0),
            (20, 23, 20, 23, 60.0),
        ],
    );
    let d1 = detect(
        &f1,
        40,
        40,
        &lats,
        &lons,
        "2026-07-10T12:05:00+00:00",
        Some(&d0),
    );
    let ids: Vec<i64> = d1["features"]
        .as_array()
        .unwrap()
        .iter()
        .map(|f| f["properties"]["cell_id"].as_i64().unwrap())
        .collect();
    // Peaks tie at 60, so label order decides: the first two are the kept cells 1 and 2.
    assert!(ids.contains(&1) && ids.contains(&2));
    let fresh = ids.iter().copied().find(|&i| i != 1 && i != 2).unwrap();
    assert_eq!(fresh, 4, "IDs are never reused (D3)");
    assert_eq!(d1["next_cell_id"], json!(5));
}

#[test]
fn empty_frame_keeps_the_python_short_shape_and_the_counter() {
    // No pixel reaches the threshold: Python returns the short shape. The counter must survive (D3).
    let (lats, lons) = (linspace(43.0, 42.6, 10), linspace(-85.0, -84.6, 10));
    let prev = json!({"timestamp": "2026-07-10T12:00:00+00:00", "features": [], "next_cell_id": 7});
    let doc = detect(
        &vec![0.0_f32; 100],
        10,
        10,
        &lats,
        &lons,
        "2026-07-10T12:05:00+00:00",
        Some(&prev),
    );
    let mut keys: Vec<&str> = doc
        .as_object()
        .unwrap()
        .keys()
        .map(String::as_str)
        .collect();
    keys.sort_unstable();
    assert_eq!(
        keys,
        vec![
            "features",
            "generated_at",
            "next_cell_id",
            "timestamp",
            "type"
        ]
    );
    assert_eq!(
        doc["next_cell_id"],
        json!(7),
        "the counter never goes backwards"
    );
}

#[test]
fn components_all_filtered_by_size_keep_the_full_shape() {
    // Python's non-empty branch still runs when every component is below MIN_PIXELS (6.10).
    let (lats, lons) = (linspace(43.0, 42.6, 10), linspace(-85.0, -84.6, 10));
    let data = rects(10, 10, &[(1, 2, 1, 3, 50.0)]);
    let doc = detect(
        &data,
        10,
        10,
        &lats,
        &lons,
        "2026-07-10T12:05:00+00:00",
        None,
    );
    assert_eq!(doc["cell_count"], json!(0));
    assert_eq!(doc["threshold_dbz"], json!(40.0));
}

#[test]
fn feature_cap_and_min_pixels() {
    // 601 five-pixel components in alternating rows: cap at 500 (STORM_MAX_CELLS).
    let h = 1201;
    let w = 5;
    let mut cells = Vec::new();
    for r in (0..h).step_by(2) {
        cells.push((r, r + 1, 0, 5, 50.0_f32));
    }
    let data = rects(h, w, &cells);
    let lats = linspace(0.0, 1.0, h);
    let lons = linspace(-90.0, -89.9, w);
    let doc = detect(&data, h, w, &lats, &lons, "2026-07-10T12:00:00+00:00", None);
    assert_eq!(doc["features"].as_array().unwrap().len(), 500);
    assert_eq!(doc["cell_count"], json!(500));
}

#[test]
fn stable_peak_order_and_size_filter() {
    // The 1x4 strip at row 17 is 4 pixels of 90 dBZ: below MIN_PIXELS, so it must vanish.
    let data = rects(
        20,
        20,
        &[
            (2, 5, 2, 5, 50.0),
            (10, 14, 10, 14, 50.0),
            (17, 18, 16, 20, 90.0),
        ],
    );
    let lats = linspace(10.0, 9.0, 20);
    let lons = linspace(-70.0, -69.0, 20);
    let doc = detect(
        &data,
        20,
        20,
        &lats,
        &lons,
        "2026-07-10T12:00:00+00:00",
        None,
    );
    // The 1x4 strip is below MIN_PIXELS=5 and is dropped. The two 50 dBZ cells keep label order.
    let peaks: Vec<f64> = doc["features"]
        .as_array()
        .unwrap()
        .iter()
        .map(|f| f["properties"]["peak_dbz"].as_f64().unwrap())
        .collect();
    assert_eq!(peaks, vec![50.0, 50.0]);
    let pix: Vec<u64> = doc["features"]
        .as_array()
        .unwrap()
        .iter()
        .map(|f| f["properties"]["pixel_count"].as_u64().unwrap())
        .collect();
    assert_eq!(pix, vec![9, 16]);
}

#[test]
fn normalises_longitudes_above_180() {
    let data = rects(20, 20, &[(5, 10, 5, 10, 55.0)]);
    let lats = linspace(43.0, 42.0, 20);
    let lons_east: Vec<f64> = linspace(-95.0, -94.0, 20)
        .into_iter()
        .map(|x| x + 360.0)
        .collect();
    let lons_west = linspace(-95.0, -94.0, 20);
    let a = detect(
        &data,
        20,
        20,
        &lats,
        &lons_east,
        "2026-07-10T12:00:00+00:00",
        None,
    );
    let b = detect(
        &data,
        20,
        20,
        &lats,
        &lons_west,
        "2026-07-10T12:00:00+00:00",
        None,
    );
    assert_eq!(
        a["features"][0]["geometry"]["coordinates"],
        b["features"][0]["geometry"]["coordinates"]
    );
}

#[test]
fn mixed_naive_and_aware_timestamps_do_not_crash() {
    // D6: Python raises TypeError on naive-minus-aware. Here naive is UTC.
    let data = rects(20, 20, &[(7, 10, 7, 10, 55.0)]);
    let lats = linspace(43.0, 42.81, 20);
    let lons = linspace(-85.0, -84.81, 20);
    let prev = detect(&data, 20, 20, &lats, &lons, "2026-07-10T12:00:00", None);
    let doc = detect(
        &data,
        20,
        20,
        &lats,
        &lons,
        "2026-07-10T12:05:00Z",
        Some(&prev),
    );
    assert_eq!(first_props(&doc)["cell_id"], first_props(&prev)["cell_id"]);
}

#[test]
fn d1_nowcast_paths_come_from_the_manifest_frame() {
    // Hand-built storm whose predicted bboxes are exactly four tiles at zoom 6 (x 15..16, y 31..32).
    let storms = json!({
        "type": "FeatureCollection", "generated_at": 1.0, "timestamp": "2026-07-10T12:00:00+00:00",
        "features": [{
            "type": "Feature",
            "geometry": {"type": "Point", "coordinates": [-90.0, 0.0]},
            "properties": {
                "peak_dbz": 50.0, "area_km2": 10.0, "pixel_count": 9, "bbox": [-90.5, -0.5, -89.5, 0.5],
                "cell_id": 1, "track_history": [], "tracking_vector": {"east_kmh": 0.0, "north_kmh": 0.0, "speed_kmh": 0.0, "bearing_deg": 0.0},
                "tracking_confidence": 0.0,
                "predicted_bboxes": [
                    {"lead_minutes": 0, "bbox": [-90.5, -0.5, -89.5, 0.5]},
                    {"lead_minutes": 5, "bbox": [-90.5, -0.5, -89.5, 0.5]},
                    {"lead_minutes": 10, "bbox": [-90.5, -0.5, -89.5, 0.5]}
                ]
            }
        }]
    });
    let manifest = json!({"schema_version": 2, "layers": {
        "radar": {"timestamps": ["2026-07-10T12:00:00+00:00"], "frames": [{"timestamp": "2026-07-10T12:00:00+00:00", "path": "2026-07-10T12:00:00+00:00"}]},
        "nowcast": {"timestamps": ["2026-07-10T12:05:00+00:00"], "frames": [{"timestamp": "2026-07-10T12:05:00+00:00", "path": "runs/2026-07-10T12:00:00+00:00/2026-07-10T12:05:00+00:00"}]}
    }});
    let tiles = tempfile::tempdir().unwrap();
    for layer_path in [
        "radar/classic/2026-07-10T12:00:00+00:00",
        "nowcast/classic/runs/2026-07-10T12:00:00+00:00/2026-07-10T12:05:00+00:00",
    ] {
        for x in [15, 16] {
            for y in [31, 32] {
                let p = tiles.path().join(format!("{layer_path}/6/{x}/{y}.png"));
                fs::create_dir_all(p.parent().unwrap()).unwrap();
                fs::write(p, b"png").unwrap();
            }
        }
    }
    let req = PrefetchRequest {
        lat: 0.0,
        lon: -90.0,
        zoom: 6,
        palette: "classic",
        base_url: "https://radar.example/",
    };
    let plan = build_storm_prefetch_plan(
        &storms,
        &manifest,
        tiles.path(),
        &req,
        &StormConfig::default(),
    )
    .unwrap();
    assert_eq!(plan.bboxes.len(), 3);
    // D1: lead 5 finds its four tiles under runs/<anchor>/<valid>.
    let nowcast = &plan.bboxes[1];
    assert_eq!(nowcast.layer.as_deref(), Some("nowcast"));
    assert_eq!(nowcast.tile_urls.len(), 4);
    assert_eq!(
        nowcast.tile_urls[0],
        "https://radar.example/tiles/nowcast/classic/runs/2026-07-10T12:00:00+00:00/2026-07-10T12:05:00+00:00/6/15/31.png"
    );
    // Tile order: x outer, y inner (_tiles_for_bbox).
    let order: Vec<String> = plan.bboxes[0]
        .tile_urls
        .iter()
        .map(|u| {
            let parts: Vec<&str> = u.split('/').collect();
            format!("{}/{}", parts[parts.len() - 2], parts[parts.len() - 1])
        })
        .collect();
    assert_eq!(order, ["15/31.png", "15/32.png", "16/31.png", "16/32.png"]);
    // style_url keeps the timestamp and escapes ':' and '+' as the API contract requires.
    assert!(
        nowcast
            .style_url
            .as_deref()
            .unwrap()
            .contains("timestamp=2026-07-10T12%3A05%3A00%2B00%3A00")
    );
    assert_eq!(
        plan.plan_id.as_deref(),
        Some("2026-07-10T12:00:00+00:00:1:classic:6")
    );
    assert_eq!(
        plan.tile_urls.len(),
        8,
        "union of radar and nowcast tiles, deduplicated"
    );
}

#[test]
fn d7_manifest_without_layers_fails_closed() {
    let dir = tempfile::tempdir().unwrap();
    assert!(
        read_manifest_value(dir.path()).unwrap()["layers"].is_object(),
        "missing file is empty"
    );
    fs::write(dir.path().join("manifest.json"), r#"{"schema_version": 2}"#).unwrap();
    assert!(matches!(
        read_manifest_value(dir.path()),
        Err(StormsError::Manifest { .. })
    ));
    fs::write(dir.path().join("manifest.json"), "").unwrap();
    assert!(matches!(
        read_manifest_value(dir.path()),
        Err(StormsError::Manifest { .. })
    ));
    fs::write(dir.path().join("manifest.json"), r#"{"layers": []}"#).unwrap();
    assert!(matches!(
        read_manifest_value(dir.path()),
        Err(StormsError::Manifest { .. })
    ));
}

#[test]
fn v2_manifest_reads_back_unchanged() {
    let dir = tempfile::tempdir().unwrap();
    let doc = json!({
        "schema_version": 2,
        "tile_url_template": "/tiles/{layer}/{palette}/{path}/{z}/{x}/{y}.png",
        "updated_at": "2026-07-10T12:00:00+00:00",
        "layers": {"radar": {
            "frames": [{"timestamp": "2026-07-10T12:00:00+00:00", "path": "run1", "palettes": ["classic"]}],
            "timestamps": ["2026-07-10T12:00:00+00:00"],
            "latest": "2026-07-10T12:00:00+00:00",
            "palettes": ["classic"],
        }},
    });
    fs::write(dir.path().join("manifest.json"), doc.to_string()).unwrap();
    assert_eq!(read_manifest_value(dir.path()).unwrap(), doc);
}

#[test]
fn no_selected_storm_gives_the_four_key_shape() {
    let storms = json!({"type": "FeatureCollection", "features": [], "timestamp": "2026-07-10T12:00:00+00:00"});
    let manifest = json!({"layers": {}});
    let req = PrefetchRequest {
        lat: 42.0,
        lon: -85.0,
        zoom: 6,
        palette: "classic",
        base_url: "https://x",
    };
    let plan = build_storm_prefetch_plan(
        &storms,
        &manifest,
        std::path::Path::new("/nonexistent"),
        &req,
        &StormConfig::default(),
    )
    .unwrap();
    let v = serde_json::to_value(&plan).unwrap();
    let mut keys: Vec<&str> = v.as_object().unwrap().keys().map(String::as_str).collect();
    keys.sort_unstable();
    assert_eq!(
        keys,
        vec!["bboxes", "plan_id", "storm_cell_id", "tile_urls"]
    );
    assert_eq!(v["plan_id"], Value::Null);
}

// ---- storm watch ----

fn write_small_meta(dir: &std::path::Path, name: &str, values: &[f32], h: usize, w: usize) {
    fs::create_dir_all(dir).unwrap();
    let bin = name.replace(".meta.json", ".bin");
    let bytes: Vec<u8> = values.iter().flat_map(|v| v.to_le_bytes()).collect();
    fs::write(dir.join(&bin), bytes).unwrap();
    let meta = json!({"height": h, "width": w, "lat_min": 0.0, "lat_max": 1.0, "lon_min": 0.0, "lon_max": 1.0, "data_file": bin});
    fs::write(dir.join(name), meta.to_string()).unwrap();
}

#[test]
fn drv08_out_of_coverage_and_fill_only_are_neg_inf_not_none() {
    let dir = tempfile::tempdir().unwrap();
    let meta = dir.path().join("t.meta.json");
    write_small_meta(dir.path(), "t.meta.json", &[-9999.0; 9], 3, 3);
    let outside = sample_window(&meta, 2.0, 0.5, 5.0).expect("a sample, not None");
    assert_eq!(outside.max_dbz, f64::NEG_INFINITY);
    let fill = sample_window(&meta, 0.5, 0.5, 5.0).expect("a sample, not None");
    assert_eq!(fill.max_dbz, f64::NEG_INFINITY);
    assert_eq!(fill.above_50_count, 0);
    assert!(!fill.has_data());
}

#[test]
fn half_even_window_rounding_and_fill_exclusion() {
    let dir = tempfile::tempdir().unwrap();
    let meta = dir.path().join("t.meta.json");
    // 5x5 grid over lat/lon 0..1: centre (2,2) with a 1-pixel radius gives a 3x3 window.
    let mut vals = vec![10.0_f32; 25];
    vals[12] = 60.0;
    vals[0] = -9999.0;
    write_small_meta(dir.path(), "t.meta.json", &vals, 5, 5);
    let s = sample_window(&meta, 0.5, 0.5, 1.0).unwrap();
    assert_eq!(s.max_dbz, 60.0);
    assert_eq!(s.above_50_count, 1);
}

#[test]
fn latest_radar_meta_picks_the_newest_visible_name() {
    let dir = tempfile::tempdir().unwrap();
    let radar = dir.path().join("radar");
    fs::create_dir_all(&radar).unwrap();
    for name in [
        "2026-07-10T12:00:00+00:00.meta.json",
        "2026-07-10T12:30:00+00:00.meta.json",
        ".zzz.meta.json",
    ] {
        fs::write(radar.join(name), "{}").unwrap();
    }
    let latest = latest_radar_meta(dir.path()).unwrap();
    assert_eq!(
        latest.file_name().unwrap(),
        "2026-07-10T12:30:00+00:00.meta.json"
    );
    assert!(latest_radar_meta(&dir.path().join("none")).is_none());
}

#[test]
fn d2_no_data_never_produces_a_change() {
    let cfg = WatchConfig::default();
    let prev = FrameSample {
        timestamp: "p".into(),
        max_dbz: 30.0,
        mean_dbz: f64::NAN,
        above_50_count: 0,
    };
    let no_data = FrameSample {
        timestamp: "c".into(),
        max_dbz: f64::NEG_INFINITY,
        mean_dbz: f64::NAN,
        above_50_count: 0,
    };
    let diff = FrameDiff {
        prev: Some(prev.clone()),
        curr: no_data.clone(),
        max_dbz_delta: f64::NEG_INFINITY,
    };
    assert!(detect_change(&diff, &cfg).is_none());
    let diff = FrameDiff {
        prev: Some(FrameSample {
            max_dbz: -99.0,
            ..prev.clone()
        }),
        curr: FrameSample {
            max_dbz: 59.0,
            ..no_data
        },
        max_dbz_delta: 158.0,
    };
    assert!(
        detect_change(&diff, &cfg).is_none(),
        "a -99 sentinel is not a previous reading"
    );
}

#[test]
fn compare_radar_frames_with_no_grid_is_not_sampled() {
    let dir = tempfile::tempdir().unwrap();
    let r = compare_radar_frames(dir.path(), 42.0, -85.0, None, None);
    assert!(!r.sampled);
    assert_eq!(r.max_dbz_delta, 0.0);
}
