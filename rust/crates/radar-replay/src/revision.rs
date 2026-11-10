//! Revision layout, resumable progress and atomic publication (design §3, §4).

use radar_core::fsutil::fsync_dir;
use std::collections::BTreeMap;
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};

use chrono::Utc;
use radar_core::manifest::{
    Frame, Frame as ManifestFrame, Layer, MANIFEST_SCHEMA_VERSION, Manifest,
};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};
use thiserror::Error;

use crate::case::CaseDefinition;
use crate::quota::replay_root;

/// Progress file name inside a staging revision.
pub const PROGRESS_FILE: &str = "progress.json";
/// Coverage file name inside a revision.
pub const COVERAGE_FILE: &str = "coverage.json";
/// Import report file name inside a revision.
pub const REPORT_FILE: &str = "import-report.json";
/// Revision manifest file name.
pub const MANIFEST_FILE: &str = "manifest.json";

/// Revision errors.
#[derive(Debug, Error)]
pub enum RevisionError {
    /// The revision name is not a safe path segment.
    #[error("invalid revision {0:?}: use lowercase letters, digits, dot, dash or underscore")]
    InvalidRevision(String),
    /// The revision is already published.
    #[error("revision {} already exists; pick a new --revision", .0.display())]
    Exists(PathBuf),
}

/// Where one revision lives.
#[derive(Debug, Clone)]
pub struct Layout {
    /// `<TILE_DIR>/replays/<case>`.
    pub case_dir: PathBuf,
    /// `<case_dir>/.staging-<revision>`.
    pub staging: PathBuf,
    /// `<case_dir>/<revision>`.
    pub final_dir: PathBuf,
    /// Revision name.
    pub revision: String,
}

/// Resolve and validate the layout for a case revision.
pub fn layout(
    tile_root: &Path,
    case: &CaseDefinition,
    revision: &str,
) -> Result<Layout, RevisionError> {
    let safe = revision.starts_with(|c: char| c.is_ascii_lowercase() || c.is_ascii_digit())
        && revision.len() <= 64
        && revision
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || matches!(c, '-' | '_' | '.'));
    if !safe {
        return Err(RevisionError::InvalidRevision(revision.to_string()));
    }
    let case_dir = replay_root(tile_root).join(case.id);
    Ok(Layout {
        staging: case_dir.join(format!(".staging-{revision}")),
        final_dir: case_dir.join(revision),
        case_dir,
        revision: revision.to_string(),
    })
}

/// One rendered frame.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FrameProgress {
    /// Layer name.
    pub layer: String,
    /// Base playhead this frame serves.
    pub playhead: String,
    /// Object valid time.
    pub timestamp: String,
    /// Source S3 key.
    pub key: String,
    /// Stable tiler source identity.
    pub source_id: String,
    pub source_sha256: String,
    pub flag_key: Option<String>,
    pub flag_sha256: Option<String>,
    /// Tiles written for this frame.
    pub tiles: u64,
    /// PNG bytes written for this frame.
    pub bytes: u64,
    /// Render time in milliseconds.
    pub render_ms: u64,
}

impl FrameProgress {
    /// Progress key: one frame per layer and playhead.
    pub fn id(&self) -> String {
        format!("{}@{}", self.layer, self.playhead)
    }
}

/// Resumable progress of a staging revision.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Progress {
    /// Revision name.
    pub revision: String,
    /// Case id.
    pub case: String,
    /// Completed frames keyed by `layer@playhead`.
    pub frames: BTreeMap<String, FrameProgress>,
}

impl Progress {
    /// Load progress, or a fresh empty record when the file does not exist.
    pub fn load(path: &Path, case: &str, revision: &str) -> anyhow::Result<Self> {
        match fs::read(path) {
            Ok(bytes) => {
                let progress: Progress = serde_json::from_slice(&bytes)?;
                anyhow::ensure!(
                    progress.case == case && progress.revision == revision,
                    "progress file belongs to {}/{}",
                    progress.case,
                    progress.revision
                );
                Ok(progress)
            }
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(Progress {
                revision: revision.to_string(),
                case: case.to_string(),
                frames: BTreeMap::new(),
            }),
            Err(err) => Err(err.into()),
        }
    }

    /// True when a progress entry exists; the importer verifies its marker before reuse.
    pub fn is_done(&self, id: &str) -> bool {
        self.frames.contains_key(id)
    }

    /// Record a frame and rewrite the file atomically.
    pub fn record(&mut self, path: &Path, frame: FrameProgress) -> anyhow::Result<()> {
        self.frames.insert(frame.id(), frame);
        write_json_atomic(path, self)
    }

