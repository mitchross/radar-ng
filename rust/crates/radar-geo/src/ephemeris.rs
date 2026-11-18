//! Low-precision sun and moon positions for the stargazing layer.
//!
//! The short series from the Astronomical Almanac (sun about 0.01°, moon about 0.3°), with the moon
//! made topocentric by its horizontal parallax. Tests hold them to `astronomy-engine` within 0.5°.

/// Julian date of the Unix epoch.
pub const UNIX_EPOCH_JD: f64 = 2_440_587.5;
const J2000_JD: f64 = 2_451_545.0;

/// Right ascension and declination, degrees.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Equatorial {
    pub ra_deg: f64,
    pub dec_deg: f64,
}

/// Sun and moon at one instant, computed once and evaluated for any place.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Sky {
    /// Greenwich mean sidereal time, degrees.
    pub gmst_deg: f64,
    pub sun: Equatorial,
    pub moon: Equatorial,
    /// Moon horizontal parallax, degrees.
    pub moon_parallax_deg: f64,
    /// Lit fraction of the moon's disc, 0 (new) to 1 (full).
    pub moon_illuminated: f64,
}

fn sin_d(x: f64) -> f64 {
    x.to_radians().sin()
}

fn cos_d(x: f64) -> f64 {
    x.to_radians().cos()
}

fn norm360(x: f64) -> f64 {
    x.rem_euclid(360.0)
}

/// Days from J2000.0 (UT, close enough to TT here) for Unix seconds.
#[must_use]
pub fn days_since_j2000(unix_s: f64) -> f64 {
    unix_s / 86_400.0 + UNIX_EPOCH_JD - J2000_JD
}

fn obliquity_deg(d: f64) -> f64 {
    23.439 - 0.000_000_4 * d
}

fn to_equatorial(lon_deg: f64, lat_deg: f64, eps_deg: f64) -> Equatorial {
    let (sl, cl) = (sin_d(lon_deg), cos_d(lon_deg));
    let (sb, cb) = (sin_d(lat_deg), cos_d(lat_deg));
    let (se, ce) = (sin_d(eps_deg), cos_d(eps_deg));
    let ra = (sl * cb * ce - sb * se).atan2(cl * cb).to_degrees();
    let dec = (sb * ce + cb * se * sl)
        .clamp(-1.0, 1.0)
        .asin()
        .to_degrees();
    Equatorial {
        ra_deg: norm360(ra),
        dec_deg: dec,
    }
}

/// Apparent ecliptic longitude of the sun, degrees.
fn sun_longitude_deg(d: f64) -> f64 {
    let l = 280.460 + 0.985_647_4 * d;
    let g = 357.528 + 0.985_600_3 * d;
    norm360(l + 1.915 * sin_d(g) + 0.020 * sin_d(2.0 * g))
}

/// Moon ecliptic longitude, latitude and horizontal parallax, degrees.
fn moon_ecliptic(d: f64) -> (f64, f64, f64) {
    let t = d / 36_525.0;
    let lon = 218.32 + 481_267.881 * t + 6.29 * sin_d(135.0 + 477_198.87 * t)
        - 1.27 * sin_d(259.3 - 413_335.36 * t)
        + 0.66 * sin_d(235.7 + 890_534.22 * t)
        + 0.21 * sin_d(269.9 + 954_397.74 * t)
        - 0.19 * sin_d(357.5 + 35_999.05 * t)
        - 0.11 * sin_d(186.5 + 966_404.03 * t);
    let lat = 5.13 * sin_d(93.3 + 483_202.02 * t) + 0.28 * sin_d(228.2 + 960_400.89 * t)
        - 0.28 * sin_d(318.3 + 6_003.15 * t)
        - 0.17 * sin_d(217.6 - 407_332.21 * t);
    let par = 0.9508
        + 0.0518 * cos_d(135.0 + 477_198.87 * t)
        + 0.0095 * cos_d(259.3 - 413_335.36 * t)
        + 0.0078 * cos_d(235.7 + 890_534.22 * t)
        + 0.0028 * cos_d(269.9 + 954_397.74 * t);
    (norm360(lon), lat, par)
}

