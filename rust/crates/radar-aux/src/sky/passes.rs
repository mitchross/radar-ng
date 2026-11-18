//! Visible ISS passes for one viewer.
//!
//! A pass is the time the ISS is at least [`MIN_ELEVATION_DEG`] high. The visible part is where the
//! sun is at least [`SUN_DOWN_DEG`] below the viewer's horizon and the ISS is in sunlight. These are
//! common practice values, chosen by us. Positions come from SGP4 in TEME, turned to Earth-fixed with
//! the IAU sidereal time. The sun uses the Astronomical Almanac low-precision formula and the shadow
//! is a cylinder; both are good to a few seconds of pass time.

use chrono::{DateTime, Duration, Utc};
use serde::Serialize;

/// The ISS must be at least this high.
pub const MIN_ELEVATION_DEG: f64 = 10.0;
/// The sun must be at least this far below the horizon.
pub const SUN_DOWN_DEG: f64 = 6.0;
/// Elements older than this are not used.
pub const MAX_ELEMENT_AGE_DAYS: i64 = 14;

const WGS84_A_KM: f64 = 6378.137;
const WGS84_F: f64 = 1.0 / 298.257_223_563;
const EARTH_RADIUS_KM: f64 = 6378.137;
const AU_KM: f64 = 149_597_870.7;
const COARSE_STEP_S: i64 = 20;

/// One visible pass. Times are UTC, angles in degrees, azimuth clockwise from north.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Pass {
    /// Start of the visible part.
    pub start: DateTime<Utc>,
    /// Azimuth at start.
    pub start_azimuth_deg: f64,
    /// Highest visible point.
    pub max_at: DateTime<Utc>,
    /// Elevation at the highest visible point.
    pub max_elevation_deg: f64,
    /// Azimuth at the highest visible point.
    pub max_azimuth_deg: f64,
    /// End of the visible part (set, or entering Earth's shadow).
    pub end: DateTime<Utc>,
    /// Azimuth at end.
    pub end_azimuth_deg: f64,
}

/// A viewer on the WGS84 ellipsoid.
#[derive(Debug, Clone, Copy)]
pub struct Viewer {
    /// Latitude, degrees.
    pub lat: f64,
    /// Longitude, degrees east.
    pub lon: f64,
    /// Height above the ellipsoid, km.
    pub height_km: f64,
}

#[derive(Debug, Clone, Copy)]
struct Look {
    elevation: f64,
    azimuth: f64,
    visible: bool,
}

/// Whether elements with this epoch are too old to use at `now`.
pub fn elements_stale(epoch: DateTime<Utc>, now: DateTime<Utc>) -> bool {
    now - epoch > Duration::days(MAX_ELEMENT_AGE_DAYS)
}

/// Visible passes that end after `from` and start before `from + hours`.
pub fn visible_passes(
    elements: &sgp4::Elements,
    viewer: Viewer,
    from: DateTime<Utc>,
    hours: i64,
) -> Vec<Pass> {
    let Ok(constants) = sgp4::Constants::from_elements(elements) else {
        return Vec::new();
    };
    let look = |t: DateTime<Utc>| look_at(elements, &constants, viewer, t);
    let until = from + Duration::hours(hours);
    let mut passes = Vec::new();
    let mut t = from;
    let mut prev_up = look(t).is_some_and(|l| l.elevation >= MIN_ELEVATION_DEG);
    let mut rise = prev_up.then_some(from);
    while t < until {
        let next = t + Duration::seconds(COARSE_STEP_S);
        let up = look(next).is_some_and(|l| l.elevation >= MIN_ELEVATION_DEG);
        if up && !prev_up {
            rise = Some(edge(&look, t, next, true));
        } else if !up && prev_up {
            let set = edge(&look, t, next, false);
            if let Some(r) = rise.take()
                && let Some(p) = visible_part(&look, r, set)
            {
                passes.push(p);
            }
        }
        prev_up = up;
        t = next;
    }
    passes
}

/// Bisect the 10° crossing between `a` and `b` to one second.
fn edge(
    look: &impl Fn(DateTime<Utc>) -> Option<Look>,
    mut a: DateTime<Utc>,
    mut b: DateTime<Utc>,
    rising: bool,
) -> DateTime<Utc> {
    while (b - a).num_milliseconds() > 1000 {
        let mid = a + (b - a) / 2;
        let up = look(mid).is_some_and(|l| l.elevation >= MIN_ELEVATION_DEG);
        if up == rising {
            b = mid;
        } else {
            a = mid;
        }
    }
    if rising { b } else { a }
}

