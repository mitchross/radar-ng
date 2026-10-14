//! Per-attempt scratch directories (spec §10, H3/H4). Removed on drop, so failure and cancellation
//! paths cannot leak them (PLAN defect D3).

use std::path::{Path, PathBuf};

/// Identifies one activity attempt. The worker fills this from the Temporal activity context.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ActivityInfo {
    /// Workflow ID.
    pub workflow_id: String,
    /// Run ID.
    pub run_id: String,
    /// Activity ID.
    pub activity_id: String,
    /// Attempt number, starting at 1.
    pub attempt: u32,
}

/// Replace every character that is not alphanumeric, `-` or `_` with `_`, strip leading and trailing
/// `_`, and use `unknown` for an empty result.
pub fn sanitize(part: &str) -> String {
    let replaced: String = part
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '-' || c == '_' {
                c
            } else {
                '_'
            }
        })
        .collect();
    let trimmed = replaced.trim_matches('_');
    if trimmed.is_empty() {
        "unknown".to_string()
    } else {
        trimmed.to_string()
    }
}

/// Directory name for an attempt: `<prefix>-<wf>-<run>-<act>-attempt<N>-<parts...>` (spec §10).
pub fn dir_name(prefix: &str, info: &ActivityInfo, parts: &[&str]) -> String {
    let mut segs = vec![
        prefix.to_string(),
        sanitize(&info.workflow_id),
        sanitize(&info.run_id),
        sanitize(&info.activity_id),
        format!("attempt{}", info.attempt),
    ];
    segs.extend(parts.iter().map(|p| sanitize(p)));
    segs.join("-")
}

/// A created scratch directory that is removed when dropped.
#[derive(Debug)]
pub struct ScratchDir {
    path: PathBuf,
}

impl ScratchDir {
    /// Create `root/<dir_name>`. The directory is empty on return.
    pub fn create(
        root: &Path,
        prefix: &str,
        info: &ActivityInfo,
        parts: &[&str],
    ) -> std::io::Result<Self> {
        let path = root.join(dir_name(prefix, info, parts));
        if path.exists() {
            std::fs::remove_dir_all(&path)?;
        }
        std::fs::create_dir_all(&path)?;
        Ok(Self { path })
    }

    /// The directory path.
    pub fn path(&self) -> &Path {
        &self.path
    }
}

impl ScratchDir {
    /// Remove the directory on the blocking pool and wait for the removal (F5). Activities call this on the
    /// success path, so the directory is gone when the activity returns and the runtime is not blocked on NFS.
    /// Error and cancellation paths still remove it in `Drop`.
    pub async fn remove(mut self) {
        let path = std::mem::take(&mut self.path);
        if let Err(err) = tokio::task::spawn_blocking(move || std::fs::remove_dir_all(&path)).await
        {
            tracing::warn!(error = %err, "scratch removal task failed");
        }
    }
}

impl Drop for ScratchDir {
    fn drop(&mut self) {
        // An empty path means `remove` already ran.
        if !self.path.as_os_str().is_empty() {
            let _ = std::fs::remove_dir_all(&self.path);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// F5: the explicit async removal on the success path deletes the directory before it returns.
    #[tokio::test]
    async fn remove_deletes_the_directory_before_returning() {
        let root = tempfile::tempdir().expect("tempdir");
        let info = ActivityInfo {
            workflow_id: "wf".into(),
            run_id: "run".into(),
            activity_id: "act".into(),
            attempt: 1,
        };
        let scratch = ScratchDir::create(root.path(), "hrrr", &info, &["part"]).expect("create");
        let path = scratch.path().to_path_buf();
        std::fs::write(path.join("file.bin"), b"x").expect("write");
        assert!(path.exists());
        scratch.remove().await;
        assert!(!path.exists(), "removed on the success path");
    }
}
