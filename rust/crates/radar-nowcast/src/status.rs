//! `nowcast-status.json` (spec-nowcast section 9.3).
//!
//! The writer is called in two places only: the early exits (`warming_up`, `degraded`) and, after a
//! successful commit, `ok` (PLAN fix D4). Nothing says `ok` before the manifest swap.

use std::io;
use std::path::Path;

use chrono::Utc;
use radar_core::pycompat::{Style, dumps_value};
use radar_core::time::isoformat_utc;
use serde_json::{Map, Value};

/// File name in the state directory.
pub const STATUS_FILE: &str = "nowcast-status.json";

/// Write the status atomically with sorted keys, compact separators and a trailing newline.
pub fn write_status(
    state_dir: &Path,
    status: &str,
    reason: Option<&str>,
    detail: Option<&str>,
) -> io::Result<()> {
    std::fs::create_dir_all(state_dir)?;
    let mut body = Map::new();
    body.insert("status".into(), Value::from(status));
    body.insert("reason".into(), reason.map_or(Value::Null, Value::from));
    body.insert("detail".into(), detail.map_or(Value::Null, Value::from));
    body.insert("updated_at".into(), Value::from(isoformat_utc(&Utc::now())));
    let mut text = dumps_value(&Value::Object(body), Style::Compact, true);
    text.push('\n');
    radar_core::fsutil::atomic_write(
        &state_dir.join(STATUS_FILE),
        text.as_bytes(),
        ".nowcast-status.",
        ".tmp",
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn writes_sorted_compact_body() {
        let dir = tempfile::tempdir().unwrap();
        write_status(
            dir.path(),
            "warming_up",
            Some("insufficient_inputs"),
            Some("count=2,required=3"),
        )
        .unwrap();
        let text = std::fs::read_to_string(dir.path().join(STATUS_FILE)).unwrap();
        assert!(text.starts_with("{\"detail\":\"count=2,required=3\",\"reason\":\"insufficient_inputs\",\"status\":\"warming_up\",\"updated_at\":"));
        assert!(text.ends_with("}\n"));
    }

    #[test]
    fn ok_status_has_null_reason_and_detail() {
        let dir = tempfile::tempdir().unwrap();
        write_status(dir.path(), "ok", None, None).unwrap();
        let v: Value =
            serde_json::from_str(&std::fs::read_to_string(dir.path().join(STATUS_FILE)).unwrap())
                .unwrap();
        assert_eq!(v["status"], "ok");
        assert!(v["reason"].is_null());
        assert!(v["detail"].is_null());
    }
}
