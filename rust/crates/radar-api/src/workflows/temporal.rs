//! The Temporal client used by the watch routes (spec 6.4).
//!
//! Connects lazily on the first request that needs it, and keeps one shared client. Any RPC
//! failure resets the client, so the next request reconnects (D3: a connect failure answers 503,
//! not Python's 500). Workflow inputs and results travel as untyped payloads with the `json/plain`
//! encoding, so the worker's JSON contract is unchanged.
//!
//! Limitation: this module compiles against `temporalio-client` 1.1.0, but it has not been run
//! against a live Temporal server in this port. The start/signal/query behavior is unverified.

use std::collections::HashMap;

use axum::http::StatusCode;
use axum::response::Response;
use serde_json::Value;
use temporalio_client::WorkflowIdReusePolicy;
use temporalio_client::{
    Client, ClientOptions, ConnectionOptions, UntypedQuery, UntypedSignal, UntypedWorkflow, Url,
    WorkflowQueryOptions, WorkflowSignalOptions, WorkflowStartOptions,
};
use temporalio_common::data_converters::RawValue;
use temporalio_common::protos::temporal::api::common::v1::Payload;

use crate::respond;
use crate::state::AppState;

/// Workflow type name registered by the worker.
pub const WATCH_WORKFLOW: &str = "WatchStormWorkflow";
/// Signal that unpins a watch.
pub const SIGNAL_UNPIN: &str = "unpinSignal";
/// Query that returns the watch state.
pub const QUERY_CURRENT_STATE: &str = "getCurrentState";

/// The `json/plain` payload for one JSON value.
#[must_use]
pub fn json_payload(value: &Value) -> Payload {
    let mut metadata = HashMap::new();
    metadata.insert("encoding".to_string(), b"json/plain".to_vec());
    Payload {
        metadata,
        data: serde_json::to_vec(value).unwrap_or_default(),
        ..Default::default()
    }
}

/// The JSON value inside the first payload of a raw result, if any.
#[must_use]
pub fn first_json(raw: &RawValue) -> Option<Value> {
    raw.payloads
        .first()
        .and_then(|p| serde_json::from_slice(&p.data).ok())
}

/// The URL that `temporalio-client` expects for a `host:port` address (plaintext gRPC).
fn target_url(address: &str) -> Result<Url, String> {
    let full = if address.contains("://") {
        address.to_string()
    } else {
        format!("http://{address}")
    };
    Url::parse(&full).map_err(|e| format!("invalid TEMPORAL_ADDRESS: {e}"))
}

/// A shared client, connecting on first use.
///
/// # Errors
/// Returns the 503 response `{"detail":"temporal unavailable"}` when the connection fails.
pub async fn get_client(state: &AppState) -> Result<Client, Response> {
    let mut slot = state.temporal.lock().await;
    if let Some(client) = slot.as_ref() {
        return Ok(client.clone());
    }
    let connect = async {
        let url = target_url(&state.config.temporal_address)?;
        let conn = ConnectionOptions::new(url).build();
        let opts = ClientOptions::new(state.config.temporal_namespace.clone()).build();
        Client::connect(conn, opts).await.map_err(|e| e.to_string())
    };
    match connect.await {
        Ok(client) => {
            *slot = Some(client.clone());
            Ok(client)
        }
        Err(err) => {
            tracing::warn!("temporal connect failed: {err}");
            Err(respond::detail(
                StatusCode::SERVICE_UNAVAILABLE,
                "temporal unavailable",
            ))
        }
    }
}

/// Drop the shared client so the next request reconnects (after an RPC failure).
pub async fn reset_client(state: &AppState) {
    *state.temporal.lock().await = None;
}

/// Start a watch. Returns `(workflow_id, run_id)`.
///
/// D1: `AllowDuplicate` lets a closed watch be started again. The Python policy
/// (`AllowDuplicateFailedOnly`) returned 409 for a completed watch. This is not yet verified
/// against a live server.
///
/// # Errors
/// Returns the start error for the caller to map (already started, or RPC failure).
pub async fn start_watch(
    client: &Client,
    workflow_id: &str,
    task_queue: &str,
    input: &Value,
) -> Result<String, temporalio_client::errors::WorkflowStartError> {
    let options = WorkflowStartOptions::new(task_queue.to_string(), workflow_id.to_string())
        .id_reuse_policy(WorkflowIdReusePolicy::AllowDuplicate)
        .build();
    let raw = RawValue::new(vec![json_payload(input)]);
    let handle = client
        .start_workflow(UntypedWorkflow::new(WATCH_WORKFLOW), raw, options)
        .await?;
    Ok(handle.run_id().unwrap_or("").to_string())
}

/// Send `unpinSignal` with no arguments.
///
/// # Errors
/// Returns the interaction error for the caller to map.
pub async fn unpin(
    client: &Client,
    workflow_id: &str,
) -> Result<(), temporalio_client::errors::WorkflowInteractionError> {
    let handle = client.get_workflow_handle::<UntypedWorkflow>(workflow_id.to_string());
    handle
        .signal(
            UntypedSignal::<UntypedWorkflow>::new(SIGNAL_UNPIN),
            RawValue::new(Vec::new()),
            WorkflowSignalOptions::default(),
        )
        .await
}

/// Query `getCurrentState` and return the raw JSON object.
///
/// # Errors
/// Returns the query error for the caller to map.
pub async fn current_state(
    client: &Client,
    workflow_id: &str,
) -> Result<Value, temporalio_client::errors::WorkflowQueryError> {
    let handle = client.get_workflow_handle::<UntypedWorkflow>(workflow_id.to_string());
    let raw = handle
        .query(
            UntypedQuery::<UntypedWorkflow>::new(QUERY_CURRENT_STATE),
            RawValue::new(Vec::new()),
            WorkflowQueryOptions::default(),
        )
        .await?;
    Ok(first_json(&raw).unwrap_or_else(|| serde_json::json!({})))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn json_payload_round_trips_with_encoding_metadata() {
        let value = serde_json::json!({"user_id": "u", "lat": 42.5});
        let payload = json_payload(&value);
        assert_eq!(
            payload.metadata.get("encoding").map(Vec::as_slice),
            Some(b"json/plain".as_slice())
        );
        let raw = RawValue::new(vec![payload]);
        assert_eq!(first_json(&raw), Some(value));
    }

    #[test]
    fn host_port_becomes_plaintext_url() {
        assert_eq!(
            target_url("localhost:7233").expect("url").as_str(),
            "http://localhost:7233/"
        );
        assert_eq!(
            target_url("https://t.example:443").expect("url").scheme(),
            "https"
        );
    }
}
