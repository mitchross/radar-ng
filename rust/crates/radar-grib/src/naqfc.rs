//! NAQFC / AQMv7 access (spec-ingest-aux §5; Python `ingest_airquality/activities.py`).
//!
//! AQM CONUS runs publish one GRIB2 per product on grid 227. The AWS mirror is tried first, then
//! NOMADS. Discovery and download use plain HTTPS HEAD/GET, not the ListObjectsV2 client.

use std::time::Duration;

use bytes::Bytes;
use chrono::{DateTime, Duration as ChronoDuration, NaiveDate, TimeZone, Utc};

use crate::error::{GribError, Result};
use crate::http::{FULL_FILE_TIMEOUT, HEAD_TIMEOUT};

/// AWS mirror of the AQMv7 CONUS (CS) products.
pub const AQM_BASE: &str = "https://noaa-nws-naqfc-pds.s3.amazonaws.com/AQMv7/CS";
/// NOMADS fallback for the same products.
pub const AQM_NOMADS_BASE: &str = "https://nomads.ncep.noaa.gov/pub/data/nccf/com/aqm/prod";
/// Cycles searched by the latest-run finder, newest first (Python `AQM_CYCLES`).
pub const AQM_CYCLES: [u32; 2] = [12, 6];
/// Product used to probe for a run (Python `_find_latest_run_sync`).
pub const PROBE_PRODUCT: &str = "ave_1hr_pm25_bc";

/// Base URLs for the two sources. Tests can point both at a local server.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AqmSources {
    /// AWS mirror base (no trailing slash).
    pub aws_base: String,
    /// NOMADS base (no trailing slash).
    pub nomads_base: String,
}

impl Default for AqmSources {
    fn default() -> Self {
        Self {
            aws_base: AQM_BASE.to_string(),
            nomads_base: AQM_NOMADS_BASE.to_string(),
        }
    }
}

/// Splits a run id `YYYYMMDD_CC` into its date string and cycle (`"12"`).
pub fn split_run_id(run_id: &str) -> Result<(&str, &str)> {
    let bad = || GribError::Parse(format!("bad AQM run id {run_id:?}, expected YYYYMMDD_CC"));
    let (date, cycle) = run_id.split_once('_').ok_or_else(bad)?;
    if date.len() != 8 || !date.bytes().all(|b| b.is_ascii_digit()) || cycle.len() != 2 {
        return Err(bad());
    }
    Ok((date, cycle))
}

impl AqmSources {
    /// Candidate URLs for one product of a run, in try order: AWS, then NOMADS
    /// (Python `_grib_urls`).
    pub fn grib_urls(&self, run_id: &str, product: &str) -> Result<Vec<String>> {
        let (date, cc) = split_run_id(run_id)?;
        Ok(vec![
            format!(
                "{}/{date}/{cc}/aqm.t{cc}z.{product}.{date}.227.grib2",
                self.aws_base
            ),
            format!(
                "{}/aqm.{date}/{cc}/aqm.t{cc}z.{product}.227.grib2",
                self.nomads_base
            ),
        ])
    }
}

/// Latest AQM run with a 200 HEAD on the probe product (Python `_find_latest_run_sync`).
///
/// Looks at the last three UTC days, cycles 12 then 6, skipping cycles later than `now`. Each cycle
/// probes AWS then NOMADS. A non-200 status or a transport error moves on to the next URL, with no
/// redirects followed. Returns `Ok(None)` when nothing answers 200.
pub async fn find_latest_run(
    http: &reqwest::Client,
    sources: &AqmSources,
    now: DateTime<Utc>,
) -> Result<Option<String>> {
    for days_ago in 0..3 {
        let date = (now - ChronoDuration::days(days_ago)).date_naive();
        for cycle in AQM_CYCLES {
            let run_dt = run_start(date, cycle);
            if run_dt > now {
                continue;
            }
            let run_id = format!("{}_{:02}", date.format("%Y%m%d"), cycle);
            for url in sources.grib_urls(&run_id, PROBE_PRODUCT)? {
                match http.head(&url).timeout(HEAD_TIMEOUT).send().await {
                    Ok(resp) if resp.status().as_u16() == 200 => return Ok(Some(run_id)),
                    Ok(_) | Err(_) => continue,
                }
            }
        }
    }
    Ok(None)
}

fn run_start(date: NaiveDate, cycle: u32) -> DateTime<Utc> {
    Utc.from_utc_datetime(
        &date
            .and_hms_opt(cycle, 0, 0)
            .expect("cycle is a valid hour"),
    )
}

/// Downloads one product of a run as a whole GRIB2 file (Python `_download` in
/// `aqm_render_chunk`, 300 s timeout).
///
/// Falls back to the next URL only on 404, as the Python does. The last source's 404 is an error.
/// Unlike the Python, the body is returned in memory rather than streamed to a temp file.
pub async fn download_product(
    http: &reqwest::Client,
    sources: &AqmSources,
    run_id: &str,
    product: &str,
) -> Result<Bytes> {
    download_product_with_timeout(http, sources, run_id, product, FULL_FILE_TIMEOUT).await
}

async fn download_product_with_timeout(
    http: &reqwest::Client,
    sources: &AqmSources,
    run_id: &str,
    product: &str,
    timeout: Duration,
) -> Result<Bytes> {
    let urls = sources.grib_urls(run_id, product)?;
    let last = urls.len() - 1;
    for (i, url) in urls.iter().enumerate() {
        let resp = http
            .get(url)
            .timeout(timeout)
            .send()
            .await
            .map_err(|e| GribError::transport(url, e))?;
        let status = resp.status();
        if status.as_u16() == 404 && i != last {
            continue;
        }
        if !status.is_success() {
            return Err(GribError::HttpStatus {
                status: status.as_u16(),
                url: url.clone(),
            });
        }
        return resp.bytes().await.map_err(|e| GribError::transport(url, e));
    }
    Err(GribError::Parse(format!("no AQM source for {run_id}")))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn urls_match_the_python_builder() {
        let urls = AqmSources::default()
            .grib_urls("20260716_12", "ave_1hr_pm25_bc")
            .unwrap();
        assert_eq!(
            urls,
            vec![
                "https://noaa-nws-naqfc-pds.s3.amazonaws.com/AQMv7/CS/20260716/12/aqm.t12z.ave_1hr_pm25_bc.20260716.227.grib2",
                "https://nomads.ncep.noaa.gov/pub/data/nccf/com/aqm/prod/aqm.20260716/12/aqm.t12z.ave_1hr_pm25_bc.227.grib2",
            ]
        );
    }

    #[test]
    fn bad_run_ids_are_rejected() {
        assert!(split_run_id("2026071612").is_err());
        assert!(split_run_id("20260716_1").is_err());
        assert!(split_run_id("2026-07-16_12").is_err());
        assert_eq!(split_run_id("20260716_06").unwrap(), ("20260716", "06"));
    }

    #[test]
    fn future_cycles_are_skipped_by_run_start() {
        let now = Utc.with_ymd_and_hms(2026, 7, 16, 9, 0, 0).unwrap();
        let date = now.date_naive();
        assert!(run_start(date, 12) > now);
        assert!(run_start(date, 6) <= now);
    }
}
