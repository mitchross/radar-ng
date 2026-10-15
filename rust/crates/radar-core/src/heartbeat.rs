//! Keep a long blocking job alive while it runs: [`run_blocking_with_heartbeat`] (spec-shared §12).
//!
//! The Python version (`run_sync_with_heartbeat`) runs `func` in a thread and beats from the event loop
//! every `heartbeat_every` seconds. The first beat fires after one full interval, not at t=0. Beats stop
//! once the work finishes.
//!
//! Fix (D18): a failing beat never affects the result. The beat runs on its own task. Panics inside the beat
//! are caught and logged. The beat task is aborted when the work completes. The work's own panic is
//! propagated to the caller, as an exception would be.
//!
//! A worker can call [`exit_when_abandoned_after`] so that work which outlives its dropped caller (a cancelled
//! or timed-out activity) ends the process. The thread cannot be stopped, and it keeps its flocks and memory.

use std::panic::{AssertUnwindSafe, catch_unwind, resume_unwind};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, OnceLock};
use std::time::Duration;
use tokio::time::{Instant, MissedTickBehavior, interval_at};

static ABANDONED_EXIT_AFTER: OnceLock<Duration> = OnceLock::new();

/// Exit the process when blocking work is still running `grace` after its caller dropped the future.
///
/// Call once at worker startup. Without it, abandoned work keeps running until it returns.
pub fn exit_when_abandoned_after(grace: Duration) {
    let _ = ABANDONED_EXIT_AFTER.set(grace);
}

fn exit_abandoned(grace: Duration) {
    tracing::error!(
        grace_s = grace.as_secs(),
        "blocking work is still running after its caller was cancelled; exiting to release its locks and memory"
    );
    std::process::exit(1);
}

/// Run `f` on the blocking pool, calling `beat` about once every `interval` until it returns.
///
/// Returns `f`'s result. Panics in `f` propagate. Panics in `beat` are caught and logged, and never mask the
/// result. Dropping the returned future aborts the beat, but the blocking thread keeps running (threads are
/// not cancellable), as in Python, unless [`exit_when_abandoned_after`] is set.
pub async fn run_blocking_with_heartbeat<T, F, B>(f: F, interval: Duration, beat: B) -> T
where
    T: Send + 'static,
    F: FnOnce() -> T + Send + 'static,
    B: Fn() + Send + Sync + 'static,
{
    let abandon = ABANDONED_EXIT_AFTER
        .get()
        .map(|&grace| (grace, exit_abandoned as fn(Duration)));
    run_blocking_inner(f, interval, beat, abandon).await
}

async fn run_blocking_inner<T, F, B>(
    f: F,
    interval: Duration,
    beat: B,
    abandon: Option<(Duration, fn(Duration))>,
) -> T
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
        // Set on unwind too, so a panicking job never counts as abandoned.
        let _done = SetOnDrop(done_for_work);
        f()
    });
    let _watch = AbandonWatch {
        done: Arc::clone(&done),
        abandon,
    };
    match work.await {
        Ok(value) => value,
        Err(join_error) if join_error.is_panic() => resume_unwind(join_error.into_panic()),
        Err(join_error) => panic!("blocking heartbeat work was cancelled: {join_error}"),
    }
}

struct SetOnDrop(Arc<AtomicBool>);

impl Drop for SetOnDrop {
    fn drop(&mut self) {
        self.0.store(true, Ordering::Release);
    }
}

/// When the caller drops the future before the work finishes, waits up to the grace period on a plain
/// thread, then calls the handler if the work is still running.
struct AbandonWatch {
    done: Arc<AtomicBool>,
    abandon: Option<(Duration, fn(Duration))>,
}

impl Drop for AbandonWatch {
    fn drop(&mut self) {
        let Some((grace, on_stuck)) = self.abandon else {
            return;
        };
        if self.done.load(Ordering::Acquire) {
            return;
        }
        let done = Arc::clone(&self.done);
        let spawned = std::thread::Builder::new()
            .name("abandoned-work".into())
            .spawn(move || {
                let deadline = std::time::Instant::now() + grace;
                while std::time::Instant::now() < deadline {
                    if done.load(Ordering::Acquire) {
                        return;
                    }
                    std::thread::sleep(grace.min(Duration::from_secs(1)));
                }
                if !done.load(Ordering::Acquire) {
                    on_stuck(grace);
                }
            });
        if let Err(err) = spawned {
            tracing::warn!(error = %err, "could not watch abandoned blocking work");
        }
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

    static STUCK_CALLS: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
    static FINISHED_CALLS: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);

    #[tokio::test]
    async fn abandoned_work_that_outlives_the_grace_calls_the_handler() {
        let release = Arc::new(AtomicBool::new(false));
        let r = Arc::clone(&release);
        let fut = run_blocking_inner(
            move || {
                while !r.load(Ordering::Acquire) {
                    std::thread::sleep(Duration::from_millis(5));
                }
            },
            Duration::from_secs(60),
            || {},
            Some((Duration::from_millis(30), |_| {
                STUCK_CALLS.fetch_add(1, Ordering::SeqCst);
            })),
        );
        // Start the work, then drop the future as an activity cancellation would.
        let _ = tokio::time::timeout(Duration::from_millis(20), fut).await;
        tokio::time::sleep(Duration::from_millis(200)).await;
        release.store(true, Ordering::Release);
        assert_eq!(STUCK_CALLS.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn abandoned_work_that_finishes_within_the_grace_is_left_alone() {
        let fut = run_blocking_inner(
            || std::thread::sleep(Duration::from_millis(40)),
            Duration::from_secs(60),
            || {},
            Some((Duration::from_millis(500), |_| {
                FINISHED_CALLS.fetch_add(1, Ordering::SeqCst);
            })),
        );
        let _ = tokio::time::timeout(Duration::from_millis(10), fut).await;
        tokio::time::sleep(Duration::from_millis(700)).await;
        assert_eq!(FINISHED_CALLS.load(Ordering::SeqCst), 0);
    }

    #[tokio::test]
    async fn completed_and_panicked_work_never_counts_as_abandoned() {
        static CALLS: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
        let on_stuck: fn(Duration) = |_| {
            CALLS.fetch_add(1, Ordering::SeqCst);
        };
        let ok = run_blocking_inner(
            || 7,
            Duration::from_secs(60),
            || {},
            Some((Duration::from_millis(10), on_stuck)),
        )
        .await;
        assert_eq!(ok, 7);
        let panicked = tokio::spawn(run_blocking_inner(
            || -> i32 { panic!("work failed") },
            Duration::from_secs(60),
            || {},
            Some((Duration::from_millis(10), on_stuck)),
        ))
        .await;
        assert!(panicked.unwrap_err().is_panic());
        tokio::time::sleep(Duration::from_millis(100)).await;
        assert_eq!(CALLS.load(Ordering::SeqCst), 0);
    }
}
