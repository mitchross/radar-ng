//! Passthrough JSON files written by the workers: `/api/lightning`, `/api/storms`, `/api/tropical`
//! (spec 5.7, 5.8, 5.10).
//!
//! Each file is read on the blocking pool and returned as parsed. A missing file answers a
//! `no_data` body and an unreadable or malformed file answers a `state_read_failed` body, both
//! with `200`. Only the lightning `no_data` body carries `generated_at`.

use std::path::Path;

use axum::extract::State;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use serde_json::{Value, json};

use crate::respond;
use crate::state::AppState;

/// The outcome of reading one worker file.
enum Read {
    /// The file parsed to this value.
    Value(Value),
    /// The file does not exist.
    Missing,
    /// The file could not be read or parsed.
    Failed,
}

fn read_file(path: &Path) -> Read {
    match std::fs::read_to_string(path) {
        Ok(text) => match serde_json::from_str::<Value>(&text) {
            Ok(value) => Read::Value(value),
            Err(_) => Read::Failed,
        },
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => Read::Missing,
        Err(_) => Read::Failed,
    }
}

/// The shared passthrough. `missing` is the body for an absent file and `cache_max_age` the CC value.
async fn passthrough(
    state: &AppState,
    file: &'static str,
    missing: Value,
    cache_max_age: u64,
) -> Response {
    let path = state.state_file(file);
    let outcome = tokio::task::spawn_blocking(move || read_file(&path))
        .await
        .unwrap_or(Read::Failed);
    let body = match outcome {
        Read::Value(value) => value,
        Read::Missing => missing,
        Read::Failed => json!({
            "type": "FeatureCollection",
            "features": [],
            "reason": "state_read_failed",
        }),
    };
    respond::json(
        StatusCode::OK,
        &body,
        Some(&respond::public_max_age(cache_max_age)),
    )
    .into_response()
}

/// `GET /api/lightning` (CC 10).
pub async fn get_lightning(State(state): State<AppState>) -> Response {
    let missing = json!({ "type": "FeatureCollection", "features": [], "generated_at": 0, "reason": "no_data" });
    passthrough(&state, "lightning.json", missing, 10).await
}

/// `GET /api/storms` (CC 30).
pub async fn get_storms(State(state): State<AppState>) -> Response {
    let missing = json!({ "type": "FeatureCollection", "features": [], "reason": "no_data" });
    passthrough(&state, "storms.json", missing, 30).await
}

/// `GET /api/tropical` (CC 180).
pub async fn get_tropical(State(state): State<AppState>) -> Response {
    let missing = json!({ "type": "FeatureCollection", "features": [], "reason": "no_data" });
    passthrough(&state, "tropical.json", missing, 180).await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn missing_malformed_and_valid_files() {
        let tmp = tempfile::tempdir().expect("tmp");
        assert!(matches!(
            read_file(&tmp.path().join("nope.json")),
            Read::Missing
        ));
        std::fs::write(tmp.path().join("bad.json"), "{").expect("write");
        assert!(matches!(
            read_file(&tmp.path().join("bad.json")),
            Read::Failed
        ));
        std::fs::write(tmp.path().join("ok.json"), "[1,2]").expect("write");
        match read_file(&tmp.path().join("ok.json")) {
            Read::Value(v) => assert_eq!(v, json!([1, 2])),
            _ => panic!("expected a value"),
        }
    }
}
