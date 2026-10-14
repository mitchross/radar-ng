//! Durable filesystem helpers: BSD `flock(2)` guards, atomic tmp+rename writes, directory fsync,
//! and JSON file readers.
//!
//! Spec §0.3 (atomic write idioms), §0.4 (locks), §5.8 (`_durable_mkdir`, `_fsync_dir`).
//!
//! Locks are `flock(2)`, not `fcntl` record locks (spec §0.4), so Python and Rust writers that share a PVC
//! exclude each other. Lock files are never removed.

use rustix::fs::{FlockOperation, Mode, OFlags};
use rustix::io::Errno;
use serde::de::DeserializeOwned;
use serde_json::Value;
use std::fs::{self, File};
use std::io::{self, Write};
use std::os::fd::{AsFd, BorrowedFd};
use std::path::{Path, PathBuf};
use thiserror::Error;

/// A directory-sync step. Production code uses [`fsync_dir`]; tests substitute a failing one to check that
/// errors propagate (spec GRD-03..06).
pub type DirSync<'a> = &'a dyn Fn(&Path) -> io::Result<()>;

/// Holds an exclusive or shared `flock(2)` on a file. The lock is released on drop.
#[derive(Debug)]
pub struct FlockGuard {
    file: File,
}

impl FlockGuard {
    /// Block until the exclusive lock on `path` is held. Creates the lock file if needed.
    pub fn exclusive(path: &Path) -> io::Result<Self> {
        let file = open_lock_file(path)?;
        flock_retrying(&file, FlockOperation::LockExclusive)?;
        Ok(Self { file })
    }

    /// Block until a shared lock on `path` is held.
    pub fn shared(path: &Path) -> io::Result<Self> {
        let file = open_lock_file(path)?;
        flock_retrying(&file, FlockOperation::LockShared)?;
        Ok(Self { file })
    }
}

impl Drop for FlockGuard {
    fn drop(&mut self) {
        // Unlocking can only fail if the fd is already gone. Closing the fd releases the lock anyway.
        let _ = rustix::fs::flock(&self.file, FlockOperation::Unlock);
    }
}

fn open_lock_file(path: &Path) -> io::Result<File> {
    fs::OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(path)
}

fn flock_retrying(file: &File, op: FlockOperation) -> io::Result<()> {
    loop {
        match rustix::fs::flock(file, op) {
            Ok(()) => return Ok(()),
            Err(Errno::INTR) => continue,
            Err(e) => return Err(e.into()),
        }
    }
}

/// `fsync(2)` on a directory, so entries created or renamed inside it are durable.
///
/// `EINVAL` and `ENOTSUP` are ignored (the filesystem does not support directory fsync). Other errors,
/// including `EIO`, propagate.
pub fn fsync_dir(dir: &Path) -> io::Result<()> {
    fsync_dir_with(dir, |fd| rustix::fs::fsync(fd))
}

