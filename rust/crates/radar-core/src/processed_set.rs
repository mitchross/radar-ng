//! Bounded, disk-backed set of processed keys (spec-shared §4): MRMS keys, HRRR run ids, nowcast anchors.
//!
//! File format (writer style C): `{"items": ["k1", "k2"]}`, Python default separators, no trailing newline,
//! oldest first. The format is shared with Python during a rolling deploy.
//!
//! Fixes applied (PLAN, spec §14 D9 and §4.5):
//! - A corrupt, unreadable, or structurally wrong file resets to empty instead of crashing the constructor.
//!   Non-string items are dropped.
//! - Duplicates in the file are collapsed on load, so trimming never drops a key that is still present.
//! - `max_entries = 0` is rejected. Python treated it as unbounded by accident.
//! - Writes are fsynced. If the write fails, the in-memory change is rolled back.
//!
//! Concurrency: as in Python there is no lock. One writer per file is the design (spec §4.7).

use crate::fsutil;
use crate::pycompat::{self, Json, Style};
use serde_json::Value;
use std::collections::HashSet;
use std::path::{Path, PathBuf};
use thiserror::Error;

/// Errors from [`ProcessedSet`].
#[derive(Debug, Error)]
pub enum ProcessedSetError {
    /// `max_entries` must be at least 1.
    #[error("max_entries must be >= 1 (got 0)")]
    ZeroCapacity,
    /// The file could not be written. The in-memory set is unchanged.
    #[error("writing {path}: {source}")]
    Write {
        /// The state file.
        path: PathBuf,
        /// Underlying error.
        #[source]
        source: std::io::Error,
    },
}

/// A bounded insertion-ordered set persisted to one JSON file.
#[derive(Debug)]
pub struct ProcessedSet {
    path: PathBuf,
    max_entries: usize,
    items: Vec<String>,
    set: HashSet<String>,
}

impl ProcessedSet {
    /// Load the set from `path` (missing or corrupt means empty). `max_entries` must be at least 1.
    pub fn open(path: impl Into<PathBuf>, max_entries: usize) -> Result<Self, ProcessedSetError> {
        if max_entries == 0 {
            return Err(ProcessedSetError::ZeroCapacity);
        }
        let path = path.into();
        let items = load_items(&path);
        let mut set = HashSet::with_capacity(items.len());
        let mut deduped = Vec::with_capacity(items.len());
        for item in items {
            if set.insert(item.clone()) {
                deduped.push(item);
            }
        }
        Ok(Self {
            path,
            max_entries,
            items: deduped,
            set,
        })
    }

    /// Whether `key` has been added (and not yet evicted).
    pub fn contains(&self, key: &str) -> bool {
        self.set.contains(key)
    }

    /// Add `key` and persist. A key that is already present is a no-op with no write.
    /// When the set is over capacity, the oldest keys are evicted.
    pub fn add(&mut self, key: &str) -> Result<(), ProcessedSetError> {
        self.update(std::iter::once(key))
    }

    /// Add every key in `keys`, then persist once if anything was new. Duplicates within `keys` collapse.
    pub fn update<I, S>(&mut self, keys: I) -> Result<(), ProcessedSetError>
    where
        I: IntoIterator<Item = S>,
        S: AsRef<str>,
    {
        let snapshot_items = self.items.clone();
        let snapshot_set = self.set.clone();
        let mut dirty = false;
        for key in keys {
            let key = key.as_ref();
            if self.set.contains(key) {
                continue;
            }
            self.items.push(key.to_string());
            self.set.insert(key.to_string());
            dirty = true;
        }
        if !dirty {
            return Ok(());
        }
        if self.items.len() > self.max_entries {
            let drop_count = self.items.len() - self.max_entries;
            let dropped: Vec<String> = self.items.drain(..drop_count).collect();
            for key in dropped {
                self.set.remove(&key);
            }
        }
        if let Err(source) = self.flush() {
            self.items = snapshot_items;
            self.set = snapshot_set;
            return Err(ProcessedSetError::Write {
                path: self.path.clone(),
                source,
            });
        }
        Ok(())
    }

    /// Number of keys currently held.
    pub fn len(&self) -> usize {
        self.items.len()
    }

    /// Whether the set is empty.
    pub fn is_empty(&self) -> bool {
        self.items.is_empty()
    }

    /// The backing file.
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// The configured capacity.
    pub fn max_entries(&self) -> usize {
        self.max_entries
    }

    /// Keys in insertion order, oldest first.
    pub fn items(&self) -> &[String] {
        &self.items
    }

    fn flush(&self) -> std::io::Result<()> {
        let body = Json::Object(vec![(
            "items".to_string(),
            Json::Array(self.items.iter().map(|s| Json::Str(s.clone())).collect()),
        )]);
        let text = pycompat::dumps(&body, Style::Default, false);
        fsutil::atomic_write(&self.path, text.as_bytes(), ".state-", ".json.tmp")
    }
}

