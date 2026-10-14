//! GRIB2 message splitting, validDate and decoding checked against eccodes-generated fixtures
//! (`testdata/gen_aqm.py`, golden manifest `testdata/golden/aqm_manifest.json`).

use std::path::PathBuf;

use chrono::{DateTime, Utc};
use radar_aux::aqm::grib::{split_messages, valid_time};
use serde_json::Value;

fn testdata(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("testdata")
        .join(name)
}

fn golden() -> Value {
    let text =
        std::fs::read_to_string(testdata("golden/aqm_manifest.json")).expect("golden manifest");
    serde_json::from_str(&text).expect("golden manifest is JSON")
}

fn file_bytes(name: &str) -> Vec<u8> {
    std::fs::read(testdata(name)).unwrap_or_else(|e| panic!("reading {name}: {e}"))
}

/// Splits the file and checks each message's `validDate` against eccodes.
fn check_valid_times(key: &str) {
    let g = golden();
    let entry = &g[key];
    let file = entry["file"].as_str().unwrap();
    let bytes = file_bytes(file);
    let ranges = split_messages(&bytes).expect("split");
    assert_eq!(
        ranges.len() as u64,
        entry["messages"].as_u64().unwrap(),
        "{key}: message count"
    );
    for (i, rec) in entry["entries"].as_array().unwrap().iter().enumerate() {
        let msg = &bytes[ranges[i].clone()];
        let got = valid_time(msg).unwrap_or_else(|e| panic!("{key} msg {i}: {e}"));
        let want: DateTime<Utc> = rec["valid"].as_str().unwrap().parse().unwrap();
        assert_eq!(got, want, "{key} msg {i} validDate");
    }
}

#[test]
fn pm25_split_and_valid_times_match_eccodes() {
    check_valid_times("pm25");
}

#[test]
fn ozone_split_and_valid_times_match_eccodes() {
    check_valid_times("ozone");
}

#[test]
fn statistical_template_valid_time_is_end_of_range() {
    // Template 4.8: the valid time is reference + forecast time + the time-range length.
    check_valid_times("stat");
}

#[test]
fn flipped_scan_valid_times_match_eccodes() {
    check_valid_times("flipped");
}

#[test]
fn message_bytes_decode_to_the_eccodes_values_bitwise() {
    let g = golden();
    for key in ["pm25", "ozone", "stat", "flipped"] {
        let entry = &g[key];
        let bytes = file_bytes(entry["file"].as_str().unwrap());
        let ranges = split_messages(&bytes).unwrap();
        let decoded = radar_grib::decode::decode_first_message(&bytes[ranges[0].clone()])
            .unwrap_or_else(|e| panic!("{key}: decode failed: {e}"));
        let raw = std::fs::read(testdata(&format!(
            "golden/{}",
            entry["msg0_values_f32le"].as_str().unwrap()
        )))
        .unwrap();
        let want: Vec<u32> = raw
            .chunks_exact(4)
            .map(|c| u32::from_le_bytes([c[0], c[1], c[2], c[3]]))
            .collect();
        let got: Vec<u32> = decoded.grid.values.iter().map(|v| v.to_bits()).collect();
        assert_eq!(got.len(), want.len(), "{key}: value count");
        assert_eq!(got, want, "{key}: values differ from eccodes (bitwise)");
        let nans = decoded.grid.values.iter().filter(|v| v.is_nan()).count() as u64;
        assert_eq!(
            nans,
            entry["msg0_nan_count"].as_u64().unwrap(),
            "{key}: NaN count"
        );
    }
}

#[test]
fn truncated_file_is_an_error_not_a_short_list() {
    let bytes = file_bytes("aqm_pm25_flipped.grib2");
    let cut = &bytes[..bytes.len() - 100];
    assert!(split_messages(cut).is_err());
}
