//! JSON log lines matching spec-shared §11, plus the `retry` helper from §11.3.
//!
//! Line shape: `{"ts": "...", "level": "INFO", "service": "...", "msg": "...", ["exc": "..."], <extra fields>}`
//! with Python's `json.dumps` separators (`", "` and `": "`) and `ensure_ascii`. Extra fields keep the order
//! they were recorded in.
//!
//! Fix (D13): extra fields named `ts`, `level`, `service`, `msg` are prefixed with `x_`, so they cannot
//! overwrite the base fields. A field named `exc` fills the `exc` slot, the way `exc_info` does in Python.
//!
//! `ts` is local time with second precision and an offset without a colon (`%Y-%m-%dT%H:%M:%S%z`).

use crate::env;
use crate::pycompat::{self, Json, Style};
use std::fmt;
use std::sync::OnceLock;
use std::time::Duration;
use tracing::field::{Field, Visit};
use tracing::{Event, Level, Subscriber};
use tracing_subscriber::fmt::format::Writer;
use tracing_subscriber::fmt::{FmtContext, FormatEvent, FormatFields};
use tracing_subscriber::registry::LookupSpan;

/// Base keys that extra fields may not overwrite (D13).
const RESERVED: [&str; 4] = ["ts", "level", "service", "msg"];

/// Python `levelname` for a tracing level.
pub fn level_name(level: &Level) -> &'static str {
    match *level {
        Level::TRACE | Level::DEBUG => "DEBUG",
        Level::INFO => "INFO",
        Level::WARN => "WARNING",
        Level::ERROR => "ERROR",
    }
}

/// Parse a `LOG_LEVEL` value (`DEBUG`, `INFO`, `WARNING`, `ERROR`, `CRITICAL`, any case).
pub fn parse_level(value: &str) -> Result<tracing::level_filters::LevelFilter, env::EnvError> {
    use tracing::level_filters::LevelFilter;
    match value.trim().to_ascii_uppercase().as_str() {
        "DEBUG" => Ok(LevelFilter::DEBUG),
        "INFO" => Ok(LevelFilter::INFO),
        "WARNING" | "WARN" => Ok(LevelFilter::WARN),
        "ERROR" | "CRITICAL" => Ok(LevelFilter::ERROR),
        _ => Err(env::EnvError {
            name: "LOG_LEVEL".into(),
            value: value.into(),
            reason: "unknown level name",
        }),
    }
}

/// Render one log line from already-collected parts. `extras` must not contain the reserved keys. Use
/// [`FieldCollector`] to build it from a tracing event.
pub fn render_line(
    service: &str,
    ts: &str,
    level: &str,
    msg: &str,
    exc: Option<&str>,
    extras: Vec<(String, Json)>,
) -> String {
    let mut pairs = vec![
        ("ts".to_string(), Json::Str(ts.to_string())),
        ("level".to_string(), Json::Str(level.to_string())),
        ("service".to_string(), Json::Str(service.to_string())),
        ("msg".to_string(), Json::Str(msg.to_string())),
    ];
    if let Some(exc) = exc {
        pairs.push(("exc".to_string(), Json::Str(exc.to_string())));
    }
    for (key, value) in extras {
        let key = if RESERVED.contains(&key.as_str()) || key == "exc" {
            format!("x_{key}")
        } else {
            key
        };
        pairs.push((key, value));
    }
    pycompat::dumps(&Json::Object(pairs), Style::Default, false)
}

/// Collects the fields of a tracing event: `message`, `exc`, and the rest in record order.
#[derive(Default)]
pub struct FieldCollector {
    /// The event message.
    pub message: Option<String>,
    /// Traceback-like text from an `exc` field.
    pub exc: Option<String>,
    /// Remaining fields.
    pub extras: Vec<(String, Json)>,
}

impl FieldCollector {
    fn push(&mut self, name: &str, value: Json) {
        match name {
            "message" => self.message = Some(json_text(&value)),
            "exc" => self.exc = Some(json_text(&value)),
            _ => self.extras.push((name.to_string(), value)),
        }
    }
}

fn json_text(value: &Json) -> String {
    match value {
        Json::Str(s) => s.clone(),
        other => pycompat::dumps(other, Style::Default, false),
    }
}

impl Visit for FieldCollector {
    fn record_str(&mut self, field: &Field, value: &str) {
        self.push(field.name(), Json::Str(value.to_string()));
    }
    fn record_i64(&mut self, field: &Field, value: i64) {
        self.push(field.name(), Json::Int(value));
    }
    fn record_u64(&mut self, field: &Field, value: u64) {
        self.push(field.name(), Json::UInt(value));
    }
    fn record_bool(&mut self, field: &Field, value: bool) {
        self.push(field.name(), Json::Bool(value));
    }
    fn record_f64(&mut self, field: &Field, value: f64) {
        self.push(field.name(), Json::Float(value));
    }
    fn record_debug(&mut self, field: &Field, value: &dyn fmt::Debug) {
        self.push(field.name(), Json::Str(format!("{value:?}")));
    }
}

