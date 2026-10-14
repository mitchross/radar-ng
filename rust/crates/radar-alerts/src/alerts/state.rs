//! Alert state files in `STATE_DIR` (spec-ingest-aux section 10.2 and 10.3).
//!
//! - `alerts_seen.json`: JSON array of alert IDs already handled. Capped at 5000 entries.
//! - `alerts_active_snapshot.json`: sorted JSON array of every ID active in the last fetch. `mark_alerts_seen` reads
//!   it to decide which seen IDs have expired.
//!
//! Both are written with the same temp-file-then-rename protocol as the Python version. Default `json.dump`
//! separators are used.

use std::collections::BTreeSet;
use std::io;
use std::path::Path;

use radar_core::fsutil::atomic_write;
use radar_core::pycompat::py_str;
use radar_core::pycompat::{Json, Style, dumps};
use serde_json::Value;

/// File name of the seen set.
pub const SEEN_FILE: &str = "alerts_seen.json";
/// File name of the active snapshot.
pub const SNAPSHOT_FILE: &str = "alerts_active_snapshot.json";
/// Upper bound on the seen set (`5000`).
pub const MAX_SEEN: usize = 5000;

/// Read a top-level JSON array. Missing files, unreadable files, invalid JSON and non-array JSON all give `None`.
fn read_array(path: &Path) -> Option<Vec<Value>> {
    let bytes = std::fs::read(path).ok()?;
    match serde_json::from_slice::<Value>(&bytes).ok()? {
        Value::Array(items) => Some(items),
        _ => None,
    }
}

/// The seen set. Missing or bad files give an empty set. Non-string entries are dropped (they could never match an
/// alert ID anyway).
#[must_use]
pub fn load_seen(state_dir: &Path) -> BTreeSet<String> {
    read_array(&state_dir.join(SEEN_FILE))
        .unwrap_or_default()
        .iter()
        .filter_map(|v| v.as_str().map(str::to_owned))
        .collect()
}

/// The active snapshot written by the last fetch. `None` when missing or bad, which means "no pruning this run".
#[must_use]
pub fn load_active_snapshot(state_dir: &Path) -> Option<BTreeSet<String>> {
    read_array(&state_dir.join(SNAPSHOT_FILE)).map(|items| items.iter().map(py_str).collect())
}

fn sorted_array(values: impl Iterator<Item = String>) -> String {
    dumps(
        &Json::Array(values.map(Json::Str).collect()),
        Style::Default,
        false,
    )
}

/// Write the seen set: every active ID survives, and expired IDs fill whatever is left of the cap. Expired IDs are
/// taken in sorted order. The write is atomic, so a torn write cannot re-fire every active alert.
pub fn save_seen(
    state_dir: &Path,
    active: &BTreeSet<String>,
    expired: &BTreeSet<String>,
) -> io::Result<()> {
    let room = MAX_SEEN.saturating_sub(active.len());
    let text = sorted_array(
        active
            .iter()
            .cloned()
            .chain(expired.iter().take(room).cloned()),
    );
    std::fs::create_dir_all(state_dir)?;
    atomic_write(
        &state_dir.join(SEEN_FILE),
        text.as_bytes(),
        ".alerts-seen.",
        ".tmp",
    )
}

/// Write the active snapshot as a sorted JSON array.
pub fn write_active_snapshot(state_dir: &Path, active: &BTreeSet<String>) -> io::Result<()> {
    let text = sorted_array(active.iter().cloned());
    std::fs::create_dir_all(state_dir)?;
    atomic_write(
        &state_dir.join(SNAPSHOT_FILE),
        text.as_bytes(),
        ".alerts-active.",
        ".tmp",
    )
}

