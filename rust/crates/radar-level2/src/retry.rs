//! Retries an async step with capped exponential backoff. `radar-api` uses it so a Level II
//! database that comes up after the API (cold start) is picked up instead of disabling Level II
//! until the pod restarts.

use std::{future::Future, time::Duration};

#[derive(Clone, Copy, Debug)]
pub struct Backoff {
    pub initial: Duration,
    pub cap: Duration,
}

impl Backoff {
    /// 1 s, doubling, capped at 30 s.
    pub const STARTUP: Self = Self {
        initial: Duration::from_secs(1),
        cap: Duration::from_secs(30),
    };

    /// The wait after failed attempt `n` (0-based).
    #[must_use]
    pub fn delay(&self, n: u32) -> Duration {
        self.initial
            .saturating_mul(2u32.saturating_pow(n))
            .min(self.cap)
    }
}

/// Runs `step` until it succeeds or fails with an error `transient` rejects. Transient errors are
/// retried forever; `on_retry(attempt, error, wait)` is called before each wait (attempt is 1-based).
pub async fn retry<T, E, F, Fut>(
    backoff: Backoff,
    mut step: F,
    transient: impl Fn(&E) -> bool,
    mut on_retry: impl FnMut(u32, &E, Duration),
) -> Result<T, E>
where
    F: FnMut() -> Fut,
    Fut: Future<Output = Result<T, E>>,
{
    let mut failures = 0u32;
    loop {
        match step().await {
            Ok(v) => return Ok(v),
            Err(e) if transient(&e) => {
                let wait = backoff.delay(failures);
                failures = failures.saturating_add(1);
                on_retry(failures, &e, wait);
                tokio::time::sleep(wait).await;
            }
            Err(e) => return Err(e),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::Cell;

    #[derive(Debug, PartialEq)]
    enum Fail {
        Transient,
        Config,
    }

    fn secs(v: &[u64]) -> Vec<Duration> {
        v.iter().copied().map(Duration::from_secs).collect()
    }

    #[test]
    fn delay_doubles_then_caps() {
        let b = Backoff::STARTUP;
        let got: Vec<_> = (0..8).map(|n| b.delay(n)).collect();
        assert_eq!(got, secs(&[1, 2, 4, 8, 16, 30, 30, 30]));
        assert_eq!(b.delay(u32::MAX), Duration::from_secs(30));
    }

    #[tokio::test(start_paused = true)]
    async fn retries_transient_until_success_with_capped_waits() {
        let calls = Cell::new(0u32);
        let mut waits = Vec::new();
        let start = tokio::time::Instant::now();
        let out = retry(
            Backoff::STARTUP,
            || {
                calls.set(calls.get() + 1);
                let n = calls.get();
                async move { if n <= 7 { Err(Fail::Transient) } else { Ok(n) } }
            },
            |e| *e == Fail::Transient,
            |_, _, wait| waits.push(wait),
        )
        .await;
        assert_eq!(out, Ok(8));
        assert_eq!(calls.get(), 8);
        assert_eq!(waits, secs(&[1, 2, 4, 8, 16, 30, 30]));
        assert_eq!(start.elapsed(), Duration::from_secs(91));
    }

    #[tokio::test(start_paused = true)]
    async fn config_error_fails_fast() {
        let calls = Cell::new(0u32);
        let start = tokio::time::Instant::now();
        let out: Result<(), _> = retry(
            Backoff::STARTUP,
            || {
                calls.set(calls.get() + 1);
                async { Err(Fail::Config) }
            },
            |e| *e == Fail::Transient,
            |_, _, _| panic!("config errors must not be retried"),
        )
        .await;
        assert_eq!(out, Err(Fail::Config));
        assert_eq!(calls.get(), 1);
        assert_eq!(start.elapsed(), Duration::ZERO);
    }

    #[tokio::test(start_paused = true)]
    async fn transient_then_config_stops() {
        let calls = Cell::new(0u32);
        let out: Result<(), _> = retry(
            Backoff::STARTUP,
            || {
                calls.set(calls.get() + 1);
                let n = calls.get();
                async move { Err(if n < 3 { Fail::Transient } else { Fail::Config }) }
            },
            |e| *e == Fail::Transient,
            |_, _, _| {},
        )
        .await;
        assert_eq!(out, Err(Fail::Config));
        assert_eq!(calls.get(), 3);
    }

    #[tokio::test]
    async fn only_unreachable_database_is_transient() {
        use crate::leases::{Pool, is_transient};
        // Port 1 on loopback refuses at once: the "Postgres not up yet" case.
        let down = Pool::connect("postgres://u:p@127.0.0.1:1/db", "level2_numeric").await;
        assert!(is_transient(&down.err().expect("refused")));
        let bad_url = Pool::connect("postgres://u@h:notaport/db", "level2_numeric").await;
        assert!(!is_transient(&bad_url.err().expect("bad url")));
        let bad_schema = Pool::connect("postgres://u:p@127.0.0.1:1/db", "public").await;
        assert!(!is_transient(&bad_schema.err().expect("bad schema")));
        assert!(!is_transient(&anyhow::anyhow!("LEVEL2_S3_BUCKET required")));
    }
}
