//! `storms.json` persistence: `storms.lock` flock, stale-frame guard, atomic replace.
//!
//! Semantics follow spec-shared 0.3, 0.4 and 6.9. The lock is `flock(2)` on the
//! sidecar `storms.lock`, which is never deleted, so Python writers interoperate.
//! Unlike the Python writer, the replacement is fsynced (tmp file, rename, then
//! directory), as spec-shared D8 recommends.

use std::fs::{self, File, OpenOptions};
use std::io::{self, Write};
use std::path::Path;

use rustix::fs::{FlockOperation, flock};
use serde_json::Value;

use crate::config::StormConfig;
use crate::detect::{detect_storms_with_clock, now_epoch_seconds};
use crate::error::StormsError;
use crate::grid::Grid;
use crate::time::parse_timestamp;

/// File name of the storm payload inside the state directory.
pub const STORMS_FILE: &str = "storms.json";
/// File name of the sidecar lock inside the state directory.
pub const STORMS_LOCK_FILE: &str = "storms.lock";

/// Result of [`write_storms_json`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WriteOutcome {
    /// A new payload was written.
    Written,
    /// The stored frame is at least as new as this one, so nothing was written.
    SkippedStale,
}

/// Exclusive `flock` on a sidecar file, released on drop (also on error paths).
#[derive(Debug)]
pub struct LockGuard {
    file: File,
}

impl LockGuard {
    /// Open (creating if needed) and lock `path` exclusively. Blocks until acquired.
    ///
    /// # Errors
    /// Returns an I/O error if the lock file cannot be opened or locked.
    pub fn acquire(path: &Path) -> io::Result<Self> {
        let file = OpenOptions::new()
            .create(true)
            .write(true)
            .truncate(false)
            .open(path)?;
        flock(&file, FlockOperation::LockExclusive)?;
        Ok(Self { file })
    }
}

impl Drop for LockGuard {
    fn drop(&mut self) {
        // Closing the descriptor releases the lock anyway; the explicit unlock makes the release prompt.
        let _ = flock(&self.file, FlockOperation::Unlock);
    }
}

/// Detect storms and atomically advance `state_dir/storms.json`.
///
/// The frame is skipped when the stored timestamp is equal or newer. A missing,
/// unreadable or non-JSON previous file is treated as "no previous frame", as in
/// Python. Such a file is logged, not fatal.
///
/// # Errors
/// Returns [`StormsError::Io`] on filesystem failures and [`StormsError::Json`]
/// if the payload cannot be serialized (for example non-finite coordinates).
pub fn write_storms_json(
    state_dir: &Path,
    grid: &Grid<'_>,
    timestamp: &str,
    cfg: &StormConfig,
) -> Result<WriteOutcome, StormsError> {
    write_storms_json_with_clock(state_dir, grid, timestamp, cfg, now_epoch_seconds())
}

/// Same as [`write_storms_json`] with an explicit wall clock (epoch seconds).
///
/// # Errors
/// See [`write_storms_json`].
pub fn write_storms_json_with_clock(
    state_dir: &Path,
    grid: &Grid<'_>,
    timestamp: &str,
    cfg: &StormConfig,
    wall_now: f64,
) -> Result<WriteOutcome, StormsError> {
    fs::create_dir_all(state_dir)?;
    let _lock = LockGuard::acquire(&state_dir.join(STORMS_LOCK_FILE))?;
    let path = state_dir.join(STORMS_FILE);
    let previous = read_previous(&path);

    if let (Some(current), Some(stored)) = (
        parse_timestamp(timestamp),
        stored_timestamp(previous.as_ref()),
    ) && current <= stored
    {
        return Ok(WriteOutcome::SkippedStale);
    }

    let doc = detect_storms_with_clock(grid, timestamp, previous.as_ref(), cfg, wall_now);
    let mut bytes = serde_json::to_vec(&doc)?;
    bytes.push(b'\n');
    atomic_replace(state_dir, &path, ".storms.", &bytes)?;
    Ok(WriteOutcome::Written)
}

/// Read the previous payload: `None` for missing, unreadable or invalid JSON.
#[must_use]
pub fn read_previous(path: &Path) -> Option<Value> {
    match fs::read(path) {
        Ok(bytes) => match serde_json::from_slice::<Value>(&bytes) {
            Ok(value) => Some(value),
            Err(err) => {
                tracing::warn!(path = %path.display(), error = %err, "storms.json is not valid JSON; treating as no previous frame");
                None
            }
        },
        Err(err) if err.kind() == io::ErrorKind::NotFound => None,
        Err(err) => {
            tracing::warn!(path = %path.display(), error = %err, "storms.json unreadable; treating as no previous frame");
            None
        }
    }
}

fn stored_timestamp(previous: Option<&Value>) -> Option<crate::time::Instant> {
    previous
        .and_then(Value::as_object)
        .and_then(|p| p.get("timestamp"))
        .and_then(Value::as_str)
        .and_then(parse_timestamp)
}

