//! The stargazing score: 0-100 per HRRR cell for one night hour, from total cloud cover, darkness and
//! the moon. Our own index (design section 7, "Star forecast map"). Day cells are NaN, so they draw
//! transparent and read as no value.

use radar_geo::ephemeris::Sky;
use radar_geo::{Grid, GridGeo};

/// The sun must be at least this far down (nautical twilight's end) for a cell to score.
pub const NIGHT_SUN_ALT_DEG: f64 = -12.0;
/// Fully dark: astronomical twilight is over.
pub const DARK_SUN_ALT_DEG: f64 = -18.0;
/// Score kept at the end of nautical twilight, rising to 1 when fully dark.
pub const TWILIGHT_FACTOR: f64 = 0.7;
/// A moon this high or higher counts at full brightness.
pub const MOON_FULL_EFFECT_ALT_DEG: f64 = 20.0;
/// Share of the score a full, high moon takes away.
pub const MOON_MAX_PENALTY: f64 = 0.55;

/// Score for one cell. NaN in daylight (sun above [`NIGHT_SUN_ALT_DEG`]) or with no cloud value.
#[must_use]
pub fn stargazing_cell(cloud_pct: f32, sun_alt_deg: f64, moon_alt_deg: f64, moon_lit: f64) -> f32 {
    if !cloud_pct.is_finite() || sun_alt_deg.is_nan() || sun_alt_deg > NIGHT_SUN_ALT_DEG {
        return f32::NAN;
    }
    // Thin cloud still hides faint stars, so the clear share counts more than linearly.
    let clear = (1.0 - f64::from(cloud_pct) / 100.0)
        .clamp(0.0, 1.0)
        .powf(1.5);
    let dark = if sun_alt_deg <= DARK_SUN_ALT_DEG {
        1.0
    } else {
        let into = (NIGHT_SUN_ALT_DEG - sun_alt_deg) / (NIGHT_SUN_ALT_DEG - DARK_SUN_ALT_DEG);
        TWILIGHT_FACTOR + (1.0 - TWILIGHT_FACTOR) * into
    };
    let moon = if moon_alt_deg <= 0.0 {
        1.0
    } else {
        let height = (moon_alt_deg / MOON_FULL_EFFECT_ALT_DEG).min(1.0);
        1.0 - MOON_MAX_PENALTY * moon_lit.clamp(0.0, 1.0) * height
    };
    (100.0 * clear * dark * moon).round().clamp(0.0, 100.0) as f32
}

/// Scores a cloud-cover grid (percent) valid at Unix time `unix_s`. Cells use their own latitude and
/// longitude; the sun and moon are placed once for the hour.
#[must_use]
pub fn stargazing_grid(cloud: &Grid, unix_s: f64) -> Vec<f32> {
    let sky = Sky::at(unix_s);
    let w = cloud.width;
    let latlon = |i: usize| -> (f64, f64) {
        match &cloud.geo {
            GridGeo::Regular { lats, lons } => (lats[i / w], lons[i % w]),
            GridGeo::Projected {
                lat_mesh, lon_mesh, ..
            } => (lat_mesh[i], lon_mesh[i]),
        }
    };
    cloud
        .values
        .iter()
        .enumerate()
        .map(|(i, &c)| {
            let (lat, lon) = latlon(i);
            let sun = sky.sun_altitude(lat, lon);
            if sun.is_nan() || sun > NIGHT_SUN_ALT_DEG {
                return f32::NAN;
            }
            stargazing_cell(c, sun, sky.moon_altitude(lat, lon), sky.moon_illuminated)
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn day_and_missing_cloud_are_transparent() {
        assert!(stargazing_cell(0.0, -11.9, -30.0, 0.0).is_nan());
        assert!(stargazing_cell(0.0, 20.0, -30.0, 0.0).is_nan());
        assert!(stargazing_cell(f32::NAN, -40.0, -30.0, 0.0).is_nan());
        assert!(stargazing_cell(0.0, f64::NAN, -30.0, 0.0).is_nan());
    }

    #[test]
    fn clear_dark_moonless_is_one_hundred() {
        assert_eq!(stargazing_cell(0.0, -40.0, -10.0, 1.0), 100.0);
        assert_eq!(stargazing_cell(0.0, -18.0, 5.0, 0.0), 100.0);
    }

    #[test]
    fn clouds_twilight_and_moon_lower_the_score() {
        assert_eq!(stargazing_cell(100.0, -40.0, -10.0, 0.0), 0.0);
        // 50% cloud: 0.5^1.5.
        assert_eq!(stargazing_cell(50.0, -40.0, -10.0, 0.0), 35.0);
        // End of nautical twilight, halfway into astronomical.
        assert_eq!(stargazing_cell(0.0, -12.0, -10.0, 0.0), 70.0);
        assert_eq!(stargazing_cell(0.0, -15.0, -10.0, 0.0), 85.0);
        // Full moon high: Fair, not Excellent. Half lit at 10°: a smaller cut.
        assert_eq!(stargazing_cell(0.0, -40.0, 45.0, 1.0), 45.0);
        assert_eq!(stargazing_cell(0.0, -40.0, 10.0, 0.5), 86.0);
    }

    #[test]
    fn grid_scores_night_and_blanks_day() {
        // 2026-11-18 03:00 UTC: night over the US east, day over the far west Pacific.
        let unix_s = 1_794_970_800.0;
        let grid = Grid {
            width: 2,
            height: 1,
            values: vec![0.0, 0.0],
            geo: GridGeo::Regular {
                lats: vec![42.33],
                lons: vec![-83.05, 151.21],
            },
        };
        let out = stargazing_grid(&grid, unix_s);
        // Detroit: sun -53°, a 56%-lit moon 23° up takes about 31 points.
        assert!((60.0..=75.0).contains(&out[0]), "{out:?}");
        assert!(out[1].is_nan(), "{out:?}");
    }
}
