//! Python-compatible serialization and coercion helpers.
//!
//! Byte-exact compatibility with the Python writers needs three things serde does not give us:
//! `json.dump` separators and `ensure_ascii` escaping, float `repr` (`1e-05`, `1e+16`, `43.0`), and
//! the bare `NaN` / `Infinity` tokens. [`Json`] is a small tree that carries those rules; [`dumps`]
//! renders it. Coercions [`py_int`] and [`py_str`] mirror Python `int()` and `str()` on JSON values.

use serde_json::Value;
use std::fmt::Write as _;

/// Separator style of `json.dumps`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Style {
    /// `separators=(",", ":")`: writer styles A (sorted) and B (unsorted) in spec §0.2.
    Compact,
    /// Python defaults `(", ", ": ")`: writer styles C and D in spec §0.2.
    Default,
}

/// A JSON value with Python's number model (`int` vs `float`) and non-finite floats.
#[derive(Debug, Clone, PartialEq)]
pub enum Json {
    Null,
    Bool(bool),
    Int(i64),
    /// Integers above `i64::MAX` (kept exact).
    UInt(u64),
    Float(f64),
    Str(String),
    Array(Vec<Json>),
    /// Key/value pairs in insertion order. `dumps(.., sort_keys = true)` sorts them.
    Object(Vec<(String, Json)>),
}

impl From<&Value> for Json {
    fn from(value: &Value) -> Self {
        match value {
            Value::Null => Json::Null,
            Value::Bool(b) => Json::Bool(*b),
            Value::Number(n) => {
                if let Some(i) = n.as_i64() {
                    Json::Int(i)
                } else if let Some(u) = n.as_u64() {
                    Json::UInt(u)
                } else {
                    Json::Float(n.as_f64().unwrap_or(f64::NAN))
                }
            }
            Value::String(s) => Json::Str(s.clone()),
            Value::Array(items) => Json::Array(items.iter().map(Json::from).collect()),
            Value::Object(map) => Json::Object(
                map.iter()
                    .map(|(k, v)| (k.clone(), Json::from(v)))
                    .collect(),
            ),
        }
    }
}

/// Render `value` the way Python's `json.dumps` does (`ensure_ascii=True`, no trailing newline).
///
/// `sort_keys` sorts object keys recursively, as `json.dump(..., sort_keys=True)` does.
pub fn dumps(value: &Json, style: Style, sort_keys: bool) -> String {
    let mut out = String::new();
    write_value(&mut out, value, style, sort_keys);
    out
}

/// Convenience: [`dumps`] on a `serde_json::Value`.
pub fn dumps_value(value: &Value, style: Style, sort_keys: bool) -> String {
    dumps(&Json::from(value), style, sort_keys)
}

fn write_value(out: &mut String, value: &Json, style: Style, sort_keys: bool) {
    let (item_sep, kv_sep) = match style {
        Style::Compact => (",", ":"),
        Style::Default => (", ", ": "),
    };
    match value {
        Json::Null => out.push_str("null"),
        Json::Bool(true) => out.push_str("true"),
        Json::Bool(false) => out.push_str("false"),
        Json::Int(i) => {
            let _ = write!(out, "{i}");
        }
        Json::UInt(u) => {
            let _ = write!(out, "{u}");
        }
        Json::Float(f) => out.push_str(&float_json(*f)),
        Json::Str(s) => escape_str_into(out, s),
        Json::Array(items) => {
            out.push('[');
            for (i, item) in items.iter().enumerate() {
                if i > 0 {
                    out.push_str(item_sep);
                }
                write_value(out, item, style, sort_keys);
            }
            out.push(']');
        }
        Json::Object(pairs) => {
            out.push('{');
            let mut order: Vec<&(String, Json)> = pairs.iter().collect();
            if sort_keys {
                order.sort_by(|a, b| a.0.cmp(&b.0));
            }
            for (i, (key, item)) in order.into_iter().enumerate() {
                if i > 0 {
                    out.push_str(item_sep);
                }
                escape_str_into(out, key);
                out.push_str(kv_sep);
                write_value(out, item, style, sort_keys);
            }
            out.push('}');
        }
    }
}

/// JSON token for a float: `json.dumps` writes `NaN`, `Infinity`, `-Infinity` for non-finite values.
pub fn float_json(f: f64) -> String {
    if f.is_nan() {
        "NaN".to_string()
    } else if f == f64::INFINITY {
        "Infinity".to_string()
    } else if f == f64::NEG_INFINITY {
        "-Infinity".to_string()
    } else {
        float_repr(f)
    }
}