impl Sky {
    /// The sky at Unix time `unix_s` (seconds).
    #[must_use]
    pub fn at(unix_s: f64) -> Sky {
        let d = days_since_j2000(unix_s);
        let t = d / 36_525.0;
        let eps = obliquity_deg(d);
        let sun_lon = sun_longitude_deg(d);
        let (moon_lon, moon_lat, par) = moon_ecliptic(d);
        // Elongation from the sun; the phase angle is close to its supplement.
        let cos_elong = cos_d(moon_lat) * cos_d(moon_lon - sun_lon);
        Sky {
            gmst_deg: norm360(280.460_618_37 + 360.985_647_366_29 * d + 0.000_387_933 * t * t),
            sun: to_equatorial(sun_lon, 0.0, eps),
            moon: to_equatorial(moon_lon, moon_lat, eps),
            moon_parallax_deg: par,
            moon_illuminated: ((1.0 - cos_elong) / 2.0).clamp(0.0, 1.0),
        }
    }

    /// Geometric altitude and azimuth (0 = north, clockwise) of `body` for a place, degrees.
    #[must_use]
    pub fn horizontal(&self, body: Equatorial, lat_deg: f64, lon_deg: f64) -> (f64, f64) {
        let h = self.gmst_deg + lon_deg - body.ra_deg;
        let (sp, cp) = (sin_d(lat_deg), cos_d(lat_deg));
        let (sd, cd) = (sin_d(body.dec_deg), cos_d(body.dec_deg));
        let (sh, ch) = (sin_d(h), cos_d(h));
        let alt = (sp * sd + cp * cd * ch)
            .clamp(-1.0, 1.0)
            .asin()
            .to_degrees();
        let az = (-cd * sh).atan2(sd * cp - cd * ch * sp).to_degrees();
        (alt, norm360(az))
    }

    /// Sun altitude, degrees, no refraction.
    #[must_use]
    pub fn sun_altitude(&self, lat_deg: f64, lon_deg: f64) -> f64 {
        self.horizontal(self.sun, lat_deg, lon_deg).0
    }

    /// Topocentric moon altitude, degrees, no refraction.
    #[must_use]
    pub fn moon_altitude(&self, lat_deg: f64, lon_deg: f64) -> f64 {
        self.moon_horizontal(lat_deg, lon_deg).0
    }