    /// Frames of one layer, ordered by playhead.
    pub fn layer_frames(&self, layer: &str) -> Vec<&FrameProgress> {
        self.frames.values().filter(|f| f.layer == layer).collect()
    }
}

/// Write JSON through a sibling temp file and a rename, fsyncing both.
pub fn write_json_atomic<T: Serialize>(path: &Path, value: &T) -> anyhow::Result<()> {
    let bytes = serde_json::to_vec_pretty(value)?;
    radar_core::fsutil::atomic_write(path, &bytes, ".replay-", ".tmp")?;
    Ok(())
}

/// Build the revision manifest (manifest v2 shape, case-only frames).
pub fn build_manifest(
    case: &CaseDefinition,
    revision: &str,
    progress: &Progress,
) -> anyhow::Result<Manifest> {
    let mut layers: BTreeMap<String, Layer> = BTreeMap::new();
    for spec in case.layers() {
        let mut frames: Vec<Frame> = progress
            .layer_frames(spec.layer)
            .into_iter()
            .map(|f| ManifestFrame {
                timestamp: f.timestamp.clone(),
                path: f.timestamp.clone(),
                palettes: vec![crate::case::CASE_PALETTE.to_string()],
                extra: json!({
                    "source": "mrms",
                    "kind": "observation",
                    "product": spec.product,
                    "unit": spec.unit,
                    "issued_at": f.timestamp,
                    "lead_minutes": 0,
                    "spatial_resolution_km": 1.0,
                    "max_zoom": 7,
                    "playhead": f.playhead,
                    "revision": revision,
                })
                .as_object()
                .cloned()
                .unwrap_or_default(),
            })
            .collect();
        frames.sort_by(|a, b| a.timestamp.cmp(&b.timestamp));
        frames.dedup_by(|a, b| a.timestamp == b.timestamp);
        let timestamps: Vec<String> = frames.iter().map(|f| f.timestamp.clone()).collect();
        let latest = timestamps.last().cloned();
        let mut extra: Map<String, Value> = json!({
            "title": spec.title(),
            "kind": "observation",
            "unit": spec.unit,
            "palette_key": spec.color_key,
            "product": spec.product,
        })
        .as_object()
        .cloned()
        .unwrap_or_default();
        extra.insert("coverage_report".into(), json!("coverage.json"));
        if spec.layer == crate::case::QPE_LAYER {
            extra.insert("retrospective".into(), json!(true));
        }
        layers.insert(
            spec.layer.to_string(),
            Layer {
                frames,
                timestamps,
                latest,
                palettes: vec![crate::case::CASE_PALETTE.to_string()],
                extra,
            },
        );
    }
    let mut extra: Map<String, Value> = json!({
        "case": {
            "id": case.id,
            "title": case.title,
            "summary": case.summary,
            "start": case.start,
            "end": case.end,
            "timezone": case.timezone,
            "bounds": case.bounds,
        },
        "revision": revision,
        "replay_schema_version": 1,
        "public_ready": false,
        "warnings": { "status": "missing", "issue": 282 },
    })
    .as_object()
    .cloned()
    .unwrap_or_default();
    extra.insert("generated_at".into(), json!(Utc::now().to_rfc3339()));
    Ok(Manifest {
        layers,
        schema_version: MANIFEST_SCHEMA_VERSION,
        tile_url_template: format!(
            "/replays/{}/{}/tiles/{{layer}}/{{palette}}/{{path}}/{{z}}/{{x}}/{{y}}.png",
            case.id, revision
        ),
        updated_at: Utc::now().format("%Y-%m-%dT%H:%M:%S+00:00").to_string(),
        extra,
    })
}

/// Write the private tile manifest and rename staging; prior revisions remain untouched.
pub fn publish(
    layout: &Layout,
    case: &CaseDefinition,
    progress: &Progress,
) -> anyhow::Result<PathBuf> {
    anyhow::ensure!(
        layout.staging.exists(),
        "staging revision {} does not exist",
        layout.staging.display()
    );
    anyhow::ensure!(
        !layout.final_dir.exists(),
        "revision {} already exists; pick a new --revision",
        layout.final_dir.display()
    );
    let manifest = build_manifest(case, &layout.revision, progress)?;
    let text = radar_core::manifest::render_manifest(&manifest)
        .map_err(|e| anyhow::anyhow!("render manifest: {e}"))?;
    let manifest_path = layout.staging.join(MANIFEST_FILE);
    {
        let mut file = fs::File::create(&manifest_path)?;
        file.write_all(text.as_bytes())?;
        file.write_all(b"\n")?;
        file.sync_all()?;
    }
    fsync_dir(&layout.staging)?;
    fs::rename(&layout.staging, &layout.final_dir)?;
    fsync_dir(&layout.case_dir)?;
    Ok(layout.final_dir.clone())
}

