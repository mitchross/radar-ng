//! Reader and sampler tests. Grids are written by hand so each expected value can be worked out from the
//! sampler formula in spec-api §9.2 (operation for operation with `_sample_grid_point`).

use super::*;
use serde_json::{Value, json};

fn f32_bytes(values: &[f32]) -> Vec<u8> {
    values.iter().flat_map(|v| v.to_le_bytes()).collect()
}

/// 2x2 grid covering lat 42..43 and lon -86..-85. Row 0 is north.
fn meta(data_file: &str, fill: Value) -> Value {
    json!({
        "data_file": data_file,
        "height": 2,
        "width": 2,
        "lat_min": 42.0,
        "lat_max": 43.0,
        "lon_min": -86.0,
        "lon_max": -85.0,
        "unit": "dBZ",
        "fill": fill,
        "stride": 1,
    })
}

/// Write `<stem>.meta.json` and the data file, returning the meta path.
fn write_pair(dir: &Path, stem: &str, meta: &Value, data_name: &str, bytes: &[u8]) -> PathBuf {
    fs::create_dir_all(dir).unwrap();
    let meta_path = dir.join(format!("{stem}.meta.json"));
    fs::write(&meta_path, meta.to_string()).unwrap();
    fs::write(dir.join(data_name), bytes).unwrap();
    meta_path
}

fn two_by_two(north: [f32; 2], south: [f32; 2]) -> Vec<u8> {
    f32_bytes(&[north[0], north[1], south[0], south[1]])
}

fn grid(values: [[f32; 2]; 2], fill: Value) -> (tempfile::TempDir, GridDump) {
    let dir = tempfile::tempdir().unwrap();
    let path = write_pair(
        dir.path(),
        "g",
        &meta("g.bin", fill),
        "g.bin",
        &two_by_two(values[0], values[1]),
    );
    let dump = read_grid(&path).unwrap();
    (dir, dump)
}

#[test]
fn values_are_little_endian_row_major_with_row_zero_north() {
    let (_dir, g) = grid([[10.0, 20.0], [30.0, 40.0]], json!(-9999.0));
    assert_eq!(g.at(0, 0), 10.0);
    assert_eq!(g.at(0, 1), 20.0);
    assert_eq!(g.at(1, 0), 30.0);
    assert_eq!(g.at(1, 1), 40.0);
    assert_eq!(g.meta.unit, "dBZ");
    assert_eq!(g.meta.stride, 1);
}

#[test]
fn sampler_is_bilinear_and_exact_at_the_corners() {
    let (_dir, g) = grid([[10.0, 20.0], [30.0, 40.0]], json!(-9999.0));
    assert_eq!(g.sample(42.5, -85.5).unwrap(), Sample::Value(25.0));
    assert_eq!(
        g.sample(43.0, -86.0).unwrap(),
        Sample::Value(10.0),
        "NW corner"
    );
    assert_eq!(
        g.sample(42.0, -85.0).unwrap(),
        Sample::Value(40.0),
        "SE corner"
    );
}

#[test]
fn nw_fill_quirk_takes_the_first_valid_neighbour_on_the_top_edge() {
    // NW is fill, so v0 = valid[0] = NE (20) with no interpolation. The bottom edge interpolates normally:
    // 0.5*10 + 0.5*40 = 25. Result: 0.5*20 + 0.5*25 = 22.5.
    let (_dir, g) = grid([[-9999.0, 20.0], [10.0, 40.0]], json!(-9999.0));
    assert_eq!(g.sample(42.5, -85.5).unwrap(), Sample::Value(22.5));
}

#[test]
fn nan_cells_count_as_fill_when_the_meta_fill_is_null() {
    let dir = tempfile::tempdir().unwrap();
    let path = write_pair(
        dir.path(),
        "n",
        &meta("n.bin", Value::Null),
        "n.bin",
        &two_by_two([f32::NAN, 20.0], [10.0, 40.0]),
    );
    let g = read_grid(&path).unwrap();
    assert!(g.meta.fill.is_nan());
    assert_eq!(g.sample(42.5, -85.5).unwrap(), Sample::Value(22.5));
}

#[test]
fn bottom_edge_fill_uses_the_last_valid_neighbour() {
    // Only NE is valid. Both edges fall back to valid[0] / valid[-1], which is that one value, so the
    // sample is 40 wherever it lands in the cell.
    let (_dir, g) = grid([[-9999.0, 40.0], [-9999.0, -9999.0]], json!(-9999.0));
    assert_eq!(g.sample(42.5, -85.5).unwrap(), Sample::Value(40.0));
}

#[test]
fn all_fill_neighbours_give_no_data() {
    let (_dir, g) = grid([[-9999.0, -9999.0], [-9999.0, -9999.0]], json!(-9999.0));
    assert_eq!(g.sample(42.5, -85.5).unwrap(), Sample::NoData);
}

