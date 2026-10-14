//! Geodesy helpers shared by detection, tracking and prefetch.
//!
//! The storm code uses 110.574 km per degree of latitude and 111.320 km per
//! degree of longitude at the equator (spec-shared 6.1). The storm-watch
//! sampler uses 111.32 for latitude instead (defect D4); see `watch.rs`.

use crate::numeric::{radians, round_digits};

/// Mean Earth radius used by the haversine formula (km).
pub const EARTH_RADIUS_KM: f64 = 6371.0088;
/// Kilometres per degree of latitude used by storms and prefetch.
pub const KM_PER_DEG_LAT: f64 = 110.574;
/// Kilometres per degree of longitude at the equator.
pub const KM_PER_DEG_LON_EQUATOR: f64 = 111.320;

/// Great-circle distance in km between two `(lon, lat)` points (degrees).
///
/// Mirrors `storms._haversine_km`, which takes `(lon, lat)` tuples.
#[must_use]
pub fn haversine_km(a: (f64, f64), b: (f64, f64)) -> f64 {
    let (lon1, lat1) = a;
    let (lon2, lat2) = b;
    let phi1 = radians(lat1);
    let phi2 = radians(lat2);
    let d_phi = phi2 - phi1;
    let d_lambda = radians(lon2 - lon1);
    let s1 = (d_phi / 2.0).sin();
    let s2 = (d_lambda / 2.0).sin();
    let h = s1 * s1 + phi1.cos() * phi2.cos() * (s2 * s2);
    2.0 * EARTH_RADIUS_KM * h.sqrt().min(1.0).asin()
}

/// Distance in km from `(lat, lon)` to `(other_lat, other_lon)`.
///
/// Mirrors `storm_prefetch._distance_km`, which takes latitude first.
#[must_use]
pub fn distance_latlon_km(lat: f64, lon: f64, other_lat: f64, other_lon: f64) -> f64 {
    haversine_km((lon, lat), (other_lon, other_lat))
}

/// Longitude scale used for motion: `max(1e-6, 111.320 * cos(lat))`.
#[must_use]
pub fn lon_scale_km(lat: f64) -> f64 {
    (KM_PER_DEG_LON_EQUATOR * radians(lat).cos()).max(1e-6)
}

/// Move `(lon, lat)` by a velocity for a number of minutes (`storms._advance`).
#[must_use]
pub fn advance(lon: f64, lat: f64, east_kmh: f64, north_kmh: f64, minutes: f64) -> (f64, f64) {
    let hours = minutes / 60.0;
    (
        lon + east_kmh * hours / lon_scale_km(lat),
        lat + north_kmh * hours / KM_PER_DEG_LAT,
    )
}

/// Translate a bbox `[west, south, east, north]` for a lead time (`storms._translate_bbox`).
///
/// Each output coordinate is rounded to six decimals, as Python does.
#[must_use]
pub fn translate_bbox(
    bbox: [f64; 4],
    center_lat: f64,
    east_kmh: f64,
    north_kmh: f64,
    lead_minutes: i64,
) -> [f64; 4] {
    let hours = lead_minutes as f64 / 60.0;
    let lat_delta = north_kmh * hours / KM_PER_DEG_LAT;
    let lon_delta = east_kmh * hours / lon_scale_km(center_lat);
    [
        round_digits(bbox[0] + lon_delta, 6),
        round_digits(bbox[1] + lat_delta, 6),
        round_digits(bbox[2] + lon_delta, 6),
        round_digits(bbox[3] + lat_delta, 6),
    ]
}

/// Web Mercator tile `(x, y)` for a point (`storm_prefetch._lat_lon_to_tile`).
///
/// Latitude is clipped to the Mercator limit and the result is clamped to
/// `[0, 2^zoom - 1]`. Python `int()` truncates toward zero, so `as i64` does too.
#[must_use]
pub fn lat_lon_to_tile(lat: f64, lon: f64, zoom: u32) -> (i64, i64) {
    let n = 2_f64.powi(zoom as i32);
    let x = ((lon + 180.0) / 360.0 * n) as i64;
    let clipped = lat.clamp(-85.051_128_78, 85.051_128_78);
    let lat_rad = radians(clipped);
    let y = ((1.0 - lat_rad.tan().asinh() / std::f64::consts::PI) / 2.0 * n) as i64;
    let max = (n as i64) - 1;
    (x.clamp(0, max), y.clamp(0, max))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn haversine_one_degree_of_latitude() {
        // One degree of latitude on the mean-radius sphere.
        let d = haversine_km((0.0, 0.0), (0.0, 1.0));
        assert!(
            (d - EARTH_RADIUS_KM * 1.0_f64.to_radians()).abs() < 1e-9,
            "{d}"
        );
    }

    #[test]
    fn distance_argument_order_is_lat_first() {
        let a = distance_latlon_km(42.9, -84.9, 42.5, -86.0);
        let b = haversine_km((-84.9, 42.9), (-86.0, 42.5));
        assert!((a - b).abs() < 1e-12);
    }

    #[test]
    fn tile_matches_web_mercator_reference() {
        // Reference values: lat 42.9, lon -84.9 at zoom 6 is tile (x=16, y=23).
        assert_eq!(lat_lon_to_tile(42.9, -84.9, 6), (16, 23));
        // Latitude beyond the Mercator limit is clipped, not NaN.
        assert_eq!(lat_lon_to_tile(89.0, 0.0, 2), (2, 0));
        // Longitude 180 would be x = n, clamped to n - 1.
        assert_eq!(lat_lon_to_tile(0.0, 180.0, 3), (7, 4));
    }

    #[test]
    fn translate_rounds_to_six_decimals() {
        let out = translate_bbox([-85.0, 42.0, -84.0, 43.0], 42.5, 0.0, 0.0, 10);
        assert_eq!(out, [-85.0, 42.0, -84.0, 43.0]);
        let out = translate_bbox([0.0, 0.0, 0.0, 0.0], 0.0, 111.32, 0.0, 60);
        assert_eq!(out[0], 1.0);
    }
}