    /// Topocentric moon altitude and azimuth, degrees (parallax lowers the altitude only).
    #[must_use]
    pub fn moon_horizontal(&self, lat_deg: f64, lon_deg: f64) -> (f64, f64) {
        let (alt, az) = self.horizontal(self.moon, lat_deg, lon_deg);
        let p = sin_d(self.moon_parallax_deg).asin().to_degrees();
        (alt - p * cos_d(alt), az)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// astronomy-engine 2.1.19 (`Equator(..., ofdate, aberration)` + `Horizon(..., null)`, observer
    /// at sea level): time, lat, lon, sun alt, sun az, moon alt, moon az, moon phase fraction.
    type Row = (&'static str, f64, f64, f64, f64, f64, f64, f64);
    const REFERENCE: [Row; 40] = [
        (
            "2026-11-18T03:00:00Z",
            42.3314,
            -83.0458,
            -52.949,
            297.918,
            23.034,
            229.08,
            0.5634,
        ), // detroit
        (
            "2026-11-18T03:00:00Z",
            39.7392,
            -104.9903,
            -37.85,
            275.99,
            35.473,
            207.963,
            0.5634,
        ), // denver
        (
            "2026-11-18T03:00:00Z",
            25.7617,
            -80.1918,
            -60.275,
            276.24,
            31.018,
            238.561,
            0.5634,
        ), // miami
        (
            "2026-11-18T03:00:00Z",
            47.6062,
            -122.3321,
            -25.003,
            268.427,
            31.626,
            186.108,
            0.5634,
        ), // seattle
        (
            "2026-11-18T03:00:00Z",
            -33.8688,
            151.2093,
            66.986,
            304.541,
            11.778,
            93.382,
            0.5634,
        ), // sydney
        (
            "2026-11-24T05:30:00Z",
            42.3314,
            -83.0458,
            -68.09,
            7.054,
            69.825,
            207.133,
            0.9958,
        ), // detroit
        (
            "2026-11-24T05:30:00Z",
            39.7392,
            -104.9903,
            -64.774,
            313.918,
            71.076,
            142.894,
            0.9958,
        ), // denver
        (
            "2026-11-24T05:30:00Z",
            25.7617,
            -80.1918,
            -82.637,
            46.093,
            78.258,
            265.059,
            0.9958,
        ), // miami
        (
            "2026-11-24T05:30:00Z",
            47.6062,
            -122.3321,
            -50.073,
            299.842,
            56.378,
            124.548,
            0.9958,
        ), // seattle
        (
            "2026-11-24T05:30:00Z",
            -33.8688,
            151.2093,
            38.196,
            269.439,
            -34.907,
            81.81,
            0.9958,
        ), // sydney
        (
            "2027-01-03T10:00:00Z",
            42.3314,
            -83.0458,
            -32.67,
            92.247,
            2.253,
            126.791,
            0.1695,
        ), // detroit
        (
            "2027-01-03T10:00:00Z",
            39.7392,
            -104.9903,
            -49.329,
            78.842,
            -10.817,
            112.9,
            0.1695,
        ), // denver
        (
            "2027-01-03T10:00:00Z",
            25.7617,
            -80.1918,
            -28.023,
            103.36,
            14.115,
            126.495,
            0.1695,
        ), // miami
        (
            "2027-01-03T10:00:00Z",
            47.6062,
            -122.3321,
            -57.375,
            49.91,
            -24.966,
            99.625,
            0.1695,
        ), // seattle
        (
            "2027-01-03T10:00:00Z",
            -33.8688,
            151.2093,
            -9.67,
            233.969,
            -32.654,
            187.249,
            0.1695,
        ), // sydney
        (
            "2027-03-20T01:00:00Z",
            42.3314,
            -83.0458,
            -14.898,
            283.562,
            43.148,
            119.419,
            0.9245,
        ), // detroit
        (
            "2027-03-20T01:00:00Z",
            39.7392,
            -104.9903,
            1.259,
            268.535,
            28.207,
            99.058,
            0.9245,
        ), // denver
        (
            "2027-03-20T01:00:00Z",
            25.7617,
            -80.1918,
            -20.658,
            280.097,
            51.813,
            104.719,
            0.9245,
        ), // miami
        (
            "2027-03-20T01:00:00Z",
            47.6062,
            -122.3321,
            12.597,
            255.326,
            15.007,
            89.842,
            0.9245,
        ), // seattle
        (
            "2027-03-20T01:00:00Z",
            -33.8688,
            151.2093,
            53.359,
            26.976,
            -64.782,
            145.904,
            0.9245,
        ), // sydney
        (
            "2027-06-21T04:00:00Z",
            42.3314,
            -83.0458,
            -20.754,
            336.998,
            9.502,
            134.984,
            0.9586,
        ), // detroit
        (
            "2027-06-21T04:00:00Z",
            39.7392,
            -104.9903,
            -13.947,
            317.679,
            -2.151,
            119.504,
            0.9586,
        ), // denver
        (
            "2027-06-21T04:00:00Z",
            25.7617,
            -80.1918,
            -36.91,
            336.171,
            22.853,
            133.445,
            0.9586,
        ), // miami
        (
            "2027-06-21T04:00:00Z",
            47.6062,
            -122.3321,
            0.602,
            305.337,
            -16.706,
            107.12,
            0.9586,
        ), // seattle
        (
            "2027-06-21T04:00:00Z",
            -33.8688,
            151.2093,
            25.644,
            328.606,
            -33.557,
            174.412,
            0.9586,
        ), // sydney
        (
            "2027-08-12T07:00:00Z",
            42.3314,
            -83.0458,
            -29.585,
            23.082,
            -11.0,
            242.26,
            0.7894,
        ), // detroit
        (
            "2027-08-12T07:00:00Z",
            39.7392,
            -104.9903,
            -35.241,
            358.493,
            4.017,
            228.083,
            0.7894,
        ), // denver
        (
            "2027-08-12T07:00:00Z",
            25.7617,
            -80.1918,
            -43.24,
            31.951,
            -5.31,
            241.855,
            0.7894,
        ), // miami
        (
            "2027-08-12T07:00:00Z",
            47.6062,
            -122.3321,
            -25.21,
            340.075,
            6.442,
            214.457,
            0.7894,
        ), // seattle
        (
            "2027-08-12T07:00:00Z",
            -33.8688,
            151.2093,
            3.682,
            290.84,
            48.43,
            93.813,
            0.7894,
        ), // sydney
        (
            "2026-12-24T23:00:00Z",
            42.3314,
            -83.0458,
            -9.973,
            247.336,
            1.065,
            56.735,
            0.9870,
        ), // detroit
        (
            "2026-12-24T23:00:00Z",
            39.7392,
            -104.9903,
            5.637,
            233.053,
            -13.087,
            41.441,
            0.9870,
        ), // denver
        (
            "2026-12-24T23:00:00Z",
            25.7617,
            -80.1918,
            -5.662,
            246.714,
            -5.811,
            58.714,
            0.9870,
        ), // miami
        (
            "2026-12-24T23:00:00Z",
            47.6062,
            -122.3321,
            9.278,
            219.118,
            -13.581,
            26.146,
            0.9870,
        ), // seattle
        (
            "2026-12-24T23:00:00Z",
            -33.8688,
            151.2093,
            50.526,
            86.411,
            -40.321,
            276.716,
            0.9870,
        ), // sydney
        (
            "2028-02-29T12:00:00Z",
            42.3314,
            -83.0458,
            -2.322,
            98.249,
            -15.195,
            54.473,
            0.1461,
        ), // detroit
        (
            "2028-02-29T12:00:00Z",
            39.7392,
            -104.9903,
            -18.776,
            84.285,
            -28.689,
            36.155,
            0.1461,
        ), // denver
        (
            "2028-02-29T12:00:00Z",
            25.7617,
            -80.1918,
            2.655,
            99.817,
            -22.041,
            61.015,
            0.1461,
        ), // miami
        (
            "2028-02-29T12:00:00Z",
            47.6062,
            -122.3321,
            -29.066,
            67.5,
            -27.263,
            16.519,
            0.1461,
        ), // seattle
        (
            "2028-02-29T12:00:00Z",
            -33.8688,
            151.2093,
            -38.644,
            222.094,
            -24.292,
            272.096,
            0.1461,
        ), // sydney
    ];

    fn unix(iso: &str) -> f64 {
        // "YYYY-MM-DDTHH:MM:SSZ", days from civil (Howard Hinnant's algorithm).
        let n = |a: usize, b: usize| iso[a..b].parse::<i64>().unwrap();
        let (y, m, d) = (n(0, 4), n(5, 7), n(8, 10));
        let (y, m) = if m <= 2 { (y - 1, m + 9) } else { (y, m - 3) };
        let era = y.div_euclid(400);
        let yoe = y - era * 400;
        let doy = (153 * m + 2) / 5 + d - 1;
        let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
        let days = era * 146_097 + doe - 719_468;
        (days * 86_400 + n(11, 13) * 3600 + n(14, 16) * 60 + n(17, 19)) as f64
    }

    fn angle_diff(a: f64, b: f64) -> f64 {
        ((a - b + 540.0).rem_euclid(360.0) - 180.0).abs()
    }

    #[test]
    fn unix_helper_matches_a_known_instant() {
        assert_eq!(unix("2000-01-01T12:00:00Z"), 946_728_000.0);
        assert!(days_since_j2000(946_728_000.0).abs() < 1e-9);
    }

    #[test]
    fn sun_and_moon_match_astronomy_engine_within_half_a_degree() {
        let (mut worst_sun, mut worst_moon) = (0.0_f64, 0.0_f64);
        for (t, lat, lon, sun_alt, sun_az, moon_alt, moon_az, frac) in REFERENCE {
            let sky = Sky::at(unix(t));
            let (sa, sz) = sky.horizontal(sky.sun, lat, lon);
            let (ma, mz) = sky.moon_horizontal(lat, lon);
            let sun_err = (sa - sun_alt)
                .abs()
                .max(angle_diff(sz, sun_az) * cos_d(sun_alt));
            let moon_err = (ma - moon_alt)
                .abs()
                .max(angle_diff(mz, moon_az) * cos_d(moon_alt));
            assert!(
                sun_err < 0.5,
                "sun {t} {lat},{lon}: {sa},{sz} vs {sun_alt},{sun_az}"
            );
            assert!(
                moon_err < 0.5,
                "moon {t} {lat},{lon}: {ma},{mz} vs {moon_alt},{moon_az}"
            );
            assert!(
                (sky.moon_illuminated - frac).abs() < 0.02,
                "phase {t}: {} vs {frac}",
                sky.moon_illuminated
            );
            worst_sun = worst_sun.max(sun_err);
            worst_moon = worst_moon.max(moon_err);
        }
        // The sun series is far better than the bound; keep it honest.
        assert!(worst_sun < 0.05, "worst sun error {worst_sun}");
        assert!(worst_moon < 0.5, "worst moon error {worst_moon}");
    }

    #[test]
    fn shortcuts_agree_with_horizontal() {
        let sky = Sky::at(unix("2026-11-18T03:00:00Z"));
        assert_eq!(
            sky.sun_altitude(42.0, -83.0),
            sky.horizontal(sky.sun, 42.0, -83.0).0
        );
        assert_eq!(
            sky.moon_altitude(42.0, -83.0),
            sky.moon_horizontal(42.0, -83.0).0
        );
    }
}
