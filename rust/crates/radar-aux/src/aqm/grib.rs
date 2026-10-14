//! Message-level access to a multi-message GRIB2 file.
//!
//! `radar-grib` decodes one message at a time and has no per-index selection (the README there
//! lists NAQFC chunking as deferred). This module splits a file into its messages by the section-0
//! length, and reads the one field the port needs beyond the decoder: the message `validDate`.
//!
//! `validDate` follows eccodes and pygrib. For instantaneous fields (template 4.0) it is the reference
//! time plus the forecast time. For statistical fields (template 4.8) it is the reference time plus
//! the end of the first time range, that is forecast time plus range length. The generated fixtures
//! (`testdata/gen_aqm.py`) pin both rules against eccodes.

use chrono::{DateTime, Duration, NaiveDate, TimeZone, Utc};
use std::ops::Range;

use super::AqmError;

/// Splits `bytes` into GRIB2 messages, returning each message's byte range (section 0 through `7777`).
///
/// Bytes between messages are skipped, as eccodes does. A message whose length runs past the end of
/// the file, or whose end marker is not `7777`, is an error (a truncated download).
pub fn split_messages(bytes: &[u8]) -> Result<Vec<Range<usize>>, AqmError> {
    let mut out = Vec::new();
    let mut pos = 0usize;
    while let Some(rel) = find(&bytes[pos..], b"GRIB") {
        let start = pos + rel;
        if bytes.len() < start + 16 {
            return Err(message_error(out.len(), "truncated section 0"));
        }
        let edition = bytes[start + 7];
        if edition != 2 {
            return Err(message_error(
                out.len(),
                &format!("GRIB edition {edition} is not 2"),
            ));
        }
        let total =
            u64::from_be_bytes(bytes[start + 8..start + 16].try_into().expect("8 bytes")) as usize;
        let end = start
            .checked_add(total)
            .filter(|&e| e <= bytes.len())
            .ok_or_else(|| {
                message_error(
                    out.len(),
                    "message length runs past the end of the file (truncated download?)",
                )
            })?;
        if total < 20 || &bytes[end - 4..end] != b"7777" {
            return Err(message_error(out.len(), "missing 7777 end marker"));
        }
        out.push(start..end);
        pos = end;
    }
    Ok(out)
}

fn message_error(index: usize, detail: &str) -> AqmError {
    AqmError::Message {
        index,
        detail: detail.to_string(),
    }
}

fn find(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    haystack.windows(needle.len()).position(|w| w == needle)
}

/// Returns the `validDate` of one message (UTC, whole seconds).
///
/// `msg` is one message, as produced by [`split_messages`].
pub fn valid_time(msg: &[u8]) -> Result<DateTime<Utc>, AqmError> {
    let sections = sections(msg)?;
    let sec1 =
        section(&sections, 1).ok_or_else(|| message_error(0, "no identification section"))?;
    let sec4 =
        section(&sections, 4).ok_or_else(|| message_error(0, "no product definition section"))?;
    if sec1.len() < 19 {
        return Err(message_error(0, "identification section too short"));
    }
    let year = u16::from_be_bytes([sec1[12], sec1[13]]) as i32;
    let reference = Utc.from_utc_datetime(
        &NaiveDate::from_ymd_opt(year, u32::from(sec1[14]), u32::from(sec1[15]))
            .and_then(|d| {
                d.and_hms_opt(
                    u32::from(sec1[16]),
                    u32::from(sec1[17]),
                    u32::from(sec1[18]),
                )
            })
            .ok_or_else(|| message_error(0, "invalid reference time"))?,
    );
    if sec4.len() < 22 {
        return Err(message_error(0, "product definition section too short"));
    }
    let template = u16::from_be_bytes([sec4[7], sec4[8]]);
    let ft_unit = unit_seconds(sec4[17])?;
    let forecast_time = u32::from_be_bytes([sec4[18], sec4[19], sec4[20], sec4[21]]) as i64;
    let offset_secs = match template {
        0 => forecast_time * ft_unit,
        8 => {
            // Template 4.8 (WMO FM 92): octet 42 is the number of time ranges, octet 43-46 the missing
            // count, octet 49 the unit of the first time range, octets 50-53 its length. Octet n is index n-1.
            if sec4.len() < 53 {
                return Err(message_error(0, "statistical product definition too short"));
            }
            if sec4[41] != 1 {
                return Err(AqmError::Unsupported(format!(
                    "statistical product with {} time ranges (only one is supported)",
                    sec4[41]
                )));
            }
            let range_unit = unit_seconds(sec4[48])?;
            let range_len = u32::from_be_bytes([sec4[49], sec4[50], sec4[51], sec4[52]]) as i64;
            forecast_time * ft_unit + range_len * range_unit
        }
        other => {
            return Err(AqmError::Unsupported(format!(
                "product definition template 4.{other} (valid time rule not implemented)"
            )));
        }
    };
    Ok(reference + Duration::seconds(offset_secs))
}

/// Seconds per unit of time for the GRIB2 time-unit codes that NAQFC files can use (Code table 4.4).
fn unit_seconds(code: u8) -> Result<i64, AqmError> {
    Ok(match code {
        0 => 60,
        1 => 3600,
        2 => 86_400,
        10 => 3 * 3600,
        11 => 6 * 3600,
        12 => 12 * 3600,
        13 => 1,
        other => return Err(AqmError::Unsupported(format!("time unit code {other}"))),
    })
}

/// Section bodies (the bytes from the section length field to the end of the section), in file order.
fn sections(msg: &[u8]) -> Result<Vec<(u8, &[u8])>, AqmError> {
    let end = msg
        .len()
        .checked_sub(4)
        .ok_or_else(|| message_error(0, "message too short"))?;
    let mut out = Vec::new();
    let mut pos = 16usize;
    while pos + 5 <= end {
        let len = u32::from_be_bytes(msg[pos..pos + 4].try_into().expect("4 bytes")) as usize;
        if len < 5 || pos + len > end {
            return Err(message_error(0, "section length is inconsistent"));
        }
        out.push((msg[pos + 4], &msg[pos..pos + len]));
        pos += len;
    }
    if pos != end {
        return Err(message_error(0, "trailing bytes before the end marker"));
    }
    Ok(out)
}

fn section<'a>(sections: &[(u8, &'a [u8])], number: u8) -> Option<&'a [u8]> {
    sections.iter().find(|(n, _)| *n == number).map(|(_, s)| *s)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unit_codes_match_code_table_4_4() {
        assert_eq!(unit_seconds(1).unwrap(), 3600);
        assert_eq!(unit_seconds(0).unwrap(), 60);
        assert!(unit_seconds(99).is_err());
    }

    #[test]
    fn split_rejects_truncated_and_bad_edition() {
        // Section 0 claiming 100 bytes in a 20-byte buffer.
        let mut bytes = Vec::new();
        bytes.extend_from_slice(b"GRIB\0\0\0\x02");
        bytes.extend_from_slice(&100u64.to_be_bytes());
        bytes.extend_from_slice(b"1234");
        assert!(split_messages(&bytes).is_err());
        bytes[7] = 1;
        assert!(split_messages(&bytes).is_err());
    }

    #[test]
    fn split_of_no_messages_is_empty() {
        assert!(split_messages(b"not grib at all").unwrap().is_empty());
    }
}
