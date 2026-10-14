//! Shared plumbing for activity bodies: heartbeat forwarding, cancellation, error construction.
//!
//! The sibling crates take plain `Fn(Value)` heartbeat closures and know nothing about Temporal.
//! [`json_heartbeat`] adapts them: each beat is pushed into an unbounded channel that a task spawned
//! on the activity's runtime drains into `record_heartbeat`. Sending never blocks the activity body.

use std::future::Future;
use std::sync::Arc;

use serde_json::Value;
use temporalio_sdk::ApplicationFailure;
use temporalio_sdk::activities::{ActivityContext, ActivityError};
use tokio::sync::mpsc;

/// Heartbeat closure shape shared by `radar_ingest::Heartbeat` and `radar_aux::Beat`.
pub type JsonBeat = Arc<dyn Fn(Value) + Send + Sync>;

/// Heartbeat closure for activities that beat without details (`nowcast_run`).
pub type UnitBeat = Arc<dyn Fn() + Send + Sync>;

/// Build a JSON heartbeat closure bound to `ctx`. Must be called from inside a Tokio runtime.
///
/// The forwarder task ends when the last clone of the closure is dropped, which is when the activity
/// body finishes. Heartbeat recording errors (cancellation already requested, or the attempt is gone)
/// are logged at debug level and otherwise ignored, matching Python's fire-and-forget `heartbeat()`.
pub fn json_heartbeat(ctx: &ActivityContext) -> JsonBeat {
    let (tx, mut rx) = mpsc::unbounded_channel::<Value>();
    let forwarder = ctx.clone();
    tokio::spawn(async move {
        while let Some(details) = rx.recv().await {
            if let Err(err) = forwarder.record_heartbeat(details).await {
                tracing::debug!(error = %err, "heartbeat not recorded");
            }
        }
    });
    Arc::new(move |details: Value| {
        let _ = tx.send(details);
    })
}

/// Heartbeat closure that sends `null` details. Used where the Python code heartbeats without data.
pub fn unit_heartbeat(ctx: &ActivityContext) -> UnitBeat {
    let beat = json_heartbeat(ctx);
    Arc::new(move || beat(Value::Null))
}

/// Run `fut` unless the activity is cancelled first. On cancellation the body is dropped at its next
/// await point and the activity reports `Cancelled`.
pub async fn with_cancel<T, E, F>(ctx: &ActivityContext, fut: F) -> Result<T, ActivityError>
where
    F: Future<Output = Result<T, E>>,
    ActivityError: From<E>,
{
    let watcher = ctx.clone();
    tokio::select! {
        biased;
        () = watcher.cancelled() => Err(ActivityError::cancelled()),
        result = fut => result.map_err(ActivityError::from),
    }
}

/// Map a Temporal activity context to the sibling crates' `ActivityInfo`, which keys scratch dirs.
pub fn activity_info(ctx: &ActivityContext) -> radar_ingest::ActivityInfo {
    let info = ctx.info();
    radar_ingest::ActivityInfo {
        workflow_id: info.workflow_id.clone().unwrap_or_default(),
        run_id: info.workflow_run_id.clone().unwrap_or_default(),
        activity_id: info.activity_id.clone(),
        attempt: info.attempt,
    }
}

/// A failure the workflow must not retry (Python `ApplicationError(non_retryable=True)`).
pub fn non_retryable(message: impl Into<String>) -> ActivityError {
    ActivityError::application(ApplicationFailure::non_retryable(message.into()))
}

/// Build the activity-level error for a `Result` that carries an error message and a retry flag.
/// `retryable = true` maps to a plain (retryable) failure.
pub fn classified(message: String, retryable: bool) -> ActivityError {
    if retryable {
        anyhow::anyhow!(message).into()
    } else {
        non_retryable(message)
    }
}
