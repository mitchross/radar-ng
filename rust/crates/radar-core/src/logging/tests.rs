//! Logger invariants (spec-shared §11): the golden line format, reserved-key prefixing, the exc slot, level
//! names, the live subscriber, and the retry schedule.

use super::*;
use crate::pycompat::Json;
use serde_json::Value;
use std::io::{self, Write};
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tracing::Level;
use tracing::level_filters::LevelFilter;

/// In-memory writer shared between the subscriber and the test.
#[derive(Clone, Default)]
struct Capture(Arc<Mutex<Vec<u8>>>);

impl Write for Capture {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.0.lock().unwrap().extend_from_slice(buf);
        Ok(buf.len())
    }
    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

/// Run `f` under a JSON-lines subscriber at `level` and return the parsed lines it wrote.
fn capture(level: LevelFilter, f: impl FnOnce()) -> Vec<Value> {
    let cap = Capture::default();
    let writer = {
        let cap = cap.clone();
        move || cap.clone()
    };
    let sub = subscriber("radar-test", level, writer);
    tracing::subscriber::with_default(sub, f);
    let text = String::from_utf8(cap.0.lock().unwrap().clone()).unwrap();
    text.lines()
        .map(|l| serde_json::from_str(l).unwrap_or_else(|e| panic!("bad line {l}: {e}")))
        .collect()
}

fn json_of(value: &Value) -> Json {
    match value {
        Value::Bool(b) => Json::Bool(*b),
        Value::Number(n) if n.is_i64() => Json::Int(n.as_i64().unwrap()),
        Value::Number(n) => Json::Float(n.as_f64().unwrap()),
        Value::String(s) => Json::Str(s.clone()),
        other => panic!("unsupported golden value {other}"),
    }
}

fn fast_policy(attempts: u32) -> RetryPolicy {
    RetryPolicy {
        attempts,
        base_delay: Duration::from_millis(1),
        max_delay: Duration::from_millis(2),
    }
}

#[test]
fn golden_lines_match_the_python_formatter_byte_for_byte() {
    let cases: Vec<Value> =
        serde_json::from_str(include_str!("../../testdata/logger/lines.json")).unwrap();
    for case in cases {
        let extras = case["extras"]
            .as_array()
            .unwrap()
            .iter()
            .map(|pair| (pair[0].as_str().unwrap().to_string(), json_of(&pair[1])))
            .collect();
        let line = render_line(
            case["service"].as_str().unwrap(),
            "2026-06-20T12:00:00+0000",
            case["level"].as_str().unwrap(),
            case["msg"].as_str().unwrap(),
            None,
            extras,
        );
        assert_eq!(line, case["line"].as_str().unwrap());
    }
}

#[test]
fn reserved_keys_from_extras_are_prefixed_with_x() {
    let line = render_line(
        "svc",
        "ts",
        "INFO",
        "m",
        None,
        vec![
            ("msg".into(), Json::Str("a".into())),
            ("exc".into(), Json::Int(1)),
            ("level".into(), Json::Bool(true)),
        ],
    );
    let v: Value = serde_json::from_str(&line).unwrap();
    assert_eq!(v["msg"], "m");
    assert_eq!(v["x_msg"], "a");
    assert_eq!(v["x_exc"], 1);
    assert_eq!(v["x_level"], true);
    assert!(v.get("exc").is_none());
}

#[test]
fn exc_slot_sits_after_msg_and_before_extras() {
    let line = render_line(
        "svc",
        "ts",
        "ERROR",
        "boom",
        Some("Traceback: x"),
        vec![("k".into(), Json::Int(2))],
    );
    let v: Value = serde_json::from_str(&line).unwrap();
    assert_eq!(v["exc"], "Traceback: x");
    let msg_at = line.find("\"msg\"").unwrap();
    let exc_at = line.find("\"exc\"").unwrap();
    let k_at = line.find("\"k\"").unwrap();
    assert!(msg_at < exc_at && exc_at < k_at, "{line}");
}