/// Python `repr(float)` / `str(float)`: shortest round-trip digits, `.0` for integral values,
/// scientific notation with a signed two-digit exponent when the decimal exponent is below -4 or
/// above 15 (e.g. `1e-05`, `1e+16`, `1.5e+300`).
pub fn float_repr(x: f64) -> String {
    if x.is_nan() {
        return "nan".to_string();
    }
    if x.is_infinite() {
        return if x > 0.0 {
            "inf".to_string()
        } else {
            "-inf".to_string()
        };
    }
    let sign = if x.is_sign_negative() { "-" } else { "" };
    let magnitude = x.abs();
    if magnitude == 0.0 {
        return format!("{sign}0.0");
    }
    // `{:e}` yields the shortest digits that round-trip, e.g. "4.3e1" or "1e-5". When two strings of that
    // length both round-trip, Python keeps the one nearest the exact value, ties to the even digit. Rust's
    // shortest formatter can pick the other one, so re-format at the same width with exact rounding (which
    // is half-to-even) and keep that string when it still round-trips.
    let shortest = format!("{magnitude:e}");
    let width = shortest
        .split('e')
        .next()
        .unwrap_or_default()
        .chars()
        .filter(char::is_ascii_digit)
        .count();
    let exact = format!("{magnitude:.*e}", width.max(1) - 1);
    let sci = if exact.parse::<f64>().ok() == Some(magnitude) {
        exact
    } else {
        shortest
    };
    let (mantissa, exponent) = sci.split_once('e').expect("LowerExp always contains 'e'");
    let exponent: i32 = exponent.parse().expect("LowerExp exponent is an integer");
    let digits: String = mantissa.chars().filter(|c| *c != '.').collect();
    // Python's decpt: the decimal point sits after `decpt` digits.
    let decpt = exponent + 1;
    let body = if decpt <= -4 || decpt > 16 {
        let mut s = String::new();
        s.push_str(&digits[..1]);
        if digits.len() > 1 {
            s.push('.');
            s.push_str(&digits[1..]);
        }
        let e = decpt - 1;
        let _ = write!(s, "e{}{:02}", if e < 0 { '-' } else { '+' }, e.abs());
        s
    } else if decpt <= 0 {
        format!("0.{}{}", "0".repeat((-decpt) as usize), digits)
    } else {
        let decpt = decpt as usize;
        if decpt >= digits.len() {
            format!("{}{}.0", digits, "0".repeat(decpt - digits.len()))
        } else {
            format!("{}.{}", &digits[..decpt], &digits[decpt..])
        }
    };
    format!("{sign}{body}")
}

/// `json.dumps` string escaping with `ensure_ascii=True`: `\"`, `\\`, `\n`, `\r`, `\t`, `\b`, `\f`,
/// and `\uXXXX` (lowercase hex, UTF-16 surrogate pairs above U+FFFF) for everything outside 0x20..=0x7E.
pub fn escape_str_into(out: &mut String, s: &str) {
    out.push('"');
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            '\u{08}' => out.push_str("\\b"),
            '\u{0c}' => out.push_str("\\f"),
            ' '..='~' => out.push(c),
            _ => {
                let mut units = [0u16; 2];
                for unit in c.encode_utf16(&mut units) {
                    let _ = write!(out, "\\u{:04x}", unit);
                }
            }
        }
    }
    out.push('"');
}

/// Python `int(value)` on a JSON value. `None` where Python would raise (`TypeError`/`ValueError`).
/// Booleans count as 0/1, floats truncate toward zero, strings parse as base-10 integers.
pub fn py_int(value: &Value) -> Option<i64> {
    match value {
        Value::Bool(b) => Some(i64::from(*b)),
        Value::Number(n) => {
            if let Some(i) = n.as_i64() {
                Some(i)
            } else if let Some(u) = n.as_u64() {
                i64::try_from(u).ok()
            } else {
                n.as_f64()
                    .filter(|f| f.is_finite())
                    .map(|f| f.trunc() as i64)
            }
        }
        Value::String(s) => s.trim().parse::<i64>().ok(),
        _ => None,
    }
}

/// Python `int(value or 0)`: falsy values (null, false, 0, "", 0.0) become 0, others go through [`py_int`].
pub fn py_int_or_zero(value: Option<&Value>) -> Option<i64> {
    match value {
        None => Some(0),
        Some(v) if is_falsy(v) => Some(0),
        Some(v) => py_int(v),
    }
}

/// Python truthiness of a JSON value.
pub fn is_falsy(value: &Value) -> bool {
    match value {
        Value::Null => true,
        Value::Bool(b) => !b,
        Value::Number(n) => n.as_f64().is_some_and(|f| f == 0.0),
        Value::String(s) => s.is_empty(),
        Value::Array(a) => a.is_empty(),
        Value::Object(o) => o.is_empty(),
    }
}