#[test]
fn points_outside_the_extent_and_nan_coordinates_are_out_of_bounds() {
    let (_dir, g) = grid([[10.0, 20.0], [30.0, 40.0]], json!(-9999.0));
    assert_eq!(g.sample(43.5, -85.5).unwrap(), Sample::OutOfBounds);
    assert_eq!(g.sample(42.5, -86.5).unwrap(), Sample::OutOfBounds);
    assert_eq!(g.sample(42.5, f64::NAN).unwrap(), Sample::OutOfBounds);
    assert_eq!(g.sample(f64::NAN, -85.5).unwrap(), Sample::OutOfBounds);
}

#[test]
fn single_row_or_zero_extent_grids_are_degenerate() {
    let dir = tempfile::tempdir().unwrap();
    let mut one_row = meta("one.bin", json!(-9999.0));
    one_row["height"] = json!(1);
    let path = write_pair(
        dir.path(),
        "one",
        &one_row,
        "one.bin",
        &f32_bytes(&[1.0, 2.0]),
    );
    let g = read_grid(&path).unwrap();
    assert!(matches!(
        g.sample(42.5, -85.5),
        Err(GridReadError::Degenerate)
    ));

    let mut flat = meta("flat.bin", json!(-9999.0));
    flat["lat_min"] = json!(42.0);
    flat["lat_max"] = json!(42.0);
    let path = write_pair(
        dir.path(),
        "flat",
        &flat,
        "flat.bin",
        &two_by_two([1.0, 2.0], [3.0, 4.0]),
    );
    let g = read_grid(&path).unwrap();
    assert!(matches!(
        g.sample(42.0, -85.5),
        Err(GridReadError::Degenerate)
    ));
}

#[test]
fn missing_metadata_and_data_are_reported_distinctly() {
    let dir = tempfile::tempdir().unwrap();
    assert!(matches!(
        read_grid(&dir.path().join("nope.meta.json")),
        Err(GridReadError::MetaMissing(_))
    ));

    fs::write(
        dir.path().join("only.meta.json"),
        meta("only.bin", json!(-9999.0)).to_string(),
    )
    .unwrap();
    assert!(matches!(
        read_grid(&dir.path().join("only.meta.json")),
        Err(GridReadError::DataMissing(_))
    ));
}

#[test]
fn short_data_file_is_a_short_read() {
    let dir = tempfile::tempdir().unwrap();
    let path = write_pair(
        dir.path(),
        "s",
        &meta("s.bin", json!(-9999.0)),
        "s.bin",
        &[0u8; 12],
    );
    match read_grid(&path) {
        Err(GridReadError::ShortRead { expected, actual }) => {
            assert_eq!((expected, actual), (16, 12));
        }
        other => panic!("expected ShortRead, got {other:?}"),
    }
}

#[test]
fn malformed_metadata_is_rejected() {
    let dir = tempfile::tempdir().unwrap();
    let cases: Vec<(&str, String)> = vec![
        ("not_json", "not json".to_string()),
        ("not_object", "[1, 2]".to_string()),
        ("missing_lat", {
            let mut m = meta("m.bin", json!(-9999.0));
            m.as_object_mut().unwrap().remove("lat_min");
            m.to_string()
        }),
        (
            "escaping_data_file",
            meta("../x.bin", json!(-9999.0)).to_string(),
        ),
    ];
    for (name, text) in cases {
        fs::write(dir.path().join(format!("{name}.meta.json")), text).unwrap();
        let result = read_grid(&dir.path().join(format!("{name}.meta.json")));
        assert!(
            matches!(result, Err(GridReadError::MetaInvalid(_))),
            "{name}: {result:?}"
        );
    }
}

#[test]
fn legacy_metadata_without_data_file_points_at_the_timestamp_bin() {
    let dir = tempfile::tempdir().unwrap();
    let ts = "2026-09-02T12:00:00+00:00";
    let mut m = meta("unused", json!(-9999.0));
    m.as_object_mut().unwrap().remove("data_file");
    m.as_object_mut().unwrap().remove("unit");
    m.as_object_mut().unwrap().remove("stride");
    m.as_object_mut().unwrap().remove("fill");
    let path = write_pair(
        dir.path(),
        ts,
        &m,
        &format!("{ts}.bin"),
        &two_by_two([1.0, 2.0], [3.0, 4.0]),
    );
    let g = read_grid(&path).unwrap();
    assert_eq!(g.meta.data_file, format!("{ts}.bin"));
    assert_eq!(g.meta.unit, "", "absent unit reads as empty");
    assert_eq!(g.meta.stride, 1, "absent stride defaults to 1");
    assert!(g.meta.fill.is_nan(), "absent fill reads as NaN");
    assert_eq!(g.at(1, 1), 4.0);
}
