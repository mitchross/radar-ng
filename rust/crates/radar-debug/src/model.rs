//! Shared plumbing: timed HTTP fetches, percentiles, check results, and rendering.
//!
//! Port of `tools/debug_harness/core.py`. Output is byte-compatible with the Python harness: the
//! JSON form is `json.dumps(..., indent=2)` (ASCII-escaped, Python float reprs), and the human form
//! prints the same icons, section titles and summary lines.

use std::collections::HashMap;
use std::fmt::Write as _;
use std::time::{Duration, Instant};

use chrono::{DateTime, Utc};
use radar_core::pycompat::{self, Json, Style};
use radar_core::time::{parse_iso_as_utc, round_digits, round_half_even};
use serde_json::Value;

/// Default per-request timeout, seconds (`DEFAULT_TIMEOUT_S`).
pub const DEFAULT_TIMEOUT: Duration = Duration::from_secs(15);

/// User-Agent sent by every probe (`radar-ng-debug-harness`).
pub const USER_AGENT: &str = "radar-ng-debug-harness";

/// Severity of one check. `Fail` flips the process exit code. The order is the severity order.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Status {
    /// Healthy.
    Ok,
    /// Degraded or suspicious; does not change the exit code.
    Warn,
    /// Broken; the process exits 1.
    Fail,
}

impl Status {
    /// Lowercase name used in JSON (`ok`, `warn`, `fail`).
    pub fn as_str(self) -> &'static str {
        match self {
            Status::Ok => "ok",
            Status::Warn => "warn",
            Status::Fail => "fail",
        }
    }

    /// The human-readable icon (`✓`, `⚠`, `✗`).
    pub fn icon(self) -> &'static str {
        match self {
            Status::Ok => "✓",
            Status::Warn => "⚠",
            Status::Fail => "✗",
        }
    }
}

/// One named result (`Check` in the Python).
#[derive(Debug, Clone, PartialEq)]
pub struct Check {
    /// Dotted check name, e.g. `api.manifest` or `temporal.schedule.nowcast`.
    pub name: String,
    /// Severity.
    pub status: Status,
    /// One-line diagnosis.
    pub detail: String,
    /// Structured extras, always a JSON object (`{}` when empty).
    pub data: Json,
}

impl Check {
    /// A check with no structured data.
    pub fn new(name: impl Into<String>, status: Status, detail: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            status,
            detail: detail.into(),
            data: Json::Object(Vec::new()),
        }
    }

    /// Attach structured data (must be a [`Json::Object`]).
    pub fn with_data(mut self, data: Json) -> Self {
        self.data = data;
        self
    }
}

/// Worst status among `checks`; `Ok` when empty.
pub fn worst(checks: &[Check]) -> Status {
    checks.iter().map(|c| c.status).max().unwrap_or(Status::Ok)
}

/// `1` when any check failed, `0` otherwise.
pub fn exit_code(checks: &[Check]) -> i32 {
    i32::from(worst(checks) == Status::Fail)
}

/// Result of one timed GET. Errors are data, not `Err`, so samplers can count failures.
#[derive(Debug, Clone, Default)]
pub struct Fetch {
    /// Requested URL.
    pub url: String,
    /// HTTP status, 0 when no response was received.
    pub status: u16,
    /// Wall time in milliseconds.
    pub elapsed_ms: f64,
    /// Body length in bytes (0 for error statuses, whose bodies are not read).
    pub bytes: usize,
    /// Response headers with lowercased names (last value wins).
    pub headers: HashMap<String, String>,
    /// Response body (empty for error statuses).
    pub body: Vec<u8>,
    /// Error text, set for transport errors, HTTP >= 400, and invalid JSON.
    pub error: Option<String>,
}

impl Fetch {
    /// No error and a 2xx or 3xx status.
    pub fn ok(&self) -> bool {
        self.error.is_none() && (200..400).contains(&self.status)
    }

    /// A response header (lowercase name), or `""`.
    pub fn header(&self, name: &str) -> &str {
        self.headers.get(name).map_or("", String::as_str)
    }
}

/// Blocking-free HTTP client shared by every probe. Self-signed certificates are accepted, as in the
/// Python harness (it reads public weather tiles).
#[derive(Clone)]
pub struct Http {
    client: reqwest::Client,
}

impl Http {
    /// Build the client.
    pub fn new() -> anyhow::Result<Self> {
        let client = reqwest::Client::builder()
            .danger_accept_invalid_certs(true)
            .timeout(DEFAULT_TIMEOUT)
            .user_agent(USER_AGENT)
            .build()?;
        Ok(Self { client })
    }

