//! APNs sender: HTTP/2 to `POST {host}/3/device/{token}` with an ES256 provider token (spec-temporal 8.4).

use jsonwebtoken::{Algorithm, EncodingKey, Header};
use serde_json::{Map, Value, json};

use super::{
    APNS_COLLAPSE_ID_MAX_CHARS, PushConfig, PushError, PushPayload, PushSender, SEND_TIMEOUT,
};
use super::{current_epoch_seconds, delete_dead_token, retryable};

/// Body for one notification: `{"aps": {...}, **extra}`. Keys in `extra` are added after `aps`, so an `aps` key in
/// `extra` replaces the alert block. Python does the same.
#[must_use]
pub fn apns_body(payload: &PushPayload) -> Value {
    let mut body = Map::new();
    body.insert(
        "aps".into(),
        json!({
            "alert": {"title": payload.title, "body": payload.body},
            "sound": "default",
            "mutable-content": 1,
        }),
    );
    for (key, value) in &payload.extra {
        body.insert(key.clone(), Value::String(value.clone()));
    }
    Value::Object(body)
}

/// ES256 provider token: header `kid = APNS_KEY_ID`, claims `iss = APNS_TEAM_ID`, `iat = now`. Minted per send, as
/// the Python code does.
pub(crate) fn provider_token(config: &PushConfig, iat: i64) -> Result<String, PushError> {
    let key_pem = std::fs::read(&config.apns_key_path).map_err(retryable)?;
    let key = EncodingKey::from_ec_pem(&key_pem).map_err(retryable)?;
    let mut header = Header::new(Algorithm::ES256);
    header.kid = Some(config.apns_key_id.clone());
    let claims = json!({"iss": config.apns_team_id, "iat": iat});
    jsonwebtoken::encode(&header, &claims, &key).map_err(retryable)
}

pub(crate) async fn send(
    sender: &PushSender,
    token: &str,
    payload: &PushPayload,
) -> Result<(), PushError> {
    let config = sender.config();
    if config.apns_topic.is_empty()
        || config.apns_key_id.is_empty()
        || config.apns_team_id.is_empty()
    {
        return Err(PushError::NonRetryable(
            "APNS not configured (APNS_TOPIC/APNS_KEY_ID/APNS_TEAM_ID missing)".into(),
        ));
    }
    let jwt = provider_token(config, current_epoch_seconds())?;
    let collapse_id: String = payload
        .collapse_id
        .chars()
        .take(APNS_COLLAPSE_ID_MAX_CHARS)
        .collect();
    let url = format!("{}/3/device/{}", config.apns_host, token);
    let response = sender
        .http
        .post(&url)
        .timeout(SEND_TIMEOUT)
        .header("authorization", format!("bearer {jwt}"))
        .header("apns-topic", config.apns_topic.as_str())
        .header("apns-push-type", "alert")
        .header("apns-priority", "10")
        .header("apns-collapse-id", collapse_id)
        .json(&apns_body(payload))
        .send()
        .await
        .map_err(retryable)?;
    let status = response.status().as_u16();
    let text = response.text().await.unwrap_or_default();

    if status == 200 {
        return Ok(());
    }
    if status == 410 || text.contains("BadDeviceToken") || text.contains("Unregistered") {
        delete_dead_token(sender.store(), token).await?;
        return Err(PushError::NonRetryable(format!("apns dead token: {text}")));
    }
    if (400..500).contains(&status) {
        return Err(PushError::NonRetryable(format!(
            "apns 4xx: {status} {text}"
        )));
    }
    Err(PushError::Retryable(format!("apns 5xx: {status} {text}")))
}
