//! NOAA/CPC global UV Index forecast (`uv` layer).
//!
//! CPC runs the TUV radiative transfer model on the 12z GFS (ozone, aerosol, cloud, elevation, snow) and
//! posts one GRIB2 file per forecast hour, f01 to f120, to NOMADS at about 18:30 UTC. NOMADS keeps two
//! days. The field is the erythemal (CIE-weighted) irradiance in W/m2, NCEP local 0.7.196 at the surface;
//! UV Index = value x 40. Source: <https://www.cpc.ncep.noaa.gov/products/stratosphere/uv_index/uv_global.shtml>.

use std::time::Duration;

use chrono::{DateTime, Duration as ChronoDuration, NaiveDate, TimeZone, Utc};
use tokio::sync::Mutex;
use tokio::time::Instant;

use crate::decode::{MessageSelector, SURFACE_GROUND};
use crate::error::Result;
use crate::hrrr::{Fetched, run_id};
use crate::http::{HEAD_TIMEOUT, MRMS_GET_TIMEOUT};
use crate::s3::S3Client;

/// NOMADS directory of the product (no trailing slash).
pub const BASE_URL: &str = "https://nomads.ncep.noaa.gov/pub/data/nccf/com/uvi/prod";
/// The only cycle CPC runs.
pub const CYCLE: u32 = 12;
/// UV Index per W/m2 of erythemal irradiance (WMO definition: 1 UVI = 25 mW/m2).
pub const UVI_PER_W_M2: f32 = 40.0;
/// Days searched by [`find_latest_run`], today first. CPC skips a day now and then.
pub const SEARCH_DAYS: i64 = 3;

/// Gap between NOMADS requests from one process. NCEP firewalls clients above 120 requests a minute
/// (SCN 21-32), and the AQM fallback shares the address.
pub const MIN_REQUEST_INTERVAL: Duration = Duration::from_secs(1);

static LAST_REQUEST: Mutex<Option<Instant>> = Mutex::const_new(None);

/// Wait until [`MIN_REQUEST_INTERVAL`] has passed since this process's previous NOMADS request.
async fn pace() {
    let mut last = LAST_REQUEST.lock().await;
    if let Some(previous) = *last {
        tokio::time::sleep_until(previous + MIN_REQUEST_INTERVAL).await;
    }
    *last = Some(Instant::now());
}

/// GRIB2 selector of the erythemal irradiance (NCEP local 0.7.196, ground surface).
pub fn selector() -> MessageSelector {
    MessageSelector::parameter(0, 7, 196).at_level(SURFACE_GROUND, 0.0)
}

/// Object key of one forecast hour: `uvi.YYYYMMDD/uv.t12z.grbfFF.grib2` (two digits up to f99).
pub fn file_key(init: DateTime<Utc>, fhr: u32) -> String {
    format!(
        "uvi.{}/uv.t{CYCLE:02}z.grbf{fhr:02}.grib2",
        init.format("%Y%m%d")
    )
}

/// The 12z inits at or before `now`, newest first.
pub fn run_candidates(now: DateTime<Utc>) -> Vec<DateTime<Utc>> {
    (0..SEARCH_DAYS)
        .map(|d| init_on((now - ChronoDuration::days(d)).date_naive()))
        .filter(|init| *init <= now)
        .collect()
}

fn init_on(date: NaiveDate) -> DateTime<Utc> {
    Utc.from_utc_datetime(&date.and_hms_opt(CYCLE, 0, 0).expect("12:00 is valid"))
}

/// Newest run whose `probe_fhr` file answers HEAD 200, as a run id `YYYYMMDD_12`.
pub async fn find_latest_run(
    s3: &S3Client,
    now: DateTime<Utc>,
    probe_fhr: u32,
) -> Result<Option<String>> {
    for init in run_candidates(now) {
        pace().await;
        if let Ok(200) = s3
            .head_status(&file_key(init, probe_fhr), HEAD_TIMEOUT)
            .await
        {
            return Ok(Some(run_id(init)));
        }
    }
    Ok(None)
}

/// One forecast hour (about 300 kB, one message), paced. 404 is `Pending`.
pub async fn fetch_hour(s3: &S3Client, init: DateTime<Utc>, fhr: u32) -> Result<Fetched> {
    pace().await;
    match s3.get_object(&file_key(init, fhr), MRMS_GET_TIMEOUT).await {
        Ok(bytes) => Ok(Fetched::Data {
            bytes,
            full_file: true,
        }),
        Err(e) if e.is_not_found() => Ok(Fetched::Pending),
        Err(e) => Err(e),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keys_pad_to_two_digits() {
        let init = Utc.with_ymd_and_hms(2026, 10, 15, 12, 0, 0).unwrap();
        assert_eq!(file_key(init, 1), "uvi.20261015/uv.t12z.grbf01.grib2");
        assert_eq!(file_key(init, 96), "uvi.20261015/uv.t12z.grbf96.grib2");
        assert_eq!(file_key(init, 116), "uvi.20261015/uv.t12z.grbf116.grib2");
    }

    #[test]
    fn candidates_skip_a_cycle_that_has_not_started() {
        let morning = Utc.with_ymd_and_hms(2026, 10, 15, 9, 0, 0).unwrap();
        let ids: Vec<String> = run_candidates(morning).into_iter().map(run_id).collect();
        assert_eq!(ids, ["20261014_12", "20261013_12"]);
        let evening = Utc.with_ymd_and_hms(2026, 10, 15, 19, 0, 0).unwrap();
        let ids: Vec<String> = run_candidates(evening).into_iter().map(run_id).collect();
        assert_eq!(ids, ["20261015_12", "20261014_12", "20261013_12"]);
    }

    #[test]
    fn selector_is_the_ncep_uvi_code() {
        let s = selector();
        assert_eq!(
            (s.discipline, s.category, s.number),
            (Some(0), Some(7), Some(196))
        );
        assert_eq!(
            (s.surface_type, s.level_value),
            (Some(SURFACE_GROUND), Some(0.0))
        );
    }
}