trait LayerTitle {
    fn title(&self) -> String;
}

impl LayerTitle for crate::case::LayerSpec {
    fn title(&self) -> String {
        match self.layer {
            crate::case::RADAR_LAYER => "MRMS observed reflectivity".into(),
            crate::case::COMPOSITE_LAYER => "MRMS composite reflectivity".into(),
            crate::case::HAIL_LAYER => "MRMS estimated hail size, past hour".into(),
            crate::case::ROTATION_LAYER => "MRMS low-level rotation tracks, past hour".into(),
            crate::case::ECHO_TOP_LAYER => "MRMS storm top height (18 dBZ)".into(),
            crate::case::QPE_LAYER => "MRMS observed precipitation, past 24 hours".into(),
            other => other.to_string(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::case::find;

    fn frame(layer: &str, playhead: &str, ts: &str) -> FrameProgress {
        FrameProgress {
            layer: layer.into(),
            playhead: playhead.into(),
            timestamp: ts.into(),
            key: format!("k/{ts}"),
            source_id: format!("replay:{layer}:{ts}"),
            source_sha256: "test".into(),
            flag_key: None,
            flag_sha256: None,
            tiles: 3,
            bytes: 900,
            render_ms: 12,
        }
    }

    #[test]
    fn revision_names_and_layout_are_safe() {
        let tmp = tempfile::tempdir().unwrap();
        let case = find("rolling-fork-2023").unwrap();
        let l = layout(tmp.path(), case, "rev1").unwrap();
        assert_eq!(
            l.final_dir,
            tmp.path().join("replays/rolling-fork-2023/rev1")
        );
        assert_eq!(
            l.staging,
            tmp.path().join("replays/rolling-fork-2023/.staging-rev1")
        );
        assert!(matches!(
            layout(tmp.path(), case, "../escape"),
            Err(RevisionError::InvalidRevision(_))
        ));
        assert!(layout(tmp.path(), case, ".").is_err());
        assert!(layout(tmp.path(), case, "..").is_err());
        assert!(layout(tmp.path(), case, ".locks").is_err());
        assert!(matches!(
            layout(tmp.path(), case, "UPPER"),
            Err(RevisionError::InvalidRevision(_))
        ));
    }

    #[test]
    fn manifest_lists_case_only_frames_and_namespaced_tiles() {
        let case = find("greenfield-2024").unwrap();
        let mut progress = Progress {
            revision: "r1".into(),
            case: case.id.into(),
            frames: BTreeMap::new(),
        };
        progress.frames.insert(
            "a".into(),
            frame(
                crate::case::RADAR_LAYER,
                "2024-05-21T19:00:10+00:00",
                "2024-05-21T19:00:10+00:00",
            ),
        );
        let manifest = build_manifest(case, "r1", &progress).unwrap();
        assert_eq!(manifest.schema_version, 2);
        assert!(
            manifest
                .tile_url_template
                .contains("/replays/greenfield-2024/r1/tiles/{layer}/")
        );
        let radar = manifest.layers.get(crate::case::RADAR_LAYER).unwrap();
        assert_eq!(radar.frames.len(), 1);
        assert_eq!(radar.palettes, vec!["classic".to_string()]);
        assert_eq!(manifest.layers.len(), 6);
    }

    #[test]
    fn publish_moves_staging_and_keeps_the_prior_revision() {
        let tmp = tempfile::tempdir().unwrap();
        let case = find("greenfield-2024").unwrap();
        let first = layout(tmp.path(), case, "r1").unwrap();
        fs::create_dir_all(first.staging.join("tiles")).unwrap();
        let progress = Progress {
            revision: "r1".into(),
            case: case.id.into(),
            frames: BTreeMap::new(),
        };
        let final_dir = publish(&first, case, &progress).unwrap();
        assert!(final_dir.join(MANIFEST_FILE).exists());
        assert!(!first.staging.exists());

        let second = layout(tmp.path(), case, "r2").unwrap();
        fs::create_dir_all(second.staging.join("tiles")).unwrap();
        let progress2 = Progress {
            revision: "r2".into(),
            case: case.id.into(),
            frames: BTreeMap::new(),
        };
        publish(&second, case, &progress2).unwrap();
        assert!(
            final_dir.join(MANIFEST_FILE).exists(),
            "prior revision must stay readable"
        );
        assert!(second.final_dir.exists());
    }
}
