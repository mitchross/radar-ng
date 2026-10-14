//! The `/v1` workflow routes: push-token registration and storm watches (spec 6.3, 6.5, 6.6).
//!
//! Every route authenticates first (401 or 503), then validates the body (422, which lists every
//! problem), then checks ownership (404), then talks to storage or Temporal. Mounting is controlled
//! by `DISABLE_WORKFLOW_ROUTES` (only the exact value `1` disables them), in `router()`.

pub mod auth;
pub mod temporal;

use axum::Router;
use axum::body::Bytes;
use axum::extract::{Path, State};
use axum::http::{HeaderMap, StatusCode, header};
use axum::response::{IntoResponse, Response};
use axum::routing::{delete, get, post};
use serde_json::{Map, Value, json};
use sha2::{Digest, Sha256};

use crate::params::{Issues, py_float};
use crate::respond;
use crate::state::AppState;
use auth::{authenticate, require_owner};
use temporal::{current_state, get_client, reset_client, start_watch, unpin};

/// The workflow routes, to be merged into the application router.
pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/v1/push-tokens", post(post_push_token))
        .route("/v1/push-tokens/{token}", delete(delete_push_token))
        .route("/v1/watches", post(post_watch))
        .route(
            "/v1/watches/{user_id}/{storm_cell_id}",
            get(get_watch).delete(delete_watch),
        )
}

/// `"watch:" + sha256_hex(user_id + "\0" + storm_cell_id)` (spec 6.3).
#[must_use]
pub fn watch_workflow_id(user_id: &str, storm_cell_id: &str) -> String {
    let digest = Sha256::digest(format!("{user_id}\0{storm_cell_id}").as_bytes());
    format!("watch:{}", hex::encode(digest))
}

/// True when a Temporal error text means the watch is already gone or finished. Covers D2, where a
/// completed watch answers "already completed".
fn is_gone(message: &str) -> bool {
    let lower = message.to_lowercase();
    lower.contains("not found") || lower.contains("already completed")
}

/// Parse a JSON object body. Issues are recorded, and `None` is returned when the body is not an object.
fn parse_object(bytes: &[u8], issues: &mut Issues) -> Option<Map<String, Value>> {
    match serde_json::from_slice::<Value>(bytes) {
        Ok(Value::Object(map)) => Some(map),
        Ok(other) => {
            issues.push(respond::issue(
                "model_attributes_type",
                &["body"],
                "Input should be a valid dictionary or object to extract fields from",
                other,
            ));
            None
        }
        Err(_) => {
            issues.push(respond::issue(
                "json_invalid",
                &["body", "__root__"],
                "JSON decode error",
                Value::Null,
            ));
            None
        }
    }
}

/// A required string field with a length window (counted in characters).
fn string_field(
    obj: &Map<String, Value>,
    name: &str,
    min: usize,
    max: usize,
    issues: &mut Issues,
) -> Option<String> {
    let Some(value) = obj.get(name) else {
        issues.push(respond::issue(
            "missing",
            &["body", name],
            "Field required",
            Value::Null,
        ));
        return None;
    };
    let Some(text) = value.as_str() else {
        issues.push(respond::issue(
            "string_type",
            &["body", name],
            "Input should be a valid string",
            value.clone(),
        ));
        return None;
    };
    let n = text.chars().count();
    if n < min {
        issues.push(respond::issue(
            "string_too_short",
            &["body", name],
            &format!("String should have at least {min} characters"),
            value.clone(),
        ));
        return None;
    }
    if n > max {
        issues.push(respond::issue(
            "string_too_long",
            &["body", name],
            &format!("String should have at most {max} characters"),
            value.clone(),
        ));
        return None;
    }
    Some(text.to_string())
}

