//! Keep a long blocking job alive while it runs: [`run_blocking_with_heartbeat`] (spec-shared §12).
//!
//! The Python version (`run_sync_with_heartbeat`) runs `func` in a thread and beats from the event loop
//! every `heartbeat_every` seconds. The first beat fires after one full interval, not at t=0. Beats stop
//! once the work finishes.
//!
//! Fix (D18): a failing beat never affects the result. The beat runs on its own task. Panics inside the beat
//! are caught and logged. The beat task is aborted when the work completes. The work's own panic is
//! propagated to the caller, as an exception would be.

use std::panic::{AssertUnwindSafe, catch_unwind, resume_unwind};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;
use tokio::time::{Instant, MissedTickBehavior, interval_at};

/// Run `f` on the blocking pool, calling `beat` about once every `interval` until it returns.
///
/// Returns `f`'s result. Panics in `f` propagate. Panics in `beat` are caught and logged, and never mask the
/// result. Dropping the returned future aborts the beat, but the blocking thread keeps running (threads are
/// not cancellable), as in Python.
pub async fn run_blocking_with_heartbeat<T, F, B>(f: F, interval: Duration, beat: B) -> T
where
    T: Send + 'static,
    F: FnOnce() -> T + Send + 'static,
    B: Fn() + Send + Sync + 'static,
{
    let interval = interval.max(Duration::from_millis(1));
    let beat = Arc::new(beat);
    let done = Arc::new(AtomicBool::new(false));

    let beat_task = {
        let beat = Arc::clone(&beat);
        let done = Arc::clone(&done);
        tokio::spawn(async move {
            let mut ticker = interval_at(Instant::now() + interval, interval);
            ticker.set_missed_tick_behavior(MissedTickBehavior::Delay);
            loop {
                ticker.tick().await;
                if done.load(Ordering::Acquire) {
                    break;
                }
                if catch_unwind(AssertUnwindSafe(|| beat())).is_err() {
                    tracing::warn!("heartbeat callback panicked; continuing");
                }
            }
        })
    };
    let _abort_beat = AbortOnDrop(beat_task.abort_handle());

    let done_for_work = Arc::clone(&done);
    let work = tokio::task::spawn_blocking(move || {
        let out = f();
        done_for_work.store(true, Ordering::Release);
        out
    });
    match work.await {
        Ok(value) => value,
        Err(join_error) if join_error.is_panic() => resume_unwind(join_error.into_panic()),
        Err(join_error) => panic!("blocking heartbeat work was cancelled: {join_error}"),
    }
}

/// Aborts the beat task when dropped, so cancellation of the caller stops the beats.
struct AbortOnDrop(tokio::task::AbortHandle);

impl Drop for AbortOnDrop {
    fn drop(&mut self) {
        self.0.abort();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    #[tokio::test]
    async fn hbt_01_beats_while_work_runs_and_first_beat_has_expected_details() {
        let beats: Arc<Mutex<Vec<serde_json::Value>>> = Arc::new(Mutex::new(Vec::new()));
        let sink = Arc::clone(&beats);
        let result = run_blocking_with_heartbeat(
            || {
                std::thread::sleep(Duration::from_millis(60));
                "done"
            },
            Duration::from_millis(10),
            move || {
                sink.lock()
                    .unwrap()
                    .push(serde_json::json!({"phase": "test"}))
            },
        )
        .await;
        assert_eq!(result, "done");
        let recorded = beats.lock().unwrap().clone();
        assert!(
            !recorded.is_empty(),
            "at least one heartbeat while the thread runs"
        );
        assert_eq!(recorded[0], serde_json::json!({"phase": "test"}));
    }

    #[tokio::test]
    async fn no_beat_is_delivered_after_completion() {
        let count = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let c = Arc::clone(&count);
        let _ = run_blocking_with_heartbeat(
            || 1,
            Duration::from_millis(5),
            move || {
                c.fetch_add(1, Ordering::SeqCst);
            },
        )
        .await;
        let after_return = count.load(Ordering::SeqCst);
        tokio::time::sleep(Duration::from_millis(40)).await;
        assert_eq!(
            count.load(Ordering::SeqCst),
            after_return,
            "beat task aborted on completion"
        );
    }

    #[tokio::test]
    async fn d18_panicking_beat_does_not_mask_the_result() {
        let result = run_blocking_with_heartbeat(
            || {
                std::thread::sleep(Duration::from_millis(30));
                42
            },
            Duration::from_millis(5),
            || panic!("transient heartbeat failure"),
        )
        .await;
        assert_eq!(result, 42);
    }

    #[tokio::test]
    async fn work_panic_propagates_to_caller() {
        let handle = tokio::spawn(run_blocking_with_heartbeat(
            || -> i32 { panic!("work failed") },
            Duration::from_millis(5),
            || {},
        ));
        let err = handle.await.unwrap_err();
        assert!(err.is_panic());
    }
}
