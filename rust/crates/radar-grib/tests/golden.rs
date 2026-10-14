//! Golden tests: decoded arrays and geometry must match eccodes (pygrib backend) and pyproj.
//!
//! Fixtures and goldens are produced by `testdata/gen_fixtures.py` and `testdata/gen_projection.py`.
use std::path::PathBuf;

use radar_geo::{Crs, GridGeo};
use radar_grib::{MessageSelector, decode_message, find_message};

fn testdata(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("testdata")
        .join(name)
}

fn read_f64(name: &str) -> Vec<f64> {
    std::fs::read(testdata(&format!("golden/{name}")))
        .unwrap()
        .chunks_exact(8)
        .map(|c| f64::from_le_bytes(c.try_into().unwrap()))
        .collect()
}

fn read_f32(name: &str) -> Vec<f32> {
    std::fs::read(testdata(&format!("golden/{name}")))
        .unwrap()
        .chunks_exact(4)
        .map(|c| f32::from_le_bytes(c.try_into().unwrap()))
        .collect()
}

fn manifest() -> serde_json::Value {
    serde_json::from_slice(&std::fs::read(testdata("golden/manifest.json")).unwrap()).unwrap()
}

fn assert_f32_bits_eq(label: &str, got: &[f32], want: &[f32]) {
    assert_eq!(got.len(), want.len(), "{label}: length");
    for (i, (a, b)) in got.iter().zip(want).enumerate() {
        let same = (a.is_nan() && b.is_nan()) || a.to_bits() == b.to_bits();
        assert!(same, "{label}: index {i}: got {a} want {b}");
    }
}

fn max_abs_diff(a: &[f64], b: &[f64]) -> f64 {
    a.iter()
        .zip(b)
        .map(|(x, y)| (x - y).abs())
        .fold(0.0, f64::max)
}

#[test]
fn mrms_regular_ll_values_and_axes_match_python() {
    let bytes = std::fs::read(testdata("mrms_regular_ll_png.grib2")).unwrap();
    let grid = decode_message(&bytes).unwrap();
    let m = manifest();
    let nj = m["mrms"]["nj"].as_u64().unwrap() as usize;
    let ni = m["mrms"]["ni"].as_u64().unwrap() as usize;
    assert_eq!((grid.width, grid.height), (ni, nj));
    assert_f32_bits_eq("mrms values", &grid.values, &read_f32("mrms_values.f32le"));
    let GridGeo::Regular { lats, lons } = &grid.geo else {
        panic!("expected regular grid")
    };
    // Python _grid_axes: linspace over header keys; lons normalised (> 180 -> -360).
    assert!(max_abs_diff(lats, &read_f64("mrms_lat_axis.f64le")) <= 1e-9);
    assert!(max_abs_diff(lons, &read_f64("mrms_lon_axis.f64le")) <= 1e-9);
    assert_eq!(lats[0], 54.0);
    assert!(
        lats[0] > lats[lats.len() - 1],
        "stored orientation is north-first"
    );
    assert!(lons.iter().all(|v| *v <= 180.0));
    assert!(
        lons.iter().any(|v| *v < 0.0),
        "230..299 E must become -130..-61"
    );
}

#[test]
fn mrms_missing_values_are_nan() {
    let bytes = std::fs::read(testdata("mrms_regular_ll_png.grib2")).unwrap();
    let grid = decode_message(&bytes).unwrap();
    let want_missing = manifest()["mrms"]["missing_count"].as_u64().unwrap() as usize;
    let got_missing = grid.values.iter().filter(|v| v.is_nan()).count();
    assert_eq!(got_missing, want_missing);
    assert!(want_missing > 0);
}

#[test]
fn mrms_gz_round_trip_decodes_identically() {
    let gz = std::fs::read(testdata("mrms_regular_ll_png.grib2.gz")).unwrap();
    let raw = radar_grib::mrms::gunzip(&gz).unwrap();
    let a = decode_message(&raw).unwrap();
    let b = radar_grib::mrms::decode_gz(&gz).unwrap();
    // Grid's derived PartialEq treats NaN (missing) as unequal to itself, so compare bit patterns.
    assert_eq!((a.width, a.height), (b.width, b.height));
    let bits = |g: &radar_geo::Grid| g.values.iter().map(|v| v.to_bits()).collect::<Vec<u32>>();
    assert_eq!(bits(&a), bits(&b));
    assert_eq!(a.geo, b.geo);
    assert!(
        a.values.iter().any(|v| v.is_nan()),
        "fixture must exercise missing values"
    );
}

#[test]
fn hrrr_messages_match_eccodes_bitwise() {
    let bytes = std::fs::read(testdata("hrrr_lambert_mini.grib2")).unwrap();
    let m = manifest();
    let names = ["refc", "t2m", "u10", "v10", "cape", "apcp"];
    let selectors = [
        MessageSelector::parameter(0, 16, 196).at_level(10, 0.0),
        MessageSelector::parameter(0, 0, 0).at_level(103, 2.0),
        MessageSelector::parameter(0, 2, 2).at_level(103, 10.0),
        MessageSelector::parameter(0, 2, 3).at_level(103, 10.0),
        MessageSelector::parameter(0, 7, 6).at_level(1, 0.0),
        MessageSelector::parameter(0, 1, 8).at_level(1, 0.0),
    ];
    for (i, name) in names.iter().enumerate() {
        let msg = find_message(&bytes, &selectors[i])
            .unwrap()
            .unwrap_or_else(|| panic!("{name} not found"));
        let want = read_f32(&format!("hrrr_{name}_values.f32le"));
        assert_f32_bits_eq(name, &msg.grid.values, &want);
        let want_missing = m["hrrr"]["messages"][i]["missing_count"].as_u64().unwrap() as usize;
        assert_eq!(
            msg.grid.values.iter().filter(|v| v.is_nan()).count(),
            want_missing,
            "{name} missing"
        );
        let expect_flags = m["hrrr"]["messages"][i]["flags"].as_u64().unwrap() as u8;
        assert_eq!(msg.info.resolution_flags, expect_flags, "{name} flags");
        assert_eq!(msg.uv_grid_relative(), expect_flags & 8 != 0);
    }
}

