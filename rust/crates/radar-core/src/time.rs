//! Timestamp helpers: Python-compatible `isoformat()`, lenient ISO parsing, and Python rounding.
//!
//! Spec references: §0.1 (timestamp policy), §15.2 (rounding and formatting).

use chrono::{DateTime, FixedOffset, NaiveDateTime, Utc};
use thiserror::Error;

/// Errors from [`canonical_utc_timestamp`].
#[derive(Debug, Error, PartialEq, Eq)]
pub enum TimeError {
    /// The string is not an ISO-8601 / RFC 3339 timestamp.
    #[error("not an ISO-8601 timestamp")]
    Unparseable,
    /// The timestamp has no UTC offset (Python naive datetime).
    #[error("timestamp has no UTC offset")]
    Naive,
    /// The timestamp has a non-zero UTC offset.
    #[error("timestamp offset is not UTC")]
    NotUtc,
}

/// Python `datetime.isoformat()` of an aware UTC datetime: `YYYY-MM-DDTHH:MM:SS[.ffffff]+00:00`.
/// Microseconds appear only when non-zero.
pub fn isoformat_utc(dt: &DateTime<Utc>) -> String {
    let base = dt.format("%Y-%m-%dT%H:%M:%S").to_string();
    let micros = dt.timestamp_subsec_micros();
    if micros == 0 {
        format!("{base}+00:00")
    } else {
        format!("{base}.{micros:06}+00:00")
    }
}

/// `datetime.now(timezone.utc).isoformat()`.
pub fn now_isoformat() -> String {
    isoformat_utc(&Utc::now())
}

/// Python `datetime.fromisoformat(value.replace("Z", "+00:00"))` for the forms the writers emit
/// (RFC 3339 with an offset, optional fraction), plus naive `YYYY-MM-DDTHH:MM[:SS[.f]]`.
/// Returns the parsed value with its offset, or `None` (Python `ValueError`).
pub fn parse_iso(value: &str) -> Option<DateTime<FixedOffset>> {
    let normalized = value.replace('Z', "+00:00");
    DateTime::parse_from_rfc3339(&normalized).ok()
}

/// Like [`parse_iso`] but a naive timestamp is taken as UTC (`_parse_ts` in `nowcast_skill.py`, spec §0.1).
pub fn parse_iso_as_utc(value: &str) -> Option<DateTime<Utc>> {
    if let Some(dt) = parse_iso(value) {
        return Some(dt.with_timezone(&Utc));
    }
    let normalized = value.replace('Z', "+00:00");
    for fmt in [
        "%Y-%m-%dT%H:%M:%S%.f",
        "%Y-%m-%d %H:%M:%S%.f",
        "%Y-%m-%dT%H:%M",
    ] {
        if let Ok(naive) = NaiveDateTime::parse_from_str(&normalized, fmt) {
            return Some(naive.and_utc());
        }
    }
    None
}

/// Canonical manifest/frame timestamp: parse, require offset zero, and re-render with [`isoformat_utc`].
/// `...Z` becomes `...+00:00`. Naive or non-UTC inputs are rejected (spec D19: writers must emit UTC).
pub fn canonical_utc_timestamp(value: &str) -> Result<String, TimeError> {
    if let Some(parsed) = parse_iso(value) {
        if parsed.offset().local_minus_utc() != 0 {
            return Err(TimeError::NotUtc);
        }
        return Ok(isoformat_utc(&parsed.with_timezone(&Utc)));
    }
    let normalized = value.replace('Z', "+00:00");
    if NaiveDateTime::parse_from_str(&normalized, "%Y-%m-%dT%H:%M:%S%.f").is_ok() {
        return Err(TimeError::Naive);
    }
    Err(TimeError::Unparseable)
}

/// Python `round(x)` (no digits): round half to even.
pub fn round_half_even(x: f64) -> f64 {
    x.round_ties_even()
}

/// Python `round(x, digits)`: round the exact binary value half to even. Matches Rust's `{:.n}` formatting,
/// which rounds the exact decimal expansion, so `round_digits(0.125, 2) == 0.12`.
pub fn round_digits(x: f64, digits: usize) -> f64 {
    format!("{x:.digits$}").parse::<f64>().unwrap_or(x)
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    #[test]
    fn isoformat_drops_zero_microseconds() {
        let dt = Utc.with_ymd_and_hms(2026, 5, 20, 12, 0, 0).unwrap();
        assert_eq!(isoformat_utc(&dt), "2026-05-20T12:00:00+00:00");
        let dt = dt + chrono::Duration::microseconds(418221);
        assert_eq!(isoformat_utc(&dt), "2026-05-20T12:00:00.418221+00:00");
    }

    #[test]
    fn parse_and_canonicalize() {
        assert_eq!(
            canonical_utc_timestamp("2026-05-20T12:00:00Z").unwrap(),
            "2026-05-20T12:00:00+00:00"
        );
        assert_eq!(
            canonical_utc_timestamp("2026-05-20T12:00:00.5+00:00").unwrap(),
            "2026-05-20T12:00:00.500000+00:00"
        );
        assert_eq!(
            canonical_utc_timestamp("2026-05-20T07:00:00-05:00"),
            Err(TimeError::NotUtc)
        );
        assert_eq!(
            canonical_utc_timestamp("2026-05-20T12:00:00"),
            Err(TimeError::Naive)
        );
        assert_eq!(canonical_utc_timestamp("nope"), Err(TimeError::Unparseable));
    }

    #[test]
    fn naive_is_utc_for_skill_policy() {
        let dt = parse_iso_as_utc("2026-09-30T12:00:00").unwrap();
        assert_eq!(isoformat_utc(&dt), "2026-09-30T12:00:00+00:00");
        assert!(parse_iso_as_utc("not a time").is_none());
    }

    #[test]
    fn python_rounding() {
        assert_eq!(round_half_even(0.5), 0.0);
        assert_eq!(round_half_even(1.5), 2.0);
        assert_eq!(round_half_even(2.5), 2.0);
        assert_eq!(round_digits(0.125, 2), 0.12);
        assert_eq!(round_digits(0.375, 2), 0.38);
        assert_eq!(round_digits(80.0 / 96.0, 3), 0.833);
        assert_eq!(round_digits(24.0 / 104.0, 3), 0.231);
        assert_eq!(format!("{:.0}", 0.5), "0");
        assert_eq!(format!("{:.0}", 1.5), "2");
        assert_eq!(format!("{:.0}", 2.5), "2");
    }
}
