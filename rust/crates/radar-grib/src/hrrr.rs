//! HRRR access and selection (spec-ingest-mrms-hrrr §4.2, §5.3, §5.4, §12.2).
//!
//! Selection uses GRIB2 codes (discipline, category, number, first-surface type, level), not the
//! eccodes name strings the Python selectors use. See the README for the mapping and the
//! "verify against real files" list.

use bytes::Bytes;
use chrono::{DateTime, Duration as ChronoDuration, Timelike, Utc};

use crate::decode::{
    MessageSelector, SURFACE_ENTIRE_ATMOSPHERE, SURFACE_GROUND, SURFACE_HEIGHT_ABOVE_GROUND,
};
use crate::error::{GribError, Result};
use crate::http::{FULL_FILE_TIMEOUT, HEAD_TIMEOUT, IDX_TIMEOUT, RANGE_TIMEOUT};
use crate::idx::{ByteRange, parse_idx, pick_ranges};
use crate::s3::S3Client;

/// NOAA HRRR bucket (anonymous).
pub const BASE_URL: &str = "https://noaa-hrrr-bdp-pds.s3.amazonaws.com";

/// The HRRR fields the pipeline reads (Python `IDX_MATCHERS` / `VAR_SELECTORS` keys).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum HrrrVar {
    /// Composite reflectivity (REFC).
    Refc,
    /// 2 m temperature (TMP).
    T2m,
    /// 2 m dewpoint (DPT).
    Dpt2m,
    /// Convective available potential energy (CAPE).
    Cape,
    /// 10 m U wind (UGRD).
    U10,
    /// 10 m V wind (VGRD).
    V10,
    /// Categorical rain (CRAIN).
    Crain,
    /// Categorical snow (CSNOW).
    Csnow,
    /// Categorical freezing rain (CFRZR).
    Cfrzr,
    /// Categorical ice pellets (CICEP).
    Cicep,
    /// 2 m relative humidity (RH).
    Rh2m,
    /// Total precipitation (APCP).
    Apcp,
    /// Total cloud cover (TCDC).
    Tcdc,
}

impl HrrrVar {
    /// The Python key name (`refc`, `t2m`, ...), used in logs and fixtures.
    pub fn key(self) -> &'static str {
        match self {
            HrrrVar::Refc => "refc",
            HrrrVar::T2m => "t2m",
            HrrrVar::Dpt2m => "dpt2m",
            HrrrVar::Cape => "cape",
            HrrrVar::U10 => "u10",
            HrrrVar::V10 => "v10",
            HrrrVar::Crain => "crain",
            HrrrVar::Csnow => "csnow",
            HrrrVar::Cfrzr => "cfrzr",
            HrrrVar::Cicep => "cicep",
            HrrrVar::Rh2m => "rh2m",
            HrrrVar::Apcp => "apcp",
            HrrrVar::Tcdc => "tcdc",
        }
    }

    /// The `.idx` matcher terms (Python `IDX_MATCHERS`); every term must appear in the line.
    pub fn idx_matcher(self) -> &'static [&'static str] {
        match self {
            HrrrVar::Refc => &["REFC", "entire atmosphere"],
            HrrrVar::T2m => &["TMP", "2 m above ground"],
            HrrrVar::Dpt2m => &["DPT", "2 m above ground"],
            HrrrVar::Cape => &["CAPE", "surface"],
            HrrrVar::U10 => &["UGRD", "10 m above ground"],
            HrrrVar::V10 => &["VGRD", "10 m above ground"],
            HrrrVar::Crain => &["CRAIN", "surface"],
            HrrrVar::Csnow => &["CSNOW", "surface"],
            HrrrVar::Cfrzr => &["CFRZR", "surface"],
            HrrrVar::Cicep => &["CICEP", "surface"],
            HrrrVar::Rh2m => &["RH", "2 m above ground"],
            HrrrVar::Apcp => &["APCP", "surface"],
            HrrrVar::Tcdc => &["TCDC", "entire atmosphere"],
        }
    }

    /// The GRIB2 selector (Python `VAR_SELECTORS`), expressed as discipline/category/number/level.
    ///
    /// Codes are from WMO Tables 4.1, 4.2 and 4.5, with NCEP local parameters for REFC (0.16.196)
    /// and the categorical precipitation fields (0.1.192..195).
    pub fn selector(self) -> MessageSelector {
        let h2 = (SURFACE_HEIGHT_ABOVE_GROUND, 2.0);
        let h10 = (SURFACE_HEIGHT_ABOVE_GROUND, 10.0);
        let (sel, (surface, level)) = match self {
            HrrrVar::Refc => (
                MessageSelector::parameter(0, 16, 196),
                (SURFACE_ENTIRE_ATMOSPHERE, 0.0),
            ),
            HrrrVar::T2m => (MessageSelector::parameter(0, 0, 0), h2),
            HrrrVar::Dpt2m => (MessageSelector::parameter(0, 0, 6), h2),
            HrrrVar::Cape => (MessageSelector::parameter(0, 7, 6), (SURFACE_GROUND, 0.0)),
            HrrrVar::U10 => (MessageSelector::parameter(0, 2, 2), h10),
            HrrrVar::V10 => (MessageSelector::parameter(0, 2, 3), h10),
            HrrrVar::Crain => (
                MessageSelector::parameter(0, 1, 192).or_number(33),
                (SURFACE_GROUND, 0.0),
            ),
            HrrrVar::Csnow => (
                MessageSelector::parameter(0, 1, 193).or_number(36),
                (SURFACE_GROUND, 0.0),
            ),
            HrrrVar::Cfrzr => (
                MessageSelector::parameter(0, 1, 194).or_number(34),
                (SURFACE_GROUND, 0.0),
            ),
            HrrrVar::Cicep => (
                MessageSelector::parameter(0, 1, 195).or_number(35),
                (SURFACE_GROUND, 0.0),
            ),
            HrrrVar::Rh2m => (MessageSelector::parameter(0, 1, 1), h2),
            HrrrVar::Apcp => (MessageSelector::parameter(0, 1, 8), (SURFACE_GROUND, 0.0)),
            HrrrVar::Tcdc => (
                MessageSelector::parameter(0, 6, 1),
                (SURFACE_ENTIRE_ATMOSPHERE, 0.0),
            ),
        };
        sel.at_level(surface, level)
    }
}