/// The visible stretch of one pass, sampled every second.
fn visible_part(
    look: &impl Fn(DateTime<Utc>) -> Option<Look>,
    rise: DateTime<Utc>,
    set: DateTime<Utc>,
) -> Option<Pass> {
    let mut first: Option<(DateTime<Utc>, Look)> = None;
    let mut last: Option<(DateTime<Utc>, Look)> = None;
    let mut best: Option<(DateTime<Utc>, Look)> = None;
    let mut t = rise;
    while t <= set {
        if let Some(l) = look(t).filter(|l| l.visible && l.elevation >= MIN_ELEVATION_DEG) {
            if first.is_none() {
                first = Some((t, l));
            }
            last = Some((t, l));
            if best.is_none_or(|(_, b)| l.elevation > b.elevation) {
                best = Some((t, l));
            }
        } else if first.is_some() {
            break; // entered the shadow; it does not come back out in one pass
        }
        t += Duration::seconds(1);
    }
    let ((start, s), (end, e), (max_at, m)) = (first?, last?, best?);
    Some(Pass {
        start,
        start_azimuth_deg: round1(s.azimuth),
        max_at,
        max_elevation_deg: round1(m.elevation),
        max_azimuth_deg: round1(m.azimuth),
        end,
        end_azimuth_deg: round1(e.azimuth),
    })
}

fn round1(v: f64) -> f64 {
    (v * 10.0).round() / 10.0
}

fn look_at(
    elements: &sgp4::Elements,
    constants: &sgp4::Constants,
    viewer: Viewer,
    t: DateTime<Utc>,
) -> Option<Look> {
    let naive = t.naive_utc();
    let minutes = elements.datetime_to_minutes_since_epoch(&naive).ok()?;
    let prediction = constants.propagate(minutes).ok()?;
    let gmst = sgp4::iau_epoch_to_sidereal_time(sgp4::julian_years_since_j2000(&naive));
    let sat_teme = prediction.position;
    let sun_eci = sun_eci_km(t);
    let (elevation, azimuth) = topocentric(rotate_z(sat_teme, -gmst), viewer);
    let (sun_elevation, _) = topocentric(rotate_z(sun_eci, -gmst), viewer);
    let visible = sun_elevation <= -SUN_DOWN_DEG && sunlit(sat_teme, sun_eci);
    Some(Look {
        elevation,
        azimuth,
        visible,
    })
}

/// Sun position, km, mean equator of date (close enough to TEME for shadows and elevation).
fn sun_eci_km(t: DateTime<Utc>) -> [f64; 3] {
    let jd = t.timestamp_millis() as f64 / 86_400_000.0 + 2_440_587.5;
    let n = jd - 2_451_545.0;
    let l = (280.460 + 0.985_647_4 * n).to_radians();
    let g = (357.528 + 0.985_600_3 * n).to_radians();
    let lambda = l + (1.915 * g.sin() + 0.020 * (2.0 * g).sin()).to_radians();
    let eps = (23.439 - 0.000_000_4 * n).to_radians();
    let r = (1.000_14 - 0.016_71 * g.cos() - 0.000_14 * (2.0 * g).cos()) * AU_KM;
    [
        r * lambda.cos(),
        r * eps.cos() * lambda.sin(),
        r * eps.sin() * lambda.sin(),
    ]
}

/// Outside the cylinder of Earth's shadow.
fn sunlit(sat: [f64; 3], sun: [f64; 3]) -> bool {
    let sun_len = norm(sun);
    let s = [sun[0] / sun_len, sun[1] / sun_len, sun[2] / sun_len];
    let along = dot(sat, s);
    if along >= 0.0 {
        return true;
    }
    let perp = [
        sat[0] - along * s[0],
        sat[1] - along * s[1],
        sat[2] - along * s[2],
    ];
    norm(perp) > EARTH_RADIUS_KM
}

fn rotate_z(v: [f64; 3], angle: f64) -> [f64; 3] {
    let (s, c) = angle.sin_cos();
    [c * v[0] - s * v[1], s * v[0] + c * v[1], v[2]]
}

/// Elevation and azimuth, degrees, of an Earth-fixed point seen from the viewer.
fn topocentric(target: [f64; 3], viewer: Viewer) -> (f64, f64) {
    let (lat, lon) = (viewer.lat.to_radians(), viewer.lon.to_radians());
    let e2 = WGS84_F * (2.0 - WGS84_F);
    let n = WGS84_A_KM / (1.0 - e2 * lat.sin().powi(2)).sqrt();
    let obs = [
        (n + viewer.height_km) * lat.cos() * lon.cos(),
        (n + viewer.height_km) * lat.cos() * lon.sin(),
        (n * (1.0 - e2) + viewer.height_km) * lat.sin(),
    ];
    let d = [target[0] - obs[0], target[1] - obs[1], target[2] - obs[2]];
    let east = -lon.sin() * d[0] + lon.cos() * d[1];
    let north = -lat.sin() * lon.cos() * d[0] - lat.sin() * lon.sin() * d[1] + lat.cos() * d[2];
    let up = lat.cos() * lon.cos() * d[0] + lat.cos() * lon.sin() * d[1] + lat.sin() * d[2];
    let elevation = up.atan2(east.hypot(north)).to_degrees();
    let azimuth = east.atan2(north).to_degrees().rem_euclid(360.0);
    (elevation, azimuth)
}

