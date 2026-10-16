//! Completion marker and pyramid inspection (spec-tiler section 11).
//!
//! A published pyramid carries `.render-complete-v2.json`, which binds its identity to the
//! exact tile path set and content. Readers recompute both digests, so tampering with a
//! single tile is detected. The marker is written create-only (hard link of a synced temp),
//! so an existing identity can never be overwritten.

use std::collections::BTreeSet;
use std::fs::{self, File};
use std::io::{ErrorKind, Read, Write};
use std::path::Path;

use serde_json::Value;
use sha2::{Digest as _, Sha256};

use crate::error::{Result, TilerError};
use crate::identity::{Canon, MARKER_SCHEMA, PyramidIdentity, sha256_hex};
use crate::publish::{fsync_directory, fsync_file};

/// File name of the identity-bound completion marker.
pub const PYRAMID_COMPLETE_FILE: &str = ".render-complete-v2.json";
/// Schema-1 marker name. Read only to explain why a tree is unbound; never written.
pub const PREVIOUS_PYRAMID_COMPLETE_FILE: &str = ".render-complete.json";

/// A validated, identity-bound pyramid.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PyramidInspection {
    /// Number of tile PNGs.
    pub tile_count: usize,
    /// Relative tile paths, `z/x/y.png`.
    pub tile_paths: BTreeSet<String>,
    /// Identity from the marker.
    pub identity: PyramidIdentity,
}

/// Relative paths of every `*.png` file under `dir` (any depth), with `/` separators.
pub fn list_tile_paths(dir: &Path) -> Result<BTreeSet<String>> {
    let mut out = BTreeSet::new();
    collect_pngs(dir, "", &mut out)?;
    Ok(out)
}

fn collect_pngs(dir: &Path, prefix: &str, out: &mut BTreeSet<String>) -> Result<()> {
    for entry in fs::read_dir(dir)? {
        let entry = entry?;
        let name = entry.file_name().to_string_lossy().into_owned();
        let rel = if prefix.is_empty() {
            name.clone()
        } else {
            format!("{prefix}/{name}")
        };
        let file_type = entry.file_type()?;
        if file_type.is_dir() {
            collect_pngs(&entry.path(), &rel, out)?;
        } else if name.ends_with(".png") && (file_type.is_file() || file_type.is_symlink()) {
            out.insert(rel);
        }
    }
    Ok(())
}

/// `_tile_path_digest`: SHA-256 hex of the sorted paths joined by NUL.
pub fn tile_path_digest(paths: &BTreeSet<String>) -> String {
    let joined = paths
        .iter()
        .map(String::as_str)
        .collect::<Vec<_>>()
        .join("\0");
    sha256_hex(joined.as_bytes())
}

/// `_tile_content_digest`: SHA-256 over `u32_BE(len(path)) || path || u64_BE(size) || bytes`
/// for every sorted path.
pub fn tile_content_digest(dir: &Path, paths: &BTreeSet<String>) -> Result<String> {
    let mut hasher = Sha256::new();
    let mut buf = vec![0u8; 1 << 20];
    for rel in paths {
        hasher.update((rel.len() as u32).to_be_bytes());
        hasher.update(rel.as_bytes());
        let mut file = File::open(dir.join(rel))?;
        let size = file.metadata()?.len();
        hasher.update(size.to_be_bytes());
        loop {
            let n = file.read(&mut buf)?;
            if n == 0 {
                break;
            }
            hasher.update(&buf[..n]);
        }
    }
    Ok(hex::encode(hasher.finalize()))
}

/// Canonical marker body: one line, sorted keys, compact separators, trailing newline.
fn marker_body(
    paths: &BTreeSet<String>,
    content_digest: &str,
    identity: &PyramidIdentity,
) -> String {
    let doc = Canon::Object(vec![
        ("schema_version".into(), Canon::Int(MARKER_SCHEMA)),
        ("tile_count".into(), Canon::Int(paths.len() as i64)),
        (
            "tile_path_digest".into(),
            Canon::Str(tile_path_digest(paths)),
        ),
        (
            "tile_content_digest".into(),
            Canon::Str(content_digest.to_string()),
        ),
        ("identity".into(), identity.to_canon()),
    ]);
    doc.to_json() + "\n"
}

