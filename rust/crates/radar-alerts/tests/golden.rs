//! Golden replays. The fixtures in `testdata/` were produced by running the original Python functions
//! (`testdata/gen_golden.py`). Each test replays one fixture against the Rust port.

use std::collections::BTreeSet;
use std::path::PathBuf;
use std::time::{Duration, Instant};

use radar_alerts::alerts::{
    MAX_SEEN, SEEN_FILE, SNAPSHOT_FILE, alert_from_feature, commit_handled, load_seen,
    point_in_geojson, write_active_snapshot,
};
use radar_alerts::lightning::{
    EndpointPicker, Strike, decode_payload, load_existing_strikes, render_geojson, write_geojson,
};
use serde_json::{Map, Value, json};

fn load(name: &str) -> Value {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("testdata")
        .join(name);
    let text = std::fs::read_to_string(&path)
        .unwrap_or_else(|err| panic!("reading {}: {err}", path.display()));
    serde_json::from_str(&text).unwrap()
}

fn strings(value: &Value) -> Vec<String> {
    value
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v.as_str().unwrap().to_owned())
        .collect()
}

fn as_object(value: &Value) -> Map<String, Value> {
    match value {
        Value::Object(map) => map.clone(),
        _ => Map::new(),
    }
}

#[test]
fn lzw_decode_matches_python_golden() {
    let fixture = load("lzw_decode.json");
    let cases = fixture["cases"].as_array().unwrap();
    assert!(
        cases.len() > 700,
        "fixture unexpectedly small: {}",
        cases.len()
    );
    for case in cases {
        let raw = case["raw"].as_str().unwrap();
        let out = case["out"].as_str().unwrap();
        assert_eq!(decode_payload(raw), out, "raw={raw:?}");
    }
}

#[test]
fn point_in_geojson_matches_python_golden() {
    let fixture = load("point_in_geojson.json");
    let cases = fixture["cases"].as_array().unwrap();
    assert!(
        cases.len() > 5000,
        "fixture unexpectedly small: {}",
        cases.len()
    );
    for (i, case) in cases.iter().enumerate() {
        let geometry = as_object(&case["geometry"]);
        let lng = case["lng"].as_f64().unwrap();
        let lat = case["lat"].as_f64().unwrap();
        let expect = case["expect"].as_bool().unwrap();
        assert_eq!(
            point_in_geojson(lng, lat, &geometry),
            expect,
            "case {i}: lng={lng} lat={lat} geometry={}",
            case["geometry"]
        );
    }
}

#[test]
fn point_in_polygon_hole_and_outer_ring_invariants() {
    let square = json!([
        [-98.0, 35.0],
        [-96.0, 35.0],
        [-96.0, 37.0],
        [-98.0, 37.0],
        [-98.0, 35.0]
    ]);
    let hole = json!([
        [-97.5, 35.5],
        [-96.5, 35.5],
        [-96.5, 36.5],
        [-97.5, 36.5],
        [-97.5, 35.5]
    ]);
    let with_hole = as_object(&json!({"type": "Polygon", "coordinates": [square.clone(), hole]}));
    let without_hole = as_object(&json!({"type": "Polygon", "coordinates": [square]}));

    // Inside the outer ring and outside the hole: inside.
    assert!(point_in_geojson(-97.9, 35.1, &with_hole));
    // Inside the hole: outside, although it is inside the outer ring.
    assert!(!point_in_geojson(-97.0, 36.0, &with_hole));
    assert!(point_in_geojson(-97.0, 36.0, &without_hole));
    // Outside the outer ring: outside whatever the hole.
    assert!(!point_in_geojson(-95.0, 36.0, &with_hole));
    // Empty and unsupported geometries never match.
    assert!(!point_in_geojson(-97.0, 36.0, &Map::new()));
    assert!(!point_in_geojson(
        -97.0,
        36.0,
        &as_object(&json!({"type": "Point", "coordinates": [-97, 36]}))
    ));
}

#[test]
fn endpoint_picker_matches_python_scripts() {
    let fixture = load("picker.json");
    let scenarios = fixture["scenarios"].as_array().unwrap();
    assert!(scenarios.len() >= 40);
    for scenario in scenarios {
        let name = scenario["name"].as_str().unwrap();
        let mut picker =
            EndpointPicker::new(strings(&scenario["primary"]), strings(&scenario["legacy"]))
                .unwrap();
        let base = Instant::now();
        let mut clock = 0.0_f64;
        let mut rand = 0.5_f64;
        let mut results: Vec<Value> = Vec::new();
        for op in scenario["ops"].as_array().unwrap() {
            let at = base + Duration::from_secs_f64(clock);
            let result = match op[0].as_str().unwrap() {
                "next" => Value::String(picker.next(at)),
                "failed" => json!(picker.failed(op[1].as_str().unwrap(), at, rand)),
                "succeeded" => json!(picker.succeeded(op[1].as_str().unwrap())),
                "advance" => {
                    clock += op[1].as_f64().unwrap();
                    Value::Null
                }
                "rand" => {
                    rand = op[1].as_f64().unwrap();
                    Value::Null
                }
                other => panic!("unknown op {other} in {name}"),
            };
            results.push(result);
        }
        assert_eq!(Value::Array(results), scenario["expect"], "scenario {name}");
    }
}

