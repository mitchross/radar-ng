//! Allocated replay bytes, staging included, with reserved space for live ingest.

use serde::{Deserialize, Serialize};
use std::os::unix::fs::MetadataExt;
use std::path::{Path, PathBuf};

pub const REPLAY_QUOTA_BYTES: u64 = 10 * 1024 * 1024 * 1024;
pub const LIVE_HEADROOM_BYTES: u64 = 20 * 1024 * 1024 * 1024;
pub const REPLAYS_DIR: &str = "replays";

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Usage {
    pub bytes: u64,
    pub allocated_bytes: u64,
    pub files: u64,
    pub inodes: u64,
}

pub fn replay_root(tile_root: &Path) -> PathBuf {
    tile_root.join(REPLAYS_DIR)
}

pub fn dir_usage(dir: &Path) -> anyhow::Result<Usage> {
    let mut usage = Usage::default();
    if !dir.try_exists()? {
        return Ok(usage);
    }
    for entry in walkdir::WalkDir::new(dir).follow_links(false) {
        let entry = entry?;
        anyhow::ensure!(
            !entry.file_type().is_symlink(),
            "replay tree contains symlink: {}",
            entry.path().display()
        );
        let metadata = entry.metadata()?;
        usage.inodes += 1;
        usage.allocated_bytes += metadata.blocks() * 512;
        if metadata.is_file() {
            usage.files += 1;
            usage.bytes += metadata.len();
        }
    }
    Ok(usage)
}

pub fn free_bytes(path: &Path) -> anyhow::Result<u64> {
    let stat = rustix::fs::statvfs(path)?;
    Ok(stat.f_bavail * stat.f_frsize)
}

pub fn check_limits(used: u64, free: u64, reserve: u64) -> anyhow::Result<()> {
    anyhow::ensure!(
        used.saturating_add(reserve) <= REPLAY_QUOTA_BYTES,
        "replay quota exceeded: {used} allocated + {reserve} reserved > {REPLAY_QUOTA_BYTES}"
    );
    anyhow::ensure!(
        free.saturating_sub(reserve) >= LIVE_HEADROOM_BYTES,
        "live headroom insufficient: {free} free - {reserve} reserved < {LIVE_HEADROOM_BYTES}"
    );
    Ok(())
}

pub fn check(tile_root: &Path, reserve: u64) -> anyhow::Result<()> {
    check_limits(
        dir_usage(&replay_root(tile_root))?.allocated_bytes,
        free_bytes(tile_root)?,
        reserve,
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn quota_counts_staging_directories_and_files() {
        let tmp = tempfile::tempdir().unwrap();
        let staging = replay_root(tmp.path()).join("case/.staging-r1");
        std::fs::create_dir_all(&staging).unwrap();
        std::fs::write(staging.join("frame"), b"x").unwrap();
        let usage = dir_usage(&replay_root(tmp.path())).unwrap();
        assert_eq!(usage.files, 1);
        assert!(usage.allocated_bytes >= 4096);
        assert_eq!(usage.inodes, 4);
        assert!(check_limits(REPLAY_QUOTA_BYTES - 1, LIVE_HEADROOM_BYTES + 2, 2).is_err());
        assert!(check_limits(0, LIVE_HEADROOM_BYTES + 1, 2).is_err());
        assert!(check_limits(REPLAY_QUOTA_BYTES - 2, LIVE_HEADROOM_BYTES + 2, 2).is_ok());
    }
    #[test]
    fn reject_symlinks_in_replays() {
        let tmp = tempfile::tempdir().unwrap();
        std::os::unix::fs::symlink(tmp.path(), tmp.path().join("loop")).unwrap();
        assert!(dir_usage(tmp.path()).is_err());
    }
}
