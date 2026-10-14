//! Per-client token bucket, the exact algorithm of spec 4.2.
//!
//! Constants: `RPS` (default 20) and `BURST` (default 60). State is an LRU map
//! `client -> (tokens, last_update)` capped at 4096 entries. A bucket that is not
//! present starts full. `/api/livez` is exempt: it is neither limited nor debited.
//! A limited request does not consume a token. The debit is stored before the handler runs.

use std::num::NonZeroUsize;
use std::time::Instant;

use lru::LruCache;

/// Maximum number of tracked clients (`_rate_buckets` capacity).
pub const CAPACITY: usize = 4096;

/// Token-bucket limiter keyed by client address.
#[derive(Debug)]
pub struct RateLimiter {
    rps: f64,
    burst: f64,
    store: LruCache<String, (f64, Instant)>,
}

impl RateLimiter {
    /// A limiter with `rps` tokens per second and a bucket of `burst` tokens.
    #[must_use]
    pub fn new(rps: f64, burst: f64) -> Self {
        Self {
            rps,
            burst,
            store: LruCache::new(NonZeroUsize::new(CAPACITY).expect("non-zero capacity")),
        }
    }

    /// Account one request from `client` at time `now`.
    ///
    /// Returns `true` when the request may proceed (a token was spent, unless `exempt`), and
    /// `false` when it must be answered with 429.
    pub fn admit(&mut self, client: &str, now: Instant, exempt: bool) -> bool {
        let (mut tokens, updated) = self.store.pop(client).unwrap_or((self.burst, now));
        let elapsed = now.saturating_duration_since(updated).as_secs_f64();
        tokens = (tokens + elapsed * self.rps).min(self.burst);
        let limited = !exempt && tokens < 1.0;
        if !exempt && !limited {
            tokens -= 1.0;
        }
        self.store.put(client.to_string(), (tokens, now));
        !limited
    }

    /// Number of clients currently tracked.
    #[must_use]
    pub fn len(&self) -> usize {
        self.store.len()
    }

    /// True when no client is tracked.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.store.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    #[test]
    fn burst_then_429_with_zero_refill() {
        // Spec 4.2 [V]: burst 3, rps 0 gives 200, 200, 200, 429, 429.
        let mut rl = RateLimiter::new(0.0, 3.0);
        let t0 = Instant::now();
        let results: Vec<bool> = (0..5).map(|_| rl.admit("c", t0, false)).collect();
        assert_eq!(results, vec![true, true, true, false, false]);
    }

    #[test]
    fn livez_is_exempt_and_not_debited() {
        let mut rl = RateLimiter::new(0.0, 1.0);
        let t0 = Instant::now();
        for _ in 0..5 {
            assert!(rl.admit("c", t0, true));
        }
        // The bucket is still full for a normal request.
        assert!(rl.admit("c", t0, false));
        assert!(!rl.admit("c", t0, false));
    }

    #[test]
    fn refill_is_rps_times_elapsed_capped_at_burst() {
        let mut rl = RateLimiter::new(2.0, 3.0);
        let t0 = Instant::now();
        for _ in 0..3 {
            assert!(rl.admit("c", t0, false));
        }
        assert!(!rl.admit("c", t0, false));
        // Half a second at 2 rps refills exactly one token.
        let t1 = t0 + Duration::from_millis(500);
        assert!(rl.admit("c", t1, false));
        assert!(!rl.admit("c", t1, false));
        // A long pause refills to the burst, not beyond it.
        let t2 = t1 + Duration::from_secs(3600);
        let allowed = (0..10).filter(|_| rl.admit("c", t2, false)).count();
        assert_eq!(allowed, 3);
    }

    #[test]
    fn clients_are_independent() {
        let mut rl = RateLimiter::new(0.0, 1.0);
        let t0 = Instant::now();
        assert!(rl.admit("a", t0, false));
        assert!(!rl.admit("a", t0, false));
        assert!(rl.admit("b", t0, false));
    }

    #[test]
    fn store_is_capped_at_capacity() {
        let mut rl = RateLimiter::new(20.0, 60.0);
        let t0 = Instant::now();
        for i in 0..(CAPACITY + 100) {
            rl.admit(&format!("client-{i}"), t0, false);
        }
        assert_eq!(rl.len(), CAPACITY);
    }

    #[test]
    fn evicted_client_starts_full_again() {
        let mut rl = RateLimiter::new(0.0, 1.0);
        let t0 = Instant::now();
        assert!(rl.admit("victim", t0, false));
        assert!(!rl.admit("victim", t0, false));
        for i in 0..CAPACITY {
            rl.admit(&format!("flood-{i}"), t0, false);
        }
        assert!(rl.admit("victim", t0, false));
    }
}
