//! Path, query and body parameter parsing with FastAPI's validation errors.
//!
//! Floats follow Python `float()` for the inputs that matter: `nan`, `inf`, `infinity` and a
//! leading sign are accepted, then range checks reject NaN and Inf (spec 1.1). Underscores and
//! surrounding whitespace are rejected (no client sends them). Integers follow Pydantic's lax
//! `int` for strings: an optional sign and decimal digits; `6.5` is a 422.
//!
//! FastAPI collects every parameter issue before answering, so [`Issues`] does the same and the
//! 422 body lists all of them.

use axum::response::Response;
use serde_json::{Value, json};

use crate::respond;

/// Python `float(text)` restricted to the forms clients send. `None` where Python would raise.
#[must_use]
pub fn py_float(text: &str) -> Option<f64> {
    if text.is_empty() || text.chars().any(|c| c.is_whitespace() || c == '_') {
        return None;
    }
    text.parse::<f64>().ok()
}

/// Pydantic lax `int` from a string: optional sign, decimal digits, surrounding whitespace allowed.
#[must_use]
pub fn py_int_str(text: &str) -> Option<i64> {
    let t = text.trim();
    let digits = t.strip_prefix(['+', '-']).unwrap_or(t);
    if digits.is_empty() || !digits.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    t.parse::<i64>().ok()
}

/// Accumulates FastAPI-style validation issues across all parameters of one request.
#[derive(Debug, Default)]
pub struct Issues(Vec<Value>);

impl Issues {
    /// An empty collector.
    #[must_use]
    pub fn new() -> Self {
        Self(Vec::new())
    }

    /// `Ok(())` when nothing failed, otherwise the 422 response listing every issue.
    ///
    /// # Errors
    /// Returns the 422 [`Response`] when at least one parameter failed.
    #[allow(
        clippy::result_large_err,
        reason = "the Err is the ready-made HTTP response; boxing it would add a deref at about 50 call sites for no gain on this cold path"
    )]
    pub fn finish(self) -> Result<(), Response> {
        if self.0.is_empty() {
            Ok(())
        } else {
            Err(respond::validation(self.0))
        }
    }

    /// Record a missing required parameter.
    fn missing(&mut self, src: &str, name: &str) {
        self.0.push(respond::issue(
            "missing",
            &[src, name],
            "Field required",
            Value::Null,
        ));
    }

    /// Required float parameter. `None` when missing or invalid (issue recorded).
    pub fn float(&mut self, src: &str, name: &str, raw: Option<&str>) -> Option<f64> {
        let Some(raw) = raw else {
            self.missing(src, name);
            return None;
        };
        if let Some(v) = py_float(raw) {
            return Some(v);
        }
        self.0.push(respond::issue(
            "float_parsing",
            &[src, name],
            "Input should be a valid number, unable to parse string as a number",
            json!(raw),
        ));
        None
    }

    /// Float with inclusive bounds (`ge`/`le`), as Pydantic reports them.
    pub fn float_in(
        &mut self,
        src: &str,
        name: &str,
        raw: Option<&str>,
        lo: f64,
        hi: f64,
    ) -> Option<f64> {
        let v = self.float(src, name, raw)?;
        if v.is_nan() || v < lo {
            self.bound_issue(src, name, raw, lo, hi, v);
            return None;
        }
        if v > hi {
            self.bound_issue(src, name, raw, lo, hi, v);
            return None;
        }
        Some(v)
    }

    fn bound_issue(&mut self, src: &str, name: &str, raw: Option<&str>, lo: f64, hi: f64, v: f64) {
        let input = raw.map_or(Value::Null, |r| json!(r));
        if v.is_nan() || v < lo {
            self.0.push(respond::issue(
                "greater_than_equal",
                &[src, name],
                &format!("Input should be greater than or equal to {}", fmt_bound(lo)),
                input,
            ));
        } else {
            self.0.push(respond::issue(
                "less_than_equal",
                &[src, name],
                &format!("Input should be less than or equal to {}", fmt_bound(hi)),
                input,
            ));
        }
    }

    /// Integer parameter, optionally with a default when absent.
    pub fn int(
        &mut self,
        src: &str,
        name: &str,
        raw: Option<&str>,
        default: Option<i64>,
    ) -> Option<i64> {
        let Some(raw) = raw else {
            if default.is_none() {
                self.missing(src, name);
            }
            return default;
        };
        if let Some(v) = py_int_str(raw) {
            return Some(v);
        }
        self.0.push(respond::issue(
            "int_parsing",
            &[src, name],
            "Input should be a valid integer, unable to parse string as an integer",
            json!(raw),
        ));
        None
    }

    /// Integer with inclusive bounds.
    pub fn int_in(
        &mut self,
        src: &str,
        name: &str,
        raw: Option<&str>,
        default: Option<i64>,
        lo: i64,
        hi: i64,
    ) -> Option<i64> {
        let v = self.int(src, name, raw, default)?;
        if v < lo {
            self.0.push(respond::issue(
                "greater_than_equal",
                &[src, name],
                &format!("Input should be greater than or equal to {lo}"),
                json!(v),
            ));
            return None;
        }
        if v > hi {
            self.0.push(respond::issue(
                "less_than_equal",
                &[src, name],
                &format!("Input should be less than or equal to {hi}"),
                json!(v),
            ));
            return None;
        }
        Some(v)
    }

    /// String parameter with a length window (counted in characters, as Python does).
    pub fn string(
        &mut self,
        src: &str,
        name: &str,
        raw: Option<&str>,
        default: Option<&str>,
        min_len: usize,
        max_len: usize,
    ) -> Option<String> {
        let Some(value) = raw.or(default) else {
            self.missing(src, name);
            return None;
        };
        let n = value.chars().count();
        if n < min_len {
            self.0.push(respond::issue(
                "string_too_short",
                &[src, name],
                &format!("String should have at least {min_len} characters"),
                json!(value),
            ));
            return None;
        }
        if n > max_len {
            self.0.push(respond::issue(
                "string_too_long",
                &[src, name],
                &format!("String should have at most {max_len} characters"),
                json!(value),
            ));
            return None;
        }
        Some(value.to_string())
    }

    /// Record a body-level problem (for example a field of the JSON body).
    pub fn push(&mut self, issue: Value) {
        self.0.push(issue);
    }
}

