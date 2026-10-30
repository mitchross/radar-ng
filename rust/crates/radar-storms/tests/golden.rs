//! Golden tests: the Rust port must reproduce the Python outputs in `testdata/`.
//!
//! The fixtures come from `testdata/gen_golden.py`, which runs the original
//! radar-ng Python on deterministic inputs. Where a fixture records a Python
//! defect (D1, D2, D3), the test asserts the fixed behavior instead, and the
//! fixed expectation comes from a reference oracle in the generator or from the
//! invariant itself.

mod common;

use std::fs;

use common::{assert_json_close, build_grid, testdata};
use radar_storms::detect::detect_storms_with_clock;
use radar_storms::numeric::{np_mean_f32, np_sum_f32, np_sum_f64, round_digits, round_half_even};
use radar_storms::prefetch::{PrefetchRequest, build_storm_prefetch_plan_from_state};
use radar_storms::storage::{STORMS_FILE, WriteOutcome, write_storms_json_with_clock};
use radar_storms::watch::{
    ChangeKind, FrameDiff, FrameSample, WatchConfig, compare_radar_frames, compare_sample,
    detect_change, detect_storm_change, latest_radar_meta, sample_window,
};
use radar_storms::{Grid, StormConfig};
use serde_json::{Value, json};

fn f64_list(v: &Value) -> Vec<f64> {
    v.as_array()
        .unwrap()
        .iter()
        .map(|x| x.as_f64().unwrap())
        .collect()
}

#[test]
fn storms_detect_matches_python_chains() {
    let golden = testdata("storms_detect.json");
    let cfg = StormConfig::default();
    for case in golden["cases"].as_array().unwrap() {
        let name = case["name"].as_str().unwrap();
        let lats = f64_list(&case["lats"]);
        let lons = f64_list(&case["lons"]);
        let (h, w) = (lats.len(), lons.len());
        let frames = case["frames"].as_array().unwrap();
        let expected = case["expected"].as_array().unwrap();
        let mut prev: Option<Value> = None;
        for (i, frame) in frames.iter().enumerate() {
            let data = build_grid(&frame["grid"], h, w);
            let grid = Grid::new(&data, h, w, &lats, &lons).unwrap();
            let ts = frame["timestamp"].as_str().unwrap();
            let doc = detect_storms_with_clock(&grid, ts, prev.as_ref(), &cfg, 0.0);
            let actual = serde_json::to_value(&doc).unwrap();
            let mut exp = expected[i].clone();
            // D3: Python reuses cell 3 for the new cell in frame 2; the fixed port gives 4.
            if case["defect"] == "D3" && i == 2 {
                exp = replace_cell_id(&exp, 3, 4);
            }
            // generated_at is wall-clock in Python (forced to 0.0 in the fixture); next_cell_id is new.
            // The fixture still has Python's top-level `prediction_minutes` and `tracking_vectors`;
            // nothing reads them, so the port no longer writes them (C16).
            assert_json_close(
                &actual,
                &exp,
                &format!("{name}[{i}]"),
                false,
                &[
                    "generated_at",
                    "next_cell_id",
                    "prediction_minutes",
                    "tracking_vectors",
                ],
            );
            // The counter (D3) is on every payload; the full shape is on non-empty frames.
            assert!(
                actual.get("next_cell_id").is_some(),
                "{name}[{i}] missing next_cell_id"
            );
            for key in ["threshold_dbz", "cell_count"] {
                assert!(actual.get(key).is_some(), "{name}[{i}] missing {key}");
            }
            prev = Some(actual);
        }
    }
}

/// Rewrite `cell_id` values of `old` to `new` in the features of a payload.
fn replace_cell_id(doc: &Value, old: i64, new: i64) -> Value {
    let mut out = doc.clone();
    for feature in out["features"].as_array_mut().unwrap() {
        if feature["properties"]["cell_id"] == json!(old) {
            feature["properties"]["cell_id"] = json!(new);
        }
    }
    out
}

