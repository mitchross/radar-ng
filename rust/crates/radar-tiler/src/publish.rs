//! Atomic multi-palette publish (spec-tiler sections 12, 13).
//!
//! The protocol: canonicalize and check finals, take the striped `flock` locks, classify
//! finals, render unresolved palettes into hidden staging siblings, validate every staged tree,
//! make them durable, mark each one, decide every action, then rename into place with the
//! required fsync order.
//!
//! Durability of the staged tiles (spec 13.4): on Linux one `syncfs(2)` per filesystem replaces
//! one `fsync` per tile and directory. On ext4 (Longhorn) each `fsync` is a journal commit and a
//! device flush, so a 630-tile frame paid 630 of them. Elsewhere every file is synced in turn.
//!
//! Deliberate deviations from the Python publisher (see README):
//! * a final that already holds a complete marker of *any* identity is reported as
//!   [`PublishStatus::ExistingForeign`] with its tile count, instead of raising a conflict
//!   that would retry forever (PLAN collision rule);
//! * unbound (pre-v2) trees are never adopted: the call fails with [`TilerError::Unbound`]
//!   and adoption stays with Python (spec H6);
//! * only the indexed identity is produced, so `existing_compatible` and `adopted` are never
//!   returned.

use std::collections::BTreeSet;
use std::fs::{self, File, OpenOptions};
use std::io::ErrorKind;
use std::path::{Component, Path, PathBuf};

use rayon::prelude::*;
use rustix::fs::{FlockOperation, flock};
use rustix::io::Errno;

use crate::error::{Result, TilerError};
use crate::identity::PyramidIdentity;
use crate::marker::{
    PyramidInspection, inspect_complete_pyramid, list_tile_paths, write_completion_marker,
};

/// Number of lock stripes shared by unrelated frames under one root (spec 13.1).
pub const PUBLICATION_LOCK_STRIPES: u32 = 64;
/// Name of the lock directory under each lock root.
pub const LOCK_DIR_NAME: &str = ".render-publish-locks";

/// What happened to one palette's pyramid.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PublishStatus {
    /// Rendered and renamed into place.
    Created,
    /// A complete pyramid with exactly this identity already existed.
    ExistingValid,
    /// A complete pyramid with a different identity exists (for example Python's
    /// `indexed` winner). It is left untouched and reported.
    ExistingForeign,
    /// The render produced no tiles, so nothing was published.
    Empty,
}

impl PublishStatus {
    /// Stable snake_case name, as the Python publisher spells it.
    pub fn as_str(self) -> &'static str {
        match self {
            PublishStatus::Created => "created",
            PublishStatus::ExistingValid => "existing_valid",
            PublishStatus::ExistingForeign => "existing_foreign",
            PublishStatus::Empty => "empty",
        }
    }
}

/// Outcome for one palette.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PalettePublishOutcome {
    /// What happened.
    pub status: PublishStatus,
    /// Tile PNG count of the published pyramid (0 when empty).
    pub tile_count: u64,
}

/// Outcomes for every palette of one frame, in output order.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MultiPaletteRenderResult {
    /// `(palette, outcome)` in the order the outputs were given.
    pub outcomes: Vec<(String, PalettePublishOutcome)>,
}

impl MultiPaletteRenderResult {
    /// Tile counts per palette, in output order.
    pub fn counts(&self) -> Vec<(&str, u64)> {
        self.outcomes
            .iter()
            .map(|(n, o)| (n.as_str(), o.tile_count))
            .collect()
    }

    /// Palettes with at least one tile, in output order.
    pub fn rendered_palettes(&self) -> Vec<&str> {
        self.outcomes
            .iter()
            .filter(|(_, o)| o.tile_count > 0)
            .map(|(n, _)| n.as_str())
            .collect()
    }

    /// Outcome for one palette.
    pub fn outcome(&self, palette: &str) -> Option<&PalettePublishOutcome> {
        self.outcomes
            .iter()
            .find(|(n, _)| n == palette)
            .map(|(_, o)| o)
    }
}

// ---------------------------------------------------------------------------
// Paths and durability
// ---------------------------------------------------------------------------

