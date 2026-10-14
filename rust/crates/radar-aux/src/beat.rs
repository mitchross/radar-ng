//! Heartbeat plumbing shared by the long activities.
//!
//! A [`Beat`] receives the heartbeat details (a JSON value, as Python sends a dict). The worker crate
//! turns it into `activity.heartbeat(details)`. Tests record the calls.

use std::future::Future;
use std::sync::Arc;
use std::time::Duration;

use serde_json::Value;
use tokio::time::{Instant, MissedTickBehavior, interval_at};

/// Heartbeat sink. Called with the details of the current phase, possibly from another thread.
pub type Beat = Arc<dyn Fn(Value) + Send + Sync>;

/// A [`Beat`] that drops every heartbeat. Used where no activity context exists.
pub fn no_beat() -> Beat {
    Arc::new(|_| {})
}

/// Drive `fut` to completion while calling `beat` about every `every`.
///
/// This is the async counterpart of `run_sync_with_heartbeat`. The first beat fires one full interval
/// after the call, as in the Python. Beats stop when `fut` completes. Dropping the returned future drops
/// `fut`.
pub async fn with_heartbeats<F, B>(fut: F, every: Duration, beat: B) -> F::Output
where
    F: Future,
    B: Fn() + Send + Sync,
{
    let every = every.max(Duration::from_millis(1));
    tokio::pin!(fut);
    let mut ticker = interval_at(Instant::now() + every, every);
    ticker.set_missed_tick_behavior(MissedTickBehavior::Delay);
    loop {
        tokio::select! {
            out = &mut fut => return out,
            _ = ticker.tick() => beat(),
        }
    }
}