#[test]
fn d3_reuse_reference_ids_are_monotonic() {
    // Python gives the new cell id 3 (reuse); the port must give 4 and never go back.
    let golden = testdata("storms_detect.json");
    let case = golden["cases"]
        .as_array()
        .unwrap()
        .iter()
        .find(|c| c["name"] == "d3_id_reuse_reference")
        .unwrap();
    let lats = f64_list(&case["lats"]);
    let lons = f64_list(&case["lons"]);
    let cfg = StormConfig::default();
    let mut prev: Option<Value> = None;
    let mut ids_per_frame = Vec::new();
    for frame in case["frames"].as_array().unwrap() {
        let data = build_grid(&frame["grid"], lats.len(), lons.len());
        let grid = Grid::new(&data, lats.len(), lons.len(), &lats, &lons).unwrap();
        let doc = detect_storms_with_clock(
            &grid,
            frame["timestamp"].as_str().unwrap(),
            prev.as_ref(),
            &cfg,
            0.0,
        );
        let v = serde_json::to_value(&doc).unwrap();
        let ids: Vec<i64> = v["features"]
            .as_array()
            .unwrap()
            .iter()
            .map(|f| f["properties"]["cell_id"].as_i64().unwrap())
            .collect();
        ids_per_frame.push((ids, v["next_cell_id"].as_i64().unwrap()));
        prev = Some(v);
    }
    assert_eq!(ids_per_frame[0].0, vec![1, 2, 3]);
    assert_eq!(ids_per_frame[1].0, vec![1, 2]);
    // Cell 3 vanished; the new cell must not take 3 while the counter says 4 is next.
    let mut frame2 = ids_per_frame[2].0.clone();
    frame2.sort_unstable();
    assert_eq!(frame2, vec![1, 2, 4]);
    assert_eq!(ids_per_frame[2].1, 5);
    // Python's reference output for the same frame, kept to document the defect.
    let python_frame2 = case["expected"][2]["features"]
        .as_array()
        .unwrap()
        .iter()
        .map(|f| f["properties"]["cell_id"].as_i64().unwrap())
        .collect::<Vec<_>>();
    assert!(
        python_frame2.contains(&3),
        "reference shows the Python reuse"
    );
}

#[test]
fn storms_write_does_not_regress() {
    let golden = testdata("storms_write.json");
    let dir = tempfile::tempdir().unwrap();
    let lats: Vec<f64> = (0..20)
        .map(|i| 43.0 + (42.81 - 43.0) * f64::from(i) / 19.0)
        .collect();
    let lons: Vec<f64> = (0..20)
        .map(|i| -85.0 + (-84.81 + 85.0) * f64::from(i) / 19.0)
        .collect();
    let mut data = vec![0.0_f32; 400];
    for r in 7..10 {
        for c in 7..10 {
            data[r * 20 + c] = 55.0;
        }
    }
    let grid = Grid::new(&data, 20, 20, &lats, &lons).unwrap();
    let cfg = StormConfig::default();
    let w = |ts: &str| write_storms_json_with_clock(dir.path(), &grid, ts, &cfg, 1.0).unwrap();
    assert_eq!(w("2026-07-10T12:05:00+00:00"), WriteOutcome::Written);
    assert_eq!(w("2026-07-10T12:00:00+00:00"), WriteOutcome::SkippedStale);
    let stored: Value =
        serde_json::from_str(&fs::read_to_string(dir.path().join(STORMS_FILE)).unwrap()).unwrap();
    assert_eq!(stored["timestamp"], golden["after_newer_then_backfill"]);
    assert_eq!(w("2026-07-10T12:05:00+00:00"), WriteOutcome::SkippedStale);
    assert_eq!(w("2026-07-10T12:10:00+00:00"), WriteOutcome::Written);
    let stored: Value =
        serde_json::from_str(&fs::read_to_string(dir.path().join(STORMS_FILE)).unwrap()).unwrap();
    assert_eq!(stored["timestamp"], golden["after_newer"]);
}

fn write_tiles(root: &std::path::Path, rels: &[Value]) {
    for rel in rels {
        let p = root.join(rel.as_str().unwrap());
        fs::create_dir_all(p.parent().unwrap()).unwrap();
        fs::write(p, b"png").unwrap();
    }
}

