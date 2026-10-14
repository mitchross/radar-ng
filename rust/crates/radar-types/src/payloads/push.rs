//! Push notification payloads (token registration, fan-out, and the send path).
//!
//! Source: `backend/api/api/storm_watch_activities.py`, `temporal/shared/push.py`,
//! `temporal/workflows/register_push_token.py`.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

/// Platform string for APNS.
pub const PLATFORM_IOS: &str = "ios";
/// Platform string for FCM.
pub const PLATFORM_ANDROID: &str = "android";

/// `RegisterPushTokenInput`: input to the compatibility `RegisterPushTokenWorkflow`.
/// Retained histories may contain plaintext tokens; treat them as sensitive.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RegisterPushTokenInput {
    /// Owner of the token.
    pub user_id: String,
    /// Device push token.
    pub token: String,
    /// `ios` or `android`.
    pub platform: String,
}

/// `PushTokenInput`: input to the `persist_push_token` activity (same shape as the workflow input).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PushTokenInput {
    /// Owner of the token.
    pub user_id: String,
    /// Device push token.
    pub token: String,
    /// `ios` or `android`.
    pub platform: String,
}

/// `PushToken`: a stored token, passed to the send path.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PushToken {
    /// Owner of the token.
    pub user_id: String,
    /// Device push token.
    pub token: String,
    /// `ios` or `android`.
    pub platform: String,
}

/// `PushPayload`: the notification content sent to one device.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PushPayload {
    /// Notification title.
    pub title: String,
    /// Notification body.
    pub body: String,
    /// APNS and FCM collapse key.
    pub collapse_id: String,
    /// Extra string data merged into the payload.
    pub extra: BTreeMap<String, String>,
}

/// `FanOutPushInput`: input to `fan_out_push_to_user`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FanOutPushInput {
    /// Recipient user.
    pub user_id: String,
    /// Notification title.
    pub title: String,
    /// Notification body.
    pub body: String,
    /// Collapse key shared by retries of the same notification.
    pub collapse_id: String,
    /// Extra string data (`storm_cell_id`, `kind`, `alert_id`, ...).
    #[serde(default)]
    pub extra: BTreeMap<String, String>,
}

/// `FanOutPushResult`: output of `fan_out_push_to_user`.
///
/// `sent` counts attempted-and-accepted sends. With push disabled it counts
/// no-op sends too, so it is not a delivery count (spec-temporal D6).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct FanOutPushResult {
    /// Number of sends reported as successful.
    pub sent: i64,
}