    /// One timed GET. Never fails: transport errors and HTTP >= 400 come back in [`Fetch::error`].
    pub async fn get(&self, url: &str) -> Fetch {
        let start = Instant::now();
        let mut out = Fetch {
            url: url.to_string(),
            ..Fetch::default()
        };
        let resp = match self.client.get(url).send().await {
            Ok(resp) => resp,
            Err(err) => {
                out.elapsed_ms = elapsed_ms(start);
                out.error = Some(transport_error(&err));
                return out;
            }
        };
        out.status = resp.status().as_u16();
        for (name, value) in resp.headers() {
            if let Ok(v) = value.to_str() {
                out.headers
                    .insert(name.as_str().to_ascii_lowercase(), v.to_string());
            }
        }
        if out.status >= 400 {
            out.elapsed_ms = elapsed_ms(start);
            out.error = Some(format!("HTTP {}", out.status));
            return out;
        }
        match resp.bytes().await {
            Ok(body) => {
                out.elapsed_ms = elapsed_ms(start);
                out.bytes = body.len();
                out.body = body.to_vec();
            }
            Err(err) => {
                out.elapsed_ms = elapsed_ms(start);
                out.error = Some(transport_error(&err));
            }
        }
        out
    }

    /// GET and parse the body as JSON. Returns `(fetch, None)` when the fetch failed or the body
    /// is not JSON (then `fetch.error` says why).
    pub async fn get_json(&self, url: &str) -> (Fetch, Option<Value>) {
        let mut f = self.get(url).await;
        if !f.ok() {
            return (f, None);
        }
        match serde_json::from_slice::<Value>(&f.body) {
            Ok(v) => (f, Some(v)),
            Err(err) => {
                f.error = Some(format!("invalid JSON: {err}"));
                (f, None)
            }
        }
    }
}

fn elapsed_ms(start: Instant) -> f64 {
    start.elapsed().as_secs_f64() * 1000.0
}

/// Map a transport error to the Python harness's wording (`URLError`, `TimeoutError`).
fn transport_error(err: &reqwest::Error) -> String {
    let kind = if err.is_timeout() {
        "TimeoutError"
    } else if err.is_connect() {
        "URLError"
    } else {
        "Error"
    };
    format!("{kind}: {err}")
}

/// Latency summary of a sample set (`percentiles` in the Python).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Stats {
    /// Sample count.
    pub n: usize,
    /// Smallest sample, rounded to 0.1 ms.
    pub min_ms: f64,
    /// Median, rounded to 0.1 ms.
    pub p50_ms: f64,
    /// 95th percentile (nearest rank, Python `round` on the index), rounded to 0.1 ms.
    pub p95_ms: f64,
    /// Largest sample, rounded to 0.1 ms.
    pub max_ms: f64,
}

impl Stats {
    /// The Python dict shape: `{"n", "min_ms", "p50_ms", "p95_ms", "max_ms"}`.
    pub fn to_json(self) -> Json {
        Json::Object(vec![
            ("n".into(), Json::Int(self.n as i64)),
            ("min_ms".into(), Json::Float(self.min_ms)),
            ("p50_ms".into(), Json::Float(self.p50_ms)),
            ("p95_ms".into(), Json::Float(self.p95_ms)),
            ("max_ms".into(), Json::Float(self.max_ms)),
        ])
    }
}

/// Stats or the empty object, as the Python returns `{}` for no samples.
pub fn stats_json(stats: Option<Stats>) -> Json {
    stats.map_or_else(|| Json::Object(Vec::new()), Stats::to_json)
}

/// Nearest-rank percentiles with Python's `round()` (half to even) on the index. `None` when empty.
pub fn percentiles(samples_ms: &[f64]) -> Option<Stats> {
    if samples_ms.is_empty() {
        return None;
    }
    let mut s = samples_ms.to_vec();
    s.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    let n = s.len();
    let pct = |p: f64| {
        let raw = round_half_even(p / 100.0 * (n as f64 - 1.0));
        let idx = raw.clamp(0.0, (n - 1) as f64) as usize;
        s[idx]
    };
    let median = if n % 2 == 1 {
        s[n / 2]
    } else {
        (s[n / 2 - 1] + s[n / 2]) / 2.0
    };
    Some(Stats {
        n,
        min_ms: round_digits(s[0], 1),
        p50_ms: round_digits(median, 1),
        p95_ms: round_digits(pct(95.0), 1),
        max_ms: round_digits(s[n - 1], 1),
    })
}