/// Object key of one forecast hour: `hrrr.YYYYMMDD/conus/hrrr.tHHz.wrfsfcFF.grib2`.
pub fn file_key(init: DateTime<Utc>, fhr: u32) -> String {
    format!(
        "hrrr.{}/conus/hrrr.t{:02}z.wrfsfcf{:02}.grib2",
        init.format("%Y%m%d"),
        init.hour(),
        fhr
    )
}

/// Run id `YYYYMMDD_HH` for an init time (UTC hour).
pub fn run_id(init: DateTime<Utc>) -> String {
    format!("{}_{:02}", init.format("%Y%m%d"), init.hour())
}

/// Parses a run id back into its init time (`%Y%m%d_%H`, UTC).
pub fn parse_run_id(run: &str) -> Option<DateTime<Utc>> {
    let (date, hour) = run.split_once('_')?;
    if date.len() != 8 || hour.len() != 2 {
        return None;
    }
    let naive = chrono::NaiveDate::parse_from_str(date, "%Y%m%d").ok()?;
    let h: u32 = hour.parse().ok()?;
    let t = naive.and_hms_opt(h, 0, 0)?;
    Some(t.and_utc())
}

/// Forecast horizon for a run (Python `_horizon_for_run`): 48 for 00/06/12/18z, else 18.
pub fn horizon_for_run(run_hour: u32, forecast_hours: u32, extended_hours: u32) -> u32 {
    if [0, 6, 12, 18].contains(&run_hour) {
        extended_hours
    } else {
        forecast_hours
    }
}

/// The f01 keys probed by the latest-run finder: the hours 0..12 before `now`, truncated to the
/// hour, newest first (Python `_find_latest_run_sync`).
pub fn latest_run_candidates(now: DateTime<Utc>) -> Vec<(String, String)> {
    (0..12)
        .map(|hours_ago| {
            let dt = now - ChronoDuration::hours(hours_ago);
            let run_dt = dt
                .with_minute(0)
                .and_then(|d| d.with_second(0))
                .and_then(|d| d.with_nanosecond(0))
                .unwrap_or(dt);
            (run_id(run_dt), file_key(run_dt, 1))
        })
        .collect()
}

/// Latest run whose f01 object answers HEAD with 200 (Python `_find_latest_run_sync`).
///
/// Transport errors skip a candidate. So does any status other than 200, including 3xx and 403,
/// because redirects are not followed. `Ok(None)` when no candidate succeeds.
pub async fn find_latest_run(s3: &S3Client, now: DateTime<Utc>) -> Result<Option<String>> {
    for (run, key) in latest_run_candidates(now) {
        match s3.head_status(&key, HEAD_TIMEOUT).await {
            Ok(200) => return Ok(Some(run)),
            Ok(_) | Err(_) => continue,
        }
    }
    Ok(None)
}

/// Result of fetching one forecast hour's data.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Fetched {
    /// The hour is not published yet (404 on the full object, or on the index with no full object).
    Pending,
    /// The index has no record for the requested variables. Nothing to decode.
    NoMatchingFields,
    /// Bytes to decode. `full_file` is true when the whole object was fetched (index missing).
    Data { bytes: Bytes, full_file: bool },
}

/// Fetches the GRIB2 bytes an hour needs (Python `_download_subset_sync`).
///
/// 1. GET the `.idx` (15 s). A 404 falls back to the whole object (300 s); a 404 there means pending.
///    Any other index failure is an error. Python falls back on every HTTP error, which fetches about
///    130 MB for a 5xx or a timeout (PLAN D4). This port falls back only on 404.
/// 2. Plan byte ranges for `needed` (no merging). None matched gives `NoMatchingFields`.
/// 3. GET each range (120 s) and require 206 (D5). The ranges are concatenated in index order.
pub async fn fetch_hour(
    s3: &S3Client,
    init: DateTime<Utc>,
    fhr: u32,
    needed: &[HrrrVar],
) -> Result<Fetched> {
    let key = file_key(init, fhr);
    let matchers: Vec<&[&str]> = needed.iter().map(|v| v.idx_matcher()).collect();
    match fetch_subset(s3, &key, &matchers).await? {
        Some(fetched) => Ok(fetched),
        None => fetch_full_file(s3, &key).await,
    }
}