/// Write the completion marker into `dir` (spec 11, `_write_completion_marker`).
///
/// The body goes to a synced temporary that is hard-linked to the marker name. A different
/// existing marker is a conflict. The temporary is removed on every path.
pub fn write_completion_marker(
    dir: &Path,
    paths: &BTreeSet<String>,
    identity: &PyramidIdentity,
) -> Result<()> {
    let content = tile_content_digest(dir, paths)?;
    let body = marker_body(paths, &content, identity);
    let marker = dir.join(PYRAMID_COMPLETE_FILE);
    let temporary = dir.join(format!(
        "..{PYRAMID_COMPLETE_FILE}.tmp-{}",
        uuid::Uuid::new_v4().simple()
    ));
    let result = (|| -> Result<()> {
        {
            let mut file = fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&temporary)?;
            file.write_all(body.as_bytes())?;
            file.flush()?;
            fsync_file(&file)?;
        }
        match fs::hard_link(&temporary, &marker) {
            Ok(()) => {}
            Err(e) if e.kind() == ErrorKind::AlreadyExists => {
                if fs::read(&marker)? != body.as_bytes() {
                    return Err(TilerError::Conflict(format!(
                        "completion marker already binds a different identity: {}",
                        marker.display()
                    )));
                }
            }
            Err(e) => return Err(e.into()),
        }
        fsync_directory(dir)
    })();
    let _ = fs::remove_file(&temporary);
    let cleanup = fsync_directory(dir);
    result.and(cleanup)
}

fn py_int(v: Option<&Value>) -> Option<i64> {
    match v? {
        Value::Number(n) => n.as_i64().or_else(|| n.as_f64().map(|f| f.trunc() as i64)),
        Value::String(s) => s.trim().parse().ok(),
        Value::Bool(b) => Some(i64::from(*b)),
        _ => None,
    }
}

/// Validate a v2 pyramid (spec 11, `_inspect_complete_pyramid`).
///
/// Returns [`TilerError::Unbound`] when the marker is missing (the only case the Python
/// publisher may adopt). Every other failure is `Corrupt` or `Conflict`, which fail closed.
pub fn inspect_complete_pyramid(
    dir: &Path,
    expected: Option<&PyramidIdentity>,
) -> Result<PyramidInspection> {
    if !dir.is_dir() {
        return Err(TilerError::Corrupt(format!(
            "published pyramid is not a directory: {}",
            dir.display()
        )));
    }
    let paths = list_tile_paths(dir)?;
    let marker = dir.join(PYRAMID_COMPLETE_FILE);
    // An empty pyramid is valid only when a marker binds zero tiles (`FrameOptions::keep_empty`).
    if paths.is_empty() && !marker.is_file() {
        return Err(TilerError::Corrupt(format!(
            "published pyramid has no PNG tiles: {}",
            dir.display()
        )));
    }
    if !marker.is_file() {
        let detail = if dir.join(PREVIOUS_PYRAMID_COMPLETE_FILE).exists() {
            format!("legacy completion marker {PREVIOUS_PYRAMID_COMPLETE_FILE:?}")
        } else {
            "no identity-bound completion marker".to_string()
        };
        return Err(TilerError::Unbound(format!(
            "published pyramid has {detail} and requires adoption: {}",
            dir.display()
        )));
    }
    let invalid = || {
        TilerError::Corrupt(format!(
            "invalid pyramid completion marker: {}",
            marker.display()
        ))
    };
    let document: Value = fs::read_to_string(&marker)
        .map_err(|_| invalid())
        .and_then(|text| serde_json::from_str(&text).map_err(|_| invalid()))?;
    let schema = py_int(document.get("schema_version")).ok_or_else(invalid)?;
    if schema != MARKER_SCHEMA {
        return Err(TilerError::Corrupt(format!(
            "unsupported pyramid completion schema {schema}: {}",
            marker.display()
        )));
    }
    let marked = py_int(document.get("tile_count")).ok_or_else(invalid)?;
    let marked_digest = document
        .get("tile_path_digest")
        .and_then(Value::as_str)
        .ok_or_else(invalid)?;
    if marked != paths.len() as i64 {
        return Err(TilerError::Corrupt(format!(
            "pyramid completion count mismatch at {}: marker={marked}, actual={}",
            dir.display(),
            paths.len()
        )));
    }
    if marked_digest != tile_path_digest(&paths) {
        return Err(TilerError::Corrupt(format!(
            "pyramid completion path mismatch at {}",
            dir.display()
        )));
    }
    let marked_content = document
        .get("tile_content_digest")
        .and_then(Value::as_str)
        .ok_or_else(invalid)?;
    let identity = PyramidIdentity::from_document(document.get("identity").ok_or_else(invalid)?)
        .map_err(|_| invalid())?;
    if marked_content != tile_content_digest(dir, &paths)? {
        return Err(TilerError::Corrupt(format!(
            "pyramid completion content mismatch at {}",
            dir.display()
        )));
    }
    if let Some(expected) = expected
        && &identity != expected
    {
        return Err(TilerError::Conflict(format!(
            "pyramid identity mismatch at {}: existing renderer={:?} algorithm={:?}, expected renderer={:?} algorithm={:?}",
            dir.display(),
            identity.renderer,
            identity.algorithm,
            expected.renderer,
            expected.algorithm
        )));
    }
    Ok(PyramidInspection {
        tile_count: paths.len(),
        tile_paths: paths,
        identity,
    })
}