/// A required float field with inclusive bounds. Numeric strings are accepted (Pydantic lax mode).
fn float_field(
    obj: &Map<String, Value>,
    name: &str,
    lo: f64,
    hi: f64,
    issues: &mut Issues,
) -> Option<f64> {
    let Some(value) = obj.get(name) else {
        issues.push(respond::issue(
            "missing",
            &["body", name],
            "Field required",
            Value::Null,
        ));
        return None;
    };
    let number = match value {
        Value::Number(n) => n.as_f64(),
        Value::String(s) => py_float(s),
        _ => None,
    };
    let Some(number) = number else {
        issues.push(respond::issue(
            "float_parsing",
            &["body", name],
            "Input should be a valid number",
            value.clone(),
        ));
        return None;
    };
    if !number.is_finite() {
        issues.push(respond::issue(
            "finite_number",
            &["body", name],
            "Input should be a finite number",
            value.clone(),
        ));
        return None;
    }
    if number < lo {
        issues.push(respond::issue(
            "greater_than_equal",
            &["body", name],
            &format!("Input should be greater than or equal to {lo}"),
            value.clone(),
        ));
        return None;
    }
    if number > hi {
        issues.push(respond::issue(
            "less_than_equal",
            &["body", name],
            &format!("Input should be less than or equal to {hi}"),
            value.clone(),
        ));
        return None;
    }
    Some(number)
}

struct PushBody {
    user_id: String,
    token: String,
    platform: String,
}

#[allow(
    clippy::result_large_err,
    reason = "the Err is the ready-made HTTP response; boxing it would add a deref at about 50 call sites for no gain on this cold path"
)]
fn parse_push_body(bytes: &[u8]) -> Result<PushBody, Response> {
    let mut issues = Issues::new();
    let obj = parse_object(bytes, &mut issues);
    let user_id = obj
        .as_ref()
        .and_then(|o| string_field(o, "user_id", 1, 128, &mut issues));
    let token = obj
        .as_ref()
        .and_then(|o| string_field(o, "token", 16, 512, &mut issues));
    let platform = match obj.as_ref().and_then(|o| o.get("platform")) {
        Some(Value::String(p)) if p == "ios" || p == "android" => Some(p.clone()),
        Some(other) => {
            issues.push(respond::issue(
                "literal_error",
                &["body", "platform"],
                "Input should be 'ios' or 'android'",
                other.clone(),
            ));
            None
        }
        None => {
            issues.push(respond::issue(
                "missing",
                &["body", "platform"],
                "Field required",
                Value::Null,
            ));
            None
        }
    };
    issues.finish()?;
    match (user_id, token, platform) {
        (Some(user_id), Some(token), Some(platform)) => Ok(PushBody {
            user_id,
            token,
            platform,
        }),
        _ => Err(respond::internal_error()),
    }
}

struct WatchBody {
    storm_cell_id: String,
    user_id: String,
    lat: f64,
    lng: f64,
}

#[allow(
    clippy::result_large_err,
    reason = "the Err is the ready-made HTTP response; boxing it would add a deref at about 50 call sites for no gain on this cold path"
)]
fn parse_watch_body(bytes: &[u8]) -> Result<WatchBody, Response> {
    let mut issues = Issues::new();
    let obj = parse_object(bytes, &mut issues);
    let storm_cell_id = obj
        .as_ref()
        .and_then(|o| string_field(o, "storm_cell_id", 1, 128, &mut issues));
    let user_id = obj
        .as_ref()
        .and_then(|o| string_field(o, "user_id", 1, 128, &mut issues));
    let lat = obj
        .as_ref()
        .and_then(|o| float_field(o, "lat", -90.0, 90.0, &mut issues));
    let lng = obj
        .as_ref()
        .and_then(|o| float_field(o, "lng", -180.0, 180.0, &mut issues));
    issues.finish()?;
    match (storm_cell_id, user_id, lat, lng) {
        (Some(storm_cell_id), Some(user_id), Some(lat), Some(lng)) => Ok(WatchBody {
            storm_cell_id,
            user_id,
            lat,
            lng,
        }),
        _ => Err(respond::internal_error()),
    }
}

