use radar_level2::polar::{ground_to_slant, inverse_wgs84};

#[test]
fn independent_geodetic_gate_centres_stay_within_half_a_z13_pixel() {
    let fixtures: serde_json::Value =
        serde_json::from_str(include_str!("../testdata/tile-geometry.json")).unwrap();
    let mut worst: f64 = 0.0;
    for p in fixtures.as_array().unwrap() {
        let n = |key: &str| p[key].as_f64().unwrap();
        let (s, a) = inverse_wgs84(n("site_lat"), n("site_lon"), n("lat"), n("lon")).unwrap();
        let r = ground_to_slant(s, n("tilt")).unwrap();
        let cross = s * (a - n("bearing")).to_radians().sin();
        let along = r - n("slant");
        let pixel =
            2.0 * std::f64::consts::PI * 6378137.0 * n("lat").to_radians().cos() / (256.0 * 8192.0);
        let error = along.hypot(cross) / pixel;
        worst = worst.max(error);
        assert!(error <= 0.5, "{p}: {error} pixels");
    }
    println!("independent WGS84/4-3-Earth worst_z13_pixel_error={worst}");
}
