//! NOHRSC 24 hour snowfall analysis (`snowfall-24h` layer).
//!
//! NWS NOHRSC posts the National Snowfall Analysis twice a day, for the 24 h ending 00Z and 12Z, about
//! 5 h later: `YYYYMM/sfav2_CONUS_24h_YYYYMMDDHH_grid184.grb2`, about 0.6 MB. GRIB2 `ASNOW` (0.1.29) in
//! metres on the 2.5 km Lambert grid 184, 2145 x 1377, JPEG 2000 packed with a bitmap. NWS public domain.
//! The hourly SNODAS GRIB2 in the `collaborators` folder states no terms, so it is not used.

use bytes::Bytes;
use chrono::{DateTime, Duration as ChronoDuration, Timelike, Utc};
use radar_geo::Grid;
use radar_grib::decode::{MessageSelector, find_message};
use radar_grib::hrrr::run_id;
use radar_grib::http::{HEAD_TIMEOUT, MRMS_GET_TIMEOUT};
use radar_grib::mrms::ensure_south_up;
use radar_grib::nbm::IN_PER_M;
use radar_grib::s3::S3Client;

/// NOHRSC snowfall data directory, no trailing slash.
pub const BASE_URL: &str = "https://www.nohrsc.noaa.gov/snowfall/data";
/// Hours between analyses.
pub const STEP_HOURS: i64 = 12;
/// Analyses searched by [`find_latest_run`], newest first (two days).
pub const SEARCH_CYCLES: i64 = 4;

/// `ASNOW` in the analysis.
pub fn selector() -> MessageSelector {
    MessageSelector::parameter(0, 1, 29)
}

/// Object key of the analysis ending at `end`.
pub fn file_key(end: DateTime<Utc>) -> String {
    format!(
        "{}/sfav2_CONUS_24h_{}_grid184.grb2",
        end.format("%Y%m"),
        end.format("%Y%m%d%H")
    )
}

/// Analysis end times (00Z, 12Z) at or before `now`, newest first.
pub fn run_candidates(now: DateTime<Utc>) -> Vec<DateTime<Utc>> {
    let hour = i64::from(now.hour()) / STEP_HOURS * STEP_HOURS;
    let newest = now
        .date_naive()
        .and_hms_opt(hour as u32, 0, 0)
        .expect("00:00 or 12:00 is valid")
        .and_utc();
    (0..SEARCH_CYCLES)
        .map(|k| newest - ChronoDuration::hours(k * STEP_HOURS))
        .collect()
}

/// Newest analysis that answers HEAD 200, as a run id `YYYYMMDD_HH` of its end time.
pub async fn find_latest_run(
    http: &S3Client,
    now: DateTime<Utc>,
) -> anyhow::Result<Option<String>> {
    for end in run_candidates(now) {
        if let Ok(200) = http.head_status(&file_key(end), HEAD_TIMEOUT).await {
            return Ok(Some(run_id(end)));
        }
    }
    Ok(None)
}

/// The analysis GRIB2, or `None` when it is not posted.
pub async fn fetch(http: &S3Client, end: DateTime<Utc>) -> anyhow::Result<Option<Bytes>> {
    match http.get_object(&file_key(end), MRMS_GET_TIMEOUT).await {
        Ok(b) => Ok(Some(b)),
        Err(e) if e.is_not_found() => Ok(None),
        Err(e) => Err(e.into()),
    }
}

/// The 24 h snowfall in inches, south-up. Negative values (none seen, but cheap to guard) read as 0.
pub fn snowfall_grid(bytes: &[u8]) -> anyhow::Result<Grid> {
    let mut msg = find_message(bytes, &selector())?
        .ok_or_else(|| anyhow::anyhow!("ASNOW message missing from the NOHRSC analysis"))?;
    ensure_south_up(&mut msg.grid);
    for v in &mut msg.grid.values {
        *v = v.max(0.0) * IN_PER_M;
    }
    Ok(msg.grid)
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;
    use radar_geo::GridGeo;

    #[test]
    fn keys_and_candidates_follow_the_twice_daily_analysis() {
        let end = Utc.with_ymd_and_hms(2026, 11, 17, 12, 0, 0).unwrap();
        assert_eq!(
            file_key(end),
            "202611/sfav2_CONUS_24h_2026111712_grid184.grb2"
        );
        let now = Utc.with_ymd_and_hms(2026, 11, 17, 21, 30, 0).unwrap();
        let ids: Vec<String> = run_candidates(now).into_iter().map(run_id).collect();
        assert_eq!(
            ids,
            ["20261117_12", "20261117_00", "20261116_12", "20261116_00"]
        );
        let now = Utc.with_ymd_and_hms(2026, 12, 1, 3, 0, 0).unwrap();
        assert_eq!(
            file_key(run_candidates(now)[1]),
            "202611/sfav2_CONUS_24h_2026113012_grid184.grb2"
        );
    }

    /// The fixture is the real analysis ending 2026-11-17 12Z, cut to the 40 x 40 cells (i 357..397,
    /// j 1043..1083) around its maximum in west-central Idaho and repacked as simple packing (values
    /// kept to 1e-5 m). The expected values were read from the full 2145 x 1377 grid at the same cells.
    #[test]
    fn reads_real_snowfall_at_known_points() {
        let p = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("testdata/nohrsc_24h_2026111712_idaho.grib2");
        let grid = snowfall_grid(&std::fs::read(p).unwrap()).unwrap();
        assert_eq!((grid.width, grid.height), (40, 40));
        let GridGeo::Projected {
            lat_mesh,
            lon_mesh,
            y,
            ..
        } = &grid.geo
        else {
            panic!("NOHRSC is Lambert");
        };
        assert!(y[0] < y[y.len() - 1], "south-up");
        let near = |lat: f64, lon: f64| {
            let k = (0..lat_mesh.len())
                .min_by(|a, b| {
                    let d = |k: usize| (lat_mesh[k] - lat).hypot(lon_mesh[k] - lon);
                    d(*a).total_cmp(&d(*b))
                })
                .unwrap();
            grid.values[k]
        };
        let known = [
            (45.1583, -116.6200, 30.7382),
            (45.3201, -116.7184, 11.5197),
            (44.8141, -116.0226, 4.7783),
        ];
        for (lat, lon, inches) in known {
            assert!((near(lat, lon) - inches).abs() < 0.01, "{lat},{lon}");
        }
        assert!(grid.values.iter().all(|v| *v >= 0.0));
    }
}
