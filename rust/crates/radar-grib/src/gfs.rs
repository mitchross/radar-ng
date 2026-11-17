//! GFS 0.25 degree access for the long-range simulated radar (`radar-gfs`).
//!
//! `pgrb2.0p25` files are hourly to f120 on the NOAA open-data bucket. Each has a `.idx`, so only the
//! composite reflectivity message (about 0.9 MB of a 500 MB file) is fetched by byte range. There is no
//! whole-file fallback: a missing index means the hour is not published yet.

use chrono::{DateTime, Duration as ChronoDuration, Timelike, Utc};

use crate::decode::{MessageSelector, SURFACE_ENTIRE_ATMOSPHERE};
use crate::error::Result;
use crate::hrrr::{Fetched, fetch_subset, run_id};
use crate::http::HEAD_TIMEOUT;
use crate::s3::S3Client;

/// NOAA GFS bucket (anonymous).
pub const BASE_URL: &str = "https://noaa-gfs-bdp-pds.s3.amazonaws.com";
/// `.idx` terms of the composite reflectivity record (`REFC:entire atmosphere`).
pub const REFC_IDX: &[&str] = &["REFC", "entire atmosphere"];
/// Cycles searched by [`find_latest_run`]: the last five 6-hourly runs, so a late run never leaves a gap.
pub const RUN_CANDIDATES: usize = 5;

/// GRIB2 selector of composite reflectivity (NCEP local 0.16.196, entire atmosphere), as in HRRR.
pub fn refc_selector() -> MessageSelector {
    MessageSelector::parameter(0, 16, 196).at_level(SURFACE_ENTIRE_ATMOSPHERE, 0.0)
}

/// Object key of one forecast hour: `gfs.YYYYMMDD/HH/atmos/gfs.tHHz.pgrb2.0p25.fFFF`.
pub fn file_key(init: DateTime<Utc>, fhr: u32) -> String {
    format!(
        "gfs.{}/{:02}/atmos/gfs.t{:02}z.pgrb2.0p25.f{:03}",
        init.format("%Y%m%d"),
        init.hour(),
        init.hour(),
        fhr
    )
}

/// The 00/06/12/18z inits at or before `now`, newest first.
pub fn run_candidates(now: DateTime<Utc>) -> Vec<DateTime<Utc>> {
    let floor = now
        .with_hour(now.hour() - now.hour() % 6)
        .and_then(|d| d.with_minute(0))
        .and_then(|d| d.with_second(0))
        .and_then(|d| d.with_nanosecond(0))
        .unwrap_or(now);
    (0..RUN_CANDIDATES as i64)
        .map(|k| floor - ChronoDuration::hours(6 * k))
        .collect()
}

/// Newest run whose `probe_fhr` index answers HEAD 200, as a run id `YYYYMMDD_HH`.
///
/// GFS posts hours in order, so the last hour the pipeline needs being present means every hour is.
pub async fn find_latest_run(
    s3: &S3Client,
    now: DateTime<Utc>,
    probe_fhr: u32,
) -> Result<Option<String>> {
    for init in run_candidates(now) {
        let key = format!("{}.idx", file_key(init, probe_fhr));
        if let Ok(200) = s3.head_status(&key, HEAD_TIMEOUT).await {
            return Ok(Some(run_id(init)));
        }
    }
    Ok(None)
}

/// The REFC message of one forecast hour, by `.idx` byte range. A missing index is `Pending`.
pub async fn fetch_refc(s3: &S3Client, init: DateTime<Utc>, fhr: u32) -> Result<Fetched> {
    Ok(fetch_subset(s3, &file_key(init, fhr), &[REFC_IDX])
        .await?
        .unwrap_or(Fetched::Pending))
}

/// `.idx` names of the categorical precipitation-type records, in `CRAIN, CSNOW, CFRZR, CICEP` order.
pub const TYPE_FLAG_IDX: [&str; 4] = [
    "CRAIN:surface",
    "CSNOW:surface",
    "CFRZR:surface",
    "CICEP:surface",
];

/// `.idx` terms of the four type flags valid at `fhr`. The file also holds an average of each flag
/// over the last hours (`18-24 hour ave fcst`); the `:<fhr> hour fcst:` term leaves it out.
pub fn type_flag_terms(fhr: u32) -> [[String; 2]; 4] {
    TYPE_FLAG_IDX.map(|name| [name.to_string(), format!(":{fhr} hour fcst:")])
}

