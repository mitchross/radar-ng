//! HRRR/NAQFC `.idx` parsing and byte-range planning (spec-ingest-mrms-hrrr §4.2).
//!
//! An index is one record per line, colon-separated:
//! `num:offset:d=YYYYMMDDHH:VAR:level:fcst_time:...`. A record matches a matcher when every term of
//! the matcher is a substring of the raw line. Each matching record becomes one byte range from its
//! offset to the next record's offset minus one. The last record has an open end. Ranges are not
//! merged, as in the Python.

use crate::error::{GribError, Result};

/// One parsed index record.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IdxRecord {
    /// Message number (field 0).
    pub num: u64,
    /// Byte offset of the message (field 1).
    pub offset: u64,
    /// The raw line, as matchers see it.
    pub line: String,
    /// Variable name (field 3), e.g. `REFC`.
    pub var_name: String,
    /// Level (field 4), e.g. `entire atmosphere`.
    pub level: String,
    /// Forecast time (field 5), empty when absent.
    pub fcst_time: String,
}

/// An inclusive HTTP byte range. `end == None` means "to the end of the object".
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ByteRange {
    /// First byte.
    pub start: u64,
    /// Last byte, inclusive.
    pub end: Option<u64>,
}

impl ByteRange {
    /// The `Range` header value: `bytes=start-end` or `bytes=start-`.
    pub fn header_value(&self) -> String {
        match self.end {
            Some(end) => format!("bytes={}-{}", self.start, end),
            None => format!("bytes={}-", self.start),
        }
    }

    /// Number of bytes the range covers, when the end is known.
    pub fn len(&self) -> Option<u64> {
        self.end.map(|e| e - self.start + 1)
    }

    /// True for a range that covers no bytes (never produced by [`pick_ranges`]).
    pub fn is_empty(&self) -> bool {
        self.len() == Some(0)
    }
}

/// Parses an `.idx` document (Python `_get_idx_sync`).
///
/// Lines are split with `strip().splitlines()`. A line with fewer than five colon-separated fields
/// is skipped. A non-integer `num` or `offset` is an error, which the Python raises as ValueError.
pub fn parse_idx(text: &str) -> Result<Vec<IdxRecord>> {
    let mut out = Vec::new();
    for raw in text.trim().lines() {
        let parts: Vec<&str> = raw.split(':').collect();
        if parts.len() < 5 {
            continue;
        }
        let bad = |detail: String| GribError::IdxParse {
            line: raw.to_string(),
            detail,
        };
        let num = parts[0]
            .trim()
            .parse::<u64>()
            .map_err(|e| bad(format!("num: {e}")))?;
        let offset = parts[1]
            .trim()
            .parse::<u64>()
            .map_err(|e| bad(format!("offset: {e}")))?;
        out.push(IdxRecord {
            num,
            offset,
            line: raw.to_string(),
            var_name: parts[3].to_string(),
            level: parts[4].to_string(),
            fcst_time: parts.get(5).map(|s| s.to_string()).unwrap_or_default(),
        });
    }
    Ok(out)
}

