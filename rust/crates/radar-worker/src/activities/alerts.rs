//! Alerts, push and WatchStorm activities (spec sections 4.10 to 4.12 and 8).

use std::path::PathBuf;
use std::sync::Arc;

use radar_alerts::alerts::{
    AlertsConfig, WatchSignaler, fetch_nws_active_alerts, mark_alerts_seen,
    signal_matching_storm_watches,
};
use radar_alerts::push::{PushError, PushSender, send_push_notification};
use radar_alerts::storm_watch::{
    compare_radar_frames, delete_push_token, detect_storm_change, fan_out_push_to_user,
    persist_push_token,
};
use radar_core::push_tokens::PushTokenStore;
use radar_storms::watch::WatchConfig;
use radar_types::payloads::alerts::{
    FetchAlertsResult, MarkAlertsSeenInput, SignalWatchesInput, SignalWatchesResult,
};
use radar_types::payloads::push::{
    FanOutPushInput, FanOutPushResult, PushPayload, PushToken, PushTokenInput,
};
use radar_types::payloads::watch_storm::{
    CompareFramesInput, CompareFramesResult, DetectChangeInput, DetectChangeResult,
};
use temporalio_macros::activities;
use temporalio_sdk::activities::{ActivityContext, ActivityError};

use crate::support::{non_retryable, with_cancel};

/// Everything the alerts pool needs. Built once at startup.
pub struct AlertsDeps {
    /// NWS poll configuration.
    pub alerts: AlertsConfig,
    /// APNS/FCM sender (holds the token store that dead tokens are deleted from).
    pub push: PushSender,
    /// Push-token database handle.
    pub tokens: PushTokenStore,
    /// Grid directory read by `compare_radar_frames` (`GRID_DIR`).
    pub grid_dir: PathBuf,
    /// Watch lookup and signalling over the Temporal client.
    pub signaler: Arc<dyn WatchSignaler>,
    /// Storm-change thresholds (`WatchConfig::from_env`).
    pub watch: WatchConfig,
}

/// Activity implementer for the alerts pool (`alerts` role, and `legacy`/`all`).
pub struct AlertsActivities {
    deps: AlertsDeps,
}

impl AlertsActivities {
    /// Wrap the alerts dependencies.
    pub fn new(deps: AlertsDeps) -> Self {
        Self { deps }
    }
}

/// Map a push failure to the activity error class the Python code raises (`ApplicationError`
/// non-retryable versus `RuntimeError`).
pub fn push_failure(err: PushError) -> ActivityError {
    match err {
        PushError::NonRetryable(message) => non_retryable(message),
        PushError::Retryable(message) => anyhow::anyhow!(message).into(),
    }
}

#[activities]
impl AlertsActivities {
    /// `fetch_nws_active_alerts`: poll the NWS feed and diff against the seen set.
    #[activity(name = "fetch_nws_active_alerts")]
    pub async fn fetch_nws_active_alerts(
        self: Arc<Self>,
        ctx: ActivityContext,
    ) -> Result<FetchAlertsResult, ActivityError> {
        with_cancel(&ctx, fetch_nws_active_alerts(&self.deps.alerts)).await
    }

    /// `signal_matching_storm_watches`: signal every running watch whose point lies inside the geometry.
    #[activity(name = "signal_matching_storm_watches")]
    pub async fn signal_matching_storm_watches(
        self: Arc<Self>,
        ctx: ActivityContext,
        input: SignalWatchesInput,
    ) -> Result<SignalWatchesResult, ActivityError> {
        with_cancel(
            &ctx,
            signal_matching_storm_watches(self.deps.signaler.as_ref(), &input),
        )
        .await
    }

    /// `mark_alerts_seen`: commit handled alert IDs to the seen set.
    #[activity(name = "mark_alerts_seen")]
    pub async fn mark_alerts_seen(
        self: Arc<Self>,
        ctx: ActivityContext,
        input: MarkAlertsSeenInput,
    ) -> Result<(), ActivityError> {
        with_cancel(
            &ctx,
            mark_alerts_seen(&self.deps.alerts, &input.handled_ids),
        )
        .await
    }

    /// `persist_push_token`: upsert a device token.
    #[activity(name = "persist_push_token")]
    pub async fn persist_push_token(
        self: Arc<Self>,
        ctx: ActivityContext,
        input: PushTokenInput,
    ) -> Result<(), ActivityError> {
        with_cancel(&ctx, persist_push_token(&self.deps.tokens, &input)).await
    }

    /// `delete_push_token`: delete a device token; returns the number of rows removed.
    #[activity(name = "delete_push_token")]
    pub async fn delete_push_token(
        self: Arc<Self>,
        ctx: ActivityContext,
        token: String,
    ) -> Result<usize, ActivityError> {
        with_cancel(&ctx, delete_push_token(&self.deps.tokens, &token)).await
    }

    /// `compare_radar_frames`: sample the newest frame's maximum reflectivity for a point.
    #[activity(name = "compare_radar_frames")]
    pub async fn compare_radar_frames(
        self: Arc<Self>,
        ctx: ActivityContext,
        input: CompareFramesInput,
    ) -> Result<CompareFramesResult, ActivityError> {
        with_cancel(&ctx, compare_radar_frames(&self.deps.grid_dir, &input)).await
    }

    /// `detect_storm_change`: classify the change between two samples.
    #[activity(name = "detect_storm_change")]
    pub async fn detect_storm_change(
        self: Arc<Self>,
        ctx: ActivityContext,
        input: DetectChangeInput,
    ) -> Result<DetectChangeResult, ActivityError> {
        let watch = self.deps.watch.clone();
        with_cancel(&ctx, async move {
            anyhow::Ok(detect_storm_change(&input, &watch).await)
        })
        .await
    }

    /// `fan_out_push_to_user`: send one notification to every device the user has registered.
    #[activity(name = "fan_out_push_to_user")]
    pub async fn fan_out_push_to_user(
        self: Arc<Self>,
        ctx: ActivityContext,
        input: FanOutPushInput,
    ) -> Result<FanOutPushResult, ActivityError> {
        with_cancel(&ctx, fan_out_push_to_user(&self.deps.push, &input)).await
    }

    /// `send_push_notification`: one device, one payload. Registered for compatibility (spec 8.1).
    #[activity(name = "send_push_notification")]
    pub async fn send_push_notification(
        self: Arc<Self>,
        ctx: ActivityContext,
        token: PushToken,
        payload: PushPayload,
    ) -> Result<(), ActivityError> {
        with_cancel(&ctx, async {
            send_push_notification(&self.deps.push, &token, &payload)
                .await
                .map_err(push_failure)
        })
        .await
    }
}
