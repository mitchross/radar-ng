//! `GET /api/health`, `GET /api/livez` and `GET /api/metrics` (spec 5.11, 5.12, 10).
//!
//! Health reads the manifest snapshot for the radar age, the nowcast status file, the skill
//! summary, and `statvfs` on `TILE_DIR`. It answers 200 when everything is fine and 503 with the
//! same body otherwise. There is no `Cache-Control`.
//!
//! Disk usage follows the Python code, which computes `used = total - disk_usage().free` and
//! `free` is `f_bavail`. So `used_bytes` excludes only the blocks available to the calling user,
//! not the root-reserved blocks. Spec 18.1 suggested `f_bfree`; the README records the choice.

use axum::extract::State;
use axum::http::{StatusCode, header};
use axum::response::{IntoResponse, Response};
use chrono::{DateTime, Utc};
use radar_core::nowcast_skill;
use radar_core::pycompat::{is_falsy, py_str};
use radar_core::time::{now_isoformat, parse_iso_as_utc, round_digits};
use serde_json::{Map, Value, json};

use crate::manifest::cached_manifest;
use crate::metrics::Snapshot;
use crate::nowcast::skill_summary;
use crate::respond;
use crate::state::{AppState, ManifestSnapshot};

/// `GET /api/livez`: `{"status":"ok"}`, never touches disk, no `Cache-Control`.
pub async fn get_livez() -> Response {
    respond::json(StatusCode::OK, &json!({ "status": "ok" }), None)
}

/// Whole seconds since the newest radar frame, `max(0, int(now - latest))`. `None` when missing.
#[must_use]
pub fn radar_age_seconds(snap: &ManifestSnapshot, now: DateTime<Utc>) -> Option<i64> {
    let latest = snap
        .value
        .get("layers")
        .and_then(|l| l.get("radar"))
        .and_then(|r| r.get("latest"))
        .and_then(Value::as_str)?;
    let at = parse_iso_as_utc(latest)?;
    let millis = (now - at).num_milliseconds();
    Some((millis as f64 / 1000.0).trunc().max(0.0) as i64)
}

/// `nowcast-status.json` when it parses to an object, else `None`.
fn read_nowcast_status(state: &AppState) -> Option<Map<String, Value>> {
    let text = std::fs::read_to_string(state.state_file("nowcast-status.json")).ok()?;
    match serde_json::from_str::<Value>(&text).ok()? {
        Value::Object(map) => Some(map),
        _ => None,
    }
}

/// `(total_bytes, used_bytes, percent)` for `path`, or `None` when `statvfs` fails.
fn tiles_disk(path: &std::path::Path) -> Option<Value> {
    let st = rustix::fs::statvfs(path).ok()?;
    let frsize = st.f_frsize as u128;
    let total = (st.f_blocks as u128) * frsize;
    let free = (st.f_bavail as u128) * frsize;
    let used = total.saturating_sub(free);
    let percent = if total == 0 {
        0.0
    } else {
        round_digits(used as f64 / total as f64 * 100.0, 1)
    };
    Some(json!({
        "total_bytes": total as u64,
        "used_bytes": used as u64,
        "percent": percent,
    }))
}

/// The `nowcast_skill` object: exactly `available`, `runs_scored`, `headline`, `reason`.
fn skill_block(summary: &Value) -> Value {
    let field = |key: &str, default: Value| summary.get(key).cloned().unwrap_or(default);
    json!({
        "available": field("available", json!(false)),
        "runs_scored": field("runs_scored", json!(0)),
        "headline": field("headline", Value::Null),
        "reason": field("reason", Value::Null),
    })
}