/// [`fetch_refc`] plus the instantaneous CRAIN, CSNOW, CFRZR and CICEP records (about 150 KB more).
pub async fn fetch_refc_typed(s3: &S3Client, init: DateTime<Utc>, fhr: u32) -> Result<Fetched> {
    let flags = type_flag_terms(fhr);
    let flag_terms: Vec<[&str; 2]> = flags
        .iter()
        .map(|[a, b]| [a.as_str(), b.as_str()])
        .collect();
    let mut matchers: Vec<&[&str]> = vec![REFC_IDX];
    matchers.extend(flag_terms.iter().map(|t| t.as_slice()));
    Ok(fetch_subset(s3, &file_key(init, fhr), &matchers)
        .await?
        .unwrap_or(Fetched::Pending))
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    #[test]
    fn keys_use_three_digit_hours() {
        let init = Utc.with_ymd_and_hms(2026, 10, 15, 12, 0, 0).unwrap();
        assert_eq!(
            file_key(init, 72),
            "gfs.20261015/12/atmos/gfs.t12z.pgrb2.0p25.f072"
        );
        let init = Utc.with_ymd_and_hms(2026, 10, 15, 6, 0, 0).unwrap();
        assert_eq!(
            file_key(init, 120),
            "gfs.20261015/06/atmos/gfs.t06z.pgrb2.0p25.f120"
        );
    }

    #[test]
    fn candidates_are_synoptic_newest_first() {
        let now = Utc.with_ymd_and_hms(2026, 10, 15, 4, 59, 1).unwrap();
        let ids: Vec<String> = run_candidates(now).into_iter().map(run_id).collect();
        assert_eq!(
            ids,
            [
                "20261015_00",
                "20261014_18",
                "20261014_12",
                "20261014_06",
                "20261014_00"
            ]
        );
        let at_cycle = Utc.with_ymd_and_hms(2026, 10, 15, 18, 0, 0).unwrap();
        assert_eq!(run_id(run_candidates(at_cycle)[0]), "20261015_18");
    }

    #[test]
    fn refc_selector_matches_hrrr_codes() {
        let s = refc_selector();
        assert_eq!((s.category, s.number), (Some(16), Some(196)));
        assert_eq!(s.surface_type, Some(SURFACE_ENTIRE_ATMOSPHERE));
    }

    #[test]
    fn type_flags_pick_the_instantaneous_records_only() {
        // Lines of the real gfs.t12z.pgrb2.0p25.f019.idx of 2026-11-16.
        let idx = "\
9:3381362:d=2026111612:REFC:entire atmosphere:19 hour fcst:
10:4251121:d=2026111612:VIS:surface:19 hour fcst:
600:427609596:d=2026111612:WATR:surface:18-19 hour acc fcst:
601:427934417:d=2026111612:CSNOW:surface:19 hour fcst:
602:427973814:d=2026111612:CICEP:surface:19 hour fcst:
603:427974063:d=2026111612:CFRZR:surface:19 hour fcst:
604:427974665:d=2026111612:CRAIN:surface:19 hour fcst:
605:428079058:d=2026111612:CSNOW:surface:18-19 hour ave fcst:
606:428114079:d=2026111612:CICEP:surface:18-19 hour ave fcst:
607:428114352:d=2026111612:CFRZR:surface:18-19 hour ave fcst:
608:428115067:d=2026111612:CRAIN:surface:18-19 hour ave fcst:
609:428212576:d=2026111612:LHTFL:surface:18-19 hour ave fcst:
";
        let records = crate::idx::parse_idx(idx).unwrap();
        let flags = type_flag_terms(19);
        let flag_terms: Vec<[&str; 2]> = flags
            .iter()
            .map(|[a, b]| [a.as_str(), b.as_str()])
            .collect();
        let mut matchers: Vec<&[&str]> = vec![REFC_IDX];
        matchers.extend(flag_terms.iter().map(|t| t.as_slice()));
        let starts: Vec<u64> = crate::idx::pick_ranges(&records, &matchers)
            .iter()
            .map(|r| r.start)
            .collect();
        assert_eq!(
            starts,
            [3381362, 427934417, 427973814, 427974063, 427974665]
        );
    }
}
