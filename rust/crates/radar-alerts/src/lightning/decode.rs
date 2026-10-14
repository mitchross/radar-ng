//! Frame decoding and strike extraction (spec-ingest-aux sections 6.5, 6.6 and 6.7).

use radar_storms::numeric::{py_float, py_int};
use serde_json::Value;

use super::config::BoundingBox;
use super::store::Strike;

/// Largest decoded output the LZW decoder will produce, in UTF-8 bytes. Python has no cap. A pathological frame
/// can grow the dictionary quadratically, so the port gives up and returns the raw frame (as Python does on an
/// exception). Real frames decode to a few hundred bytes.
pub const MAX_DECODED_BYTES: usize = 4 << 20;

/// Total size of the decoder's dictionary, in UTF-8 bytes. Same purpose as [`MAX_DECODED_BYTES`].
pub const MAX_DICTIONARY_BYTES: usize = 8 << 20;

/// The Blitzortung LZW-variant decode (Python `_decode_payload`, spec 6.6), on Unicode code points.
///
/// Code points below 256 are literals. Code points from 256 up are dictionary references, and a reference to an
/// entry not yet defined is the KwKwK case (`curr + curr[0]`). Input that is not valid for the variant, or that
/// would exceed the caps above, returns `raw` unchanged. An empty string returns itself.
#[must_use]
pub fn decode_payload(raw: &str) -> String {
    if raw.is_empty() {
        return raw.to_owned();
    }
    lzw_decode(raw).unwrap_or_else(|| raw.to_owned())
}

fn lzw_decode(raw: &str) -> Option<String> {
    let chars: Vec<char> = raw.chars().collect();
    let mut curr = chars[0].to_string();
    let mut result = curr.clone();
    // Entry `i` is the dictionary value for code `256 + i`. Python inserts exactly one entry per loop iteration,
    // so the codes are contiguous and the index is `code - 256`.
    let mut dictionary: Vec<String> = Vec::new();
    let mut dictionary_bytes = 0usize;
    for &ch in &chars[1..] {
        let code = ch as u32;
        let entry: String = if code < 256 {
            ch.to_string()
        } else if let Some(known) = (code as usize)
            .checked_sub(256)
            .and_then(|i| dictionary.get(i))
        {
            known.clone()
        } else {
            let mut kwkwk = curr.clone();
            kwkwk.push(first_char(&curr));
            kwkwk
        };
        result.push_str(&entry);
        let mut new_entry = curr.clone();
        new_entry.push(first_char(&entry));
        dictionary_bytes += new_entry.len();
        if result.len() > MAX_DECODED_BYTES || dictionary_bytes > MAX_DICTIONARY_BYTES {
            return None;
        }
        dictionary.push(new_entry);
        curr = entry;
    }
    Some(result)
}

fn first_char(s: &str) -> char {
    // Dictionary entries and emitted symbols are never empty (each starts with a code point), so this is safe.
    s.chars().next().unwrap_or('\0')
}

/// Parse one websocket text frame the way the Python loop does (spec 6.5, step 3).
///
/// The raw text is tried first. Only when it is not JSON is the decoded form tried. A frame that parses to JSON
/// `null` is returned as `Some(Value::Null)`. The caller skips it without counting it as parsed, as Python does.
/// Returns `None` when neither form parses.
#[must_use]
pub fn decode_frame(text: &str) -> Option<Value> {
    if let Ok(value) = serde_json::from_str::<Value>(text) {
        return Some(value);
    }
    serde_json::from_str::<Value>(&decode_payload(text)).ok()
}

