//! One websocket session (spec-ingest-aux section 6.4, the `try` body).
//!
//! The Python loop runs `async for msg in ws` with keepalive pings every 30 s and a 10 s open timeout. Here the
//! same loop is a `select!` over the socket, the keepalive timer, the run deadline and cooperative cancellation.

use std::sync::Mutex;
use std::time::Duration;

use anyhow::{Context, anyhow};
use futures_util::{SinkExt, StreamExt};
use tokio::time::Instant as TokioInstant;
use tokio_tungstenite::tungstenite::Message;
use tokio_tungstenite::tungstenite::protocol::WebSocketConfig;
use tokio_util::sync::CancellationToken;

use super::LightningError;
use super::decode::{decode_frame, extract_strike};
use super::store::{Strike, StrikeBuffer};
use super::{Shared, flush, prune_buffer};
use crate::now_epoch;

/// Keepalive ping interval (`ping_interval=30`).
pub const PING_INTERVAL: Duration = Duration::from_secs(30);
/// A ping without a pong for this long ends the session (`ping_timeout=30`).
pub const PING_TIMEOUT: Duration = Duration::from_secs(30);
/// Connect and handshake timeout (`open_timeout=10`).
pub const OPEN_TIMEOUT: Duration = Duration::from_secs(10);
/// Largest accepted message, as the Python `websockets` default `max_size` (spec section 16.1).
pub const MAX_MESSAGE_BYTES: usize = 1 << 20;
/// Subscription frame sent after connecting. Exact bytes, as in Python `json.dumps({"a": 111})`.
pub const SUBSCRIBE_FRAME: &str = r#"{"a": 111}"#;

/// What one session produced.
#[derive(Debug)]
pub struct SessionOutcome {
    /// Data messages received in this session (text or binary).
    pub msgs: i64,
    /// Frames that could not become a strike (spec D-4: skipped and counted).
    pub malformed: i64,
    /// Why the session ended early, if it did. `None` for a clean close or the run deadline.
    pub error: Option<anyhow::Error>,
}

fn websocket_config() -> WebSocketConfig {
    WebSocketConfig::default().max_message_size(Some(MAX_MESSAGE_BYTES))
}