/// `GET /api/health`.
pub async fn get_health(State(state): State<AppState>) -> Response {
    let snap = match cached_manifest(&state).await {
        Ok(s) => s,
        Err(resp) => return resp,
    };
    let now = Utc::now();
    let mut status = "ok";
    let mut reasons: Vec<String> = Vec::new();

    let mrms_age = radar_age_seconds(&snap, now);
    match mrms_age {
        None => {
            status = "degraded";
            reasons.push("no_mrms_tiles".into());
        }
        Some(age) if age > state.config.mrms_max_age_s => {
            status = "degraded";
            reasons.push(format!("mrms_stale_{age}s"));
        }
        Some(_) => {}
    }

    // F5: the status file read can hang on NFS, so it runs on the blocking pool, not a runtime worker.
    let nowcast_status = {
        let st = state.clone();
        tokio::task::spawn_blocking(move || read_nowcast_status(&st))
            .await
            .ok()
            .flatten()
    };
    if let Some(ns) = nowcast_status.as_ref().filter(|m| !m.is_empty())
        && ns.get("status") == Some(&json!("degraded"))
    {
        {
            status = "degraded";
            let reason = ns.get("reason").filter(|v| !is_falsy(v));
            reasons.push(reason.map_or_else(|| "nowcast_degraded".to_string(), py_str));
        }
    }
    let nowcast = match nowcast_status {
        Some(map) => Value::Object(map),
        None => json!({ "status": "unknown", "reason": "no_status" }),
    };

    let tile_dir = state.config.tile_dir.clone();
    let disk = tokio::task::spawn_blocking(move || tiles_disk(&tile_dir))
        .await
        .ok()
        .flatten();
    let skill = skill_summary(&state).await;

    let body = json!({
        "status": status,
        "mrms_age_s": mrms_age,
        "mrms_max_age_s": state.config.mrms_max_age_s,
        "nowcast": nowcast,
        "nowcast_skill": skill_block(&skill),
        "reasons": reasons,
        "tiles_disk": disk,
        "upstream_forecast": state.config.open_meteo_base,
        "checked_at": now_isoformat(),
    });
    let code = if status == "ok" {
        StatusCode::OK
    } else {
        StatusCode::SERVICE_UNAVAILABLE
    };
    respond::json(code, &body, None)
}

/// `GET /api/metrics`: the Prometheus text (spec 10.2).
pub async fn get_metrics(State(state): State<AppState>) -> Response {
    let snap = match cached_manifest(&state).await {
        Ok(s) => s,
        Err(resp) => return resp,
    };
    let now = Utc::now();
    let mut tile_timestamps: Vec<(String, usize)> = Vec::new();
    if let Some(layers) = snap.value.get("layers").and_then(Value::as_object) {
        let mut names: Vec<&String> = layers.keys().collect();
        names.sort();
        for name in names {
            let Some(layer) = layers.get(name).and_then(Value::as_object) else {
                continue;
            };
            let list = layer.get("frames").or_else(|| layer.get("timestamps"));
            let count = list.and_then(Value::as_array).map_or(0, Vec::len);
            tile_timestamps.push((name.clone(), count));
        }
    }
    // F5: the skill log read is file I/O, so it runs on the blocking pool.
    let summary = {
        let st = state.clone();
        tokio::task::spawn_blocking(move || {
            let log = nowcast_skill::read_log(&st.config.state_dir, &st.skill_cfg);
            nowcast_skill::summarize(&log, now, None, &st.skill_cfg)
        })
        .await
        .unwrap_or_else(|_| {
            nowcast_skill::summarize(
                &radar_core::nowcast_skill::SkillLog::empty(&state.skill_cfg),
                now,
                None,
                &state.skill_cfg,
            )
        })
    };
    let text = state.metrics.render(&Snapshot {
        tile_timestamps,
        mrms_age_s: radar_age_seconds(&snap, now),
        skill_lines: nowcast_skill::prometheus_lines(&summary),
    });
    (
        StatusCode::OK,
        [(header::CONTENT_TYPE, "text/plain; charset=utf-8")],
        text,
    )
        .into_response()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn skill_block_has_exactly_four_keys() {
        let block = skill_block(
            &json!({"available": true, "runs_scored": 3, "headline": {"pod": 0.8}, "by_lead": []}),
        );
        let keys: Vec<&str> = block
            .as_object()
            .expect("object")
            .keys()
            .map(String::as_str)
            .collect();
        assert_eq!(keys, vec!["available", "runs_scored", "headline", "reason"]);
        assert_eq!(block["reason"], Value::Null);
    }

    #[test]
    fn disk_usage_is_total_minus_available() {
        let tmp = tempfile::tempdir().expect("tmp");
        let disk = tiles_disk(tmp.path()).expect("statvfs");
        let total = disk["total_bytes"].as_u64().expect("total");
        let used = disk["used_bytes"].as_u64().expect("used");
        assert!(used <= total);
    }
}
