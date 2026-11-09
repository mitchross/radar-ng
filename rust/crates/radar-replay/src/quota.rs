//! Replay disk quota and live-data headroom (design §3).

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use thiserror::Error;

/// Combined replay quota including staging (10 GiB).
pub const REPLAY_QUOTA_BYTES: u64 = 10 * 1024 * 1024 * 1024;
/// Free space that must remain for live data after an import (20 GiB).
pub const LIVE_HEADROOM_BYTES: u64 = 20 * 1024 * 1024 * 1024;
/// Directory under the tiles root that holds every replay revision.
pub const REPLAYS_DIR: &str = "replays";

/// Quota failure.
#[derive(Debug, Error)]
pub enum QuotaError {
    /// The filesystem could not be measured.
    #[error("cannot measure free space at {path}: {source}")]
    Stat {
        /// Path measured.
        path: PathBuf,
        /// Underlying error.
        source: std::io::Error,
    },
    /// Used bytes were not measured.
    #[error("cannot measure replay usage at {path}: {source}")]
    Walk {
        /// Path walked.
        path: PathBuf,
        /// Underlying error.
        source: std::io::Error,
    },
    /// The import would exceed the replay quota.
    #[error("replay would use {needed} bytes, over the {limit} byte replay quota")]
    OverQuota {
        /// Projected bytes.
        needed: u64,
        /// Quota.
        limit: u64,
    },
    /// The import would leave too little free space.
    #[error("import would leave {free_after} bytes free, under the {headroom} byte live headroom")]
    OverHeadroom {
        /// Free bytes after the import.
        free_after: u64,
        /// Required headroom.
        headroom: u64,
    },
}

/// Bytes and files under a directory.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Usage {
    /// Sum of file sizes.
    pub bytes: u64,
    /// Regular files counted.
    pub files: u64,
}

/// The replay root below the tiles root.
pub fn replay_root(tile_root: &Path) -> PathBuf {
    tile_root.join(REPLAYS_DIR)
}

/// Every replay write goes through this root; nothing else may be touched.
pub fn ensure_under_replay_root(path: &Path, tile_root: &Path) -> anyhow::Result<()> {
    let root = replay_root(tile_root);
    anyhow::ensure!(
        path.starts_with(&root),
        "refusing to write {} outside the replay root {}",
        path.display(),
        root.display()
    );
    Ok(())
}

/// Walk `dir` and sum file bytes. A missing directory is empty.
pub fn dir_usage(dir: &Path) -> Result<Usage, QuotaError> {
    let mut usage = Usage::default();
    if !dir.exists() {
        return Ok(usage);
    }
    for entry in walkdir::WalkDir::new(dir).follow_links(false) {
        let entry = entry.map_err(|e| QuotaError::Walk {
            path: dir.to_path_buf(),
            source: std::io::Error::other(e),
        })?;
        if entry.file_type().is_file() {
            usage.files += 1;
            usage.bytes += entry
                .metadata()
                .map_err(|e| QuotaError::Walk {
                    path: entry.path().to_path_buf(),
                    source: std::io::Error::other(e),
                })?
                .len();
        }
    }
    Ok(usage)
}

/// Free bytes available to this user at `path`.
pub fn free_bytes(path: &Path) -> Result<u64, QuotaError> {
    let stat = rustix::fs::statvfs(path).map_err(|e| QuotaError::Stat {
        path: path.to_path_buf(),
        source: std::io::Error::from(e),
    })?;
    Ok(stat.f_bavail as u64 * stat.f_frsize as u64)
}

/// Check `needed` new bytes against the quota and the live headroom.
pub fn check(tile_root: &Path, needed: u64) -> Result<(), QuotaError> {
    let used = dir_usage(&replay_root(tile_root))?
        .bytes
        .saturating_add(needed);
    if used > REPLAY_QUOTA_BYTES {
        return Err(QuotaError::OverQuota {
            needed: used,
            limit: REPLAY_QUOTA_BYTES,
        });
    }
    let free = free_bytes(tile_root)?;
    let free_after = free.saturating_sub(needed);
    if free_after < LIVE_HEADROOM_BYTES {
        return Err(QuotaError::OverHeadroom {
            free_after,
            headroom: LIVE_HEADROOM_BYTES,
        });
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn usage_walks_files_and_replay_root_is_a_sibling_of_layers() {
        let tmp = tempfile::tempdir().unwrap();
        let tiles = tmp.path();
        let root = replay_root(tiles);
        assert_eq!(root, tiles.join("replays"));
        std::fs::create_dir_all(root.join("rolling-fork-2023/rev1/tiles/radar")).unwrap();
        std::fs::write(root.join("rolling-fork-2023/rev1/manifest.json"), b"{}").unwrap();
        std::fs::write(
            root.join("rolling-fork-2023/rev1/tiles/radar/0001.png"),
            b"png",
        )
        .unwrap();
        let usage = dir_usage(&root).unwrap();
        assert_eq!(usage.files, 2);
        assert_eq!(usage.bytes, 5);
    }

    #[test]
    fn writes_outside_the_replay_root_are_refused() {
        let tmp = tempfile::tempdir().unwrap();
        let root = replay_root(tmp.path());
        assert!(ensure_under_replay_root(&root.join("case/rev"), tmp.path()).is_ok());
        assert!(ensure_under_replay_root(&tmp.path().join("radar/2026"), tmp.path()).is_err());
    }

    #[test]
    fn headroom_rejects_an_import_larger_than_free_space() {
        let tmp = tempfile::tempdir().unwrap();
        let free = free_bytes(tmp.path()).unwrap();
        assert!(free > 0);
        let huge = free.saturating_add(LIVE_HEADROOM_BYTES);
        assert!(matches!(
            check(tmp.path(), huge),
            Err(QuotaError::OverQuota { .. } | QuotaError::OverHeadroom { .. })
        ));
        assert!(check(tmp.path(), 1024).is_ok());
    }
}