/// `POST /v1/push-tokens`.
async fn post_push_token(
    State(state): State<AppState>,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    let subject = match authenticate(&headers, &state.config.workflow_signing_key) {
        Ok(subject) => subject,
        Err(resp) => return resp,
    };
    let push = match parse_push_body(&body) {
        Ok(push) => push,
        Err(resp) => return resp,
    };
    if let Err(resp) = require_owner(&subject, &push.user_id) {
        return resp;
    }
    // F9: the shipped manifests mount STATE_DIR read-only, so the SQLite write cannot succeed. Answer 503 with a
    // log line, not an opaque 500. The write itself stays a direct SQLite write, as in Python (routes_workflows.py
    // calls `upsert` directly and does not start RegisterPushTokenWorkflow).
    if let Err(err) = rustix::fs::access(&state.config.state_dir, rustix::fs::Access::WRITE_OK)
        && read_only_errno(err)
    {
        tracing::error!(
            state_dir = %state.config.state_dir.display(),
            error = %err,
            "push token store is read-only; /v1/push-tokens needs a writable STATE_DIR mount"
        );
        return respond::detail(
            StatusCode::SERVICE_UNAVAILABLE,
            "push token store is read-only",
        );
    }
    let st = state.clone();
    let stored = tokio::task::spawn_blocking(move || {
        st.push_tokens
            .upsert(&push.user_id, &push.token, &push.platform)
    })
    .await;
    match stored {
        Ok(Ok(())) => respond::json(StatusCode::OK, &json!({ "registered": true }), None),
        _ => respond::internal_error(),
    }
}

/// Errors from `access(W_OK)` that mean the store cannot be written: a read-only mount, or no permission.
/// Other errors (for example a missing directory, which the store creates) are left to the store.
fn read_only_errno(err: rustix::io::Errno) -> bool {
    matches!(
        err,
        rustix::io::Errno::ROFS | rustix::io::Errno::ACCESS | rustix::io::Errno::PERM
    )
}

/// `DELETE /v1/push-tokens/{token}`: always 204 with an empty `application/json` body.
async fn delete_push_token(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(token): Path<String>,
) -> Response {
    let subject = match authenticate(&headers, &state.config.workflow_signing_key) {
        Ok(subject) => subject,
        Err(resp) => return resp,
    };
    let st = state.clone();
    let removed =
        tokio::task::spawn_blocking(move || st.push_tokens.delete_for_user(&subject, &token)).await;
    match removed {
        Ok(Ok(_)) => (
            StatusCode::NO_CONTENT,
            [(header::CONTENT_TYPE, "application/json")],
        )
            .into_response(),
        _ => respond::internal_error(),
    }
}

/// `POST /v1/watches`: 201 with the workflow id, 409 when already watching.
async fn post_watch(State(state): State<AppState>, headers: HeaderMap, body: Bytes) -> Response {
    let subject = match authenticate(&headers, &state.config.workflow_signing_key) {
        Ok(subject) => subject,
        Err(resp) => return resp,
    };
    let watch = match parse_watch_body(&body) {
        Ok(watch) => watch,
        Err(resp) => return resp,
    };
    if let Err(resp) = require_owner(&subject, &watch.user_id) {
        return resp;
    }
    let client = match get_client(&state).await {
        Ok(client) => client,
        Err(resp) => return resp,
    };
    let workflow_id = watch_workflow_id(&watch.user_id, &watch.storm_cell_id);
    let input = json!({
        "user_id": watch.user_id,
        "storm_cell_id": watch.storm_cell_id,
        "lat": watch.lat,
        "lng": watch.lng,
    });
    let queue = &state.config.temporal_alerts_task_queue;
    match start_watch(&client, &workflow_id, queue, &input).await {
        Ok(run_id) => respond::json(
            StatusCode::CREATED,
            &json!({ "workflow_id": workflow_id, "run_id": run_id }),
            None,
        ),
        Err(temporalio_client::errors::WorkflowStartError::AlreadyStarted { .. }) => {
            respond::detail(StatusCode::CONFLICT, "already watching this storm")
        }
        Err(err) => {
            tracing::warn!("watch start failed: {err}");
            reset_client(&state).await;
            respond::detail(StatusCode::SERVICE_UNAVAILABLE, "temporal unavailable")
        }
    }
}