fn dot(a: [f64; 3], b: [f64; 3]) -> f64 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}

fn norm(a: [f64; 3]) -> f64 {
    dot(a, a).sqrt()
}

#[cfg(test)]
mod tests {
    use chrono::TimeZone;

    use super::*;
    use crate::sky::iss::parse_omm_csv;

    const FIXTURE: &str = include_str!("../../testdata/sky/iss-25544-2026-11-17.csv");
    const DETROIT: Viewer = Viewer {
        lat: 42.3314,
        lon: -83.0458,
        height_km: 0.19,
    };

    fn utc(d: u32, h: u32, m: u32, s: u32) -> DateTime<Utc> {
        Utc.with_ymd_and_hms(2026, 11, d, h, m, s).unwrap()
    }

    fn near(a: DateTime<Utc>, b: DateTime<Utc>) -> bool {
        (a - b).num_seconds().abs() <= 60
    }

    /// Heavens-Above's visible passes for Detroit (42.3314, -83.0458, 190 m), UTC, read on
    /// 2026-11-17 while these elements were current: start, highest point, end. Every one from
    /// 17 Nov 12:00 to 21 Nov 00:00 must match within one minute, and nothing else may show.
    #[test]
    fn matches_published_detroit_passes() {
        let elements = parse_omm_csv(FIXTURE).unwrap();
        let passes = visible_passes(&elements, DETROIT, utc(17, 12, 0, 0), 84);
        let published = [
            (
                utc(17, 23, 47, 42),
                utc(17, 23, 50, 20),
                21.0,
                utc(17, 23, 50, 53),
            ),
            (
                utc(18, 22, 59, 25),
                utc(18, 23, 2, 17),
                25.0,
                utc(18, 23, 4, 53),
            ),
            (
                utc(19, 0, 37, 40),
                utc(19, 0, 37, 49),
                11.0,
                utc(19, 0, 37, 49),
            ),
            (
                utc(19, 23, 49, 30),
                utc(19, 23, 51, 46),
                17.0,
                utc(19, 23, 51, 48),
            ),
            (
                utc(20, 23, 1, 13),
                utc(20, 23, 3, 38),
                18.0,
                utc(20, 23, 5, 46),
            ),
        ];
        assert_eq!(passes.len(), published.len(), "{passes:#?}");
        for (p, (start, max_at, max_el, end)) in passes.iter().zip(published) {
            assert!(near(p.start, start), "start {} vs {start}", p.start);
            assert!(near(p.max_at, max_at), "max {} vs {max_at}", p.max_at);
            assert!(near(p.end, end), "end {} vs {end}", p.end);
            assert!((p.max_elevation_deg - max_el).abs() <= 1.5, "{p:?}");
        }
        // 17 Nov 23:47 rises in the WNW (published 10° WNW, high 21° NNW, set NE).
        assert!(
            (270.0..315.0).contains(&passes[0].start_azimuth_deg),
            "{:?}",
            passes[0]
        );
    }

    #[test]
    fn stale_after_fourteen_days() {
        let elements = parse_omm_csv(FIXTURE).unwrap();
        let epoch = elements.datetime.and_utc();
        assert!(!elements_stale(epoch, epoch + Duration::days(14)));
        assert!(elements_stale(
            epoch,
            epoch + Duration::days(14) + Duration::minutes(1)
        ));
    }

    #[test]
    fn sun_is_down_at_detroit_midnight_and_up_at_noon() {
        let night = rotate_z(sun_eci_km(utc(18, 5, 0, 0)), -gmst(utc(18, 5, 0, 0)));
        let noon = rotate_z(sun_eci_km(utc(18, 17, 20, 0)), -gmst(utc(18, 17, 20, 0)));
        assert!(topocentric(night, DETROIT).0 < -40.0);
        let (el, az) = topocentric(noon, DETROIT);
        // Local noon in late November: about 28° up, due south.
        assert!(
            (el - 28.0).abs() < 2.0 && (az - 180.0).abs() < 5.0,
            "{el} {az}"
        );
    }

    fn gmst(t: DateTime<Utc>) -> f64 {
        sgp4::iau_epoch_to_sidereal_time(sgp4::julian_years_since_j2000(&t.naive_utc()))
    }
}
