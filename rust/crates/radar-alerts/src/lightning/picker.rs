//! Endpoint selection and reconnect backoff (spec-ingest-aux section 6.3, Python `EndpointPicker`).
//!
//! Pure bookkeeping. The caller supplies `now` and the jitter value, so every rule can be tested without sockets.

use std::collections::HashMap;
use std::time::{Duration, Instant};

use anyhow::ensure;

/// Delay after a session that delivered at least one message (Python `RECONNECT_AFTER_CLOSE_S`).
pub const RECONNECT_AFTER_CLOSE_S: f64 = 1.0;
/// Upper bound of the failure backoff (`FAILURE_BACKOFF_MAX_S`).
pub const FAILURE_BACKOFF_MAX_S: f64 = 30.0;
/// Demotion of a failed primary endpoint (`DEMOTE_PRIMARY_S`).
pub const DEMOTE_PRIMARY_S: f64 = 60.0;
/// Demotion of a failed legacy endpoint (`DEMOTE_LEGACY_S`).
pub const DEMOTE_LEGACY_S: f64 = 900.0;

/// Round-robin over healthy primaries, with legacy endpoints only when no primary is healthy.
///
/// `cursor` is global and is never reset, so the modulo index matches Python exactly (spec 6.3).
#[derive(Debug, Clone)]
pub struct EndpointPicker {
    primary: Vec<String>,
    legacy: Vec<String>,
    demoted_until: HashMap<String, Instant>,
    cursor: usize,
    consecutive_failures: u32,
}

impl EndpointPicker {
    /// New picker. Fails when there is no primary endpoint (Python `ValueError`).
    pub fn new(primary: Vec<String>, legacy: Vec<String>) -> anyhow::Result<Self> {
        ensure!(
            !primary.is_empty(),
            "at least one primary endpoint is required"
        );
        Ok(Self {
            primary,
            legacy,
            demoted_until: HashMap::new(),
            cursor: 0,
            consecutive_failures: 0,
        })
    }

    /// Failures since the last session that delivered a message.
    #[must_use]
    pub fn consecutive_failures(&self) -> u32 {
        self.consecutive_failures
    }

    fn healthy(&self, urls: &[String], now: Instant) -> Vec<String> {
        urls.iter()
            .filter(|u| {
                self.demoted_until
                    .get(u.as_str())
                    .is_none_or(|until| *until <= now)
            })
            .cloned()
            .collect()
    }

    /// The next endpoint to try. Never stalls: when everything is demoted, the demotions are forgotten.
    pub fn next(&mut self, now: Instant) -> String {
        let mut candidates = self.healthy(&self.primary, now);
        if candidates.is_empty() {
            candidates = self.healthy(&self.legacy, now);
        }
        if candidates.is_empty() {
            self.demoted_until.clear();
            candidates = self.primary.clone();
        }
        let url = candidates[self.cursor % candidates.len()].clone();
        self.cursor += 1;
        url
    }

    /// Record a session that delivered data. Clears the failure streak and the demotion. Returns the delay before
    /// reconnecting, in seconds.
    pub fn succeeded(&mut self, url: &str) -> f64 {
        self.consecutive_failures = 0;
        self.demoted_until.remove(url);
        RECONNECT_AFTER_CLOSE_S
    }

