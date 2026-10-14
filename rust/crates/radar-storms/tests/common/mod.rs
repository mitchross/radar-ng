//! Shared helpers for the integration tests: fixture loading, grid builders, JSON comparison.

#![allow(dead_code)]

use serde_json::Value;

/// Load a golden fixture from `testdata/`.
pub fn testdata(name: &str) -> Value {
    let path = concat!(env!("CARGO_MANIFEST_DIR"), "/testdata/");
    let text = std::fs::read_to_string(format!("{path}{name}")).expect("fixture exists");
    serde_json::from_str(&text).expect("fixture is valid JSON")
}

/// Build a row-major f32 grid exactly as `gen_golden.py::build_grid` does.
pub fn build_grid(spec: &Value, h: usize, w: usize) -> Vec<f32> {
    match spec["kind"].as_str().unwrap() {
        "rects" => {
            let mut data = vec![0.0_f32; h * w];
            for rect in spec["rects"].as_array().unwrap() {
                let v = rect[4].as_f64().unwrap() as f32;
                let (r0, r1, c0, c1) = (
                    rect[0].as_u64().unwrap() as usize,
                    rect[1].as_u64().unwrap() as usize,
                    rect[2].as_u64().unwrap() as usize,
                    rect[3].as_u64().unwrap() as usize,
                );
                for r in r0..r1 {
                    for c in c0..c1 {
                        data[r * w + c] = v;
                    }
                }
            }
            data
        }
        "formula" => {
            let shift = spec["shift"].as_i64().unwrap();
            let name = spec["name"].as_str().unwrap();
            let mut data = vec![0.0_f32; h * w];
            for r in 0..h as i64 {
                for c in 0..w as i64 {
                    let idx = r as usize * w + c as usize;
                    data[idx] = match name {
                        "linear_mod45" => {
                            let k = (r * 37 + c * 53 + r * c * 7 + shift).rem_euclid(45);
                            (30 + k) as f32
                        }
                        "blocks4" => {
                            let cond = ((r / 4) * 7 + (c / 4) * 5 + shift).rem_euclid(3) != 0;
                            let var = (r * c + shift).rem_euclid(4) as f32 * 0.5_f32;
                            if cond { 42.0_f32 + var } else { 10.0_f32 }
                        }
                        other => panic!("unknown formula {other}"),
                    };
                }
            }
            data
        }
        other => panic!("unknown grid kind {other}"),
    }
}

/// Relative-or-absolute float closeness used for all Python comparisons (spec 15.2 tolerance).
pub fn close(a: f64, b: f64) -> bool {
    if a == b {
        return true;
    }
    let d = (a - b).abs();
    d <= 1e-9 || d <= 1e-12 * b.abs()
}

/// Compare JSON structurally. Numbers use [`close`]. `strict` also requires identical key sets.
/// Keys listed in `ignore` are skipped at every level.
pub fn assert_json_close(
    actual: &Value,
    expected: &Value,
    path: &str,
    strict: bool,
    ignore: &[&str],
) {
    match (actual, expected) {
        (Value::Object(a), Value::Object(e)) => {
            for (k, ev) in e {
                if ignore.contains(&k.as_str()) {
                    continue;
                }
                let av = a
                    .get(k)
                    .unwrap_or_else(|| panic!("{path}.{k} missing from actual"));
                assert_json_close(av, ev, &format!("{path}.{k}"), strict, ignore);
            }
            if strict {
                for k in a.keys() {
                    assert!(
                        ignore.contains(&k.as_str()) || e.contains_key(k),
                        "unexpected key {path}.{k} in actual"
                    );
                }
            }
        }
        (Value::Array(a), Value::Array(e)) => {
            assert_eq!(a.len(), e.len(), "{path}: array length");
            for (i, (x, y)) in a.iter().zip(e).enumerate() {
                assert_json_close(x, y, &format!("{path}[{i}]"), strict, ignore);
            }
        }
        (Value::Number(a), Value::Number(e)) => {
            let (x, y) = (a.as_f64().unwrap(), e.as_f64().unwrap());
            assert!(close(x, y), "{path}: actual {x} != expected {y}");
        }
        (a, e) => assert_eq!(a, e, "{path}"),
    }
}