#[test]
fn hrrr_levels_and_codes_match_eccodes_names() {
    let bytes = std::fs::read(testdata("hrrr_lambert_mini.grib2")).unwrap();
    let infos = radar_grib::list_messages(&bytes).unwrap();
    assert_eq!(infos.len(), 6);
    // REFC: entire atmosphere (eccodes typeOfLevel=atmosphere), level 0
    assert_eq!(infos[0].surface_type, Some(10));
    assert_eq!(infos[0].level_value, Some(0.0));
    // 2 m temperature: height above ground, level 2 (scale octet 0xFF = missing, must give 2 not 2e127)
    assert_eq!(infos[1].surface_type, Some(103));
    assert_eq!(infos[1].level_value, Some(2.0));
    assert_eq!(infos[2].level_value, Some(10.0));
    assert_eq!(infos[1].grid_template, 30);
    assert_eq!(infos[1].data_template, 3);
    assert_eq!(infos[3].data_template, 0);
    assert_eq!(infos[4].data_template, 2);
}

#[test]
fn hrrr_lat_lon_match_eccodes() {
    let bytes = std::fs::read(testdata("hrrr_lambert_mini.grib2")).unwrap();
    let grid = find_message(&bytes, &MessageSelector::parameter(0, 16, 196))
        .unwrap()
        .unwrap()
        .grid;
    let GridGeo::Projected {
        lat_mesh, lon_mesh, ..
    } = &grid.geo
    else {
        panic!("expected projected grid")
    };
    let want_lat = read_f64("hrrr_refc_lats.f64le");
    let want_lon = read_f64("hrrr_refc_lons.f64le");
    let dlat = max_abs_diff(lat_mesh, &want_lat);
    let dlon = max_abs_diff(
        lon_mesh,
        &want_lon
            .iter()
            .map(|v| if *v > 180.0 { v - 360.0 } else { *v })
            .collect::<Vec<_>>(),
    );
    println!("lat max diff {dlat:e}, lon max diff {dlon:e}");
    assert!(dlat <= 1e-9, "lat diff {dlat}");
    assert!(dlon <= 1e-9, "lon diff {dlon}");
}

#[test]
fn hrrr_projected_axes_match_pyproj_spec_procedure() {
    let bytes = std::fs::read(testdata("hrrr_lambert_mini.grib2")).unwrap();
    let grid = find_message(&bytes, &MessageSelector::parameter(0, 16, 196))
        .unwrap()
        .unwrap()
        .grid;
    let GridGeo::Projected {
        crs,
        x,
        y,
        lat_mesh,
        ..
    } = &grid.geo
    else {
        panic!("expected projected grid")
    };
    let want_x = read_f64("hrrr_src_x.f64le");
    let want_y = read_f64("hrrr_src_y.f64le");
    let dx = max_abs_diff(x, &want_x);
    let dy = max_abs_diff(y, &want_y);
    println!("x axis max diff {dx:e} m, y axis max diff {dy:e} m");
    // Projected metres; the lat/lon inputs agree to 1e-9 deg (about 1e-4 m), so 1e-3 m is a tight bound.
    assert!(dx <= 1e-3, "x axis diff {dx} m");
    assert!(dy <= 1e-3, "y axis diff {dy} m");
    assert!(x.windows(2).all(|w| w[1] > w[0]), "x ascending");
    assert!(
        y.windows(2).all(|w| w[1] > w[0]),
        "y ascending for +j scanning"
    );
    assert_eq!(x.len(), grid.width);
    assert_eq!(y.len(), grid.height);
    assert!(lat_mesh.iter().all(|v| v.is_finite()));
    // The CRS is the HRRR Lambert sphere.
    match crs {
        Crs::LambertConformal {
            lat_1,
            lat_2,
            lat_0,
            lon_0,
            radius_m,
        } => {
            assert_eq!((*lat_1, *lat_2, *lat_0), (38.5, 38.5, 38.5));
            assert!((lon_0 - 262.5).abs() < 1e-9 || (lon_0 + 97.5).abs() < 1e-9);
            assert_eq!(*radius_m, 6371229.0);
        }
        other => panic!("unexpected CRS {other:?}"),
    }
}

#[test]
fn radar_geo_lcc_forward_matches_pyproj() {
    let table = read_f64("lcc_points.f64le");
    let crs = Crs::LambertConformal {
        lat_1: 38.5,
        lat_2: 38.5,
        lat_0: 38.5,
        lon_0: -97.5,
        radius_m: 6371229.0,
    };
    let (mut worst_xy, mut worst_inv) = (0.0_f64, 0.0_f64);
    for row in table.chunks_exact(6) {
        let (lon, lat, x, y, ilon, ilat) = (row[0], row[1], row[2], row[3], row[4], row[5]);
        let (fx, fy) = crs.forward(lon, lat);
        worst_xy = worst_xy.max((fx - x).abs()).max((fy - y).abs());
        let (il, ia) = crs.inverse(x, y);
        worst_inv = worst_inv.max((il - ilon).abs()).max((ia - ilat).abs());
    }
    println!("radar-geo LCC forward vs pyproj: {worst_xy:e} m; inverse: {worst_inv:e} deg");
    assert!(worst_xy < 1e-3, "forward {worst_xy} m");
    assert!(worst_inv < 1e-9, "inverse {worst_inv} deg");
}
