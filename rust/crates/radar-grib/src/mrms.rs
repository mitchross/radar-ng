//! MRMS object handling (spec-ingest-mrms-hrrr §4.1, §5.2, §12.1).
//!
//! Key parsing, gunzip and orientation helpers. The listing and download wrappers live in
//! [`crate::mrms::list_recent_keys`] and [`decode_gz`].

use std::io::Read;

use bytes::Bytes;
use chrono::{DateTime, NaiveDate, NaiveDateTime, NaiveTime, Utc};
use radar_geo::Grid;

use crate::decode::decode_message;
use crate::error::{GribError, Result};
use crate::http::{LIST_TIMEOUT, MRMS_GET_TIMEOUT};
use crate::s3::S3Client;

/// Gunzips a complete `.grib2.gz` (multi-member gzip is accepted, as Python's gzip.open does).
pub fn gunzip(gz: &[u8]) -> Result<Vec<u8>> {
    let mut out = Vec::with_capacity(gz.len() * 4);
    flate2::read::MultiGzDecoder::new(gz).read_to_end(&mut out)?;
    Ok(out)
}

/// Gunzips and decodes the first message of an MRMS file into a grid, stored orientation.
pub fn decode_gz(gz: &[u8]) -> Result<Grid> {
    decode_message(&gunzip(gz)?)
}

/// Makes the grid south-up: flips rows when `lats[0] > lats[-1]` (Python `flip = lats[0] > lats[-1]`).
///
/// Returns true when a flip happened. Longitudes are untouched.
pub fn ensure_south_up(grid: &mut Grid) -> bool {
    let flip = match &grid.geo {
        radar_geo::GridGeo::Regular { lats, .. } => {
            lats.len() > 1 && lats[0] > lats[lats.len() - 1]
        }
        radar_geo::GridGeo::Projected { y, .. } => y.len() > 1 && y[0] > y[y.len() - 1],
    };
    if flip {
        grid.flip_rows();
    }
    flip
}

/// The basename of an object key with `.grib2.gz` removed (Python `timestamp_hint`).
pub fn timestamp_hint(key: &str) -> &str {
    let base = key.rsplit('/').next().unwrap_or(key);
    base.strip_suffix(".grib2.gz").unwrap_or(base)
}

/// Parses the valid time from an MRMS object key (Python `_extract_timestamp`).
///
/// The basename minus `.grib2.gz` is split on `_`; the last token is read as `%Y%m%d-%H%M%S` UTC.
/// The result is `datetime.isoformat()` of that instant, e.g. `2026-09-30T00:00:00+00:00`.
pub fn extract_timestamp(key: &str) -> Result<String> {
    let hint = timestamp_hint(key);
    let token = hint.rsplit('_').next().unwrap_or(hint);
    let bad = |detail: &str| GribError::KeyTimestamp {
        key: key.to_string(),
        detail: detail.to_string(),
    };
    // Fixed layout YYYYMMDD-HHMMSS (strptime's %Y%m%d-%H%M%S for every key MRMS publishes).
    let b = token.as_bytes();
    if b.len() != 15
        || b[8] != b'-'
        || !b
            .iter()
            .enumerate()
            .all(|(i, c)| i == 8 || c.is_ascii_digit())
    {
        return Err(bad("expected YYYYMMDD-HHMMSS"));
    }
    let num = |r: std::ops::Range<usize>| token[r].parse::<u32>().map_err(|_| bad("not numeric"));
    let date = NaiveDate::from_ymd_opt(num(0..4)? as i32, num(4..6)?, num(6..8)?)
        .ok_or_else(|| bad("invalid date"))?;
    let time = NaiveTime::from_hms_opt(num(9..11)?, num(11..13)?, num(13..15)?)
        .ok_or_else(|| bad("invalid time"))?;
    let dt: DateTime<Utc> = NaiveDateTime::new(date, time).and_utc();
    Ok(format_valid_time(dt))
}