/// Event formatter that writes one Python-compatible JSON line per event.
#[derive(Debug, Clone)]
pub struct JsonLines {
    service: String,
}

impl JsonLines {
    /// Formatter whose `service` field is `service`.
    pub fn new(service: impl Into<String>) -> Self {
        Self {
            service: service.into(),
        }
    }
}

impl<S, N> FormatEvent<S, N> for JsonLines
where
    S: Subscriber + for<'a> LookupSpan<'a>,
    N: for<'a> FormatFields<'a> + 'static,
{
    fn format_event(
        &self,
        _ctx: &FmtContext<'_, S, N>,
        mut writer: Writer<'_>,
        event: &Event<'_>,
    ) -> fmt::Result {
        let mut collector = FieldCollector::default();
        event.record(&mut collector);
        let ts = chrono::Local::now()
            .format("%Y-%m-%dT%H:%M:%S%z")
            .to_string();
        let line = render_line(
            &self.service,
            &ts,
            level_name(event.metadata().level()),
            collector.message.as_deref().unwrap_or(""),
            collector.exc.as_deref(),
            collector.extras,
        );
        writeln!(writer, "{line}")
    }
}

/// Build a subscriber that writes JSON lines for `service` to `writer` at `level`.
pub fn subscriber<W>(
    service: &str,
    level: tracing::level_filters::LevelFilter,
    writer: W,
) -> impl Subscriber + for<'a> LookupSpan<'a> + Send + Sync
where
    W: for<'w> tracing_subscriber::fmt::MakeWriter<'w> + Send + Sync + 'static,
{
    tracing_subscriber::fmt()
        .event_format(JsonLines::new(service))
        .with_max_level(level)
        .with_writer(writer)
        .finish()
}

/// Install the global JSON logger on stdout for `service`, at `LOG_LEVEL` (default `INFO`).
/// Idempotent: the first call wins, later calls return `Ok` and change nothing.
pub fn init(service: &str) -> anyhow::Result<()> {
    static INSTALLED: OnceLock<()> = OnceLock::new();
    let level = parse_level(&env::var("LOG_LEVEL").unwrap_or_else(|| "INFO".to_string()))?;
    INSTALLED.get_or_init(|| {
        let subscriber = subscriber(service, level, std::io::stdout);
        // Another subscriber may already be installed (for example by a test harness). That is fine.
        let _ = tracing::subscriber::set_global_default(subscriber);
    });
    Ok(())
}

/// Backoff policy for [`retry`] (spec §11.3 defaults: 4 attempts, 1 s base, 30 s cap).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RetryPolicy {
    /// Total attempts, including the first.
    pub attempts: u32,
    /// Sleep before the second attempt. Doubles after each failure.
    pub base_delay: Duration,
    /// Upper bound on the sleep.
    pub max_delay: Duration,
}

impl Default for RetryPolicy {
    fn default() -> Self {
        Self {
            attempts: 4,
            base_delay: Duration::from_secs(1),
            max_delay: Duration::from_secs(30),
        }
    }
}

/// Run `op` up to `policy.attempts` times with exponential backoff. Only errors for which `retryable` returns
/// true are retried. Each retry logs a `retry` warning (fields `fn`, `attempt`, `max_attempts`, `delay`, `err`)
/// before sleeping. The final failure is returned without a warning, as in Python.
pub fn retry<T, E, F, P>(name: &str, policy: RetryPolicy, retryable: P, mut op: F) -> Result<T, E>
where
    E: fmt::Display,
    F: FnMut() -> Result<T, E>,
    P: Fn(&E) -> bool,
{
    let attempts = policy.attempts.max(1);
    let mut delay = policy.base_delay;
    for attempt in 1..=attempts {
        match op() {
            Ok(value) => return Ok(value),
            Err(err) => {
                if !retryable(&err) || attempt == attempts {
                    return Err(err);
                }
                tracing::warn!(
                    "fn" = name,
                    attempt = attempt,
                    max_attempts = attempts,
                    delay = delay.as_secs_f64(),
                    err = %err,
                    "retry"
                );
                std::thread::sleep(delay);
                delay = (delay * 2).min(policy.max_delay);
            }
        }
    }
    unreachable!("the loop returns on the last attempt")
}

#[cfg(test)]
mod tests;