#[test]
fn level_names_and_level_parsing_follow_python() {
    assert_eq!(level_name(&Level::TRACE), "DEBUG");
    assert_eq!(level_name(&Level::DEBUG), "DEBUG");
    assert_eq!(level_name(&Level::WARN), "WARNING");
    assert_eq!(level_name(&Level::ERROR), "ERROR");
    assert_eq!(parse_level("warning").unwrap(), LevelFilter::WARN);
    assert_eq!(parse_level(" critical ").unwrap(), LevelFilter::ERROR);
    assert_eq!(parse_level("debug").unwrap(), LevelFilter::DEBUG);
    assert!(parse_level("loud").is_err());
}

#[test]
fn live_subscriber_writes_fields_and_respects_the_level() {
    let lines = capture(LevelFilter::WARN, || {
        tracing::info!("hidden");
        tracing::warn!("fn" = "fetch_grib", attempt = 2u32, delay = 0.5f64, "retry");
    });
    assert_eq!(lines.len(), 1);
    let v = &lines[0];
    assert_eq!(v["level"], "WARNING");
    assert_eq!(v["service"], "radar-test");
    assert_eq!(v["msg"], "retry");
    assert_eq!(v["fn"], "fetch_grib");
    assert_eq!(v["attempt"], 2);
    assert_eq!(v["delay"], 0.5);
    let ts = v["ts"].as_str().unwrap();
    assert!(
        chrono::DateTime::parse_from_str(ts, "%Y-%m-%dT%H:%M:%S%z").is_ok(),
        "{ts}"
    );
}

#[test]
fn live_subscriber_puts_exc_in_its_own_slot() {
    let lines = capture(LevelFilter::ERROR, || {
        tracing::error!(exc = "Traceback: boom", "failed");
    });
    assert_eq!(lines[0]["exc"], "Traceback: boom");
    assert_eq!(lines[0]["msg"], "failed");
    assert_eq!(lines[0]["level"], "ERROR");
}

#[test]
fn retry_recovers_after_transient_failures_with_capped_backoff() {
    let mut calls = 0;
    let lines = capture(LevelFilter::WARN, || {
        let out: Result<u32, String> = retry(
            "fetch_grib",
            fast_policy(4),
            |_| true,
            || {
                calls += 1;
                if calls < 4 {
                    Err(format!("reset {calls}"))
                } else {
                    Ok(7)
                }
            },
        );
        assert_eq!(out.unwrap(), 7);
    });
    assert_eq!(calls, 4);
    assert_eq!(
        lines.len(),
        3,
        "one warning per failed attempt that is retried"
    );
    let delays: Vec<f64> = lines.iter().map(|l| l["delay"].as_f64().unwrap()).collect();
    assert_eq!(
        delays,
        vec![0.001, 0.002, 0.002],
        "doubles, then caps at max_delay"
    );
    assert_eq!(lines[0]["attempt"], 1);
    assert_eq!(lines[2]["attempt"], 3);
    assert_eq!(lines[0]["max_attempts"], 4);
    assert_eq!(lines[0]["fn"], "fetch_grib");
    assert_eq!(lines[0]["err"], "reset 1");
}

#[test]
fn retry_returns_non_retryable_errors_immediately_without_a_warning() {
    let mut calls = 0;
    let lines = capture(LevelFilter::WARN, || {
        let out: Result<(), &str> = retry(
            "x",
            fast_policy(4),
            |e: &&str| *e != "fatal",
            || {
                calls += 1;
                Err("fatal")
            },
        );
        assert_eq!(out, Err("fatal"));
    });
    assert_eq!(calls, 1);
    assert!(lines.is_empty());
}

#[test]
fn retry_gives_up_after_the_last_attempt_without_an_extra_warning() {
    let mut calls = 0;
    let lines = capture(LevelFilter::WARN, || {
        let out: Result<(), String> = retry(
            "x",
            fast_policy(3),
            |_| true,
            || {
                calls += 1;
                Err(format!("down {calls}"))
            },
        );
        assert_eq!(out, Err("down 3".to_string()));
    });
    assert_eq!(calls, 3);
    assert_eq!(
        lines.len(),
        2,
        "warnings only before the retries, not after the final failure"
    );
}

#[test]
fn default_retry_policy_matches_python() {
    let p = RetryPolicy::default();
    assert_eq!(p.attempts, 4);
    assert_eq!(p.base_delay, Duration::from_secs(1));
    assert_eq!(p.max_delay, Duration::from_secs(30));
}