/// Connect to `url` and consume frames until the server closes, the socket fails, the deadline passes, or the
/// token is cancelled. Cancellation is the only error that is returned as `Err`. Everything else is in
/// [`SessionOutcome::error`], so the caller can apply the reconnect rules.
pub(crate) async fn run(
    shared: &Shared,
    url: &str,
    deadline: TokioInstant,
    cancel: &CancellationToken,
    last_flush: &mut f64,
) -> Result<SessionOutcome, LightningError> {
    tracing::info!(event = "ws_connect", url);
    let connect =
        tokio_tungstenite::connect_async_with_config(url, Some(websocket_config()), false);
    let connected = tokio::select! {
        _ = cancel.cancelled() => return Err(LightningError::Cancelled),
        result = tokio::time::timeout(OPEN_TIMEOUT, connect) => result,
    };
    let socket = match connected {
        Err(_elapsed) => {
            return Ok(failed(
                0,
                anyhow!("open timeout after {}s", OPEN_TIMEOUT.as_secs()),
            ));
        }
        Ok(Err(err)) => return Ok(failed(0, err.into())),
        Ok(Ok((socket, _response))) => socket,
    };

    let (mut write, mut read) = socket.split();
    if let Err(err) = write.send(Message::Text(SUBSCRIBE_FRAME.into())).await {
        return Ok(failed(0, err.into()));
    }

    let mut keepalive =
        tokio::time::interval_at(TokioInstant::now() + PING_INTERVAL, PING_INTERVAL);
    keepalive.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    let mut awaiting_pong_since: Option<TokioInstant> = None;
    let mut msgs = 0i64;
    let mut malformed = 0i64;
    let mut warned_malformed = false;
    let mut error: Option<anyhow::Error> = None;

    loop {
        tokio::select! {
            _ = cancel.cancelled() => {
                let _ = tokio::time::timeout(Duration::from_secs(2), write.close()).await;
                return Err(LightningError::Cancelled);
            }
            _ = tokio::time::sleep_until(deadline) => break,
            _ = keepalive.tick() => {
                if awaiting_pong_since.is_some_and(|since| since.elapsed() >= PING_TIMEOUT) {
                    error = Some(anyhow!("ping timeout after {}s", PING_TIMEOUT.as_secs()));
                    break;
                }
                if let Err(err) = write.send(Message::Ping(Vec::new().into())).await {
                    error = Some(err.into());
                    break;
                }
                awaiting_pong_since.get_or_insert_with(TokioInstant::now);
            }
            next = read.next() => {
                match next {
                    None => break,
                    Some(Err(err)) => {
                        error = Some(err.into());
                        break;
                    }
                    Some(Ok(Message::Close(_))) => break,
                    Some(Ok(Message::Pong(_))) => awaiting_pong_since = None,
                    Some(Ok(Message::Ping(_) | Message::Frame(_))) => {}
                    Some(Ok(Message::Text(text))) => {
                        let outcome = on_frame(shared, text.as_str(), &mut msgs, &mut malformed, &mut warned_malformed, last_flush).await;
                        if let Err(err) = outcome {
                            error = Some(err);
                            break;
                        }
                    }
                    Some(Ok(Message::Binary(bytes))) => {
                        // latin-1: every byte becomes the code point of the same value. Never fails.
                        let text: String = bytes.iter().map(|&b| char::from(b)).collect();
                        let outcome = on_frame(shared, &text, &mut msgs, &mut malformed, &mut warned_malformed, last_flush).await;
                        if let Err(err) = outcome {
                            error = Some(err);
                            break;
                        }
                    }
                }
            }
        }
    }

    if error.is_some() {
        // The socket is already broken. Dropping it is enough.
    } else {
        let _ = tokio::time::timeout(Duration::from_secs(2), write.close()).await;
    }
    Ok(SessionOutcome {
        msgs,
        malformed,
        error,
    })
}

fn failed(msgs: i64, error: anyhow::Error) -> SessionOutcome {
    SessionOutcome {
        msgs,
        malformed: 0,
        error: Some(error),
    }
}

/// Handle one data message (spec 6.4 body and 6.5 to 6.7). An `Err` is a session-level failure: a flush that
/// failed. Bad frames are counted in `malformed` and skipped.
async fn on_frame(
    shared: &Shared,
    text: &str,
    msgs: &mut i64,
    malformed: &mut i64,
    warned_malformed: &mut bool,
    last_flush: &mut f64,
) -> anyhow::Result<()> {
    *msgs += 1;
    shared.with_stats(|s| s.msgs += 1);
    let Some(value) = decode_frame(text) else {
        return Ok(());
    };
    if value.is_null() {
        return Ok(());
    }
    shared.with_stats(|s| s.parsed += 1);
    match extract_strike(&value, &shared.config.bbox) {
        Ok(None) => Ok(()),
        Ok(Some(strike)) => {
            shared.with_stats(|s| s.in_bbox += 1);
            push_strike(shared, strike);
            let now = now_epoch();
            if now - *last_flush >= shared.config.flush_every_s {
                flush(shared).await.context("strike-driven flush")?;
                *last_flush = now;
            }
            Ok(())
        }
        Err(err) => {
            *malformed += 1;
            if !*warned_malformed {
                *warned_malformed = true;
                tracing::warn!(event = "malformed_frame", error = %err, "skipping malformed frame");
            }
            Ok(())
        }
    }
}

fn push_strike(shared: &Shared, strike: Strike) {
    let buffer: &Mutex<StrikeBuffer> = &shared.buffer;
    let mut guard = buffer
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    guard.push(strike);
    prune_buffer(&mut guard, shared.config.retention_min);
}
