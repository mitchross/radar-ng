//! In-process counters and the `GET /api/metrics` text (spec 10.2).
//!
//! Counters that the Python code bumps by name are plain atomics. The HTTP counters are keyed by
//! `(method, route template, status)` and by `(method, route template)` for durations. Route
//! labels are the matched template (`/api/forecast/{lat}/{lon}`) or `unmatched`, never a raw URL.

use std::collections::BTreeMap;
use std::sync::Mutex;
use std::sync::atomic::{AtomicU64, Ordering};

/// Named counters in the order `/api/metrics` prints them.
#[derive(Debug, Default)]
pub struct Counters {
    /// `forecast_requests_total`, counted before validation.
    pub forecast_requests_total: AtomicU64,
    /// `forecast_cache_hits_total`.
    pub forecast_cache_hits_total: AtomicU64,
    /// `forecast_upstream_errors_total`.
    pub forecast_upstream_errors_total: AtomicU64,
    /// `manifest_requests_total`.
    pub manifest_requests_total: AtomicU64,
    /// `nowcast_point_requests_total`.
    pub nowcast_point_requests_total: AtomicU64,
    /// `nowcast_point_cache_hits_total`.
    pub nowcast_point_cache_hits_total: AtomicU64,
    /// `rate_limited_requests_total`.
    pub rate_limited_requests_total: AtomicU64,
}

impl Counters {
    fn named(&self) -> [(&'static str, u64); 7] {
        [
            (
                "forecast_requests_total",
                self.forecast_requests_total.load(Ordering::Relaxed),
            ),
            (
                "forecast_cache_hits_total",
                self.forecast_cache_hits_total.load(Ordering::Relaxed),
            ),
            (
                "forecast_upstream_errors_total",
                self.forecast_upstream_errors_total.load(Ordering::Relaxed),
            ),
            (
                "manifest_requests_total",
                self.manifest_requests_total.load(Ordering::Relaxed),
            ),
            (
                "nowcast_point_requests_total",
                self.nowcast_point_requests_total.load(Ordering::Relaxed),
            ),
            (
                "nowcast_point_cache_hits_total",
                self.nowcast_point_cache_hits_total.load(Ordering::Relaxed),
            ),
            (
                "rate_limited_requests_total",
                self.rate_limited_requests_total.load(Ordering::Relaxed),
            ),
        ]
    }
}

/// Everything the registry holds.
#[derive(Debug, Default)]
pub struct Metrics {
    /// Named counters.
    pub counters: Counters,
    http: Mutex<HttpStats>,
}

#[derive(Debug, Default)]
struct HttpStats {
    counts: BTreeMap<(String, String, u16), u64>,
    durations: BTreeMap<(String, String), f64>,
}

/// Inputs that come from application state at scrape time.
#[derive(Debug, Default)]
pub struct Snapshot {
    /// `(layer name, frame count)` for every dict layer, sorted by name.
    pub tile_timestamps: Vec<(String, usize)>,
    /// Age of the newest radar frame in seconds, when known.
    pub mrms_age_s: Option<i64>,
    /// Lines from `radar_core::nowcast_skill::prometheus_lines`.
    pub skill_lines: Vec<String>,
}

impl Metrics {
    /// Record one finished request. Called after the handler, so a scrape never counts itself.
    pub fn record(&self, method: &str, route: &str, status: u16, elapsed_secs: f64) {
        let mut http = self
            .http
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        *http
            .counts
            .entry((method.to_string(), route.to_string(), status))
            .or_insert(0) += 1;
        *http
            .durations
            .entry((method.to_string(), route.to_string()))
            .or_insert(0.0) += elapsed_secs;
    }

    /// The exact `/api/metrics` body: `"\n".join(lines) + "\n"`.
    #[must_use]
    pub fn render(&self, snap: &Snapshot) -> String {
        let mut lines: Vec<String> = Vec::new();
        for (name, value) in self.counters.named() {
            lines.push(format!("# TYPE radar_ng_{name} counter"));
            lines.push(format!("radar_ng_{name} {value}"));
        }
        lines.push("# TYPE radar_ng_tile_timestamps gauge".into());
        for (layer, count) in &snap.tile_timestamps {
            lines.push(format!(
                "radar_ng_tile_timestamps{{layer=\"{layer}\"}} {count}"
            ));
        }
        if let Some(age) = snap.mrms_age_s {
            lines.push("# TYPE radar_ng_mrms_age_seconds gauge".into());
            lines.push(format!("radar_ng_mrms_age_seconds {age}"));
        }
        lines.extend(snap.skill_lines.iter().cloned());

        let http = self
            .http
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        lines.push("# TYPE radar_ng_http_requests_total counter".into());
        for ((method, path, status), count) in &http.counts {
            lines.push(format!(
                "radar_ng_http_requests_total{{method=\"{method}\",path=\"{path}\",status=\"{status}\"}} {count}"
            ));
        }
        lines.push("# TYPE radar_ng_http_request_duration_seconds_sum counter".into());
        for ((method, path), duration) in &http.durations {
            lines.push(format!(
                "radar_ng_http_request_duration_seconds_sum{{method=\"{method}\",path=\"{path}\"}} {duration:.6}"
            ));
        }
        let mut body = lines.join("\n");
        body.push('\n');
        body
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn counter_block_order_and_format() {
        let m = Metrics::default();
        m.counters
            .rate_limited_requests_total
            .fetch_add(2, Ordering::Relaxed);
        let text = m.render(&Snapshot::default());
        assert!(text.starts_with(
            "# TYPE radar_ng_forecast_requests_total counter\nradar_ng_forecast_requests_total 0\n"
        ));
        assert!(text.contains("radar_ng_rate_limited_requests_total 2\n"));
        assert!(text.ends_with('\n'));
    }

    #[test]
    fn http_lines_are_sorted_and_durations_use_six_decimals() {
        let m = Metrics::default();
        m.record("GET", "/api/livez", 200, 0.0012);
        m.record("GET", "/api/livez", 200, 0.0003);
        m.record("GET", "unmatched", 404, 0.5);
        let text = m.render(&Snapshot::default());
        let a = text
            .find("path=\"/api/livez\",status=\"200\"} 2")
            .expect("count line");
        let b = text
            .find("path=\"unmatched\",status=\"404\"} 1")
            .expect("count line");
        assert!(a < b);
        assert!(text.contains("radar_ng_http_request_duration_seconds_sum{method=\"GET\",path=\"/api/livez\"} 0.001500\n"));
    }

    #[test]
    fn tile_gauge_and_age_lines() {
        let m = Metrics::default();
        let snap = Snapshot {
            tile_timestamps: vec![("radar".into(), 3)],
            mrms_age_s: Some(42),
            skill_lines: vec!["radar_ng_nowcast_skill_runs 8".into()],
        };
        let text = m.render(&snap);
        assert!(text.contains("radar_ng_tile_timestamps{layer=\"radar\"} 3\n"));
        assert!(text.contains("radar_ng_mrms_age_seconds 42\n"));
        assert!(text.contains("radar_ng_nowcast_skill_runs 8\n"));
    }
}