/// Resolve `path` like `Path.resolve(strict=False)`: canonicalize the longest existing prefix
/// and append the rest lexically.
pub fn resolve_nonstrict(path: &Path) -> Result<PathBuf> {
    let absolute = if path.is_absolute() {
        path.to_path_buf()
    } else {
        std::env::current_dir()?.join(path)
    };
    let mut existing = absolute.clone();
    let mut tail: Vec<std::ffi::OsString> = Vec::new();
    while !existing.exists() {
        match existing.file_name() {
            Some(name) => {
                tail.push(name.to_os_string());
                existing.pop();
            }
            None => break,
        }
    }
    let mut resolved = fs::canonicalize(&existing).unwrap_or(existing);
    for name in tail.iter().rev() {
        match Path::new(name).components().next() {
            Some(Component::ParentDir) => {
                resolved.pop();
            }
            Some(Component::CurDir) => {}
            _ => resolved.push(name),
        }
    }
    Ok(resolved)
}

fn is_unsupported(e: &std::io::Error) -> bool {
    matches!(
        e.raw_os_error().map(Errno::from_raw_os_error),
        Some(Errno::INVAL) | Some(Errno::NOTSUP)
    )
}

/// `fsync` a regular file. Errors are never suppressed (spec 13.8).
pub fn fsync_file(file: &File) -> Result<()> {
    file.sync_all()?;
    Ok(())
}

/// `fsync` a directory, ignoring `EINVAL`/`ENOTSUP` as the Python publisher does.
pub fn fsync_directory(path: &Path) -> Result<()> {
    let handle = match File::open(path) {
        Ok(h) => h,
        Err(e) if is_unsupported(&e) => return Ok(()),
        Err(e) => return Err(e.into()),
    };
    match handle.sync_all() {
        Ok(()) => Ok(()),
        Err(e) if is_unsupported(&e) => Ok(()),
        Err(e) => Err(e.into()),
    }
}

/// `fsync` `path`, then each parent up to and including `ancestor` (or the filesystem root).
pub fn fsync_directory_chain(path: &Path, ancestor: &Path) -> Result<()> {
    let mut current = resolve_nonstrict(path)?;
    let ancestor = resolve_nonstrict(ancestor)?;
    loop {
        fsync_directory(&current)?;
        if current == ancestor {
            return Ok(());
        }
        match current.parent() {
            Some(parent) if parent != current => current = parent.to_path_buf(),
            _ => return Ok(()),
        }
    }
}

/// Create a directory tree and persist each newly created ancestor entry (`_durable_mkdir`).
pub fn durable_mkdir(path: &Path) -> Result<()> {
    let path = if path.is_absolute() {
        path.to_path_buf()
    } else {
        std::env::current_dir()?.join(path)
    };
    let mut ancestor = path.clone();
    while !ancestor.exists() {
        match ancestor.parent() {
            Some(parent) if parent != ancestor => ancestor = parent.to_path_buf(),
            _ => break,
        }
    }
    fs::create_dir_all(&path)?;
    fsync_directory_chain(&path, &ancestor)
}

/// Flush the whole filesystem that holds `handle` with `syncfs(2)`.
///
/// `handle` must be opened before the data it covers is written: Linux 5.8+ then reports any
/// writeback error since that open, as `fsync` would for the file itself.
#[cfg(target_os = "linux")]
pub fn sync_filesystem(handle: &File) -> Result<()> {
    rustix::fs::syncfs(handle).map_err(std::io::Error::from)?;
    Ok(())
}

/// Sync every tile, then every directory from the deepest up, including `dir` (spec 13.4).
pub fn fsync_pyramid(dir: &Path, paths: &BTreeSet<String>) -> Result<()> {
    paths.par_iter().try_for_each(|rel| -> Result<()> {
        let file = File::open(dir.join(rel))?;
        fsync_file(&file)
    })?;
    let mut directories: BTreeSet<PathBuf> = BTreeSet::new();
    directories.insert(dir.to_path_buf());
    for rel in paths {
        let mut parent = dir.join(rel).parent().map(Path::to_path_buf);
        while let Some(p) = parent {
            if p == dir || !p.starts_with(dir) {
                break;
            }
            parent = p.parent().map(Path::to_path_buf);
            directories.insert(p);
        }
    }
    let mut ordered: Vec<PathBuf> = directories.into_iter().collect();
    ordered.sort_by_key(|p| std::cmp::Reverse(p.components().count()));
    for d in ordered {
        fsync_directory(&d)?;
    }
    Ok(())
}

/// Finish a prior publisher's possibly interrupted directory commit (`_fsync_bound_publication`).
pub fn fsync_bound_publication(final_dir: &Path, durability_root: &Path) -> Result<()> {
    fsync_directory(final_dir)?;
    match final_dir.parent() {
        Some(parent) => fsync_directory_chain(parent, durability_root),
        None => Ok(()),
    }
}

