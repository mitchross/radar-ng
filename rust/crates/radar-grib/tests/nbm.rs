//! NBM records through our Lambert decoder.
//!
//! Fixtures are real 6 hour records from blend.t12z.core.f006.co.grib2 (2026-11-17), cut to a 1.2 by
//! 1.1 degree box over southern Utah by the NOMADS `filter_blend.pl` subregion (wgrib2). The expected
//! values were read from the full 2345 x 1597 records at the same points.

use std::path::PathBuf;

use radar_geo::GridGeo;
use radar_grib::nbm::NbmVar;
use radar_grib::{find_message, list_messages};

fn fixture(name: &str) -> Vec<u8> {
    let p = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("testdata")
        .join(name);
    std::fs::read(p).unwrap()
}

fn value_near(bytes: &[u8], var: NbmVar, lat: f64, lon: f64) -> (f32, f64) {
    let grid = find_message(bytes, &var.selector()).unwrap().unwrap().grid;
    let GridGeo::Projected {
        lat_mesh, lon_mesh, ..
    } = &grid.geo
    else {
        panic!("NBM is Lambert");
    };
    let (k, d) = (0..lat_mesh.len())
        .map(|k| (k, (lat_mesh[k] - lat).hypot(lon_mesh[k] - lon)))
        .min_by(|a, b| a.1.total_cmp(&b.1))
        .unwrap();
    (grid.values[k], d)
}

#[test]
fn nbm_records_decode_on_the_lambert_grid() {
    let ice = fixture("nbm_ficeac_f006_utah.grib2");
    let info = &list_messages(&ice).unwrap()[0];
    assert_eq!(
        (info.discipline, info.category, info.number),
        (0, Some(1), Some(228))
    );
    assert_eq!(info.grid_template, 30);
    let (v, d) = value_near(&ice, NbmVar::Ficeac, 37.539_11, -112.804_80);
    assert!(d < 0.001, "{d}");
    assert!((v - 1.297).abs() < 1e-3, "FICEAC {v} mm");

    let snow = fixture("nbm_asnow_f006_utah.grib2");
    let (v, d) = value_near(&snow, NbmVar::Asnow, 37.674_48, -112.799_01);
    assert!(d < 0.001, "{d}");
    assert!((v - 0.268).abs() < 1e-3, "ASNOW {v} m");
}

/// The full NBM files use scanning mode 0x50 (every second row runs east to west). The NOMADS cut is
/// 0x40, so set the alternating-rows bit and check odd rows come back reversed.
#[test]
fn alternating_row_scan_is_unflipped() {
    let plain = fixture("nbm_asnow_f006_utah.grib2");
    let s1 = u32::from_be_bytes(plain[16..20].try_into().unwrap()) as usize;
    let scan_at = 16 + s1 + 64;
    assert_eq!(plain[scan_at], 0x40);
    let mut alt = plain.clone();
    alt[scan_at] = 0x50;

    let a = find_message(&plain, &NbmVar::Asnow.selector())
        .unwrap()
        .unwrap()
        .grid;
    let b = find_message(&alt, &NbmVar::Asnow.selector())
        .unwrap()
        .unwrap()
        .grid;
    let w = a.width;
    for r in 0..a.height {
        for c in 0..w {
            let want = if r % 2 == 1 {
                a.values[r * w + (w - 1 - c)]
            } else {
                a.values[r * w + c]
            };
            assert_eq!(
                b.values[r * w + c].to_bits(),
                want.to_bits(),
                "row {r} col {c}"
            );
        }
    }
    // Coordinates are unflipped with the values, so the mesh stays the same.
    assert_eq!(a.geo, b.geo);
}

/// Live check against the NBM bucket: `cargo test -p radar-grib --test nbm -- --ignored`.
#[tokio::test]
#[ignore = "network"]
async fn live_latest_run_has_both_six_hour_records() {
    let s3 = radar_grib::s3::S3Client::new(reqwest::Client::new(), radar_grib::nbm::BASE_URL);
    let run = radar_grib::nbm::find_latest_run(&s3, chrono::Utc::now(), 72)
        .await
        .unwrap()
        .expect("a run with f072 posted");
    let init = radar_grib::hrrr::parse_run_id(&run).unwrap();
    for fhr in [6, 72] {
        let got = radar_grib::nbm::fetch_hour(&s3, init, fhr, &[NbmVar::Asnow, NbmVar::Ficeac])
            .await
            .unwrap()
            .expect("hour posted");
        assert_eq!(got.len(), 2, "{run} f{fhr:03}");
        for (var, bytes) in got {
            assert!(find_message(&bytes, &var.selector()).unwrap().is_some());
            println!("{run} f{fhr:03} {var:?}: {} bytes", bytes.len());
        }
    }
}
