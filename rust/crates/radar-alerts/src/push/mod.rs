//! Push notification senders (spec-temporal section 8, Python `temporal/shared/push.py`).
//!
//! [`send_push_notification`] is the single entry point. It applies the `PUSH_DISABLED` gate, dispatches on
//! platform (`ios` to APNs, `android` to FCM), and classifies every failure as retryable or not, which is what the
//! Python `ApplicationError(non_retryable=True)` decides. Dead tokens are deleted from the local store before the
//! non-retryable error is returned.

mod apns;
mod fcm;

use std::path::PathBuf;
use std::sync::Mutex;
use std::time::Duration;

use anyhow::Context;
use radar_core::env::var;
use radar_core::push_tokens::{PushTokenError, PushTokenStore};

use crate::now_epoch;
pub use radar_types::payloads::push::{PushPayload, PushToken};

/// Default APNs host (`APNS_HOST`).
pub const DEFAULT_APNS_HOST: &str = "https://api.push.apple.com";
/// Default APNs key path (`APNS_KEY_PATH`).
pub const DEFAULT_APNS_KEY_PATH: &str = "/secrets/apns.p8";
/// Default FCM credentials path (`FCM_CREDENTIALS_PATH`).
pub const DEFAULT_FCM_CREDENTIALS_PATH: &str = "/secrets/fcm.json";
/// Default FCM API host.
pub const DEFAULT_FCM_BASE_URL: &str = "https://fcm.googleapis.com";
/// Google OAuth token endpoint, used to exchange the service-account assertion.
pub const GOOGLE_TOKEN_URL: &str = "https://oauth2.googleapis.com/token";
/// Per-request timeout for APNs, FCM and the OAuth exchange (`timeout=10.0`).
pub const SEND_TIMEOUT: Duration = Duration::from_secs(10);

/// APNs caps `apns-collapse-id` at 64 bytes. The Python code cuts the string at 64 characters, and this port
/// keeps that rule (spec D15).
pub const APNS_COLLAPSE_ID_MAX_CHARS: usize = 64;

/// Decide the `PUSH_DISABLED` gate from the raw variable value. `None` (unset) means the Python default `"1"`,
/// which disables sends.
///
/// Only `1`, `true` and `yes` disable, compared case-insensitively after trimming. Any other value enables sends,
/// including `""`, `"0"` and `"false"`. This is the PLAN fix to temporal D5. The Python code disabled only for the
/// exact string `"1"`.
#[must_use]
pub fn push_disabled_from(raw: Option<&str>) -> bool {
    match raw {
        None => true,
        Some(value) => matches!(
            value.trim().to_ascii_lowercase().as_str(),
            "1" | "true" | "yes"
        ),
    }
}

/// Push configuration. Build once per worker from the environment.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PushConfig {
    /// `PUSH_DISABLED` after [`push_disabled_from`]. When true, sends are logged and report success.
    pub disabled: bool,
    /// `APNS_HOST`.
    pub apns_host: String,
    /// `APNS_TOPIC`: the bundle ID.
    pub apns_topic: String,
    /// `APNS_KEY_ID`: the `kid` header.
    pub apns_key_id: String,
    /// `APNS_TEAM_ID`: the `iss` claim.
    pub apns_team_id: String,
    /// `APNS_KEY_PATH`: PEM file with the ES256 key.
    pub apns_key_path: PathBuf,
    /// `FCM_PROJECT_ID`.
    pub fcm_project_id: String,
    /// `FCM_CREDENTIALS_PATH`: service-account JSON file.
    pub fcm_credentials_path: PathBuf,
    /// FCM API host. Tests point it at a mock.
    pub fcm_base_url: String,
    /// OAuth token endpoint. Tests point it at a mock.
    pub fcm_token_url: String,
}

impl PushConfig {
    /// Read the push environment. Empty values use the defaults, as elsewhere in radar-core.
    #[must_use]
    pub fn from_env() -> Self {
        let raw_disabled = std::env::var("PUSH_DISABLED").ok();
        Self {
            disabled: push_disabled_from(raw_disabled.as_deref()),
            apns_host: var("APNS_HOST").unwrap_or_else(|| DEFAULT_APNS_HOST.to_owned()),
            apns_topic: var("APNS_TOPIC").unwrap_or_default(),
            apns_key_id: var("APNS_KEY_ID").unwrap_or_default(),
            apns_team_id: var("APNS_TEAM_ID").unwrap_or_default(),
            apns_key_path: PathBuf::from(
                var("APNS_KEY_PATH").unwrap_or_else(|| DEFAULT_APNS_KEY_PATH.to_owned()),
            ),
            fcm_project_id: var("FCM_PROJECT_ID").unwrap_or_default(),
            fcm_credentials_path: PathBuf::from(
                var("FCM_CREDENTIALS_PATH")
                    .unwrap_or_else(|| DEFAULT_FCM_CREDENTIALS_PATH.to_owned()),
            ),
            fcm_base_url: DEFAULT_FCM_BASE_URL.to_owned(),
            fcm_token_url: GOOGLE_TOKEN_URL.to_owned(),
        }
    }
}