/// Parse the file into items. Any failure gives an empty list, with a warning logged.
fn load_items(path: &Path) -> Vec<String> {
    let value = match fsutil::read_json_opt(path) {
        Ok(None) => return Vec::new(),
        Ok(Some(value)) => value,
        Err(err) => {
            tracing::warn!(path = %path.display(), err = %err, "processed set unreadable, starting empty");
            return Vec::new();
        }
    };
    let raw = match value {
        Value::Array(items) => items,
        Value::Object(mut obj) => match obj.remove("items") {
            Some(Value::Array(items)) => items,
            None => Vec::new(),
            Some(_) => {
                tracing::warn!(path = %path.display(), "processed set has non-array items, starting empty");
                return Vec::new();
            }
        },
        _ => {
            tracing::warn!(path = %path.display(), "processed set has an unsupported shape, starting empty");
            return Vec::new();
        }
    };
    raw.into_iter()
        .filter_map(|item| match item {
            Value::String(s) => Some(s),
            _ => None,
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn read(path: &Path) -> String {
        std::fs::read_to_string(path).unwrap()
    }

    #[test]
    fn writes_style_c_without_newline_and_reloads() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("ingest-hrrr.json");
        let mut set = ProcessedSet::open(&path, 200).unwrap();
        set.add("2026-05-20T12:00:00+00:00").unwrap();
        set.update(["2026-05-20T12:02:00+00:00", "2026-05-20T12:02:00+00:00"])
            .unwrap();
        assert_eq!(
            read(&path),
            r#"{"items": ["2026-05-20T12:00:00+00:00", "2026-05-20T12:02:00+00:00"]}"#
        );
        let reloaded = ProcessedSet::open(&path, 200).unwrap();
        assert!(reloaded.contains("2026-05-20T12:02:00+00:00"));
        assert_eq!(reloaded.len(), 2);
    }

    #[test]
    fn evicts_oldest_beyond_capacity_and_discards_from_membership() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("s.json");
        let mut set = ProcessedSet::open(&path, 2).unwrap();
        set.update(["a", "b", "c"]).unwrap();
        assert_eq!(set.items(), ["b", "c"]);
        assert!(!set.contains("a"));
        assert_eq!(read(&path), r#"{"items": ["b", "c"]}"#);
    }

    #[test]
    fn no_write_when_nothing_is_new() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("s.json");
        let mut set = ProcessedSet::open(&path, 5).unwrap();
        set.add("x").unwrap();
        std::fs::remove_file(&path).unwrap();
        set.add("x").unwrap();
        assert!(!path.exists(), "a duplicate add must not rewrite the file");
    }

    #[test]
    fn zero_capacity_is_rejected() {
        let dir = tempfile::tempdir().unwrap();
        assert!(matches!(
            ProcessedSet::open(dir.path().join("s.json"), 0),
            Err(ProcessedSetError::ZeroCapacity)
        ));
    }

    #[test]
    fn legacy_array_top_level_is_accepted() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("s.json");
        std::fs::write(&path, r#"["a","b"]"#).unwrap();
        let set = ProcessedSet::open(&path, 5).unwrap();
        assert!(set.contains("a") && set.contains("b"));
    }

    #[test]
    fn corrupt_or_wrong_shape_resets_instead_of_crashing() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("s.json");
        for bad in ["{nope", "5", "null", r#"{"items": 5}"#, "\"abc\""] {
            std::fs::write(&path, bad).unwrap();
            let set = ProcessedSet::open(&path, 5).unwrap();
            assert!(set.is_empty(), "{bad}");
        }
    }

    #[test]
    fn duplicates_on_load_collapse_and_trim_keeps_membership_consistent() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("s.json");
        std::fs::write(&path, r#"{"items": ["a", "b", "a", "c"]}"#).unwrap();
        let mut set = ProcessedSet::open(&path, 2).unwrap();
        assert_eq!(set.items(), ["a", "b", "c"]);
        set.add("d").unwrap();
        assert_eq!(set.items(), ["c", "d"]);
        assert!(set.contains("c") && set.contains("d"));
        assert!(!set.contains("a") && !set.contains("b"));
    }

    #[test]
    fn non_string_items_are_dropped() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("s.json");
        std::fs::write(&path, r#"{"items": ["a", 3, null, "b"]}"#).unwrap();
        let set = ProcessedSet::open(&path, 5).unwrap();
        assert_eq!(set.items(), ["a", "b"]);
    }

    #[test]
    fn failed_flush_rolls_back_memory() {
        let dir = tempfile::tempdir().unwrap();
        // A path whose parent is a regular file cannot be written.
        let blocker = dir.path().join("blocker");
        std::fs::write(&blocker, "x").unwrap();
        let mut set = ProcessedSet::open(blocker.join("s.json"), 5).unwrap();
        assert!(set.add("k").is_err());
        assert!(!set.contains("k"));
        assert!(set.is_empty());
    }
}

#[cfg(test)]
mod golden {
    use super::*;

    /// Replays `testdata/processed_set/ops.json` and compares the flushed file with the Python output.
    #[test]
    fn golden_processed_set_matches_python() {
        let ops: serde_json::Value =
            serde_json::from_str(include_str!("../testdata/processed_set/ops.json")).unwrap();
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("ingest-hrrr.json");
        let max = ops["max_entries"].as_u64().unwrap() as usize;
        let mut set = ProcessedSet::open(path.clone(), max).unwrap();
        for op in ops["ops"].as_array().unwrap() {
            match op[0].as_str().unwrap() {
                "add" => set.add(op[1].as_str().unwrap()).unwrap(),
                "update" => {
                    let keys: Vec<String> = op[1]
                        .as_array()
                        .unwrap()
                        .iter()
                        .map(|k| k.as_str().unwrap().to_string())
                        .collect();
                    set.update(keys).unwrap();
                }
                other => panic!("unknown op {other}"),
            }
        }
        let expected_items: Vec<String> = ops["expected_items"]
            .as_array()
            .unwrap()
            .iter()
            .map(|k| k.as_str().unwrap().to_string())
            .collect();
        assert_eq!(set.items(), expected_items.as_slice());
        let written = std::fs::read_to_string(&path).unwrap();
        assert_eq!(
            written,
            include_str!("../testdata/processed_set/expected.json")
        );
    }
}
