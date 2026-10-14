//! Heartbeat plumbing. The worker passes a closure that forwards details to Temporal.

use std::future::Future;
use std::sync::Arc;
use std::time::Duration;

use serde_json::Value;
use tokio::time::{Instant, MissedTickBehavior, interval_at};

/// Sends heartbeat details. Called from the activity's event loop, never from blocking work directly.
pub type Heartbeat = Arc<dyn Fn(Value) + Send + Sync>;

/// A heartbeat that drops every beat (tests, benchmarks).
pub fn noop() -> Heartbeat {
    Arc::new(|_| {})
}

/// Drive `fut` while beating `details` every `period` (async equivalent of `run_sync_with_heartbeat`
/// for I/O). The first beat fires after one full period.
pub async fn during<F: Future>(
    period: Duration,
    hb: &Heartbeat,
    details: Value,
    fut: F,
) -> F::Output {
    let mut ticker = interval_at(Instant::now() + period, period);
    ticker.set_missed_tick_behavior(MissedTickBehavior::Delay);
    tokio::pin!(fut);
    loop {
        tokio::select! {
            out = &mut fut => return out,
            _ = ticker.tick() => hb(details.clone()),
        }
    }
}

/// Run blocking work with beats every `period` (wraps `radar_core::heartbeat`).
pub async fn blocking<T, F>(period: Duration, hb: &Heartbeat, details: Value, f: F) -> T
where
    T: Send + 'static,
    F: FnOnce() -> T + Send + 'static,
{
    let hb = Arc::clone(hb);
    radar_core::heartbeat::run_blocking_with_heartbeat(f, period, move || hb(details.clone())).await
}
