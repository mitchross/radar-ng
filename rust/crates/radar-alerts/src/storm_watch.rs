//! Storm-watch activities (spec-shared sections 8 and 9, spec-temporal sections 4.11 and 8.1, Python
//! `backend/api/api/storm_watch_activities.py`).
//!
//! - [`persist_push_token`], [`delete_push_token`]: the push-token store (`radar-core`).
//! - [`compare_radar_frames`], [`detect_storm_change`]: frame sampling and classification (`radar-storms`).
//! - [`fan_out_push_to_user`]: send one notification to every device registered for a user.
//!
//! Each function is `async` and takes its inputs as plain values, so the worker crate can wrap it as an activity
//! without any Temporal types in this crate.

use std::path::Path;

use anyhow::{Context, anyhow};
use radar_core::push_tokens::PushTokenStore;
use radar_storms::watch::{
    FrameComparison, WatchConfig, compare_radar_frames as compare_frames,
    detect_storm_change as detect,
};
use radar_types::payloads::push::{
    FanOutPushInput, FanOutPushResult, PushPayload, PushToken, PushTokenInput,
};
use radar_types::payloads::watch_storm::{
    CompareFramesInput, CompareFramesResult, DetectChangeInput, DetectChangeResult,
};

use crate::push::{PushSender, send_push_notification};

/// Register `token` for `user_id` on `platform` (upsert). Re-registering keeps `created_at`.
pub async fn persist_push_token(
    store: &PushTokenStore,
    input: &PushTokenInput,
) -> anyhow::Result<()> {
    let store = store.clone();
    let input = input.clone();
    tokio::task::spawn_blocking(move || store.upsert(&input.user_id, &input.token, &input.platform))
        .await
        .context("persist_push_token task")?
        .context("writing push token")
}

/// Delete every row for `token`. Returns the number of rows removed.
pub async fn delete_push_token(store: &PushTokenStore, token: &str) -> anyhow::Result<usize> {
    let store = store.clone();
    let token = token.to_owned();
    tokio::task::spawn_blocking(move || store.delete_by_token(&token))
        .await
        .context("delete_push_token task")?
        .context("deleting push token")
}

/// Sample the latest radar frame at the watch point and compare it with the previous poll (spec 8.6).
///
/// `sampled = false` when there is no grid, no usable sample, or the frame is the same as last time. In that case
/// the other fields are zero. `grid_dir` is the directory that holds `radar/*.meta.json` (`GRID_DIR`).
pub async fn compare_radar_frames(
    grid_dir: &Path,
    input: &CompareFramesInput,
) -> anyhow::Result<CompareFramesResult> {
    let grid_dir = grid_dir.to_path_buf();
    let input = input.clone();
    let comparison = tokio::task::spawn_blocking(move || {
        compare_frames(
            &grid_dir,
            input.lat,
            input.lng,
            input.prev_timestamp.as_deref(),
            input.prev_max_dbz,
        )
    })
    .await
    .context("compare_radar_frames task")?;
    Ok(to_compare_result(comparison))
}

fn to_compare_result(c: FrameComparison) -> CompareFramesResult {
    CompareFramesResult {
        sampled: c.sampled,
        curr_timestamp: c.curr_timestamp,
        curr_max_dbz: c.curr_max_dbz,
        prev_max_dbz: c.prev_max_dbz,
        has_prev: c.has_prev,
        max_dbz_delta: c.max_dbz_delta,
    }
}

/// Classify the change between two samples (spec 8.5). `kind` and `summary` are empty when nothing changed or when
/// there is no previous sample. `cfg` holds the thresholds, which the worker reads once from the environment.
pub async fn detect_storm_change(
    input: &DetectChangeInput,
    cfg: &WatchConfig,
) -> DetectChangeResult {
    let comparison = FrameComparison {
        sampled: true,
        curr_timestamp: String::new(),
        curr_max_dbz: input.curr_max_dbz,
        prev_max_dbz: input.prev_max_dbz,
        has_prev: input.has_prev,
        max_dbz_delta: input.max_dbz_delta,
    };
    match detect(&comparison, cfg) {
        Some(event) => DetectChangeResult {
            kind: event.kind.as_str().to_owned(),
            summary: event.summary,
        },
        None => DetectChangeResult {
            kind: String::new(),
            summary: String::new(),
        },
    }
}

/// Send one notification to every device registered for `input.user_id` (spec 8.1).
///
/// The tokens are read first. Each send is attempted in turn. A failed send is logged and skipped, and the
/// partial-failure case still returns `Ok`, because a retry would double-send to the devices that succeeded. A
/// dead token is deleted inside the send. If there were tokens and every send failed, the call returns an error,
/// so the activity retry policy gets a chance. With push disabled each send counts as successful (spec D6).
pub async fn fan_out_push_to_user(
    sender: &PushSender,
    input: &FanOutPushInput,
) -> anyhow::Result<FanOutPushResult> {
    let store = sender.store().clone();
    let user_id = input.user_id.clone();
    let tokens = tokio::task::spawn_blocking(move || store.list_for_user(&user_id))
        .await
        .context("listing push tokens task")?
        .context("listing push tokens")?;

    let mut sent = 0i64;
    let mut last_err: Option<String> = None;
    for registered in &tokens {
        let token = PushToken {
            user_id: registered.user_id.clone(),
            token: registered.token.clone(),
            platform: registered.platform.clone(),
        };
        let payload = PushPayload {
            title: input.title.clone(),
            body: input.body.clone(),
            collapse_id: input.collapse_id.clone(),
            extra: input.extra.clone(),
        };
        match send_push_notification(sender, &token, &payload).await {
            Ok(()) => sent += 1,
            Err(err) => {
                tracing::warn!(event = "push_failed_for_token", platform = %token.platform, error = %err);
                last_err = Some(err.to_string());
            }
        }
    }
    if !tokens.is_empty()
        && sent == 0
        && let Some(last) = last_err
    {
        return Err(anyhow!(
            "all {} push sends failed for user; last error: {last}",
            tokens.len()
        ));
    }
    Ok(FanOutPushResult { sent })
}
