//! Timestamp parsing with the UTC policy from spec-shared 0.1 and D6/D16.
//!
//! Python's `fromisoformat` accepts many forms and raises on mixed naive and
//! aware values. Here every value is normalized to a UTC instant at microsecond
//! precision, and a timestamp without an offset is read as UTC.

use chrono::DateTime;

/// A UTC instant as microseconds since the Unix epoch (Python `datetime` precision).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct Instant(pub i64);

impl Instant {
    /// Epoch seconds as Python's `datetime.timestamp()` returns them.
    #[must_use]
    pub fn epoch_seconds(self) -> f64 {
        self.0 as f64 / 1_000_000.0
    }

    /// Minutes from `other` to `self` (Python `(self - other).total_seconds() / 60`).
    #[must_use]
    pub fn minutes_since(self, other: Instant) -> f64 {
        ((self.0 - other.0) as f64 / 1_000_000.0) / 60.0
    }

    /// Shift by a number of minutes (Python `dt + timedelta(minutes=m)`).
    #[must_use]
    pub fn plus_minutes(self, minutes: i64) -> Instant {
        Instant(self.0 + minutes * 60 * 1_000_000)
    }
}

/// Parse an ISO-8601 / RFC 3339 timestamp into a UTC instant.
///
/// Accepts `Z` or an explicit offset, a `T` or space separator, and fractional
/// seconds. A value with no offset is read as UTC. Returns `None` for anything
/// that does not parse.
#[must_use]
pub fn parse_timestamp(text: &str) -> Option<Instant> {
    let normalized = normalize(text)?;
    DateTime::parse_from_rfc3339(&normalized)
        .ok()
        .map(|dt| Instant(dt.timestamp_micros()))
}

fn normalize(text: &str) -> Option<String> {
    let replaced = text.replace('Z', "+00:00");
    let sep = replaced.find(['T', ' ']);
    let Some(sep) = sep else {
        // Date only: Python accepts it as midnight; treat it as UTC midnight.
        return Some(format!("{replaced}T00:00:00+00:00"));
    };
    let (date, rest) = replaced.split_at(sep);
    let time_part = &rest[1..];
    let has_offset = time_part.contains('+') || time_part.contains('-');
    let mut out = format!("{date}T{time_part}");
    if !has_offset {
        out.push_str("+00:00");
    }
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_writer_forms_to_the_same_instant() {
        let a = parse_timestamp("2026-07-10T12:00:00+00:00").unwrap();
        let b = parse_timestamp("2026-07-10T12:00:00Z").unwrap();
        let c = parse_timestamp("2026-07-10T12:00:00").unwrap();
        let d = parse_timestamp("2026-07-10 12:00:00+00:00").unwrap();
        assert_eq!(a, b);
        assert_eq!(a, c);
        assert_eq!(a, d);
        assert_eq!(a.epoch_seconds(), 1_783_684_800.0);
    }

    #[test]
    fn offsets_are_converted_to_utc() {
        let east = parse_timestamp("2026-07-10T14:00:00+02:00").unwrap();
        let utc = parse_timestamp("2026-07-10T12:00:00+00:00").unwrap();
        assert_eq!(east, utc);
    }

    #[test]
    fn minutes_and_plus_minutes_round_trip() {
        let t0 = parse_timestamp("2026-07-10T12:00:00+00:00").unwrap();
        let t5 = t0.plus_minutes(5);
        assert_eq!(t5.minutes_since(t0), 5.0);
        assert_eq!(parse_timestamp("2026-07-10T12:05:00+00:00").unwrap(), t5);
    }

    #[test]
    fn rejects_garbage() {
        assert_eq!(parse_timestamp("not a time"), None);
        assert_eq!(parse_timestamp(""), None);
    }
}