    /// Record a session that delivered nothing, at time `now`. `rand` is a uniform value in `[0, 1)`. Returns the
    /// backoff in seconds: `min(30, 2^n) * (0.5 + 0.5 * rand)` for failure count `n`.
    pub fn failed(&mut self, url: &str, now: Instant, rand: f64) -> f64 {
        self.consecutive_failures += 1;
        let demote = if self.primary.iter().any(|u| u == url) {
            DEMOTE_PRIMARY_S
        } else {
            DEMOTE_LEGACY_S
        };
        self.demoted_until
            .insert(url.to_owned(), now + Duration::from_secs_f64(demote));
        let exponent = self.consecutive_failures.min(8) as i32;
        let base = 2f64.powi(exponent).min(FAILURE_BACKOFF_MAX_S);
        base * (0.5 + rand * 0.5)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn picker() -> EndpointPicker {
        EndpointPicker::new(
            crate::lightning::primary_endpoints(),
            crate::lightning::legacy_endpoints(),
        )
        .unwrap()
    }

    #[test]
    fn requires_a_primary_endpoint() {
        assert!(EndpointPicker::new(vec![], vec!["ws://x/".into()]).is_err());
    }

    #[test]
    fn round_robins_healthy_primaries_with_rand_half() {
        let mut p = EndpointPicker::new(vec!["a".into(), "b".into()], vec![]).unwrap();
        let now = Instant::now();
        let picks: Vec<String> = (0..4).map(|_| p.next(now)).collect();
        assert_eq!(picks, ["a", "b", "a", "b"]);
    }

    #[test]
    fn spec_delay_table_at_rand_half() {
        let mut p = picker();
        let now = Instant::now();
        let url = "wss://ws1.blitzortung.org:443/";
        let got: Vec<f64> = (0..9).map(|_| p.failed(url, now, 0.5)).collect();
        assert_eq!(got, vec![1.5, 3.0, 6.0, 12.0, 22.5, 22.5, 22.5, 22.5, 22.5]);
    }

    #[test]
    fn delay_is_bounded_by_base_and_grows() {
        let mut p = picker();
        let now = Instant::now();
        let url = "wss://ws1.blitzortung.org:443/";
        let first = p.failed(url, now, 0.999_999);
        let second = p.failed(url, now, 0.999_999);
        assert!(second > first && first > 0.0);
        assert!(second <= 30.0);
        let mut last = 0.0;
        for _ in 0..12 {
            last = p.failed(url, now, 0.999_999);
            assert!(last <= FAILURE_BACKOFF_MAX_S);
        }
        assert!(last >= first);
    }

    #[test]
    fn failed_primary_is_skipped_and_the_global_cursor_picks_the_next_healthy_one() {
        let mut p = picker();
        let now = Instant::now();
        let a = "wss://ws1.blitzortung.org:443/";
        assert_eq!(p.next(now), a);
        p.failed(a, now, 0.5);
        // Cursor is 1 and the healthy list is [ws2, ws7, ws8], so index 1 is ws7 (matches the golden failover).
        assert_eq!(p.next(now), "wss://ws7.blitzortung.org:443/");
    }

    #[test]
    fn legacy_endpoints_are_used_only_when_every_primary_is_demoted() {
        let mut p = picker();
        let now = Instant::now();
        for url in crate::lightning::primary_endpoints() {
            p.failed(&url, now, 0.5);
        }
        let pick = p.next(now);
        assert!(pick.starts_with("ws://"), "got {pick}");
    }

    #[test]
    fn never_stalls_when_primaries_and_legacy_are_all_demoted() {
        let mut p = picker();
        let now = Instant::now();
        for url in crate::lightning::primary_endpoints()
            .into_iter()
            .chain(crate::lightning::legacy_endpoints())
        {
            p.failed(&url, now, 0.5);
        }
        let pick = p.next(now);
        assert!(
            crate::lightning::primary_endpoints().contains(&pick),
            "got {pick}"
        );
    }

    #[test]
    fn demotion_expires_after_sixty_seconds_for_primaries() {
        let mut p = picker();
        let t0 = Instant::now();
        let ws1 = "wss://ws1.blitzortung.org:443/";
        p.failed(ws1, t0, 0.5);
        let mut picks = Vec::new();
        for _ in 0..4 {
            picks.push(p.next(t0 + Duration::from_secs(59)));
        }
        assert!(!picks.iter().any(|u| u == ws1), "still demoted at 59 s");
        let later: Vec<String> = (0..4)
            .map(|_| p.next(t0 + Duration::from_secs(61)))
            .collect();
        assert!(later.iter().any(|u| u == ws1), "eligible again after 60 s");
    }

    #[test]
    fn success_returns_one_second_and_resets_failures_and_demotion() {
        let mut p = picker();
        let now = Instant::now();
        let ws1 = "wss://ws1.blitzortung.org:443/";
        p.failed(ws1, now, 0.5);
        p.failed(ws1, now, 0.5);
        assert_eq!(p.consecutive_failures(), 2);
        assert_eq!(p.succeeded(ws1), 1.0);
        assert_eq!(p.consecutive_failures(), 0);
        let picks: Vec<String> = (0..4).map(|_| p.next(now)).collect();
        assert!(
            picks.iter().any(|u| u == ws1),
            "healed endpoint is eligible again"
        );
    }
}