#[test]
fn prefetch_plans_match_python_or_the_d1_fixed_oracle() {
    let golden = testdata("prefetch.json");
    let cfg = StormConfig::default();
    for case in golden["cases"].as_array().unwrap() {
        let name = case["name"].as_str().unwrap();
        let state = tempfile::tempdir().unwrap();
        let tiles = tempfile::tempdir().unwrap();
        fs::write(
            state.path().join("manifest.json"),
            case["manifest"].to_string(),
        )
        .unwrap();
        write_tiles(tiles.path(), case["existing_tiles"].as_array().unwrap());

        let req_json = &case["request"];
        let palette = req_json["palette"].as_str().unwrap().to_owned();
        let base_url = req_json["base_url"].as_str().unwrap().to_owned();
        let req = PrefetchRequest {
            lat: req_json["lat"].as_f64().unwrap(),
            lon: req_json["lon"].as_f64().unwrap(),
            zoom: req_json["zoom"].as_i64().unwrap(),
            palette: &palette,
            base_url: &base_url,
        };
        let plan = build_storm_prefetch_plan_from_state(
            &case["storms"],
            state.path(),
            tiles.path(),
            &req,
            &cfg,
        )
        .unwrap_or_else(|e| panic!("{name}: {e}"));
        let actual = serde_json::to_value(&plan).unwrap();

        let expected = case
            .get("expected_fixed")
            .unwrap_or(&case["expected_python"]);
        assert_json_close(&actual, expected, name, true, &[]);
    }
}

#[test]
fn nowcast_run_paths_produce_tiles_d1() {
    // D1: with run-scoped manifest paths, leads 5 and 10 must have tile URLs (Python had none).
    let golden = testdata("prefetch.json");
    let case = golden["cases"]
        .as_array()
        .unwrap()
        .iter()
        .find(|c| c["name"] == "nowcast_run_paths_D1")
        .unwrap();
    let python_nowcast_urls: usize = case["expected_python"]["bboxes"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|b| b["layer"] == "nowcast")
        .map(|b| b["tile_urls"].as_array().unwrap().len())
        .sum();
    assert_eq!(python_nowcast_urls, 0, "reference shows the Python defect");

    let state = tempfile::tempdir().unwrap();
    let tiles = tempfile::tempdir().unwrap();
    fs::write(
        state.path().join("manifest.json"),
        case["manifest"].to_string(),
    )
    .unwrap();
    write_tiles(tiles.path(), case["existing_tiles"].as_array().unwrap());
    let req = PrefetchRequest {
        lat: 42.9,
        lon: -84.9,
        zoom: 6,
        palette: "classic",
        base_url: "https://radar.example",
    };
    let plan = build_storm_prefetch_plan_from_state(
        &case["storms"],
        state.path(),
        tiles.path(),
        &req,
        &StormConfig::default(),
    )
    .unwrap();
    for entry in &plan.bboxes {
        if entry.layer.as_deref() == Some("nowcast") {
            assert!(
                !entry.tile_urls.is_empty(),
                "lead {} has no tiles",
                entry.lead_minutes
            );
            assert!(
                entry
                    .tile_urls
                    .iter()
                    .all(|u| u.contains("/tiles/nowcast/classic/runs/2026-07-10T12:00:00+00:00/"))
            );
        }
    }
}