#[test]
fn seed_matches_python_golden() {
    let fixture = load("seed.json");
    let now = fixture["now"].as_f64().unwrap();
    let retention = fixture["retention_min"].as_i64().unwrap();
    let dir = tempfile::tempdir().unwrap();
    for case in fixture["cases"].as_array().unwrap() {
        let name = case["name"].as_str().unwrap();
        let path = dir.path().join(format!("{name}.json"));
        if let Some(text) = case["file"].as_str() {
            std::fs::write(&path, text).unwrap();
        }
        let seeded = load_existing_strikes(&path, retention, now, 5000);
        let actual: Vec<Value> = seeded
            .iter()
            .map(|s| json!([s.t, s.lat, s.lon, s.pol, s.mds]))
            .collect();
        assert_eq!(Value::Array(actual), case["expect"], "seed case {name}");
    }
}

#[test]
fn writer_matches_python_golden_and_leaves_no_temp_files() {
    let fixture = load("writer.json");
    let now = fixture["now"].as_f64().unwrap();
    let retention = fixture["retention_min"].as_i64().unwrap();
    let strikes: Vec<Strike> = fixture["strikes"]
        .as_array()
        .unwrap()
        .iter()
        .map(|s| Strike {
            t: s["t"].as_f64().unwrap(),
            lat: s["lat"].as_f64().unwrap(),
            lon: s["lon"].as_f64().unwrap(),
            pol: s["pol"].as_i64().unwrap(),
            mds: s["mds"].as_i64().unwrap(),
        })
        .collect();
    assert_eq!(
        render_geojson(&strikes, now, retention),
        fixture["expect_text"].as_str().unwrap()
    );

    let dir = tempfile::tempdir().unwrap();
    write_geojson(&dir.path().join("lightning.json"), &strikes, retention).unwrap();
    let mut names: Vec<String> = std::fs::read_dir(dir.path())
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    names.sort();
    assert_eq!(names, strings(&fixture["leftovers"]));
}

#[test]
fn alert_from_feature_matches_python_golden() {
    let fixture = load("alert_features.json");
    for (i, case) in fixture["cases"].as_array().unwrap().iter().enumerate() {
        let got = alert_from_feature(&case["feature"]);
        match &case["alert_id"] {
            Value::Null => assert!(
                got.is_none(),
                "case {i} should be skipped: {}",
                case["feature"]
            ),
            Value::String(expected) => {
                let alert = got.unwrap_or_else(|| {
                    panic!("case {i} should yield an alert: {}", case["feature"])
                });
                assert_eq!(&alert.alert_id, expected, "case {i}");
                assert_eq!(
                    Value::Object(alert.geometry),
                    case["geometry"],
                    "case {i} geometry"
                );
            }
            other => panic!("unexpected alert_id in fixture: {other}"),
        }
    }
}

#[test]
fn seen_state_matches_python_golden() {
    let fixture = load("seen_state.json");
    for case in fixture["cases"].as_array().unwrap() {
        let name = case["name"].as_str().unwrap();
        let dir = tempfile::tempdir().unwrap();
        if !case["seen"].is_null() {
            std::fs::write(dir.path().join(SEEN_FILE), case["seen"].to_string()).unwrap();
        }
        if !case["snapshot"].is_null() {
            // A string snapshot (the "corrupt" case) is written as a JSON string, which is not a list.
            std::fs::write(dir.path().join(SNAPSHOT_FILE), case["snapshot"].to_string()).unwrap();
        }
        commit_handled(dir.path(), &strings(&case["handled"])).unwrap();
        let got: BTreeSet<String> = load_seen(dir.path());
        let expect: BTreeSet<String> = strings(&case["expect"]).into_iter().collect();
        assert!(got.len() <= MAX_SEEN, "case {name}: seen set over the cap");
        assert_eq!(got, expect, "seen case {name}");
    }
}

#[test]
fn snapshot_text_matches_python_golden() {
    let fixture = load("snapshot_format.json");
    let dir = tempfile::tempdir().unwrap();
    let active: BTreeSet<String> = ["urn:b", "urn:a", "\u{e9}-x"]
        .iter()
        .map(|s| (*s).to_owned())
        .collect();
    write_active_snapshot(dir.path(), &active).unwrap();
    let text = std::fs::read_to_string(dir.path().join(SNAPSHOT_FILE)).unwrap();
    assert_eq!(text, fixture["expect_text"].as_str().unwrap());
}