/// [`fsync_dir`] with the fsync step injected. The open step is not injectable; it is a plain
/// `O_RDONLY | O_DIRECTORY` open.
pub fn fsync_dir_with<F>(dir: &Path, sync: F) -> io::Result<()>
where
    F: FnOnce(BorrowedFd<'_>) -> rustix::io::Result<()>,
{
    let fd = match rustix::fs::open(
        dir,
        OFlags::RDONLY | OFlags::DIRECTORY | OFlags::CLOEXEC,
        Mode::empty(),
    ) {
        Ok(fd) => fd,
        Err(e) if unsupported_dir_sync(e) => return Ok(()),
        Err(e) => return Err(e.into()),
    };
    match sync(fd.as_fd()) {
        Ok(()) => Ok(()),
        Err(e) if unsupported_dir_sync(e) => Ok(()),
        Err(e) => Err(e.into()),
    }
}

fn unsupported_dir_sync(e: Errno) -> bool {
    e == Errno::INVAL || e == Errno::NOTSUP
}

/// `_durable_mkdir`: create `path` and fsync every directory from `path` up to the nearest pre-existing
/// ancestor (inclusive), so a first-time layer creation persists its parent entry too.
pub fn durable_mkdir(path: &Path, sync: DirSync<'_>) -> io::Result<()> {
    let path = std::path::absolute(path)?;
    let mut ancestor = path.clone();
    while !ancestor.exists() {
        match ancestor.parent() {
            Some(parent) if parent != ancestor => ancestor = parent.to_path_buf(),
            _ => break,
        }
    }
    fs::create_dir_all(&path)?;
    let mut current = path;
    loop {
        sync(&current)?;
        if current == ancestor {
            return Ok(());
        }
        match current.parent() {
            Some(parent) => current = parent.to_path_buf(),
            None => return Ok(()),
        }
    }
}

/// Atomically replace `path` with `bytes`: write a `0600` temp file in the same directory, fsync it,
/// `rename(2)` it over `path`, then fsync the directory. `prefix` and `suffix` shape the temp name, as the
/// Python `mkstemp(prefix=..., suffix=...)` calls do.
pub fn atomic_write(path: &Path, bytes: &[u8], prefix: &str, suffix: &str) -> io::Result<()> {
    atomic_write_with(path, bytes, prefix, suffix, &fsync_dir)
}

/// [`atomic_write`] with the directory-sync step injected.
pub fn atomic_write_with(
    path: &Path,
    bytes: &[u8],
    prefix: &str,
    suffix: &str,
    dir_sync: DirSync<'_>,
) -> io::Result<()> {
    let dir = parent_dir(path);
    let mut tmp = tempfile::Builder::new()
        .prefix(prefix)
        .suffix(suffix)
        .tempfile_in(dir)?;
    tmp.write_all(bytes)?;
    tmp.flush()?;
    tmp.as_file().sync_all()?;
    tmp.persist(path).map_err(|e| e.error)?;
    dir_sync(dir)
}

/// The parent of `path`, or `.` for a bare file name.
pub fn parent_dir(path: &Path) -> &Path {
    match path.parent() {
        Some(p) if !p.as_os_str().is_empty() => p,
        _ => Path::new("."),
    }
}

/// Errors from reading a JSON file.
#[derive(Debug, Error)]
pub enum JsonFileError {
    /// The file exists but could not be read (EIO, EISDIR, EACCES, ...).
    #[error("reading {path}: {source}")]
    Io {
        /// The file.
        path: PathBuf,
        /// Underlying error.
        #[source]
        source: io::Error,
    },
    /// The file is not valid JSON.
    #[error("parsing {path}: {source}")]
    Parse {
        /// The file.
        path: PathBuf,
        /// Underlying error.
        #[source]
        source: serde_json::Error,
    },
}

/// Read a file as UTF-8 text. A missing file is `Ok(None)`. Any other error is returned.
pub fn read_text_opt(path: &Path) -> io::Result<Option<String>> {
    match fs::read_to_string(path) {
        Ok(text) => Ok(Some(text)),
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(e),
    }
}

/// Read and parse a JSON file. Missing file: `Ok(None)`. Unreadable or unparseable: `Err`, so a writer never
/// rewrites a file it could not read (spec §3.4, MAN-07).
pub fn read_json_opt(path: &Path) -> Result<Option<Value>, JsonFileError> {
    let Some(text) = read_text_opt(path).map_err(|source| JsonFileError::Io {
        path: path.to_path_buf(),
        source,
    })?
    else {
        return Ok(None);
    };
    serde_json::from_str(&text)
        .map(Some)
        .map_err(|source| JsonFileError::Parse {
            path: path.to_path_buf(),
            source,
        })
}

/// Read a JSON file into `T`, returning `default()` when the file is missing, unreadable or invalid.
/// Use only for readers that the spec says degrade silently (for example the nowcast skill log, NSK-03).
pub fn read_json_lenient<T, F>(path: &Path, default: F) -> T
where
    T: DeserializeOwned,
    F: FnOnce() -> T,
{
    match fs::read_to_string(path)
        .ok()
        .and_then(|text| serde_json::from_str::<T>(&text).ok())
    {
        Some(value) => value,
        None => default(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;

    #[test]
    fn exclusive_lock_blocks_second_locker_until_release() {
        let dir = tempfile::tempdir().unwrap();
        let lock_path = dir.path().join("x.lock");
        let first = FlockGuard::exclusive(&lock_path).unwrap();
        let (tx, rx) = std::sync::mpsc::channel();
        let p = lock_path.clone();
        let handle = std::thread::spawn(move || {
            let _g = FlockGuard::exclusive(&p).unwrap();
            tx.send(()).unwrap();
        });
        assert!(
            rx.recv_timeout(std::time::Duration::from_millis(100))
                .is_err(),
            "should be blocked"
        );
        drop(first);
        rx.recv_timeout(std::time::Duration::from_secs(5))
            .expect("acquired after release");
        handle.join().unwrap();
    }

    #[test]
    fn fsync_dir_propagates_eio_and_ignores_einval() {
        let dir = tempfile::tempdir().unwrap();
        let eio = fsync_dir_with(dir.path(), |_| Err(Errno::IO));
        assert_eq!(
            eio.unwrap_err().raw_os_error(),
            Some(Errno::IO.raw_os_error())
        );
        assert!(fsync_dir_with(dir.path(), |_| Err(Errno::INVAL)).is_ok());
        assert!(fsync_dir_with(dir.path(), |_| Err(Errno::NOTSUP)).is_ok());
    }

    #[test]
    fn durable_mkdir_syncs_new_ancestors_up_to_existing_one() {
        let root = tempfile::tempdir().unwrap();
        let synced = RefCell::new(Vec::new());
        let record = |p: &Path| -> io::Result<()> {
            synced.borrow_mut().push(p.to_path_buf());
            Ok(())
        };
        let target = root.path().join("radar");
        durable_mkdir(&target, &record).unwrap();
        let got = synced.into_inner();
        assert!(got.contains(&target));
        assert!(got.contains(&root.path().to_path_buf()));
        assert!(target.is_dir());
    }

    #[test]
    fn atomic_write_replaces_and_leaves_no_tmp() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("state.json");
        atomic_write(&path, b"one", ".state.", ".tmp").unwrap();
        atomic_write(&path, b"two", ".state.", ".tmp").unwrap();
        assert_eq!(fs::read(&path).unwrap(), b"two");
        let names: Vec<_> = fs::read_dir(dir.path())
            .unwrap()
            .map(|e| e.unwrap().file_name())
            .collect();
        assert_eq!(names.len(), 1);
    }

    #[test]
    fn atomic_write_temp_file_is_0600() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("m.json");
        atomic_write(&path, b"{}", ".m.", ".tmp").unwrap();
        assert_eq!(
            fs::metadata(&path).unwrap().permissions().mode() & 0o777,
            0o600
        );
    }

    #[test]
    fn read_json_opt_distinguishes_missing_directory_and_corrupt() {
        let dir = tempfile::tempdir().unwrap();
        assert!(
            read_json_opt(&dir.path().join("missing.json"))
                .unwrap()
                .is_none()
        );
        assert!(matches!(
            read_json_opt(dir.path()),
            Err(JsonFileError::Io { .. })
        ));
        let bad = dir.path().join("bad.json");
        fs::write(&bad, "{\"layers\": ").unwrap();
        assert!(matches!(
            read_json_opt(&bad),
            Err(JsonFileError::Parse { .. })
        ));
    }
}