/// Format a bound the way Python prints it (`-90`, `180`, `-90.5`).
fn fmt_bound(v: f64) -> String {
    if v.fract() == 0.0 && v.abs() < 1e15 {
        format!("{v:.0}")
    } else {
        format!("{v}")
    }
}

/// First value of `name` in a raw query string, percent-decoded (`+` is a space, as in Python).
#[must_use]
pub fn query_param(raw_query: Option<&str>, name: &str) -> Option<String> {
    let raw = raw_query?;
    url::form_urlencoded::parse(raw.as_bytes())
        .find(|(k, _)| k == name)
        .map(|(_, v)| v.into_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn python_float_forms() {
        assert_eq!(py_float("42.5"), Some(42.5));
        assert_eq!(py_float("+3"), Some(3.0));
        assert!(py_float("nan").expect("nan parses").is_nan());
        assert_eq!(py_float("inf"), Some(f64::INFINITY));
        assert_eq!(py_float("1_0"), None);
        assert_eq!(py_float(" 4"), None);
        assert_eq!(py_float("abc"), None);
        assert_eq!(py_float(""), None);
    }

    #[test]
    fn pydantic_int_forms() {
        assert_eq!(py_int_str("6"), Some(6));
        assert_eq!(py_int_str("-2"), Some(-2));
        assert_eq!(py_int_str("6.5"), None);
        assert_eq!(py_int_str(""), None);
    }

    #[test]
    fn range_issue_uses_pydantic_kind() {
        let mut issues = Issues::new();
        assert_eq!(
            issues.float_in("path", "lat", Some("91"), -90.0, 90.0),
            None
        );
        assert!(issues.finish().is_err());
    }

    #[test]
    fn first_query_value_wins_and_plus_is_space() {
        assert_eq!(
            query_param(Some("q=grand+rap&q=x"), "q").as_deref(),
            Some("grand rap")
        );
        assert_eq!(query_param(Some("a=%2B"), "a").as_deref(), Some("+"));
        assert_eq!(query_param(None, "a"), None);
    }
}