/// Why a parsed frame could not become a strike. The caller counts these and skips the frame (spec D-4).
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum FrameError {
    /// The JSON value is not an object (for example an array or a number).
    #[error("frame is not a JSON object")]
    NotAnObject,
    /// A field that Python would coerce with `float()` or `int()` cannot be coerced.
    #[error("field {0} has a value that cannot be converted to a number")]
    BadField(&'static str),
}

/// Turn a parsed frame into a strike, or `None` when the frame is outside the box or has no timestamp (spec 6.7).
///
/// Field defaults match Python: `lat` and `lon` default to 0, `time` to 0 (which is skipped), `pol` and `mds` to 0.
/// `lat`, `lon` and `time` are coerced before the box check. `pol` and `mds` are coerced only for in-box strikes.
pub fn extract_strike(value: &Value, bbox: &BoundingBox) -> Result<Option<Strike>, FrameError> {
    let object = value.as_object().ok_or(FrameError::NotAnObject)?;
    let lat = float_field(object.get("lat"), 0.0, "lat")?;
    let lon = float_field(object.get("lon"), 0.0, "lon")?;
    let t_ns = int_field(object.get("time"), 0, "time")?;
    if !bbox.contains(lat, lon) || t_ns == 0 {
        return Ok(None);
    }
    let pol = int_field(object.get("pol"), 0, "pol")?;
    let mds = int_field(object.get("mds"), 0, "mds")?;
    Ok(Some(Strike {
        t: t_ns as f64 / 1e9,
        lat,
        lon,
        pol,
        mds,
    }))
}

fn float_field(value: Option<&Value>, default: f64, name: &'static str) -> Result<f64, FrameError> {
    match value {
        None => Ok(default),
        Some(v) => py_float(v).ok_or(FrameError::BadField(name)),
    }
}

fn int_field(value: Option<&Value>, default: i64, name: &'static str) -> Result<i64, FrameError> {
    match value {
        None => Ok(default),
        Some(v) => py_int(v).ok_or(FrameError::BadField(name)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn spec_vectors_6_6() {
        assert_eq!(decode_payload(""), "");
        assert_eq!(decode_payload("abc"), "abc");
        assert_eq!(decode_payload("a\u{100}"), "aaa");
        assert_eq!(decode_payload("ab\u{100}"), "abab");
        assert_eq!(decode_payload("ab\u{100}\u{101}"), "ababba");
        assert_eq!(decode_payload(r#"{"a":1}"#), r#"{"a":1}"#);
    }

    #[test]
    fn plain_json_is_not_changed_by_decode() {
        let text = r#"{"lat":36.1,"lon":-97.2,"time":1700000000000000000}"#;
        assert_eq!(decode_payload(text), text);
    }

    #[test]
    fn decode_frame_prefers_raw_json_and_falls_back_to_decoded() {
        let raw = decode_frame(r#"{"lat":1}"#).unwrap();
        assert_eq!(raw, json!({"lat": 1}));
        assert_eq!(decode_frame("null"), Some(Value::Null));
        assert!(decode_frame("not json at all").is_none());
        // Not JSON as sent ("\u{100}" is not a digit), but the decoded text "111" is a JSON number.
        assert_eq!(decode_frame("1\u{100}"), Some(json!(111)));
    }

    #[test]
    fn oversized_expansion_returns_raw_input_quickly() {
        // Each unknown code reuses the growing previous symbol, so the output grows quadratically in the input.
        let mut raw = String::from("a");
        for _ in 0..20_000 {
            raw.push('\u{1000}');
        }
        let started = std::time::Instant::now();
        assert_eq!(
            decode_payload(&raw),
            raw,
            "cap trips and the raw frame comes back"
        );
        assert!(started.elapsed() < std::time::Duration::from_secs(5));
    }

    #[test]
    fn extract_applies_bbox_and_timestamp_rules() {
        let bbox = BoundingBox::default();
        let ok = json!({"lat": 36.0, "lon": -97.0, "time": 1_700_000_000_123_456_789i64, "pol": -1, "mds": 7});
        let s = extract_strike(&ok, &bbox).unwrap().unwrap();
        assert_eq!((s.pol, s.mds), (-1, 7));
        assert!((s.t - 1_700_000_000.123_456_7).abs() < 1e-6);

        let outside = json!({"lat": 60.0, "lon": -97.0, "time": 5});
        assert_eq!(extract_strike(&outside, &bbox).unwrap(), None);
        let no_time = json!({"lat": 36.0, "lon": -97.0});
        assert_eq!(extract_strike(&no_time, &bbox).unwrap(), None);
    }

    #[test]
    fn extract_coerces_like_python_and_reports_bad_fields() {
        let bbox = BoundingBox::default();
        let strings =
            json!({"lat": "36.5", "lon": "-97", "time": "1700000000000000000", "pol": "1"});
        let s = extract_strike(&strings, &bbox).unwrap().unwrap();
        assert_eq!((s.lat, s.lon, s.pol), (36.5, -97.0, 1));
        let float_time = json!({"lat": 36.0, "lon": -97.0, "time": 1.7e18});
        assert!(extract_strike(&float_time, &bbox).unwrap().is_some());

        let null_lat = json!({"lat": null, "lon": -97.0, "time": 5});
        assert_eq!(
            extract_strike(&null_lat, &bbox),
            Err(FrameError::BadField("lat"))
        );
        let bad_pol = json!({"lat": 36.0, "lon": -97.0, "time": 5, "pol": "x"});
        assert_eq!(
            extract_strike(&bad_pol, &bbox),
            Err(FrameError::BadField("pol"))
        );
        // pol is not read for an out-of-box strike, so a bad pol there is not an error.
        let bad_pol_outside = json!({"lat": 80.0, "lon": -97.0, "time": 5, "pol": "x"});
        assert_eq!(extract_strike(&bad_pol_outside, &bbox).unwrap(), None);
        assert_eq!(
            extract_strike(&json!([1, 2]), &bbox),
            Err(FrameError::NotAnObject)
        );
    }
}