/// The validated identity of a pyramid, or `None` when absent, unbound or invalid.
pub fn complete_pyramid_identity(path: &Path) -> Option<PyramidIdentity> {
    inspect_complete_pyramid(path, None)
        .ok()
        .map(|i| i.identity)
}

/// Whether `path` is a content- and identity-validated pyramid, optionally of one renderer.
pub fn is_complete_pyramid(path: &Path, renderer: Option<&str>) -> bool {
    complete_pyramid_identity(path).is_some_and(|id| renderer.is_none_or(|r| id.renderer == r))
}

/// Paths of a directory's tile set, used by the publish protocol. The argument is a
/// `PathBuf` so callers can keep staging paths alongside.
#[cfg(test)]
mod tests {
    use super::*;
    use crate::identity::tests::sample_identity;

    fn write_tiles(dir: &Path, n: usize) {
        for i in 0..n {
            let p = dir.join("4").join("3").join(format!("{i}.png"));
            fs::create_dir_all(p.parent().unwrap()).unwrap();
            fs::write(&p, format!("tile-{i}")).unwrap();
        }
    }

    #[test]
    fn marker_roundtrips_and_detects_tampering() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path().join("pyr");
        write_tiles(&dir, 3);
        let paths = list_tile_paths(&dir).unwrap();
        let id = sample_identity();
        write_completion_marker(&dir, &paths, &id).unwrap();
        let insp = inspect_complete_pyramid(&dir, Some(&id)).unwrap();
        assert_eq!(insp.tile_count, 3);
        assert!(is_complete_pyramid(&dir, Some("indexed")));

        // Same-count path tamper: rename one tile.
        fs::rename(dir.join("4/3/0.png"), dir.join("4/3/9.png")).unwrap();
        let err = inspect_complete_pyramid(&dir, None).unwrap_err();
        assert!(err.to_string().contains("path mismatch"), "{err}");
        assert!(!is_complete_pyramid(&dir, None));
    }

    #[test]
    fn content_tamper_is_detected() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path().join("pyr");
        write_tiles(&dir, 2);
        let paths = list_tile_paths(&dir).unwrap();
        write_completion_marker(&dir, &paths, &sample_identity()).unwrap();
        fs::write(dir.join("4/3/1.png"), "tile-X").unwrap();
        let err = inspect_complete_pyramid(&dir, None).unwrap_err();
        assert!(err.to_string().contains("content mismatch"), "{err}");
    }

    #[test]
    fn marker_is_create_only_and_rejects_a_different_identity() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path().join("pyr");
        write_tiles(&dir, 1);
        let paths = list_tile_paths(&dir).unwrap();
        write_completion_marker(&dir, &paths, &sample_identity()).unwrap();
        let before = fs::read(dir.join(PYRAMID_COMPLETE_FILE)).unwrap();
        let mut other = sample_identity();
        other.policy_digest = "different".into();
        let err = write_completion_marker(&dir, &paths, &other).unwrap_err();
        assert!(err.to_string().contains("different identity"), "{err}");
        assert_eq!(fs::read(dir.join(PYRAMID_COMPLETE_FILE)).unwrap(), before);
        // No temporaries left behind.
        assert!(
            !fs::read_dir(&dir).unwrap().any(|e| e
                .unwrap()
                .file_name()
                .to_string_lossy()
                .contains(".tmp-"))
        );
    }

    #[test]
    fn unmarked_tree_is_unbound_and_empty_tree_is_corrupt() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path().join("pyr");
        write_tiles(&dir, 1);
        assert!(matches!(
            inspect_complete_pyramid(&dir, None),
            Err(TilerError::Unbound(_))
        ));
        let empty = tmp.path().join("empty");
        fs::create_dir_all(&empty).unwrap();
        fs::write(empty.join("stale"), "x").unwrap();
        assert!(matches!(
            inspect_complete_pyramid(&empty, None),
            Err(TilerError::Corrupt(_))
        ));
    }

    #[test]
    fn schema_one_marker_never_blesses_a_tree() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path().join("pyr");
        write_tiles(&dir, 1);
        fs::write(dir.join(PREVIOUS_PYRAMID_COMPLETE_FILE), "{}").unwrap();
        let err = inspect_complete_pyramid(&dir, None).unwrap_err();
        assert!(matches!(err, TilerError::Unbound(_)));
        assert!(err.to_string().contains("legacy completion marker"));
    }

    #[test]
    fn path_digest_matches_golden_vector() {
        // spec 18: _tile_path_digest({"4/3/5.png", "4/3/6.png"})
        let set: BTreeSet<String> = ["4/3/5.png", "4/3/6.png"]
            .iter()
            .map(|s| s.to_string())
            .collect();
        assert_eq!(
            tile_path_digest(&set),
            "5e456afeca265edaf43315d96b76dcf760778666ae3d5ee5882b7b1584e4e5c9"
        );
    }
}