/// `DELETE /v1/watches/{user_id}/{storm_cell_id}`: 204 when unpinned or already gone.
async fn delete_watch(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path((user_id, storm_cell_id)): Path<(String, String)>,
) -> Response {
    let subject = match authenticate(&headers, &state.config.workflow_signing_key) {
        Ok(subject) => subject,
        Err(resp) => return resp,
    };
    if let Err(resp) = require_owner(&subject, &user_id) {
        return resp;
    }
    let client = match get_client(&state).await {
        Ok(client) => client,
        Err(resp) => return resp,
    };
    let workflow_id = watch_workflow_id(&user_id, &storm_cell_id);
    match unpin(&client, &workflow_id).await {
        Ok(()) => StatusCode::NO_CONTENT.into_response(),
        Err(err) if is_gone(&err.to_string()) => StatusCode::NO_CONTENT.into_response(),
        Err(err) => {
            tracing::warn!("watch unpin failed: {err}");
            reset_client(&state).await;
            respond::detail(StatusCode::SERVICE_UNAVAILABLE, "temporal unavailable")
        }
    }
}

/// `GET /v1/watches/{user_id}/{storm_cell_id}`: the five public watch fields.
async fn get_watch(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path((user_id, storm_cell_id)): Path<(String, String)>,
) -> Response {
    let subject = match authenticate(&headers, &state.config.workflow_signing_key) {
        Ok(subject) => subject,
        Err(resp) => return resp,
    };
    if let Err(resp) = require_owner(&subject, &user_id) {
        return resp;
    }
    let client = match get_client(&state).await {
        Ok(client) => client,
        Err(resp) => return resp,
    };
    let workflow_id = watch_workflow_id(&user_id, &storm_cell_id);
    match current_state(&client, &workflow_id).await {
        Ok(full) => {
            let pick = |key: &str| full.get(key).cloned().unwrap_or(Value::Null);
            let count = |key: &str| {
                full.get(key)
                    .filter(|v| v.is_number())
                    .cloned()
                    .unwrap_or(json!(0))
            };
            let body = json!({
                "last_frame_ts": pick("last_frame_ts"),
                "last_change_kind": pick("last_change_kind"),
                "last_notified_at": pick("last_notified_at"),
                "poll_count": count("poll_count"),
                "push_count": count("push_count"),
            });
            respond::json(StatusCode::OK, &body, None)
        }
        Err(err) if is_gone(&err.to_string()) => {
            respond::detail(StatusCode::GONE, "watch no longer exists")
        }
        Err(err) => {
            tracing::warn!("watch query failed: {err}");
            reset_client(&state).await;
            respond::detail(StatusCode::SERVICE_UNAVAILABLE, "temporal unavailable")
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn workflow_id_is_prefixed_sha256_of_user_and_cell() {
        let id = watch_workflow_id("user-1", "7");
        assert!(id.starts_with("watch:"));
        assert_eq!(id.len(), "watch:".len() + 64);
        assert_ne!(id, watch_workflow_id("user-1", "8"));
    }

    #[test]
    fn gone_classification_covers_not_found_and_completed() {
        assert!(is_gone("Workflow not found"));
        assert!(is_gone("workflow execution already completed"));
        assert!(!is_gone("connection refused"));
    }

    #[test]
    fn push_body_validation_lists_every_problem() {
        let resp = parse_push_body(br#"{"user_id": "", "token": "short", "platform": "web"}"#)
            .err()
            .expect("422");
        assert_eq!(resp.status(), StatusCode::UNPROCESSABLE_ENTITY);
    }

    #[test]
    fn watch_body_accepts_numeric_string_latitude() {
        let body =
            parse_watch_body(br#"{"storm_cell_id":"7","user_id":"u","lat":"42.5","lng":-85.0}"#)
                .expect("valid");
        assert!((body.lat - 42.5).abs() < 1e-12);
    }
}