#[test]
fn numerics_match_numpy_and_python() {
    let golden = testdata("numerics.json");
    for row in golden["f32_mean"].as_array().unwrap() {
        let n = row["n"].as_u64().unwrap() as usize;
        let v = nth_f32(n);
        assert_eq!(
            f64::from(np_mean_f32(&v)),
            row["value"].as_f64().unwrap(),
            "f32 mean n={n}"
        );
    }
    for row in golden["f32_sum"].as_array().unwrap() {
        let n = row["n"].as_u64().unwrap() as usize;
        assert_eq!(
            f64::from(np_sum_f32(&nth_f32(n))),
            row["value"].as_f64().unwrap(),
            "f32 sum n={n}"
        );
    }
    for row in golden["f64_sum"].as_array().unwrap() {
        let n = row["n"].as_u64().unwrap() as usize;
        let v: Vec<f64> = (0..n as u64)
            .map(|k| ((k * 2_654_435_761) % 1000) as f64 * 0.1)
            .collect();
        assert_eq!(
            np_sum_f64(&v),
            row["value"].as_f64().unwrap(),
            "f64 sum n={n}"
        );
    }
    for row in golden["rounding"].as_array().unwrap() {
        let x = row["x"].as_f64().unwrap();
        assert_eq!(
            round_half_even(x),
            row["round0"].as_f64().unwrap(),
            "round0({x})"
        );
        for n in [1_usize, 2, 3, 5, 6] {
            assert_eq!(
                round_digits(x, n),
                row[format!("round{n}")].as_f64().unwrap(),
                "round{n}({x})"
            );
        }
        assert_eq!(
            format!("{x:.0}"),
            row["fmt0"].as_str().unwrap(),
            "format {x}"
        );
    }
}

/// `n` float32 values `((k * 2654435761) % 1000) * 0.1` as in `gen_golden.py`.
fn nth_f32(n: usize) -> Vec<f32> {
    (0..n as u64)
        .map(|k| (((k * 2_654_435_761) % 1000) as f32) * 0.1_f32)
        .collect()
}

fn decode_float(v: &Value) -> f64 {
    match v {
        Value::Object(o) if o.contains_key("$float") => match o["$float"].as_str().unwrap() {
            "-inf" => f64::NEG_INFINITY,
            "inf" => f64::INFINITY,
            "nan" => f64::NAN,
            other => panic!("unknown float tag {other}"),
        },
        other => other.as_f64().unwrap(),
    }
}

fn same_float(actual: f64, expected: f64) -> bool {
    (actual.is_nan() && expected.is_nan()) || actual == expected
}

/// Write the grid files for the watch fixtures exactly as `gen_golden.py::write_meta_pair` does.
fn write_watch_meta(dir: &std::path::Path, name: &str, data_file: bool) {
    fs::create_dir_all(dir).unwrap();
    let (h, w) = (100_usize, 100_usize);
    let mut vals = vec![0.0_f32; h * w];
    for r in 0..h {
        for c in 0..w {
            vals[r * w + c] = (20 + (r * 3 + c * 5) % 40) as f32;
        }
    }
    for r in 90..100 {
        for c in 90..100 {
            vals[r * w + c] = -9999.0;
        }
    }
    for r in 50..53 {
        for c in 50..53 {
            vals[r * w + c] = f32::NAN;
        }
    }
    for r in 10..14 {
        for c in 10..14 {
            vals[r * w + c] = 62.0;
        }
    }
    let bin_name = name.replace(".meta.json", ".bin");
    let bytes: Vec<u8> = vals.iter().flat_map(|v| v.to_le_bytes()).collect();
    fs::write(dir.join(&bin_name), bytes).unwrap();
    let mut meta = json!({
        "height": h, "width": w,
        "lat_min": 42.0, "lat_max": 43.0, "lon_min": -85.0, "lon_max": -84.0,
        "unit": "dBZ", "fill": -9999.0, "stride": 1
    });
    if data_file {
        meta["data_file"] = json!(bin_name);
    }
    fs::write(dir.join(name), format!("{}\n", meta)).unwrap();
}