/// Formats a UTC instant as Python's `datetime.isoformat()` does for an aware UTC datetime
/// with whole seconds: `YYYY-MM-DDTHH:MM:SS+00:00`.
pub fn format_valid_time(dt: DateTime<Utc>) -> String {
    dt.format("%Y-%m-%dT%H:%M:%S+00:00").to_string()
}

/// Keeps the `.grib2.gz` keys of a listing (Python `_list_keys_sync` filter).
fn filter_grib(keys: Vec<String>) -> Vec<String> {
    keys.into_iter()
        .filter(|k| k.ends_with(".grib2.gz"))
        .collect()
}

/// Lists the MRMS keys for the most recent UTC day (Python `_list_recent_files_sync`).
///
/// Lists `{prefix}/{YYYYMMDD}` for `now`'s UTC date. When that yields no `.grib2.gz` keys, yesterday's
/// prefix is listed instead. The result is sorted ascending by key (the S3 listing order).
pub async fn list_recent_keys(
    s3: &S3Client,
    prefix: &str,
    now: DateTime<Utc>,
) -> Result<Vec<String>> {
    let today = now.format("%Y%m%d").to_string();
    let mut keys = filter_grib(
        s3.list_all(&format!("{prefix}/{today}"), LIST_TIMEOUT)
            .await?,
    );
    if keys.is_empty() {
        let yesterday = (now - chrono::Duration::days(1))
            .format("%Y%m%d")
            .to_string();
        keys = filter_grib(
            s3.list_all(&format!("{prefix}/{yesterday}"), LIST_TIMEOUT)
                .await?,
        );
    }
    keys.sort();
    Ok(keys)
}

/// Downloads one `.grib2.gz` and returns its valid time with the raw bytes (Python
/// `_download_and_decode_sync`, 60 s timeout).
///
/// The valid time is parsed from the key before any bytes are fetched (PLAN defect D21), so a key
/// with a bad timestamp fails without a download.
pub async fn fetch_gz(s3: &S3Client, key: &str) -> Result<(String, Bytes)> {
    let valid_time = extract_timestamp(key)?;
    let bytes = s3.get_object(key, MRMS_GET_TIMEOUT).await?;
    Ok((valid_time, bytes))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn timestamp_from_key_matches_python() {
        let key = "CONUS/MergedBaseReflectivityQC_00.50/20260930/MRMS_MergedBaseReflectivityQC_00.50_20260930-000000.grib2.gz";
        assert_eq!(extract_timestamp(key).unwrap(), "2026-09-30T00:00:00+00:00");
        assert_eq!(
            extract_timestamp("x/MRMS_A_20261001-235958.grib2.gz").unwrap(),
            "2026-10-01T23:59:58+00:00"
        );
    }

    #[test]
    fn bad_key_is_an_error() {
        assert!(extract_timestamp("x/MRMS_A_notatime.grib2.gz").is_err());
        assert!(extract_timestamp("x/MRMS_A_20261001.grib2.gz").is_err());
    }

    #[test]
    fn timestamp_hint_strips_dir_and_suffix() {
        assert_eq!(
            timestamp_hint("a/b/MRMS_x_20260930-000000.grib2.gz"),
            "MRMS_x_20260930-000000"
        );
    }

    #[test]
    fn listing_filter_keeps_only_grib2_gz() {
        let keys = vec![
            "p/20260930/a_20260930-000000.grib2.gz".to_string(),
            "p/20260930/index.idx".to_string(),
            "p/20260930/b_20260930-000200.grib2.gz".to_string(),
        ];
        assert_eq!(filter_grib(keys).len(), 2);
    }

    #[test]
    fn gunzip_accepts_multi_member() {
        use flate2::{Compression, write::GzEncoder};
        use std::io::Write;
        let mut a = GzEncoder::new(Vec::new(), Compression::default());
        a.write_all(b"hello ").unwrap();
        let mut both = a.finish().unwrap();
        let mut b = GzEncoder::new(Vec::new(), Compression::default());
        b.write_all(b"world").unwrap();
        both.extend(b.finish().unwrap());
        assert_eq!(gunzip(&both).unwrap(), b"hello world");
    }
}