/// Python `datetime.fromisoformat` for the writers' formats; naive values are UTC. `None` when unparseable.
pub fn parse_iso(ts: &str) -> Option<DateTime<Utc>> {
    parse_iso_as_utc(ts)
}

/// Seconds from `then` to `now`, as a float (Python `(now - then).total_seconds()`).
pub fn seconds_between(now: DateTime<Utc>, then: DateTime<Utc>) -> f64 {
    (now - then).num_milliseconds() as f64 / 1000.0
}

/// Human age: `12s`, `3m07s`, `2h05m`. Sign and floor division follow Python (`//`), so negative
/// ages print as Python prints them.
pub fn age_str(seconds: f64) -> String {
    let s = seconds as i64; // Python int(): truncate toward zero
    let abs = s.abs();
    if abs < 120 {
        format!("{s}s")
    } else if abs < 7200 {
        format!("{}m{:02}s", s.div_euclid(60), abs % 60)
    } else {
        format!("{}h{:02}m", s.div_euclid(3600), (abs % 3600) / 60)
    }
}

/// `urllib.parse.quote(s, safe="")`: encode everything except unreserved characters.
pub fn quote_ts(timestamp: &str) -> String {
    let mut out = String::with_capacity(timestamp.len());
    for b in timestamp.bytes() {
        if b.is_ascii_alphanumeric() || matches!(b, b'_' | b'.' | b'-' | b'~') {
            out.push(b as char);
        } else {
            let _ = write!(out, "%{b:02X}");
        }
    }
    out
}

/// Python `repr(str)` for the simple strings the harness prints (single quotes unless the text
/// contains a single quote and no double quote).
pub fn py_repr_str(s: &str) -> String {
    let quote = if s.contains('\'') && !s.contains('"') {
        '"'
    } else {
        '\''
    };
    let mut out = String::with_capacity(s.len() + 2);
    out.push(quote);
    for c in s.chars() {
        match c {
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if c == quote => {
                out.push('\\');
                out.push(c);
            }
            c => out.push(c),
        }
    }
    out.push(quote);
    out
}

/// Python `repr(list_of_str)`, e.g. `['a', 'b']`.
pub fn py_repr_list(items: &[String]) -> String {
    let inner: Vec<String> = items.iter().map(|s| py_repr_str(s)).collect();
    format!("[{}]", inner.join(", "))
}

/// Python `repr(dict_of_str_to_int)`, e.g. `{'classic': 3}`.
pub fn py_repr_counts(pairs: &[(String, usize)]) -> String {
    let inner: Vec<String> = pairs
        .iter()
        .map(|(k, v)| format!("{}: {v}", py_repr_str(k)))
        .collect();
    format!("{{{}}}", inner.join(", "))
}

/// Python `str()` of a JSON value as the harness prints it (`None` for null, Python reprs for lists
/// and dicts, `repr` for floats).
pub fn py_display(value: Option<&Value>) -> String {
    match value {
        None | Some(Value::Null) => "None".to_string(),
        Some(Value::Bool(b)) => if *b { "True" } else { "False" }.to_string(),
        Some(Value::Number(n)) => match n.as_i64() {
            Some(i) => i.to_string(),
            None => pycompat::float_repr(n.as_f64().unwrap_or(f64::NAN)),
        },
        Some(Value::String(s)) => s.clone(),
        Some(Value::Array(items)) => {
            let inner: Vec<String> = items.iter().map(|v| py_display_nested(Some(v))).collect();
            format!("[{}]", inner.join(", "))
        }
        Some(Value::Object(map)) => {
            let inner: Vec<String> = map
                .iter()
                .map(|(k, v)| format!("{}: {}", py_repr_str(k), py_display_nested(Some(v))))
                .collect();
            format!("{{{}}}", inner.join(", "))
        }
    }
}

/// Like [`py_display`] but strings are quoted, as inside a Python container.
fn py_display_nested(value: Option<&Value>) -> String {
    match value {
        Some(Value::String(s)) => py_repr_str(s),
        other => py_display(other),
    }
}

/// Python `float` repr, for `{x}` formatting of floats.
pub fn float_str(x: f64) -> String {
    pycompat::float_repr(x)
}

/// `json.dumps(value, indent=2)` as Python prints it: two-space indent, `", "`-free item separators
/// on their own lines, `[]` and `{}` for empty containers, ASCII escapes.
pub fn pretty_json(value: &Json) -> String {
    let mut out = String::new();
    write_pretty(&mut out, value, 0);
    out
}