/// Steps 2 and 3 of [`fetch_hour`] for any object with a `.idx` sidecar (HRRR, GFS). `Ok(None)` when the
/// index answers 404; the caller decides whether to fall back to the whole object.
pub async fn fetch_subset(
    s3: &S3Client,
    key: &str,
    matchers: &[&[&str]],
) -> Result<Option<Fetched>> {
    let idx_key = format!("{key}.idx");
    let idx_text = match get_text(s3, &idx_key).await {
        Ok(text) => text,
        Err(e) if e.is_not_found() => return Ok(None),
        Err(e) => return Err(e),
    };
    let records = parse_idx(&idx_text)?;
    let ranges: Vec<ByteRange> = pick_ranges(&records, matchers);
    if ranges.is_empty() {
        return Ok(Some(Fetched::NoMatchingFields));
    }
    let mut out = Vec::new();
    for range in ranges {
        let chunk = s3.get_range(key, range, RANGE_TIMEOUT).await?;
        out.extend_from_slice(&chunk);
    }
    Ok(Some(Fetched::Data {
        bytes: Bytes::from(out),
        full_file: false,
    }))
}

async fn fetch_full_file(s3: &S3Client, key: &str) -> Result<Fetched> {
    match s3.get_object(key, FULL_FILE_TIMEOUT).await {
        Ok(bytes) => Ok(Fetched::Data {
            bytes,
            full_file: true,
        }),
        Err(e) if e.is_not_found() => Ok(Fetched::Pending),
        Err(e) => Err(e),
    }
}

async fn get_text(s3: &S3Client, key: &str) -> Result<String> {
    let bytes = s3.get_object(key, IDX_TIMEOUT).await?;
    String::from_utf8(bytes.to_vec())
        .map_err(|e| GribError::Parse(format!("index is not UTF-8: {e}")))
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    #[test]
    fn keys_and_run_ids_match_spec() {
        let init = Utc.with_ymd_and_hms(2026, 7, 15, 12, 0, 0).unwrap();
        assert_eq!(
            file_key(init, 1),
            "hrrr.20260715/conus/hrrr.t12z.wrfsfcf01.grib2"
        );
        assert_eq!(
            file_key(init, 18),
            "hrrr.20260715/conus/hrrr.t12z.wrfsfcf18.grib2"
        );
        assert_eq!(run_id(init), "20260715_12");
        assert_eq!(parse_run_id("20260715_12"), Some(init));
        assert_eq!(parse_run_id("20260715_1"), None);
        assert_eq!(parse_run_id("2026-07-15_12"), None);
    }

    #[test]
    fn horizons_follow_extended_runs() {
        assert_eq!(horizon_for_run(0, 18, 48), 48);
        assert_eq!(horizon_for_run(6, 18, 48), 48);
        assert_eq!(horizon_for_run(5, 18, 48), 18);
    }

    #[test]
    fn latest_run_candidates_newest_first_hour_truncated() {
        let now = Utc.with_ymd_and_hms(2026, 7, 15, 13, 42, 7).unwrap();
        let c = latest_run_candidates(now);
        assert_eq!(c.len(), 12);
        assert_eq!(c[0].0, "20260715_13");
        assert_eq!(c[0].1, "hrrr.20260715/conus/hrrr.t13z.wrfsfcf01.grib2");
        assert_eq!(c[1].0, "20260715_12");
        assert_eq!(c[11].0, "20260715_02");
        let across_midnight = Utc.with_ymd_and_hms(2026, 7, 15, 0, 30, 0).unwrap();
        assert_eq!(latest_run_candidates(across_midnight)[1].0, "20260714_23");
    }

    #[test]
    fn selectors_use_codes_and_levels() {
        let refc = HrrrVar::Refc.selector();
        assert_eq!(refc.category, Some(16));
        assert_eq!(refc.number, Some(196));
        assert_eq!(refc.surface_type, Some(10));
        let t2m = HrrrVar::T2m.selector();
        assert_eq!((t2m.surface_type, t2m.level_value), (Some(103), Some(2.0)));
        let u10 = HrrrVar::U10.selector();
        assert_eq!((u10.category, u10.number), (Some(2), Some(2)));
        assert_eq!((u10.surface_type, u10.level_value), (Some(103), Some(10.0)));
    }

    #[test]
    fn idx_matchers_are_the_python_terms() {
        assert_eq!(HrrrVar::Refc.idx_matcher(), &["REFC", "entire atmosphere"]);
        assert_eq!(HrrrVar::Apcp.idx_matcher(), &["APCP", "surface"]);
        assert_eq!(HrrrVar::Rh2m.idx_matcher(), &["RH", "2 m above ground"]);
    }
}