/// A send failure. The variant decides whether the worker retries (spec-temporal section 8).
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum PushError {
    /// Python `ApplicationError(non_retryable=True)`: configuration, dead token, 4xx, unknown platform.
    #[error("{0}")]
    NonRetryable(String),
    /// Python `RuntimeError` or a transport error: 5xx, timeouts, unreadable key files, OAuth failures.
    #[error("{0}")]
    Retryable(String),
}

impl PushError {
    /// True for [`PushError::Retryable`].
    #[must_use]
    pub fn is_retryable(&self) -> bool {
        matches!(self, Self::Retryable(_))
    }
}

pub(crate) fn retryable(err: impl std::fmt::Display) -> PushError {
    PushError::Retryable(err.to_string())
}

#[derive(Debug, Clone)]
struct CachedToken {
    token: String,
    /// Wall-clock seconds at which the cached token stops being used (`time.time() + 3000` in Python).
    expires_at: f64,
}

/// Sender for APNs and FCM. Holds the HTTP client, the FCM OAuth cache and the token store, which dead tokens are
/// deleted from.
pub struct PushSender {
    config: PushConfig,
    store: PushTokenStore,
    http: reqwest::Client,
    fcm_cache: Mutex<Option<CachedToken>>,
}

impl PushSender {
    /// New sender. Redirects are not followed, as with the Python `httpx` default.
    pub fn new(config: PushConfig, store: PushTokenStore) -> anyhow::Result<Self> {
        let http = reqwest::Client::builder()
            .redirect(reqwest::redirect::Policy::none())
            .build()
            .context("building push HTTP client")?;
        Ok(Self {
            config,
            store,
            http,
            fcm_cache: Mutex::new(None),
        })
    }

    /// The configuration in use.
    #[must_use]
    pub fn config(&self) -> &PushConfig {
        &self.config
    }

    /// The token store.
    #[must_use]
    pub fn store(&self) -> &PushTokenStore {
        &self.store
    }
}

/// Delete every row for `token` on a blocking thread. Used by the dead-token paths of APNs and FCM.
pub(crate) async fn delete_dead_token(
    store: &PushTokenStore,
    token: &str,
) -> Result<usize, PushError> {
    let store = store.clone();
    let token = token.to_owned();
    tokio::task::spawn_blocking(move || store.delete_by_token(&token))
        .await
        .map_err(retryable)?
        .map_err(|err: PushTokenError| retryable(err))
}

/// Send one notification to one device (Python `send_push_notification`).
///
/// With push disabled, logs `push_skipped` and returns `Ok`, so the caller counts the send as successful. Otherwise
/// the platform decides the path: `ios` goes to APNs, `android` to FCM, and anything else is non-retryable.
pub async fn send_push_notification(
    sender: &PushSender,
    token: &PushToken,
    payload: &PushPayload,
) -> Result<(), PushError> {
    if sender.config.disabled {
        tracing::info!(
            event = "push_skipped",
            platform = %token.platform,
            user_id = %token.user_id,
            title = %payload.title,
            "push_skipped (PUSH_DISABLED=1)"
        );
        return Ok(());
    }
    match token.platform.as_str() {
        radar_types::payloads::push::PLATFORM_IOS => {
            apns::send(sender, &token.token, payload).await?
        }
        radar_types::payloads::push::PLATFORM_ANDROID => {
            fcm::send(sender, &token.token, payload).await?
        }
        other => {
            return Err(PushError::NonRetryable(format!(
                "unknown platform: {other}"
            )));
        }
    }
    tracing::info!(event = "push_sent", platform = %token.platform, user_id = %token.user_id);
    Ok(())
}

pub(crate) fn current_epoch_seconds() -> i64 {
    now_epoch() as i64
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn push_disabled_matrix_follows_plan_fix() {
        assert!(push_disabled_from(None), "unset defaults to disabled");
        for on in ["1", "true", "yes", "TRUE", " Yes ", "1 "] {
            assert!(push_disabled_from(Some(on)), "{on:?} disables");
        }
        for off in ["", "0", "false", "no", "on", "2", "disabled"] {
            assert!(!push_disabled_from(Some(off)), "{off:?} enables sends");
        }
    }

    #[test]
    fn collapse_id_truncation_is_by_characters() {
        let id = format!("watch:{}", "ab".repeat(32));
        assert_eq!(id.len(), 70);
        let cut: String = id.chars().take(APNS_COLLAPSE_ID_MAX_CHARS).collect();
        assert_eq!(cut.len(), 64);
        assert!(cut.starts_with("watch:"));
    }

    #[test]
    fn non_retryable_flag_is_exposed() {
        assert!(PushError::Retryable("x".into()).is_retryable());
        assert!(!PushError::NonRetryable("x".into()).is_retryable());
    }
}