/// Write `bytes` to a temp file in `dir`, fsync it, rename it over `target`, and fsync `dir`.
///
/// The temp file is created with mode 0600, as Python's `mkstemp` does.
///
/// # Errors
/// Returns the first I/O error. The temp file is removed on failure.
pub fn atomic_replace(dir: &Path, target: &Path, prefix: &str, bytes: &[u8]) -> io::Result<()> {
    let mut tmp = tempfile::Builder::new()
        .prefix(prefix)
        .suffix(".tmp")
        .tempfile_in(dir)?;
    tmp.write_all(bytes)?;
    tmp.as_file().sync_all()?;
    tmp.persist(target).map_err(|e| e.error)?;
    sync_dir(dir)
}

/// fsync a directory, ignoring `EINVAL` (some filesystems do not support it), as spec-shared GRD-05 does.
fn sync_dir(dir: &Path) -> io::Result<()> {
    match File::open(dir).and_then(|d| d.sync_all()) {
        Ok(()) => Ok(()),
        Err(err) if err.raw_os_error() == Some(rustix::io::Errno::INVAL.raw_os_error()) => Ok(()),
        Err(err) => Err(err),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::PermissionsExt;

    fn grid_data() -> (Vec<f32>, Vec<f64>, Vec<f64>) {
        let mut data = vec![0.0_f32; 20 * 20];
        for r in 7..10 {
            for c in 7..10 {
                data[r * 20 + c] = 55.0;
            }
        }
        let lats: Vec<f64> = (0..20)
            .map(|i| 43.0 + (42.81 - 43.0) * i as f64 / 19.0)
            .collect();
        let lons: Vec<f64> = (0..20)
            .map(|i| -85.0 + (-84.81 + 85.0) * i as f64 / 19.0)
            .collect();
        (data, lats, lons)
    }

    #[test]
    fn backfilled_frame_does_not_regress_storms_json() {
        // STM-02 / DRV-05.
        let dir = tempfile::tempdir().unwrap();
        let (data, lats, lons) = grid_data();
        let grid = Grid::new(&data, 20, 20, &lats, &lons).unwrap();
        let cfg = StormConfig::default();
        assert_eq!(
            write_storms_json_with_clock(dir.path(), &grid, "2026-07-10T12:05:00+00:00", &cfg, 1.0)
                .unwrap(),
            WriteOutcome::Written
        );
        assert_eq!(
            write_storms_json_with_clock(dir.path(), &grid, "2026-07-10T12:00:00+00:00", &cfg, 2.0)
                .unwrap(),
            WriteOutcome::SkippedStale
        );
        assert_eq!(
            write_storms_json_with_clock(dir.path(), &grid, "2026-07-10T12:05:00+00:00", &cfg, 3.0)
                .unwrap(),
            WriteOutcome::SkippedStale
        );
        let stored: Value =
            serde_json::from_slice(&fs::read(dir.path().join(STORMS_FILE)).unwrap()).unwrap();
        assert_eq!(stored["timestamp"], "2026-07-10T12:05:00+00:00");
        assert_eq!(stored["generated_at"], 1.0);
    }

    #[test]
    fn written_file_is_compact_and_leaves_no_temp_files() {
        let dir = tempfile::tempdir().unwrap();
        let (data, lats, lons) = grid_data();
        let grid = Grid::new(&data, 20, 20, &lats, &lons).unwrap();
        write_storms_json_with_clock(
            dir.path(),
            &grid,
            "2026-07-10T12:00:00+00:00",
            &StormConfig::default(),
            5.0,
        )
        .unwrap();
        let raw = fs::read_to_string(dir.path().join(STORMS_FILE)).unwrap();
        assert!(raw.ends_with("}\n"));
        assert!(!raw.contains(": "), "writer B is compact");
        let names: Vec<String> = fs::read_dir(dir.path())
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
            .collect();
        assert!(names.iter().all(|n| !n.ends_with(".tmp")), "{names:?}");
        let mode = fs::metadata(dir.path().join(STORMS_FILE))
            .unwrap()
            .permissions()
            .mode()
            & 0o777;
        assert_eq!(mode, 0o600, "mkstemp-compatible mode");
    }

    #[test]
    fn corrupt_previous_file_is_treated_as_absent() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(dir.path().join(STORMS_FILE), b"{not json").unwrap();
        let (data, lats, lons) = grid_data();
        let grid = Grid::new(&data, 20, 20, &lats, &lons).unwrap();
        assert_eq!(
            write_storms_json_with_clock(
                dir.path(),
                &grid,
                "2026-07-10T12:00:00+00:00",
                &StormConfig::default(),
                5.0
            )
            .unwrap(),
            WriteOutcome::Written
        );
    }

    #[test]
    fn lock_is_exclusive_across_descriptors() {
        // flock(2) conflicts between open file descriptions, even in one process.
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(STORMS_LOCK_FILE);
        let held = LockGuard::acquire(&path).unwrap();
        let other = OpenOptions::new().write(true).open(&path).unwrap();
        let err = flock(&other, FlockOperation::NonBlockingLockExclusive).unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::WouldBlock);
        drop(held);
        flock(&other, FlockOperation::NonBlockingLockExclusive).unwrap();
    }
}