#[test]
fn storm_watch_samples_match_python() {
    let golden = testdata("storm_watch.json");
    let dir = tempfile::tempdir().unwrap();
    let radar = dir.path().join("radar");
    write_watch_meta(&radar, "2026-07-10T12:00:00+00:00.meta.json", true);
    write_watch_meta(&radar, "2026-07-10T13:00:00+00:00.meta.json", false);
    write_watch_meta(&radar, "2026-07-10T12:30:00+00:00.meta.json", true);
    fs::write(radar.join(".hidden.meta.json"), "{}").unwrap();
    let latest = latest_radar_meta(dir.path()).unwrap();
    assert_eq!(
        latest.file_name().unwrap().to_str().unwrap(),
        golden["latest_meta_name"].as_str().unwrap()
    );

    // Python sorts the same names; dot files are excluded.
    let expected_names: Vec<&str> = golden["latest_candidates"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v.as_str().unwrap())
        .collect();
    assert_eq!(
        expected_names,
        vec![
            ".hidden.meta.json",
            "2026-07-10T12:00:00+00:00.meta.json",
            "2026-07-10T12:30:00+00:00.meta.json",
            "2026-07-10T13:00:00+00:00.meta.json"
        ]
    );

    write_watch_meta(&radar, "2026-07-10T14:00:00+00:00.meta.json", true);
    // The 14:00 meta points at a missing binary: must sample as None.
    let text = fs::read_to_string(radar.join("2026-07-10T14:00:00+00:00.meta.json")).unwrap();
    fs::write(
        radar.join("2026-07-10T14:00:00+00:00.meta.json"),
        text.replace("2026-07-10T14:00:00+00:00.bin", "missing.bin"),
    )
    .unwrap();
    fs::write(
        radar.join("2026-07-10T15:00:00+00:00.meta.json"),
        "{not json",
    )
    .unwrap();

    for s in golden["samples"].as_array().unwrap() {
        let name = s["name"].as_str().unwrap();
        let meta_path = radar.join(s["meta"].as_str().unwrap());
        let got = sample_window(
            &meta_path,
            s["lat"].as_f64().unwrap(),
            s["lon"].as_f64().unwrap(),
            s["radius_km"].as_f64().unwrap(),
        );
        match &s["sample"] {
            Value::Null => assert!(got.is_none(), "{name}: expected None, got {got:?}"),
            exp => {
                let got = got.unwrap_or_else(|| panic!("{name}: expected a sample"));
                assert_eq!(
                    got.timestamp,
                    exp["timestamp"].as_str().unwrap(),
                    "{name} timestamp"
                );
                assert!(
                    same_float(got.max_dbz, decode_float(&exp["max_dbz"])),
                    "{name} max {} vs {}",
                    got.max_dbz,
                    decode_float(&exp["max_dbz"])
                );
                // Mean must agree bit-for-bit: numpy float32 pairwise summation.
                assert!(
                    same_float(got.mean_dbz, decode_float(&exp["mean_dbz"])),
                    "{name} mean {} vs {}",
                    got.mean_dbz,
                    decode_float(&exp["mean_dbz"])
                );
                assert_eq!(
                    got.above_50_count,
                    exp["above_50_count"].as_u64().unwrap(),
                    "{name} above50"
                );
            }
        }
    }
}

#[test]
fn detect_change_matches_python_table_except_d2() {
    let golden = testdata("storm_watch.json");
    let cfg = WatchConfig::default();
    for case in golden["change"].as_array().unwrap() {
        let name = case["name"].as_str().unwrap();
        let curr = decode_float(&case["curr"]);
        let prev = if case["prev"].is_null() {
            None
        } else {
            Some(decode_float(&case["prev"]))
        };
        let delta = decode_float(&case["delta"]);
        let diff = FrameDiff {
            prev: prev.map(|p| FrameSample {
                timestamp: "p".into(),
                max_dbz: p,
                mean_dbz: f64::NAN,
                above_50_count: 0,
            }),
            curr: FrameSample {
                timestamp: "c".into(),
                max_dbz: curr,
                mean_dbz: f64::NAN,
                above_50_count: 0,
            },
            max_dbz_delta: delta,
        };
        let got = detect_change(&diff, &cfg);
        if case["defect"] == "D2" {
            assert!(
                got.is_none(),
                "{name}: D2 must not classify no-data as dissipating"
            );
            continue;
        }
        match (&got, case["kind"].as_str()) {
            (None, None) => {}
            (Some(ev), Some(kind)) => {
                assert_eq!(ev.kind.as_str(), kind, "{name} kind");
                assert_eq!(
                    ev.summary,
                    case["summary"].as_str().unwrap(),
                    "{name} summary"
                );
            }
            (g, e) => panic!("{name}: got {g:?}, expected {e:?}"),
        }
    }
}