/// Commit `handled` to the seen set (spec 10.3). Synchronous, so the async activity wrapper runs it on a blocking
/// thread. Idempotent: running it twice with the same input gives the same file.
pub fn commit_handled(state_dir: &Path, handled: &[String]) -> io::Result<()> {
    let seen = load_seen(state_dir);
    let handled: BTreeSet<String> = handled.iter().cloned().collect();
    match load_active_snapshot(state_dir) {
        None => {
            // No snapshot: never forget. Keep everything, add the handled IDs.
            let all: BTreeSet<String> = seen.union(&handled).cloned().collect();
            save_seen(state_dir, &all, &BTreeSet::new())
        }
        Some(active) => {
            let kept: BTreeSet<String> = seen.intersection(&active).cloned().collect();
            let next_active: BTreeSet<String> = kept.union(&handled).cloned().collect();
            let expired: BTreeSet<String> = seen.difference(&active).cloned().collect();
            save_seen(state_dir, &next_active, &expired)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn set(items: &[&str]) -> BTreeSet<String> {
        items.iter().map(|s| (*s).to_owned()).collect()
    }

    fn write(dir: &Path, name: &str, text: &str) {
        std::fs::write(dir.join(name), text).unwrap();
    }

    #[test]
    fn spec_10_3_handled_and_expired_example() {
        let dir = tempfile::tempdir().unwrap();
        write(dir.path(), SEEN_FILE, r#"["old-1"]"#);
        write(
            dir.path(),
            SNAPSHOT_FILE,
            r#"["old-1", "new-failed", "new-ok"]"#,
        );
        commit_handled(dir.path(), &["new-ok".to_owned()]).unwrap();
        let seen = load_seen(dir.path());
        assert!(seen.contains("new-ok") && seen.contains("old-1"));
        assert!(!seen.contains("new-failed"));
    }

    #[test]
    fn expired_ids_are_kept_as_cap_filler_and_active_ids_survive() {
        let dir = tempfile::tempdir().unwrap();
        write(dir.path(), SEEN_FILE, r#"["expired-1", "active-1"]"#);
        write(dir.path(), SNAPSHOT_FILE, r#"["active-1"]"#);
        commit_handled(dir.path(), &[]).unwrap();
        assert_eq!(load_seen(dir.path()), set(&["active-1", "expired-1"]));
    }

    #[test]
    fn missing_snapshot_never_forgets_seen_ids() {
        let dir = tempfile::tempdir().unwrap();
        write(dir.path(), SEEN_FILE, r#"["old-1"]"#);
        commit_handled(dir.path(), &["new-ok".to_owned()]).unwrap();
        assert_eq!(load_seen(dir.path()), set(&["new-ok", "old-1"]));
    }

    #[test]
    fn corrupt_snapshot_is_treated_as_missing() {
        let dir = tempfile::tempdir().unwrap();
        write(dir.path(), SEEN_FILE, r#"["old-1"]"#);
        write(dir.path(), SNAPSHOT_FILE, r#"{"not": "a list"}"#);
        assert!(load_active_snapshot(dir.path()).is_none());
        commit_handled(dir.path(), &["x".to_owned()]).unwrap();
        assert_eq!(load_seen(dir.path()), set(&["old-1", "x"]));
    }

    #[test]
    fn commit_is_idempotent() {
        let dir = tempfile::tempdir().unwrap();
        write(dir.path(), SEEN_FILE, r#"["a"]"#);
        write(dir.path(), SNAPSHOT_FILE, r#"["a", "b"]"#);
        commit_handled(dir.path(), &["b".to_owned(), "b".to_owned()]).unwrap();
        let first = std::fs::read_to_string(dir.path().join(SEEN_FILE)).unwrap();
        commit_handled(dir.path(), &["b".to_owned(), "b".to_owned()]).unwrap();
        let second = std::fs::read_to_string(dir.path().join(SEEN_FILE)).unwrap();
        assert_eq!(first, second);
    }

    #[test]
    fn cap_keeps_every_seen_active_id_and_trims_only_expired() {
        let dir = tempfile::tempdir().unwrap();
        // 4000 seen IDs, of which 3000 are still active (2000 "act" and 1000 "old"). 3000 have expired.
        let seen: Vec<String> = (0..2000)
            .map(|i| format!("act-{i:04}"))
            .chain((0..2000).map(|i| format!("old-{i:04}")))
            .collect();
        let mut active: Vec<String> = (0..2000).map(|i| format!("act-{i:04}")).collect();
        active.extend((0..1000).map(|i| format!("old-{i:04}")));
        // Active but never seen: must not be added by the commit (only handled IDs are).
        active.extend((0..500).map(|i| format!("fresh-{i:04}")));
        write(
            dir.path(),
            SEEN_FILE,
            &serde_json::to_string(&seen).unwrap(),
        );
        write(
            dir.path(),
            SNAPSHOT_FILE,
            &serde_json::to_string(&active).unwrap(),
        );
        commit_handled(dir.path(), &["new-1".to_owned()]).unwrap();
        let on_disk: Vec<String> =
            serde_json::from_str(&std::fs::read_to_string(dir.path().join(SEEN_FILE)).unwrap())
                .unwrap();
        assert!(on_disk.len() <= MAX_SEEN);
        let got = load_seen(dir.path());
        for id in seen.iter().filter(|id| active.contains(id)) {
            assert!(
                got.contains(id),
                "seen and active id {id} must survive the cap"
            );
        }
        assert!(got.contains("new-1"));
        assert!(
            !got.contains("fresh-0000"),
            "never-handled active IDs stay unseen"
        );
    }

    #[test]
    fn snapshot_format_is_sorted_python_json() {
        let dir = tempfile::tempdir().unwrap();
        write_active_snapshot(dir.path(), &set(&["urn:b", "urn:a", "é-x"])).unwrap();
        let text = std::fs::read_to_string(dir.path().join(SNAPSHOT_FILE)).unwrap();
        // Python json.dump uses ensure_ascii=True, so the non-ASCII character is escaped.
        assert_eq!(text, r#"["urn:a", "urn:b", "\u00e9-x"]"#);
    }
}