fn write_pretty(out: &mut String, value: &Json, depth: usize) {
    match value {
        Json::Array(items) if items.is_empty() => out.push_str("[]"),
        Json::Array(items) => {
            out.push_str("[\n");
            for (i, item) in items.iter().enumerate() {
                if i > 0 {
                    out.push_str(",\n");
                }
                push_indent(out, depth + 1);
                write_pretty(out, item, depth + 1);
            }
            out.push('\n');
            push_indent(out, depth);
            out.push(']');
        }
        Json::Object(pairs) if pairs.is_empty() => out.push_str("{}"),
        Json::Object(pairs) => {
            out.push_str("{\n");
            for (i, (key, item)) in pairs.iter().enumerate() {
                if i > 0 {
                    out.push_str(",\n");
                }
                push_indent(out, depth + 1);
                out.push_str(&pycompat::dumps(
                    &Json::Str(key.clone()),
                    Style::Default,
                    false,
                ));
                out.push_str(": ");
                write_pretty(out, item, depth + 1);
            }
            out.push('\n');
            push_indent(out, depth);
            out.push('}');
        }
        scalar => out.push_str(&pycompat::dumps(scalar, Style::Default, false)),
    }
}

fn push_indent(out: &mut String, depth: usize) {
    for _ in 0..depth {
        out.push_str("  ");
    }
}

/// Print one section: the human block (`== title ==` then one line per check), or the JSON
/// document with `section`, `status`, `checked_at` and `checks`.
pub fn render_checks(title: &str, checks: &[Check], as_json: bool, now: DateTime<Utc>) {
    if as_json {
        let doc = Json::Object(vec![
            ("section".into(), Json::Str(title.to_string())),
            (
                "status".into(),
                Json::Str(worst(checks).as_str().to_string()),
            ),
            (
                "checked_at".into(),
                Json::Str(radar_core::time::isoformat_utc(&now)),
            ),
            (
                "checks".into(),
                Json::Array(
                    checks
                        .iter()
                        .map(|c| {
                            Json::Object(vec![
                                ("name".into(), Json::Str(c.name.clone())),
                                ("status".into(), Json::Str(c.status.as_str().to_string())),
                                ("detail".into(), Json::Str(c.detail.clone())),
                                ("data".into(), c.data.clone()),
                            ])
                        })
                        .collect(),
                ),
            ),
        ]);
        println!("{}", pretty_json(&doc));
        return;
    }
    println!("\n== {title} ==");
    for c in checks {
        println!("  {} {}: {}", c.status.icon(), c.name, c.detail);
    }
}

/// `"{fails} fail · {warns} warn"` summary footer used by `doctor`.
pub fn summary_line(checks: &[Check]) -> String {
    let fails = checks.iter().filter(|c| c.status == Status::Fail).count();
    let warns = checks.iter().filter(|c| c.status == Status::Warn).count();
    format!("\n{} checks · {fails} fail · {warns} warn", checks.len())
}

/// Current UTC time (`utcnow()`).
pub fn utcnow() -> DateTime<Utc> {
    Utc::now()
}