/// Plans the byte ranges for the records matching any of `matchers`, in file order.
///
/// A matcher is a slice of terms; a record matches when every term is contained in its raw line
/// (Python `_pick_ranges`). Each range runs to the next parsed record's offset minus one; the last
/// record gets an open end.
pub fn pick_ranges(records: &[IdxRecord], matchers: &[&[&str]]) -> Vec<ByteRange> {
    let mut out = Vec::new();
    for (i, rec) in records.iter().enumerate() {
        let hit = matchers
            .iter()
            .any(|terms| terms.iter().all(|t| rec.line.contains(t)));
        if !hit {
            continue;
        }
        let end = records.get(i + 1).map(|next| next.offset - 1);
        out.push(ByteRange {
            start: rec.offset,
            end,
        });
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = "\
1:0:d=2026071512:REFC:entire atmosphere:1 hour fcst:
2:4821:d=2026071512:TMP:2 m above ground:1 hour fcst:
3:9034:d=2026071512:UGRD:10 m above ground:1 hour fcst:
4:13555:d=2026071512:VGRD:10 m above ground:1 hour fcst:
5:18201:d=2026071512:CAPE:surface:1 hour fcst:
6:22120:d=2026071512:APCP:surface:0-1 hour acc fcst:
7:30000:d=2026071512:TCDC:entire atmosphere:1 hour fcst:
";

    #[test]
    fn parses_fields_and_keeps_raw_line() {
        let recs = parse_idx(SAMPLE).unwrap();
        assert_eq!(recs.len(), 7);
        assert_eq!(recs[0].num, 1);
        assert_eq!(recs[0].offset, 0);
        assert_eq!(recs[0].var_name, "REFC");
        assert_eq!(recs[0].level, "entire atmosphere");
        assert_eq!(recs[0].fcst_time, "1 hour fcst");
        assert!(recs[0].line.starts_with("1:0:d=2026071512:REFC"));
    }

    #[test]
    fn short_lines_are_skipped_and_bad_numbers_fail() {
        let recs = parse_idx("1:0:d=x:VAR\n2:10:d=x:VAR:lvl:fc\n").unwrap();
        assert_eq!(recs.len(), 1);
        assert_eq!(recs[0].num, 2);
        assert!(matches!(
            parse_idx("x:0:d:V:l:f\n"),
            Err(GribError::IdxParse { .. })
        ));
        assert!(matches!(
            parse_idx("1:-5:d:V:l:f\n"),
            Err(GribError::IdxParse { .. })
        ));
    }

    #[test]
    fn refc_is_one_range_to_next_record() {
        let recs = parse_idx(SAMPLE).unwrap();
        let refc: &[&str] = &["REFC", "entire atmosphere"];
        let ranges = pick_ranges(&recs, &[refc]);
        assert_eq!(
            ranges,
            vec![ByteRange {
                start: 0,
                end: Some(4820)
            }]
        );
        assert_eq!(ranges[0].header_value(), "bytes=0-4820");
        assert_eq!(ranges[0].len(), Some(4821));
    }

    #[test]
    fn last_record_is_open_ended_and_ranges_are_not_merged() {
        let recs = parse_idx(SAMPLE).unwrap();
        let tcdc: &[&str] = &["TCDC", "entire atmosphere"];
        let refc: &[&str] = &["REFC", "entire atmosphere"];
        let ranges = pick_ranges(&recs, &[refc, tcdc]);
        assert_eq!(
            ranges,
            vec![
                ByteRange {
                    start: 0,
                    end: Some(4820)
                },
                ByteRange {
                    start: 30000,
                    end: None
                },
            ]
        );
        assert_eq!(ranges[1].header_value(), "bytes=30000-");
        // adjacent matches are not merged
        let ugrd: &[&str] = &["UGRD", "10 m above ground"];
        let vgrd: &[&str] = &["VGRD", "10 m above ground"];
        let wind = pick_ranges(&recs, &[ugrd, vgrd]);
        assert_eq!(wind.len(), 2);
        assert_eq!(
            wind[0],
            ByteRange {
                start: 9034,
                end: Some(13554)
            }
        );
        assert_eq!(
            wind[1],
            ByteRange {
                start: 13555,
                end: Some(18200)
            }
        );
    }

    #[test]
    fn term_must_be_substring_of_raw_line_not_just_field() {
        let recs = parse_idx(SAMPLE).unwrap();
        // "10 m" appears on the TMP line? No: TMP is 2 m. A term "surface" matches CAPE and APCP.
        let surface: &[&str] = &["surface"];
        let got = pick_ranges(&recs, &[surface]);
        assert_eq!(got.len(), 2);
        let none: &[&str] = &["TMP", "10 m above ground"];
        assert!(pick_ranges(&recs, &[none]).is_empty());
    }
}