#[test]
fn detect_change_precedence_and_kinds() {
    // DRV-07: severe wins over intensifying.
    let cfg = WatchConfig::default();
    let diff = FrameDiff {
        prev: Some(FrameSample {
            timestamp: "p".into(),
            max_dbz: 45.0,
            mean_dbz: f64::NAN,
            above_50_count: 0,
        }),
        curr: FrameSample {
            timestamp: "c".into(),
            max_dbz: 62.0,
            mean_dbz: f64::NAN,
            above_50_count: 0,
        },
        max_dbz_delta: 17.0,
    };
    assert_eq!(detect_change(&diff, &cfg).unwrap().kind, ChangeKind::Severe);
    assert_eq!(ChangeKind::Dissipating.as_str(), "dissipating");
}

#[test]
fn compare_radar_frames_matches_python_except_d2() {
    let golden = testdata("storm_watch.json");
    let dir = tempfile::tempdir().unwrap();
    let grids = dir.path().join("cmp-grids");
    let radar = grids.join("radar");
    write_watch_meta(&radar, "2026-07-10T12:00:00+00:00.meta.json", true);
    let absent = dir.path().join("absent");

    for case in golden["compare"].as_array().unwrap() {
        let name = case["name"].as_str().unwrap();
        let input = &case["input"];
        let lat = input["lat"].as_f64().unwrap();
        let lng = input["lng"].as_f64().unwrap();
        let prev_ts = input["prev_timestamp"].as_str();
        let prev_max = input["prev_max_dbz"].as_f64();
        let root = if name == "no_grid_dir" {
            &absent
        } else {
            &grids
        };
        let got = compare_radar_frames(root, lat, lng, prev_ts, prev_max);
        let exp = &case["result"];

        if name == "out_of_bounds_D2" {
            // Python compares -99 (the sentinel for -inf) against 30 and reports a dissipation.
            assert!(!got.sampled, "{name}: no-data sample must not be compared");
            assert_eq!(detect_storm_change(&got, &WatchConfig::default()), None);
            continue;
        }
        if name == "sentinel_prev_D2" {
            assert!(got.sampled);
            assert!(!got.has_prev, "a -99 previous value is not data");
            assert_eq!(got.max_dbz_delta, 0.0);
            assert_eq!(detect_storm_change(&got, &WatchConfig::default()), None);
            continue;
        }
        assert_eq!(
            got.sampled,
            exp["sampled"].as_bool().unwrap(),
            "{name} sampled"
        );
        assert_eq!(
            got.has_prev,
            exp["has_prev"].as_bool().unwrap(),
            "{name} has_prev"
        );
        assert!(
            close(got.curr_max_dbz, exp["curr_max_dbz"].as_f64().unwrap()),
            "{name} curr"
        );
        assert!(
            close(got.prev_max_dbz, exp["prev_max_dbz"].as_f64().unwrap()),
            "{name} prev"
        );
        assert!(
            close(got.max_dbz_delta, decode_float(&exp["max_dbz_delta"])),
            "{name} delta"
        );
        assert_eq!(
            got.curr_timestamp,
            exp["curr_timestamp"].as_str().unwrap(),
            "{name} ts"
        );
        let change = detect_storm_change(&got, &WatchConfig::default());
        // Python reports "no change" as empty kind and summary strings.
        match (&change, &case["change"]) {
            (None, Value::Null) => {}
            (None, c) if c["kind"] == "" => {}
            (Some(ev), c) if c.is_object() => {
                assert_eq!(ev.kind.as_str(), c["kind"].as_str().unwrap(), "{name} kind");
                assert_eq!(ev.summary, c["summary"].as_str().unwrap(), "{name} summary");
            }
            (g, e) => panic!("{name}: change {g:?} vs {e:?}"),
        }
    }
    // compare_sample is the same decision with the sample supplied directly.
    let s = FrameSample {
        timestamp: "t".into(),
        max_dbz: f64::NEG_INFINITY,
        mean_dbz: f64::NAN,
        above_50_count: 0,
    };
    assert!(!compare_sample(Some(&s), None, Some(30.0)).sampled);
}

fn close(a: f64, b: f64) -> bool {
    common::close(a, b)
}