/// Python `str(value)` on a JSON value (strings unchanged, `None`/`True`/`False`, `repr`-style numbers).
pub fn py_str(value: &Value) -> String {
    match value {
        Value::Null => "None".to_string(),
        Value::Bool(true) => "True".to_string(),
        Value::Bool(false) => "False".to_string(),
        Value::Number(n) => match n.as_i64() {
            Some(i) => i.to_string(),
            None => match n.as_u64() {
                Some(u) => u.to_string(),
                None => float_repr(n.as_f64().unwrap_or(f64::NAN)),
            },
        },
        Value::String(s) => s.clone(),
        other => other.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn float_repr_matches_python_repr_rules() {
        assert_eq!(float_repr(43.0), "43.0");
        assert_eq!(float_repr(-9999.0), "-9999.0");
        assert_eq!(float_repr(0.0), "0.0");
        assert_eq!(float_repr(-0.0), "-0.0");
        assert_eq!(float_repr(0.833), "0.833");
        assert_eq!(float_repr(1.0), "1.0");
        assert_eq!(float_repr(0.0001), "0.0001");
        assert_eq!(float_repr(0.00001), "1e-05");
        assert_eq!(float_repr(1e15), "1000000000000000.0");
        assert_eq!(float_repr(1e16), "1e+16");
        assert_eq!(float_repr(1.5e300), "1.5e+300");
        assert_eq!(float_repr(5e-324), "5e-324");
        assert_eq!(float_repr(123.456), "123.456");
        assert_eq!(float_repr(0.1 + 0.2), "0.30000000000000004");
        assert_eq!(float_repr(21.10000000000001), "21.10000000000001");
        assert_eq!(float_repr(f64::NAN), "nan");
        assert_eq!(float_repr(f64::NEG_INFINITY), "-inf");
    }

    #[test]
    fn json_nonfinite_and_escaping() {
        assert_eq!(float_json(f64::NAN), "NaN");
        assert_eq!(float_json(f64::INFINITY), "Infinity");
        assert_eq!(float_json(f64::NEG_INFINITY), "-Infinity");
        assert_eq!(
            dumps(&Json::Str("°F".into()), Style::Compact, false),
            "\"\\u00b0F\""
        );
        assert_eq!(
            dumps(&Json::Str("a\u{7f}\n\"\\".into()), Style::Compact, false),
            "\"a\\u007f\\n\\\"\\\\\""
        );
        assert_eq!(
            dumps(&Json::Str("😀".into()), Style::Compact, false),
            "\"\\ud83d\\ude00\""
        );
    }

    #[test]
    fn separators_and_sorting() {
        let v = json!({"b": [1, 2.0], "a": {"y": true, "x": null}});
        assert_eq!(
            dumps_value(&v, Style::Compact, true),
            r#"{"a":{"x":null,"y":true},"b":[1,2.0]}"#
        );
        assert_eq!(
            dumps_value(&v, Style::Default, true),
            r#"{"a": {"x": null, "y": true}, "b": [1, 2.0]}"#
        );
        assert_eq!(
            dumps_value(&v, Style::Default, false),
            r#"{"b": [1, 2.0], "a": {"y": true, "x": null}}"#
        );
    }

    #[test]
    fn py_int_and_str_coercions() {
        assert_eq!(py_int(&json!(30.9)), Some(30));
        assert_eq!(py_int(&json!("15")), Some(15));
        assert_eq!(py_int(&json!(true)), Some(1));
        assert_eq!(py_int(&json!(null)), None);
        assert_eq!(py_int_or_zero(Some(&json!(null))), Some(0));
        assert_eq!(py_int_or_zero(Some(&json!("x"))), None);
        assert_eq!(py_str(&json!(null)), "None");
        assert_eq!(py_str(&json!(2.0)), "2.0");
    }
}

#[cfg(test)]
mod golden {
    use super::*;

    /// Every float in `testdata/pycompat/floats.json` (bit pattern in hex) must give Python's `repr`.
    #[test]
    fn golden_floats_match_python_repr() {
        let floats: Vec<serde_json::Value> =
            serde_json::from_str(include_str!("../testdata/pycompat/floats.json")).unwrap();
        for case in floats {
            let bits = u64::from_str_radix(case["bits"].as_str().unwrap(), 16).unwrap();
            let value = f64::from_bits(bits);
            assert_eq!(
                float_repr(value),
                case["repr"].as_str().unwrap(),
                "bits {bits:016x}"
            );
        }
    }

    /// Every string in `testdata/pycompat/strings.json` must escape exactly as `json.dumps` does.
    #[test]
    fn golden_strings_match_python_json_dumps() {
        let strings: Vec<serde_json::Value> =
            serde_json::from_str(include_str!("../testdata/pycompat/strings.json")).unwrap();
        for case in strings {
            let mut out = String::new();
            escape_str_into(&mut out, case["value"].as_str().unwrap());
            assert_eq!(
                out,
                case["json"].as_str().unwrap(),
                "value {:?}",
                case["value"]
            );
        }
    }
}