// ---------------------------------------------------------------------------
// Locks
// ---------------------------------------------------------------------------

/// Canonicalize finals and reject equal or nested paths (spec 13.1 step 1).
pub fn canonical_finals(outputs: &[(String, PathBuf)]) -> Result<Vec<(String, PathBuf)>> {
    let canonical = outputs
        .iter()
        .map(|(name, path)| Ok((name.clone(), resolve_nonstrict(path)?)))
        .collect::<Result<Vec<_>>>()?;
    for (i, (name, path)) in canonical.iter().enumerate() {
        for (other_name, other) in &canonical[i + 1..] {
            if path == other || path.starts_with(other) || other.starts_with(path) {
                return Err(TilerError::Invalid(format!(
                    "palette outputs must be distinct, non-nested paths: {name}={}, {other_name}={}",
                    path.display(),
                    other.display()
                )));
            }
        }
    }
    Ok(canonical)
}

/// Held locks plus the durability root for each final.
struct PublicationLocks {
    files: Vec<File>,
    roots: Vec<PathBuf>,
}

impl Drop for PublicationLocks {
    fn drop(&mut self) {
        // Release in reverse acquisition order, as the Python publisher does.
        while let Some(file) = self.files.pop() {
            let _ = flock(&file, FlockOperation::Unlock);
        }
    }
}

fn acquire_locks(
    finals: &[(String, PathBuf)],
    lock_root: Option<&Path>,
) -> Result<PublicationLocks> {
    let root_override = match lock_root {
        Some(root) => Some(resolve_nonstrict(root)?),
        None => None,
    };
    let mut roots = Vec::with_capacity(finals.len());
    let mut lock_paths: BTreeSet<PathBuf> = BTreeSet::new();
    for (_, final_path) in finals {
        let root = match &root_override {
            Some(root) => root.clone(),
            None => final_path
                .parent()
                .and_then(Path::parent)
                .map(Path::to_path_buf)
                .unwrap_or_else(|| PathBuf::from("/")),
        };
        if !(final_path != &root && final_path.starts_with(&root)) {
            return Err(TilerError::Invalid(format!(
                "publication lock root must contain every output: {}",
                root.display()
            )));
        }
        let lock_dir = root.join(LOCK_DIR_NAME);
        durable_mkdir(&lock_dir)?;
        let digest = crate::identity::sha256_hex(final_path.to_string_lossy().as_bytes());
        let stripe = u32::from_str_radix(&digest[..4], 16).expect("hex") % PUBLICATION_LOCK_STRIPES;
        lock_paths.insert(lock_dir.join(format!("{stripe:02x}.lock")));
        roots.push(root);
    }
    let mut files = Vec::with_capacity(lock_paths.len());
    for path in &lock_paths {
        let file = OpenOptions::new()
            .read(true)
            .append(true)
            .create(true)
            .open(path)?;
        flock(&file, FlockOperation::LockExclusive).map_err(std::io::Error::from)?;
        files.push(file);
    }
    Ok(PublicationLocks { files, roots })
}

// ---------------------------------------------------------------------------
// Publish
// ---------------------------------------------------------------------------

fn staging_path(final_path: &Path) -> PathBuf {
    let name = final_path
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    final_path
        .parent()
        .unwrap_or_else(|| Path::new("."))
        .join(format!(".{name}.tmp-{}", uuid::Uuid::new_v4().simple()))
}

/// Removes staging directories that are still present when it goes out of scope.
struct StagingSet(Vec<PathBuf>);

impl Drop for StagingSet {
    fn drop(&mut self) {
        for path in &self.0 {
            let _ = fs::remove_dir_all(path);
        }
    }
}

fn is_exists_error(e: &std::io::Error) -> bool {
    matches!(
        e.raw_os_error().map(Errno::from_raw_os_error),
        Some(Errno::EXIST) | Some(Errno::NOTEMPTY)
    ) || e.kind() == ErrorKind::AlreadyExists
}

