//! `lightning_consume_stream`: the Blitzortung websocket consumer (spec-ingest-aux section 6).
//!
//! One long-running call consumes the feed for `duration` (about 50 minutes in production), publishes
//! `lightning.json` atomically, and returns counters. A background task beats every 30 s, which prunes, flushes
//! and reports stats through the caller's heartbeat callback. Cancellation is cooperative through a
//! [`CancellationToken`]. A cancelled run returns [`LightningError::Cancelled`] without a final flush, as the
//! Python `CancelledError` path does.

mod config;
mod decode;
mod picker;
mod session;
mod store;

use std::sync::{Arc, Mutex};
use std::time::Duration;

use anyhow::Context;
use radar_core::time::round_digits;
use radar_types::payloads::lightning::LightningRunResult;
use serde::Serialize;
use tokio::task::JoinHandle;
use tokio::time::{Instant as TokioInstant, MissedTickBehavior};
use tokio_util::sync::CancellationToken;

pub use config::{
    BoundingBox, LightningConfig, OUT_FILE, legacy_endpoints, parse_endpoint_override,
    primary_endpoints,
};
pub use decode::{
    FrameError, MAX_DECODED_BYTES, MAX_DICTIONARY_BYTES, decode_frame, decode_payload,
    extract_strike,
};
pub use picker::{
    DEMOTE_LEGACY_S, DEMOTE_PRIMARY_S, EndpointPicker, FAILURE_BACKOFF_MAX_S,
    RECONNECT_AFTER_CLOSE_S,
};
pub use session::{MAX_MESSAGE_BYTES, OPEN_TIMEOUT, PING_INTERVAL, PING_TIMEOUT, SUBSCRIBE_FRAME};
pub use store::{Strike, StrikeBuffer, load_existing_strikes, render_geojson, write_geojson};

use crate::{now_epoch, unit_random};

/// Heartbeat and flush cadence in production (`_beat` sleeps 30 s). See [`LightningConfig::heartbeat_every`].
pub const HEARTBEAT_EVERY: Duration = Duration::from_secs(30);

/// Error from [`lightning_consume_stream`].
#[derive(Debug, thiserror::Error)]
pub enum LightningError {
    /// The cancellation token fired. The worker should report a cancelled activity.
    #[error("lightning stream cancelled")]
    Cancelled,
    /// A file, task or configuration problem that Python would raise as an exception.
    #[error(transparent)]
    Other(#[from] anyhow::Error),
}

/// Counters reported in every heartbeat. The key names match the Python `stats` dict.
#[derive(Debug, Default, Clone, Copy, Serialize)]
pub(crate) struct Stats {
    msgs: i64,
    parsed: i64,
    in_bbox: i64,
    buffer: i64,
    seeded: i64,
}

/// State shared by the session loop and the heartbeat task.
pub(crate) struct Shared {
    pub(crate) config: LightningConfig,
    pub(crate) buffer: Mutex<StrikeBuffer>,
    stats: Mutex<Stats>,
}

impl Shared {
    fn new(config: LightningConfig) -> Self {
        let cap = config.max_strikes;
        Self {
            config,
            buffer: Mutex::new(StrikeBuffer::new(cap)),
            stats: Mutex::new(Stats::default()),
        }
    }

    pub(crate) fn with_stats(&self, update: impl FnOnce(&mut Stats)) {
        let mut stats = self
            .stats
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        update(&mut stats);
    }