/// Fetch and parse a manifest, requiring a `layers` object (`_get_manifest`).
pub async fn get_manifest(http: &Http, server: &str) -> (Fetch, Option<Value>) {
    let (mut f, body) = http.get_json(&format!("{server}/api/manifest.json")).await;
    let body = match body {
        Some(b) if b.get("layers").is_some_and(Value::is_object) => Some(b),
        Some(_) => {
            f.error = Some("manifest missing 'layers'".to_string());
            None
        }
        None => None,
    };
    (f, body)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn percentiles_basic() {
        let s = percentiles(&[10.0, 20.0, 30.0, 40.0, 100.0]).unwrap();
        assert_eq!(s.n, 5);
        assert_eq!(s.min_ms, 10.0);
        assert_eq!(s.p50_ms, 30.0);
        assert_eq!(s.max_ms, 100.0);
        assert!(percentiles(&[]).is_none());
    }

    #[test]
    fn percentiles_index_uses_half_even_rounding() {
        // 4 samples: p95 index = round(0.95 * 3) = round(2.85) = 3 -> largest.
        // 3 samples: p95 index = round(0.95 * 2) = round(1.9) = 2 -> largest.
        // p50 of an even count averages the two middle samples.
        let s = percentiles(&[1.0, 2.0, 3.0, 4.0]).unwrap();
        assert_eq!(s.p50_ms, 2.5);
        assert_eq!(s.p95_ms, 4.0);
        // 21 samples: p95 index = round(0.95 * 20) = 19 -> 20th value.
        let v: Vec<f64> = (1..=21).map(|i| i as f64).collect();
        assert_eq!(percentiles(&v).unwrap().p95_ms, 20.0);
    }

    #[test]
    fn stats_round_to_tenth_like_python() {
        let s = percentiles(&[1.04, 1.06]).unwrap();
        assert_eq!(s.min_ms, 1.0);
        assert_eq!(s.max_ms, 1.1);
    }

    #[test]
    fn parse_iso_handles_ingest_format_and_naive() {
        let dt = parse_iso("2026-07-04T20:02:00+00:00").unwrap();
        assert_eq!(dt.to_rfc3339(), "2026-07-04T20:02:00+00:00");
        // naive strings are taken as UTC
        assert!(parse_iso("2026-07-04T20:02:00").is_some());
        assert!(parse_iso("not-a-timestamp").is_none());
    }

    #[test]
    fn quote_ts_encodes_colon_and_plus() {
        assert_eq!(
            quote_ts("2026-07-04T20:02:00+00:00"),
            "2026-07-04T20%3A02%3A00%2B00%3A00"
        );
        assert_eq!(quote_ts("a_b.c-d~e"), "a_b.c-d~e");
    }

    #[test]
    fn age_str_matches_python_bands() {
        assert_eq!(age_str(0.0), "0s");
        assert_eq!(age_str(119.9), "119s");
        assert_eq!(age_str(120.0), "2m00s");
        // Python switches to hours at 7200 s, so 3723 s is still minutes.
        assert_eq!(age_str(3723.0), "62m03s");
        assert_eq!(age_str(7199.0), "119m59s");
        assert_eq!(age_str(7200.0), "2h00m");
        assert_eq!(age_str(8000.0), "2h13m");
    }

    #[test]
    fn age_str_negative_uses_python_floor_division() {
        // Python: -150 // 60 == -3 and abs(-150) % 60 == 30.
        assert_eq!(age_str(-150.0), "-3m30s");
        // Python: -3723 // 60 == -63, abs(-3723) % 60 == 3.
        assert_eq!(age_str(-3723.0), "-63m03s");
        // Python: -8000 // 3600 == -3, abs % 3600 // 60 == 13.
        assert_eq!(age_str(-8000.0), "-3h13m");
        assert_eq!(age_str(-45.0), "-45s");
    }

    #[test]
    fn worst_and_exit_code() {
        let ok = Check::new("a", Status::Ok, "");
        let warn = Check::new("b", Status::Warn, "");
        let fail = Check::new("c", Status::Fail, "");
        assert_eq!(worst(&[ok.clone(), warn.clone()]), Status::Warn);
        assert_eq!(worst(&[warn, fail.clone()]), Status::Fail);
        assert_eq!(worst(&[]), Status::Ok);
        assert_eq!(exit_code(std::slice::from_ref(&ok)), 0);
        assert_eq!(exit_code(&[ok, fail]), 1);
    }

    #[test]
    fn pretty_json_matches_python_indent_2() {
        let doc = Json::Object(vec![
            ("a".into(), Json::Int(1)),
            (
                "b".into(),
                Json::Array(vec![Json::Str("x".into()), Json::Float(2.0)]),
            ),
            ("c".into(), Json::Object(vec![])),
            ("d".into(), Json::Array(vec![])),
            ("e".into(), Json::Str("\u{00b7}".into())),
        ]);
        let expected = "{\n  \"a\": 1,\n  \"b\": [\n    \"x\",\n    2.0\n  ],\n  \"c\": {},\n  \"d\": [],\n  \"e\": \"\\u00b7\"\n}";
        assert_eq!(pretty_json(&doc), expected);
    }

    #[test]
    fn python_reprs() {
        assert_eq!(py_repr_str("abc"), "'abc'");
        assert_eq!(py_repr_str("it's"), "\"it's\"");
        assert_eq!(py_repr_list(&["a".into(), "b".into()]), "['a', 'b']");
        assert_eq!(py_repr_counts(&[("classic".into(), 3)]), "{'classic': 3}");
        assert_eq!(py_repr_counts(&[]), "{}");
        assert_eq!(py_display(None), "None");
        assert_eq!(py_display(Some(&serde_json::json!(["x"]))), "['x']");
        assert_eq!(float_str(10.0), "10.0");
    }

    #[test]
    fn seconds_between_is_signed() {
        let a = parse_iso("2026-01-01T00:00:00+00:00").unwrap();
        let b = parse_iso("2026-01-01T00:01:30+00:00").unwrap();
        assert_eq!(seconds_between(b, a), 90.0);
        assert_eq!(seconds_between(a, b), -90.0);
    }
}
