//! Process-wide Level II metrics as Prometheus text, served by `serve` on a private port.

use anyhow::Result;
use std::{
    collections::BTreeMap,
    sync::{Mutex, OnceLock},
    time::Duration,
};

/// Upper bounds in seconds shared by every timing histogram.
pub const BUCKETS: [f64; 13] = [
    0.01, 0.025, 0.05, 0.1, 0.25, 0.5, 1.0, 2.5, 5.0, 10.0, 30.0, 60.0, 120.0,
];

#[derive(Default)]
struct Hist {
    buckets: [u64; BUCKETS.len()],
    count: u64,
    sum: f64,
}

#[derive(Default)]
struct Inner {
    gauges: BTreeMap<String, f64>,
    counters: BTreeMap<String, f64>,
    hists: BTreeMap<String, Hist>,
}

fn registry() -> &'static Mutex<Inner> {
    static REG: OnceLock<Mutex<Inner>> = OnceLock::new();
    REG.get_or_init(Default::default)
}

fn lock() -> std::sync::MutexGuard<'static, Inner> {
    registry().lock().unwrap_or_else(|e| e.into_inner())
}

fn clean(v: &str) -> String {
    v.chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '_' || c == '-' {
                c
            } else {
                '_'
            }
        })
        .take(48)
        .collect()
}

/// `name{k="v",...}` with sanitised label values.
pub fn series(name: &str, labels: &[(&str, &str)]) -> String {
    if labels.is_empty() {
        return name.to_owned();
    }
    let inner: Vec<String> = labels
        .iter()
        .map(|(k, v)| format!("{k}=\"{}\"", clean(v)))
        .collect();
    format!("{name}{{{}}}", inner.join(","))
}

pub fn gauge(name: &str, value: f64) {
    lock().gauges.insert(name.to_owned(), value);
}

/// Replaces every series of `name` with `values` (label value, number), so stale stations disappear.
pub fn gauge_family(name: &str, label: &str, values: &[(String, f64)]) {
    let mut g = lock();
    g.gauges
        .retain(|k, _| !(k == name || k.starts_with(&format!("{name}{{"))));
    for (l, v) in values {
        g.gauges.insert(series(name, &[(label, l)]), *v);
    }
}

pub fn incr(name: &str, labels: &[(&str, &str)]) {
    add(name, labels, 1.0);
}

pub fn add(name: &str, labels: &[(&str, &str)], by: f64) {
    *lock().counters.entry(series(name, labels)).or_insert(0.0) += by;
}

pub fn observe(name: &str, d: Duration) {
    let secs = d.as_secs_f64();
    let mut g = lock();
    let h = g.hists.entry(name.to_owned()).or_default();
    h.count += 1;
    h.sum += secs;
    for (i, le) in BUCKETS.iter().enumerate() {
        if secs <= *le {
            h.buckets[i] += 1;
        }
    }
}

/// Current value of a counter series, for tests and the harness.
pub fn counter(name: &str, labels: &[(&str, &str)]) -> f64 {
    lock()
        .counters
        .get(&series(name, labels))
        .copied()
        .unwrap_or(0.0)
}

pub fn gauge_value(name: &str) -> Option<f64> {
    lock().gauges.get(name).copied()
}

pub fn hist_count(name: &str) -> u64 {
    lock().hists.get(name).map_or(0, |h| h.count)
}

/// Prometheus text exposition of every series.
pub fn render() -> String {
    let g = lock();
    let mut out = String::new();
    for (k, v) in &g.gauges {
        out.push_str(&format!("{k} {v}\n"));
    }
    for (k, v) in &g.counters {
        out.push_str(&format!("{k} {v}\n"));
    }
    for (name, h) in &g.hists {
        for (i, le) in BUCKETS.iter().enumerate() {
            out.push_str(&format!("{name}_bucket{{le=\"{le}\"}} {}\n", h.buckets[i]));
        }
        out.push_str(&format!("{name}_bucket{{le=\"+Inf\"}} {}\n", h.count));
        out.push_str(&format!("{name}_sum {}\n{name}_count {}\n", h.sum, h.count));
    }
    out
}

/// Serves `/metrics` until the process stops. One request per connection, no routing beyond the path.
pub async fn serve(addr: std::net::SocketAddr) -> Result<()> {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    let listener = tokio::net::TcpListener::bind(addr).await?;
    loop {
        let (mut stream, _) = listener.accept().await?;
        tokio::spawn(async move {
            let mut buf = [0u8; 1024];
            let n = tokio::time::timeout(Duration::from_secs(2), stream.read(&mut buf))
                .await
                .ok()
                .and_then(Result::ok)
                .unwrap_or(0);
            let head = String::from_utf8_lossy(&buf[..n]);
            let ok = head.starts_with("GET /metrics");
            let (status, body) = if ok {
                ("200 OK", render())
            } else {
                ("404 Not Found", String::new())
            };
            let reply = format!(
                "HTTP/1.1 {status}\r\nContent-Type: text/plain; version=0.0.4\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                body.len()
            );
            let _ = stream.write_all(reply.as_bytes()).await;
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn renders_gauges_counters_and_histograms() {
        gauge("t_active", 3.0);
        incr("t_fallback_total", &[("reason", "a b/c")]);
        observe("t_seconds", Duration::from_millis(30));
        let text = render();
        assert!(text.contains("t_active 3"));
        assert!(text.contains("t_fallback_total{reason=\"a_b_c\"} 1"));
        assert!(text.contains("t_seconds_bucket{le=\"0.025\"} 0"));
        assert!(text.contains("t_seconds_bucket{le=\"0.05\"} 1"));
        assert!(text.contains("t_seconds_count 1"));
    }

    #[test]
    fn gauge_family_drops_stale_series() {
        gauge_family("t_age", "station", &[("KGRR".into(), 5.0), ("KDTX".into(), 9.0)]);
        gauge_family("t_age", "station", &[("KGRR".into(), 7.0)]);
        let text = render();
        assert!(text.contains("t_age{station=\"KGRR\"} 7"));
        assert!(!text.contains("KDTX"));
    }
}