    fn stats(&self) -> Stats {
        *self
            .stats
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    /// Prune the buffer and record its size. Runs on every beat, as in `_beat`.
    fn prune_and_count(&self) {
        let len = {
            let mut buffer = self
                .buffer
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            prune_buffer(&mut buffer, self.config.retention_min);
            buffer.len()
        };
        self.with_stats(|s| s.buffer = len as i64);
    }
}

/// Drop strikes older than the retention window (Python `_prune`).
pub(crate) fn prune_buffer(buffer: &mut StrikeBuffer, retention_min: i64) {
    buffer.prune_before(now_epoch() - retention_min as f64 * 60.0);
}

/// Publish the current buffer to `lightning.json` on a blocking thread. The snapshot is taken under the lock,
/// so the write itself does not block the reader or the heartbeat.
pub(crate) async fn flush(shared: &Shared) -> anyhow::Result<()> {
    let snapshot = {
        let buffer = shared
            .buffer
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        buffer.snapshot()
    };
    let path = shared.config.out_path();
    let retention = shared.config.retention_min;
    tokio::task::spawn_blocking(move || write_geojson(&path, &snapshot, retention))
        .await
        .context("lightning flush task")?
        .context("writing lightning.json")
}

/// Aborts the heartbeat task when dropped, so every exit path stops the beats.
struct BeatGuard(JoinHandle<()>);

impl Drop for BeatGuard {
    fn drop(&mut self) {
        self.0.abort();
    }
}

fn spawn_beat<H>(shared: Arc<Shared>, heartbeat: Arc<H>, cancel: CancellationToken) -> BeatGuard
where
    H: Fn(serde_json::Value) + Send + Sync + 'static,
{
    let every = shared.config.heartbeat_every;
    let handle = tokio::spawn(async move {
        let mut ticker = tokio::time::interval_at(TokioInstant::now() + every, every);
        ticker.set_missed_tick_behavior(MissedTickBehavior::Delay);
        loop {
            tokio::select! {
                _ = cancel.cancelled() => break,
                _ = ticker.tick() => {}
            }
            shared.prune_and_count();
            if let Err(err) = flush(&shared).await {
                // A write error must not stop heartbeats (Python logs geojson_flush_failed).
                tracing::warn!(event = "geojson_flush_failed", error = %format!("{err:#}"));
            }
            let details = serde_json::to_value(shared.stats()).unwrap_or(serde_json::Value::Null);
            (*heartbeat)(details);
        }
    });
    BeatGuard(handle)
}

/// Run the Blitzortung consumer for up to `duration` (spec 6.4).
///
/// Steps: seed the buffer from the previous `lightning.json`, publish at once, start the 30 s heartbeat task, then
/// loop over endpoints until the deadline. Each session is a connection that is read until it ends. A session
/// that delivered data reconnects after 1 s. One that delivered nothing backs off, and its endpoint is demoted.
///
/// `heartbeat` receives the counters (`msgs`, `parsed`, `in_bbox`, `buffer`, `seeded`) every 30 seconds. `cancel`
/// stops the run at the next await point.
pub async fn lightning_consume_stream<H>(
    config: LightningConfig,
    duration: Duration,
    cancel: CancellationToken,
    heartbeat: H,
) -> Result<LightningRunResult, LightningError>
where
    H: Fn(serde_json::Value) + Send + Sync + 'static,
{
    let started_wall = now_epoch();
    let started = TokioInstant::now();
    let deadline = started + duration;
    let shared = Arc::new(Shared::new(config.clone()));

    // Seed from the previous run so the layer does not blank at the hourly rollover (spec 6.10).
    let seed_path = config.out_path();
    let retention = config.retention_min;
    let max_strikes = config.max_strikes;
    let seeded = tokio::task::spawn_blocking(move || {
        load_existing_strikes(&seed_path, retention, now_epoch(), max_strikes)
    })
    .await
    .context("lightning seed task")?;
    {
        let mut buffer = shared
            .buffer
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        for strike in seeded {
            buffer.push(strike);
        }
        prune_buffer(&mut buffer, config.retention_min);
        let len = buffer.len() as i64;
        drop(buffer);
        shared.with_stats(|s| s.seeded = len);
    }
    flush(&shared).await?;

    let beat = spawn_beat(Arc::clone(&shared), Arc::new(heartbeat), cancel.clone());
    let mut picker = EndpointPicker::new(config.primary.clone(), config.legacy.clone())?;
    let mut connect_failures = 0i64;
    let mut malformed_total = 0i64;
    let mut last_flush = 0.0f64;

    while TokioInstant::now() < deadline {
        let url = picker.next(std::time::Instant::now());
        let session = session::run(&shared, &url, deadline, &cancel, &mut last_flush).await?;
        malformed_total += session.malformed;
        let delivered = session.msgs > 0;

        let delay = match session.error {
            Some(err) => {
                let delay = if delivered {
                    picker.succeeded(&url)
                } else {
                    connect_failures += 1;
                    picker.failed(&url, std::time::Instant::now(), unit_random())
                };
                if delivered {
                    tracing::info!(event = "ws_closed", url = %url, error = %format!("{err:#}"), session_msgs = session.msgs);
                } else {
                    tracing::warn!(
                        event = "ws_disconnect",
                        url = %url,
                        error = %format!("{err:#}"),
                        consecutive_failures = picker.consecutive_failures(),
                        retry_in_s = delay,
                    );
                }
                flush(&shared).await?;
                delay
            }
            None => {
                // A clean close, or the run deadline, arrives here. Python takes this path as well.
                let delay = if delivered {
                    picker.succeeded(&url)
                } else {
                    connect_failures += 1;
                    picker.failed(&url, std::time::Instant::now(), unit_random())
                };
                tracing::info!(event = "ws_closed", url = %url, session_msgs = session.msgs);
                delay
            }
        };

        let remaining = deadline.saturating_duration_since(TokioInstant::now());
        if remaining.is_zero() {
            break;
        }
        let pause = Duration::from_secs_f64(delay).min(remaining);
        tokio::select! {
            _ = cancel.cancelled() => return Err(LightningError::Cancelled),
            _ = tokio::time::sleep(pause) => {}
        }
    }

    drop(beat);
    flush(&shared).await?;
    let stats = shared.stats();
    let final_buffer = shared
        .buffer
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .len() as i64;
    tracing::info!(
        event = "lightning_run_done",
        malformed = malformed_total,
        connect_failures
    );
    Ok(LightningRunResult {
        duration_s: round_digits(now_epoch() - started_wall, 1),
        msgs: stats.msgs,
        parsed: stats.parsed,
        in_bbox: stats.in_bbox,
        final_buffer,
        seeded: stats.seeded,
        connect_failures,
    })
}
