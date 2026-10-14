//! FCM sender: HTTP v1 `messages:send` with a service-account OAuth access token (spec-temporal 8.5).

use jsonwebtoken::{Algorithm, EncodingKey, Header};
use serde_json::{Value, json};

use super::{CachedToken, PushError, PushPayload, PushSender, SEND_TIMEOUT};
use super::{current_epoch_seconds, delete_dead_token, retryable};
use crate::now_epoch;

/// OAuth scope for FCM v1.
pub const FCM_SCOPE: &str = "https://www.googleapis.com/auth/firebase.messaging";
/// `aud` claim of the service-account assertion. Python fixes it to the Google endpoint, whatever the POST goes to.
pub const FCM_AUDIENCE: &str = "https://oauth2.googleapis.com/token";
/// Grant type for the JWT bearer exchange.
pub const JWT_BEARER_GRANT: &str = "urn:ietf:params:oauth:grant-type:jwt-bearer";
/// Access tokens are reused for this long (`now + 3000`).
pub const TOKEN_REUSE_S: f64 = 3000.0;
/// A cached token is used only while it has more than this many seconds left (`> now + 60`).
pub const TOKEN_MIN_LEFT_S: f64 = 60.0;

/// The FCM v1 request body (spec 8.5). `data` values are strings, and `collapse_key` carries the full collapse ID.
#[must_use]
pub fn fcm_body(token: &str, payload: &PushPayload) -> Value {
    json!({
        "message": {
            "token": token,
            "notification": {"title": payload.title, "body": payload.body},
            "data": payload.extra,
            "android": {
                "collapse_key": payload.collapse_id,
                "priority": "high",
                "notification": {"sound": "default"},
            },
        }
    })
}

/// Build the service-account assertion: RS256, `iss = client_email`, `scope`, `aud`, `iat`, and `exp = iat + 3600`.
pub(crate) fn assertion(
    client_email: &str,
    private_key_pem: &str,
    iat: i64,
) -> Result<String, PushError> {
    let key = EncodingKey::from_rsa_pem(private_key_pem.as_bytes()).map_err(retryable)?;
    let claims = json!({
        "iss": client_email,
        "scope": FCM_SCOPE,
        "aud": FCM_AUDIENCE,
        "iat": iat,
        "exp": iat + 3600,
    });
    jsonwebtoken::encode(&Header::new(Algorithm::RS256), &claims, &key).map_err(retryable)
}

/// Percent-encode a form value, as Python `httpx` does for `application/x-www-form-urlencoded`.
#[must_use]
pub fn form_encode(pairs: &[(&str, &str)]) -> String {
    fn encode(value: &str, out: &mut String) {
        for byte in value.bytes() {
            if byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.' | b'~') {
                out.push(char::from(byte));
            } else {
                out.push_str(&format!("%{byte:02X}"));
            }
        }
    }
    let mut out = String::new();
    for (i, (key, value)) in pairs.iter().enumerate() {
        if i > 0 {
            out.push('&');
        }
        encode(key, &mut out);
        out.push('=');
        encode(value, &mut out);
    }
    out
}

/// The cached OAuth access token, or a fresh one from the service-account exchange (spec 8.5, "Access token").
async fn access_token(sender: &PushSender) -> Result<String, PushError> {
    {
        let cache = sender
            .fcm_cache
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if let Some(cached) = cache.as_ref()
            && cached.expires_at > now_epoch() + TOKEN_MIN_LEFT_S
        {
            return Ok(cached.token.clone());
        }
    }
    let config = sender.config();
    let creds_bytes = std::fs::read(&config.fcm_credentials_path).map_err(retryable)?;
    let creds: Value = serde_json::from_slice(&creds_bytes).map_err(retryable)?;
    let client_email = creds
        .get("client_email")
        .and_then(Value::as_str)
        .ok_or_else(|| retryable("service account has no client_email"))?;
    let private_key = creds
        .get("private_key")
        .and_then(Value::as_str)
        .ok_or_else(|| retryable("service account has no private_key"))?;
    let assertion = assertion(client_email, private_key, current_epoch_seconds())?;
    let body = form_encode(&[("grant_type", JWT_BEARER_GRANT), ("assertion", &assertion)]);
    let response = sender
        .http
        .post(&config.fcm_token_url)
        .timeout(SEND_TIMEOUT)
        .header("content-type", "application/x-www-form-urlencoded")
        .body(body)
        .send()
        .await
        .map_err(retryable)?
        .error_for_status()
        .map_err(retryable)?;
    let parsed: Value = response.json().await.map_err(retryable)?;
    let access = parsed
        .get("access_token")
        .and_then(Value::as_str)
        .ok_or_else(|| retryable("OAuth response has no access_token"))?
        .to_owned();
    let mut cache = sender
        .fcm_cache
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    *cache = Some(CachedToken {
        token: access.clone(),
        expires_at: now_epoch() + TOKEN_REUSE_S,
    });
    Ok(access)
}

pub(crate) async fn send(
    sender: &PushSender,
    token: &str,
    payload: &PushPayload,
) -> Result<(), PushError> {
    let config = sender.config();
    if config.fcm_project_id.is_empty() {
        return Err(PushError::NonRetryable(
            "FCM not configured (FCM_PROJECT_ID missing)".into(),
        ));
    }
    let access = access_token(sender).await?;
    let url = format!(
        "{}/v1/projects/{}/messages:send",
        config.fcm_base_url, config.fcm_project_id
    );
    let response = sender
        .http
        .post(&url)
        .timeout(SEND_TIMEOUT)
        .header("authorization", format!("Bearer {access}"))
        .header("content-type", "application/json")
        .json(&fcm_body(token, payload))
        .send()
        .await
        .map_err(retryable)?;
    let status = response.status().as_u16();
    let text = response.text().await.unwrap_or_default();

    if status == 200 {
        return Ok(());
    }
    if text.contains("NOT_REGISTERED") || text.contains("INVALID_ARGUMENT") || status == 404 {
        delete_dead_token(sender.store(), token).await?;
        return Err(PushError::NonRetryable(format!("fcm dead token: {text}")));
    }
    if (400..500).contains(&status) {
        return Err(PushError::NonRetryable(format!("fcm 4xx: {status} {text}")));
    }
    Err(PushError::Retryable(format!("fcm 5xx: {status} {text}")))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn form_encoding_escapes_colons_and_keeps_unreserved() {
        let encoded = form_encode(&[("grant_type", JWT_BEARER_GRANT), ("assertion", "a.b-c_d~e")]);
        assert_eq!(
            encoded,
            "grant_type=urn%3Aietf%3Aparams%3Aoauth%3Agrant-type%3Ajwt-bearer&assertion=a.b-c_d~e"
        );
    }

    #[test]
    fn body_matches_spec_shape() {
        let payload = PushPayload {
            title: "Storm intensifying".into(),
            body: "+12 dBZ".into(),
            collapse_id: "watch:abc:2026:intensifying".into(),
            extra: [("kind".to_owned(), "intensifying".to_owned())]
                .into_iter()
                .collect(),
        };
        let body = fcm_body("tok", &payload);
        assert_eq!(
            body["message"]["android"]["collapse_key"],
            "watch:abc:2026:intensifying"
        );
        assert_eq!(body["message"]["android"]["priority"], "high");
        assert_eq!(body["message"]["data"]["kind"], "intensifying");
    }
}