/// Publish one frame's pyramids atomically per palette (spec 13).
///
/// `outputs` pairs palette names with final directories. `identities` must hold one identity
/// per palette. `render` receives the staging targets (only for unresolved palettes, in
/// output order) and returns one tile count per target. It must write the tiles and return
/// how many it wrote.
pub fn publish_palette_set<F>(
    outputs: &[(String, PathBuf)],
    identities: &[(String, PyramidIdentity)],
    lock_root: Option<&Path>,
    render: F,
) -> Result<MultiPaletteRenderResult>
where
    F: FnOnce(&[(String, PathBuf)]) -> Result<Vec<u64>>,
{
    if outputs.is_empty() {
        return Ok(MultiPaletteRenderResult {
            outcomes: Vec::new(),
        });
    }
    let identity_for = |name: &str| identities.iter().find(|(n, _)| n == name).map(|(_, id)| id);
    if identities.len() != outputs.len() || outputs.iter().any(|(n, _)| identity_for(n).is_none()) {
        return Err(TilerError::Invalid(
            "every output palette requires one publication identity".into(),
        ));
    }
    let finals = canonical_finals(outputs)?;
    let locks = acquire_locks(&finals, lock_root)?;

    let mut outcomes: Vec<Option<PalettePublishOutcome>> = vec![None; finals.len()];
    let mut unresolved: Vec<usize> = Vec::new();
    for (i, (name, final_path)) in finals.iter().enumerate() {
        if !final_path.exists() {
            unresolved.push(i);
            continue;
        }
        // Unbound and corrupt trees propagate: the Python publisher adopts unbound ones (H6).
        let inspection = inspect_complete_pyramid(final_path, None)?;
        fsync_bound_publication(final_path, &locks.roots[i])?;
        let status = if identity_for(name) == Some(&inspection.identity) {
            PublishStatus::ExistingValid
        } else {
            PublishStatus::ExistingForeign
        };
        outcomes[i] = Some(PalettePublishOutcome {
            status,
            tile_count: inspection.tile_count as u64,
        });
    }

    let assemble = |outcomes: Vec<Option<PalettePublishOutcome>>| MultiPaletteRenderResult {
        outcomes: outputs
            .iter()
            .zip(outcomes)
            .map(|((name, _), o)| (name.clone(), o.expect("every palette is resolved")))
            .collect(),
    };
    if unresolved.is_empty() {
        return Ok(assemble(outcomes));
    }

    let staged_targets: Vec<(String, PathBuf)> = unresolved
        .iter()
        .map(|&i| (finals[i].0.clone(), staging_path(&finals[i].1)))
        .collect();
    for &i in &unresolved {
        if let Some(parent) = finals[i].1.parent() {
            durable_mkdir(parent)?;
        }
    }
    let staging = StagingSet(staged_targets.iter().map(|(_, p)| p.clone()).collect());
    // Opened before the render so `syncfs` reports writeback errors from the render itself.
    #[cfg(target_os = "linux")]
    let fs_handles: Vec<File> = unresolved
        .iter()
        .map(|&i| File::open(finals[i].1.parent().unwrap_or(Path::new("/"))))
        .collect::<std::io::Result<_>>()?;
    let counts = render(&staged_targets)?;
    if counts.len() != staged_targets.len() {
        return Err(TilerError::Corrupt(
            "renderer did not report every staged palette".into(),
        ));
    }

    // Validate, sync and mark every staged tree before touching any final. A marker is written
    // only after every staged tile is durable.
    let mut staged_paths: Vec<BTreeSet<String>> = Vec::with_capacity(unresolved.len());
    for (k, &i) in unresolved.iter().enumerate() {
        let staged_dir = &staged_targets[k].1;
        let final_path = &finals[i].1;
        let count = counts[k];
        let paths = if staged_dir.exists() {
            list_tile_paths(staged_dir)?
        } else {
            BTreeSet::new()
        };
        if paths.len() as u64 != count {
            return Err(TilerError::Corrupt(format!(
                "staged pyramid tile count mismatch at {}: expected={count}, actual={}",
                staged_dir.display(),
                paths.len()
            )));
        }
        if count == 0 && final_path.exists() {
            return Err(TilerError::Conflict(format!(
                "existing pyramid conflicts with an empty render: {}",
                final_path.display()
            )));
        }
        staged_paths.push(paths);
    }
    #[cfg(target_os = "linux")]
    {
        let mut synced = BTreeSet::new();
        for (k, handle) in fs_handles.iter().enumerate() {
            let dev = rustix::fs::fstat(handle)
                .map_err(std::io::Error::from)?
                .st_dev;
            if counts[k] > 0 && synced.insert(dev) {
                sync_filesystem(handle)?;
            }
        }
    }
    #[cfg(not(target_os = "linux"))]
    for (k, paths) in staged_paths.iter().enumerate() {
        if counts[k] > 0 {
            fsync_pyramid(&staged_targets[k].1, paths)?;
        }
    }
    let mut staged: Vec<Option<PyramidInspection>> = Vec::with_capacity(unresolved.len());
    for (k, paths) in staged_paths.iter().enumerate() {
        let (name, staged_dir) = &staged_targets[k];
        if counts[k] == 0 {
            staged.push(None);
            continue;
        }
        let identity = identity_for(name).expect("checked above");
        write_completion_marker(staged_dir, paths, identity)?;
        staged.push(Some(inspect_complete_pyramid(staged_dir, Some(identity))?));
    }

    // Decide every action before mutating: an unbound final fails here, before any rename.
    enum Action {
        Empty,
        Create,
        Existing(PublishStatus, u64),
    }
    let mut actions = Vec::with_capacity(unresolved.len());
    for (k, &i) in unresolved.iter().enumerate() {
        let (name, _) = &staged_targets[k];
        let final_path = &finals[i].1;
        let identity = identity_for(name).expect("checked above");
        if counts[k] == 0 {
            actions.push(Action::Empty);
        } else if !final_path.exists() {
            actions.push(Action::Create);
        } else {
            let winner = inspect_complete_pyramid(final_path, None)?;
            let status = if &winner.identity == identity {
                PublishStatus::ExistingValid
            } else {
                PublishStatus::ExistingForeign
            };
            actions.push(Action::Existing(status, winner.tile_count as u64));
        }
    }

    for (k, &i) in unresolved.iter().enumerate() {
        let (_, staged_dir) = &staged_targets[k];
        let final_path = &finals[i].1;
        let identity = identity_for(&finals[i].0).expect("checked above");
        let outcome = match &actions[k] {
            Action::Empty => {
                let _ = fs::remove_dir_all(staged_dir);
                PalettePublishOutcome {
                    status: PublishStatus::Empty,
                    tile_count: 0,
                }
            }
            Action::Existing(status, count) => {
                let _ = fs::remove_dir_all(staged_dir);
                PalettePublishOutcome {
                    status: *status,
                    tile_count: *count,
                }
            }
            Action::Create => match fs::rename(staged_dir, final_path) {
                Ok(()) => {
                    fsync_directory_chain(
                        final_path.parent().unwrap_or(Path::new("/")),
                        &locks.roots[i],
                    )?;
                    let created = inspect_complete_pyramid(final_path, Some(identity))?;
                    PalettePublishOutcome {
                        status: PublishStatus::Created,
                        tile_count: created.tile_count as u64,
                    }
                }
                Err(e) if is_exists_error(&e) => {
                    // Another writer published this immutable path first. Its tree must be bound.
                    let winner = inspect_complete_pyramid(final_path, None)?;
                    let _ = fs::remove_dir_all(staged_dir);
                    let status = if &winner.identity == identity {
                        PublishStatus::ExistingValid
                    } else {
                        PublishStatus::ExistingForeign
                    };
                    PalettePublishOutcome {
                        status,
                        tile_count: winner.tile_count as u64,
                    }
                }
                Err(e) => return Err(e.into()),
            },
        };
        outcomes[i] = Some(outcome);
    }
    drop(staging);
    drop(locks);
    Ok(assemble(outcomes))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::identity::tests::sample_identity;

    #[test]
    fn nested_and_equal_outputs_fail_before_any_mutation() {
        let tmp = tempfile::tempdir().unwrap();
        let a = tmp.path().join("radar/classic");
        let nested = a.join("ts");
        let outputs = vec![("a".to_string(), a.clone()), ("b".to_string(), nested)];
        let err = canonical_finals(&outputs).unwrap_err();
        assert!(err.to_string().contains("distinct, non-nested"));
        let eq = vec![("a".to_string(), a.clone()), ("b".to_string(), a)];
        assert!(canonical_finals(&eq).is_err());
        assert!(!tmp.path().join("radar").exists());
    }

    #[test]
    fn lock_root_must_be_a_strict_output_ancestor() {
        let tmp = tempfile::tempdir().unwrap();
        let out = tmp.path().join("radar/classic/ts");
        let outputs = vec![("classic".to_string(), out.clone())];
        let finals = canonical_finals(&outputs).unwrap();
        let err = acquire_locks(&finals, Some(&out)).err().expect("error");
        assert!(err.to_string().contains("lock root"));
        assert!(!out.exists());
    }

    #[test]
    fn fsync_directory_chain_stops_at_ancestor() {
        let tmp = tempfile::tempdir().unwrap();
        let deep = tmp.path().join("a/b/c");
        fs::create_dir_all(&deep).unwrap();
        fsync_directory_chain(&deep, tmp.path()).unwrap();
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn sync_filesystem_flushes_through_a_directory_handle() {
        let tmp = tempfile::tempdir().unwrap();
        let handle = File::open(tmp.path()).unwrap();
        fs::write(tmp.path().join("t.png"), b"png").unwrap();
        sync_filesystem(&handle).unwrap();
    }

    #[test]
    fn publish_creates_then_reports_existing_valid_and_foreign() {
        let tmp = tempfile::tempdir().unwrap();
        let out = tmp.path().join("radar/classic/ts");
        let outputs = vec![("classic".to_string(), out.clone())];
        let id = sample_identity();
        let ids = vec![("classic".to_string(), id.clone())];
        let render = |targets: &[(String, PathBuf)]| -> Result<Vec<u64>> {
            let dir = &targets[0].1;
            for i in 0..2 {
                let p = dir.join("4/0").join(format!("{i}.png"));
                fs::create_dir_all(p.parent().unwrap()).unwrap();
                fs::write(&p, format!("t{i}")).unwrap();
            }
            Ok(vec![2])
        };
        let first = publish_palette_set(&outputs, &ids, None, render).unwrap();
        assert_eq!(
            first.outcome("classic").unwrap().status,
            PublishStatus::Created
        );
        assert_eq!(first.counts(), vec![("classic", 2)]);
        assert!(
            !fs::read_dir(out.parent().unwrap()).unwrap().any(|e| e
                .unwrap()
                .file_name()
                .to_string_lossy()
                .contains(".tmp-"))
        );

        let retry =
            publish_palette_set(&outputs, &ids, None, |_| panic!("no render on retry")).unwrap();
        assert_eq!(
            retry.outcome("classic").unwrap().status,
            PublishStatus::ExistingValid
        );

        let mut other = id.clone();
        other.policy_digest = "foreign".into();
        let foreign =
            publish_palette_set(&outputs, &[("classic".to_string(), other)], None, |_| {
                panic!("no render")
            })
            .unwrap();
        assert_eq!(
            foreign.outcome("classic").unwrap().status,
            PublishStatus::ExistingForeign
        );
        assert_eq!(foreign.outcome("classic").unwrap().tile_count, 2);
    }

    #[test]
    fn transparent_render_publishes_nothing() {
        let tmp = tempfile::tempdir().unwrap();
        let out = tmp.path().join("radar/classic/ts");
        let outputs = vec![("classic".to_string(), out.clone())];
        let ids = vec![("classic".to_string(), sample_identity())];
        let result = publish_palette_set(&outputs, &ids, None, |_| Ok(vec![0])).unwrap();
        assert_eq!(
            result.outcome("classic").unwrap().status,
            PublishStatus::Empty
        );
        assert!(!out.exists());
        assert!(result.rendered_palettes().is_empty());
    }

    #[test]
    fn render_failure_cleans_every_staging_directory() {
        let tmp = tempfile::tempdir().unwrap();
        let out = tmp.path().join("radar/classic/ts");
        let outputs = vec![("classic".to_string(), out.clone())];
        let ids = vec![("classic".to_string(), sample_identity())];
        let err = publish_palette_set(&outputs, &ids, None, |targets| {
            fs::create_dir_all(targets[0].1.join("4/0")).unwrap();
            fs::write(targets[0].1.join("4/0/0.png"), "x").unwrap();
            Err(TilerError::Invalid("boom".into()))
        });
        assert!(err.is_err());
        assert!(!out.exists());
        let parent = out.parent().unwrap();
        assert!(
            !fs::read_dir(parent).unwrap().any(|e| e
                .unwrap()
                .file_name()
                .to_string_lossy()
                .contains(".tmp-"))
        );
    }

    #[test]
    fn unbound_final_is_an_error_not_an_adoption() {
        let tmp = tempfile::tempdir().unwrap();
        let out = tmp.path().join("radar/classic/ts");
        fs::create_dir_all(out.join("4/0")).unwrap();
        fs::write(out.join("4/0/0.png"), "legacy").unwrap();
        let outputs = vec![("classic".to_string(), out.clone())];
        let ids = vec![("classic".to_string(), sample_identity())];
        let err =
            publish_palette_set(&outputs, &ids, None, |_| panic!("must not render")).unwrap_err();
        assert!(matches!(err, TilerError::Unbound(_)));
        assert!(!out.join(crate::marker::PYRAMID_COMPLETE_FILE).exists());
    }
}
