//! Publish a finished whole-case revision (ticket #285; contract in `docs/develop/replay-api.md`).
//!
//! The source revision is never edited. A new revision is staged beside it, verified, and
//! appears through one durable directory rename.

use crate::case::{self, CASE_ZOOMS, CaseDefinition};
use crate::import::{PlannedFrame, bind_progress, frame_dir, verify_frame};
use crate::{objects, plan, quota, revision};
use chrono::{DateTime, Utc};
use radar_core::fsutil::{FlockGuard, atomic_write, durable_mkdir, fsync_dir};
use radar_grib::mrms as gmrms;
use serde_json::{Value, json};
use std::collections::BTreeSet;
use std::fs;
use std::io::Read;
use std::path::{Path, PathBuf};

/// Warnings file name inside a public revision.
pub const WARNINGS_FILE: &str = "warnings.geojson";
const PLAN_FILE: &str = "plan.json";
const SELECTION_GAPS_FILE: &str = "selection-gaps.json";
const MAX_JSON_BYTES: u64 = 64 * 1024 * 1024;
const MAX_TILE_BYTES: u64 = 1024 * 1024;
const PNG_MAGIC: [u8; 8] = [0x89, b'P', b'N', b'G', 0x0d, 0x0a, 0x1a, 0x0a];
const COPIED_FILES: [&str; 5] = [
    revision::COVERAGE_FILE,
    SELECTION_GAPS_FILE,
    PLAN_FILE,
    revision::PROGRESS_FILE,
    revision::REPORT_FILE,
];

/// `radar replay publish` options.
pub struct Options {
    pub case_id: String,
    /// The finished private revision.
    pub revision: String,
    pub warnings: PathBuf,
    pub tile_root: PathBuf,
    /// Public revision name; defaults to `<revision>-public`.
    pub published_revision: Option<String>,
}

/// Publish with the real quota check.
pub fn run(options: Options) -> anyhow::Result<Value> {
    run_with_check(options, quota::check)
}

fn read_regular(path: &Path, cap: u64) -> anyhow::Result<Vec<u8>> {
    let meta = fs::symlink_metadata(path)?;
    anyhow::ensure!(
        meta.is_file() && meta.len() <= cap,
        "{} is not a regular file under {cap} bytes",
        path.display()
    );
    Ok(fs::read(path)?)
}

fn read_json(path: &Path) -> anyhow::Result<Value> {
    let bytes = read_regular(path, MAX_JSON_BYTES)
        .map_err(|e| anyhow::anyhow!("cannot read {}: {e}", path.display()))?;
    serde_json::from_slice(&bytes)
        .map_err(|e| anyhow::anyhow!("{} is not valid JSON: {e}", path.display()))
}

fn text<'a>(value: &'a Value, key: &str) -> Option<&'a str> {
    value.get(key).and_then(Value::as_str)
}

fn bounds_equal(value: &Value, expected: &[f64; 4]) -> bool {
    value.as_array().is_some_and(|a| {
        a.len() == 4
            && a.iter()
                .zip(expected)
                .all(|(v, e)| v.as_f64().is_some_and(|v| (v - e).abs() < 1e-9))
    })
}

/// What the automated coverage review found.
struct Reviewed {
    metadata: Value,
    progress: revision::Progress,
    frames: Vec<PlannedFrame>,
    tiles: Vec<(u32, u32, u32)>,
    summary: Value,
}

/// Every frame has its full allowlisted tile footprint, intact and byte-bound by its marker.
fn check_tiles(
    dir: &Path,
    frames: &[PlannedFrame],
    progress: &revision::Progress,
    tiles: &[(u32, u32, u32)],
) -> anyhow::Result<u64> {
    let mut count = 0;
    for frame in frames {
        let id = format!("{}@{}", frame.layer, frame.timestamp);
        let done = progress
            .frames
            .get(&id)
            .ok_or_else(|| anyhow::anyhow!("no progress for {id}"))?;
        bind_progress(done, frame)?;
        let frame_root = frame_dir(dir, &frame.layer, &frame.timestamp);
        verify_frame(&frame_root, done, tiles)?;
        for (z, x, y) in tiles {
            let path = frame_root.join(format!("{z}/{x}/{y}.png"));
            let meta = fs::symlink_metadata(&path)
                .map_err(|e| anyhow::anyhow!("{id}: missing tile {}: {e}", path.display()))?;
            anyhow::ensure!(
                meta.is_file() && meta.len() > 0 && meta.len() <= MAX_TILE_BYTES,
                "{id}: bad tile {}",
                path.display()
            );
            let mut magic = [0u8; 8];
            fs::File::open(&path)?.read_exact(&mut magic)?;
            anyhow::ensure!(magic == PNG_MAGIC, "{id}: not a PNG: {}", path.display());
            count += 1;
        }
    }
    Ok(count)
}

/// Refuse anything but a complete whole-case revision; fail closed on the first mismatch.
fn review(
    dir: &Path,
    case: &CaseDefinition,
    revision_name: &str,
    progress_revision: &str,
    require_private: bool,
) -> anyhow::Result<Reviewed> {
    let metadata = read_json(&dir.join("revision.json"))?;
    let ok = metadata.get("schema_version").and_then(Value::as_u64) == Some(1)
        && text(&metadata, "case") == Some(case.id)
        && text(&metadata, "revision") == Some(revision_name)
        && text(&metadata, "start") == Some(case.start)
        && text(&metadata, "end") == Some(case.end)
        && text(&metadata, "timezone") == Some(case.timezone)
        && metadata
            .get("bounds")
            .is_some_and(|b| bounds_equal(b, &case.bounds));
    anyhow::ensure!(
        ok,
        "revision.json does not describe {}/{revision_name}",
        case.id
    );
    let scope = text(&metadata, "scope").unwrap_or("missing");
    anyhow::ensure!(
        scope == "case",
        "refusing a {scope}-scope revision: only a whole-case import can be published"
    );
    anyhow::ensure!(
        text(&metadata, "status") == Some("tiles_complete"),
        "refusing an incomplete revision: status is not tiles_complete"
    );
    if require_private {
        anyhow::ensure!(
            metadata.get("public_ready") == Some(&json!(false)),
            "revision is already public; publish from the private import revision"
        );
    }
    let plan_doc = read_json(&dir.join(PLAN_FILE))?;
    anyhow::ensure!(
        plan_doc.get("schema_version").and_then(Value::as_u64) == Some(1)
            && text(&plan_doc, "case") == Some(case.id)
            && plan_doc.get("max_frames").is_some_and(Value::is_null)
            && plan_doc
                .get("layers")
                .and_then(Value::as_array)
                .is_some_and(Vec::is_empty),
        "plan.json is not a whole-case plan for {}",
        case.id
    );
    let tiles = plan::case_tiles(&case.bounds, &CASE_ZOOMS);
    anyhow::ensure!(
        plan_doc.get("tiles") == Some(&serde_json::to_value(&tiles)?),
        "plan.json tile allowlist differs from the case footprint"
    );
    let frames: Vec<PlannedFrame> = serde_json::from_value(
        plan_doc
            .get("frames")
            .cloned()
            .ok_or_else(|| anyhow::anyhow!("plan.json has no frames"))?,
    )?;
    let coverage: objects::Coverage =
        serde_json::from_value(read_json(&dir.join(revision::COVERAGE_FILE))?)?;
    let (from, to) = case.window()?;
    let plan_excluded = plan_doc
        .get("excluded")
        .cloned()
        .unwrap_or_else(|| json!([]));
    anyhow::ensure!(
        plan_excluded == serde_json::to_value(&coverage.excluded)?,
        "plan.json and coverage.json disagree on excluded archive objects"
    );
    anyhow::ensure!(
        coverage.case == case.id && coverage.start == case.start && coverage.end == case.end,
        "coverage.json is for a different case or window"
    );
    let gaps_doc = read_json(&dir.join(SELECTION_GAPS_FILE))?;
    let gap_layers: BTreeSet<&str> = gaps_doc
        .as_object()
        .ok_or_else(|| anyhow::anyhow!("selection-gaps.json is not an object"))?
        .keys()
        .map(String::as_str)
        .collect();
    let spec_layers: BTreeSet<&str> = case.layers().iter().map(|l| l.layer).collect();
    anyhow::ensure!(
        gap_layers == spec_layers && coverage.layers.len() == spec_layers.len(),
        "coverage or selection gaps do not cover exactly the case layers"
    );
    let base = coverage
        .layers
        .iter()
        .find(|l| l.layer == case::RADAR_LAYER)
        .ok_or_else(|| anyhow::anyhow!("coverage.json has no base-reflectivity layer"))?;
    let base_times: Vec<DateTime<Utc>> = base
        .keys
        .iter()
        .map(|k| gmrms::key_time(k))
        .collect::<Result<_, _>>()?;
    let mut per_layer = serde_json::Map::new();
    let mut planned_total = 0;
    for spec in case.layers() {
        let layer = coverage
            .layers
            .iter()
            .find(|l| l.layer == spec.layer)
            .ok_or_else(|| anyhow::anyhow!("coverage.json is missing {}", spec.layer))?;
        anyhow::ensure!(
            !layer.keys.is_empty() && layer.objects == layer.keys.len(),
            "{}: coverage lists no objects or a wrong count",
            spec.layer
        );
        let listed: Vec<(DateTime<Utc>, String)> = layer
            .keys
            .iter()
            .map(|k| Ok((gmrms::key_time(k)?, k.clone())))
            .collect::<anyhow::Result<_>>()?;
        let again =
            objects::coverage_for(spec.layer, spec.prefix, spec.cadence_s, from, to, &listed);
        anyhow::ensure!(
            again.gaps == layer.gaps
                && again.missing_expected == layer.missing_expected
                && again.expected == layer.expected
                && again.cadence_s == layer.cadence_s,
            "{}: coverage.json gaps do not match its object list",
            spec.layer
        );
        let planned: Vec<&PlannedFrame> = frames.iter().filter(|f| f.layer == spec.layer).collect();
        anyhow::ensure!(
            planned.len() == listed.len()
                && planned.iter().zip(&listed).all(|(f, (t, key))| {
                    f.key == *key && f.timestamp == gmrms::format_valid_time(*t)
                }),
            "{}: the import did not cover every listed archive object",
            spec.layer
        );
        planned_total += planned.len();
        let (_, missing) = plan::choose_frames(&base_times, &listed, plan::staleness_for(spec));
        anyhow::ensure!(
            gaps_doc.get(spec.layer) == Some(&serde_json::to_value(&missing)?),
            "{}: selection-gaps.json does not match the playheads without an observation",
            spec.layer
        );
        per_layer.insert(
            spec.layer.to_string(),
            json!({
                "frames": planned.len(),
                "coverage_gaps": layer.gaps.len(),
                "missing_expected": layer.missing_expected,
                "selection_gaps": missing.len(),
            }),
        );
    }
    anyhow::ensure!(
        planned_total == frames.len(),
        "plan.json lists frames for unknown layers"
    );
    let progress = revision::Progress::load(
        &dir.join(revision::PROGRESS_FILE),
        case.id,
        progress_revision,
    )?;
    anyhow::ensure!(
        dir.join(revision::PROGRESS_FILE).is_file() && progress.frames.len() == frames.len(),
        "progress.json does not hold exactly the planned frames"
    );
    let tile_files = check_tiles(dir, &frames, &progress, &tiles)?;
    let expected = revision::build_manifest(case, revision_name, &progress)?;
    let expected: Value = serde_json::from_str(&radar_core::manifest::render_manifest(&expected)?)?;
    let manifest = read_json(&dir.join(revision::MANIFEST_FILE))?;
    for key in [
        "schema_version",
        "tile_url_template",
        "revision",
        "case",
        "layers",
    ] {
        anyhow::ensure!(
            manifest.get(key) == expected.get(key),
            "manifest.json {key} does not match the verified frames"
        );
    }
    let without_flag = frames
        .iter()
        .filter(|f| {
            matches!(f.layer.as_str(), case::RADAR_LAYER | case::COMPOSITE_LAYER)
                && f.flag_key.is_none()
        })
        .count();
    let summary = json!({
        "frames": frames.len(), "tile_files": tile_files,
        "frames_without_precip_flag": without_flag, "layers": per_layer,
        "excluded_objects": coverage.excluded,
    });
    Ok(Reviewed {
        metadata,
        progress,
        frames,
        tiles,
        summary,
    })
}

fn copy_synced(from: &Path, to: &Path) -> anyhow::Result<()> {
    let meta = fs::symlink_metadata(from)?;
    anyhow::ensure!(meta.is_file(), "not a regular file: {}", from.display());
    fs::copy(from, to)?;
    fs::File::open(to)?.sync_all()?;
    Ok(())
}

fn copy_tree(from: &Path, to: &Path) -> anyhow::Result<()> {
    fs::create_dir_all(to)?;
    for entry in fs::read_dir(from)? {
        let entry = entry?;
        let kind = entry.file_type()?;
        let target = to.join(entry.file_name());
        if kind.is_dir() {
            copy_tree(&entry.path(), &target)?;
        } else {
            anyhow::ensure!(
                kind.is_file(),
                "unexpected entry {}",
                entry.path().display()
            );
            copy_synced(&entry.path(), &target)?;
        }
    }
    Ok(())
}

fn sync_dirs(root: &Path) -> anyhow::Result<()> {
    for entry in walkdir::WalkDir::new(root).contents_first(true) {
        let entry = entry?;
        if entry.file_type().is_dir() {
            fsync_dir(entry.path())?;
        }
    }
    Ok(())
}

fn warnings_sha(bytes: &[u8]) -> String {
    radar_tiler::identity::sha256_hex(bytes)
}

/// Publish without the live-headroom check; for tests that run on small disks.
#[cfg(any(test, feature = "testkit"))]
pub fn run_for_tests(options: Options) -> anyhow::Result<Value> {
    run_with_check(options, |_, _| Ok(()))
}

pub(crate) fn run_with_check(
    options: Options,
    check: fn(&Path, u64) -> anyhow::Result<()>,
) -> anyhow::Result<Value> {
    let case = *case::find(&options.case_id)?;
    let target_name = options
        .published_revision
        .clone()
        .unwrap_or_else(|| format!("{}-public", options.revision));
    anyhow::ensure!(
        target_name != options.revision,
        "the public revision must differ from the source revision"
    );
    anyhow::ensure!(
        options.tile_root.is_dir(),
        "--tile-root must be an existing directory"
    );
    let tile_root = fs::canonicalize(&options.tile_root)?;
    let source = revision::layout(&tile_root, &case, &options.revision)?;
    let target = revision::layout(&tile_root, &case, &target_name)?;
    anyhow::ensure!(
        source.final_dir.is_dir(),
        "source revision {} does not exist (a revision still in .staging is not finished)",
        source.final_dir.display()
    );
    let warnings_bytes = read_regular(
        &options.warnings,
        radar_alerts::replay::MAX_PUBLIC_BYTES as u64,
    )?;
    let history = radar_alerts::replay::validate_file(case.id, &warnings_bytes)?;
    let sha = warnings_sha(&warnings_bytes);
    let replay_root = quota::replay_root(&tile_root);
    let _lock = FlockGuard::exclusive(&replay_root.join(".import.lock"))?;

    if target.final_dir.try_exists()? {
        return confirm_existing(&target.final_dir, &case, &source.revision, &target, &sha);
    }
    let reviewed = review(
        &source.final_dir,
        &case,
        &source.revision,
        &source.revision,
        true,
    )?;
    let source_usage = quota::dir_usage(&source.final_dir)?;
    if target.staging.try_exists()? {
        // Our own half-built copy from an interrupted run; it was never public.
        fs::remove_dir_all(&target.staging)?;
    }
    check(
        &tile_root,
        source_usage.allocated_bytes + warnings_bytes.len() as u64 * 2 + 8 * 1024 * 1024,
    )?;
    durable_mkdir(&target.staging, &fsync_dir)?;
    let staged = stage(
        &target.staging,
        &source.final_dir,
        &case,
        &target_name,
        &reviewed,
        &warnings_bytes,
        &sha,
        history.features.len(),
    );
    if staged.is_err() {
        let _ = fs::remove_dir_all(&target.staging);
    }
    staged?;
    let copied = quota::dir_usage(&target.staging.join("tiles"))?;
    let original = quota::dir_usage(&source.final_dir.join("tiles"))?;
    anyhow::ensure!(
        copied.files == original.files && copied.bytes == original.bytes,
        "copied tiles differ from the source in file count or size"
    );
    check_tiles(
        &target.staging,
        &reviewed.frames,
        &reviewed.progress,
        &reviewed.tiles,
    )?;
    sync_dirs(&target.staging)?;
    fs::rename(&target.staging, &target.final_dir)?;
    fsync_dir(&target.case_dir)?;
    check(&tile_root, 0)?;
    Ok(json!({
        "case": case.id, "source_revision": source.revision, "published_revision": target_name,
        "created": true, "published": target.final_dir, "warnings_sha256": sha,
        "warning_versions": history.features.len(), "review": reviewed.summary,
        "usage": quota::dir_usage(&target.final_dir)?,
    }))
}

#[allow(clippy::too_many_arguments)]
fn stage(
    staging: &Path,
    source: &Path,
    case: &CaseDefinition,
    target_name: &str,
    reviewed: &Reviewed,
    warnings: &[u8],
    sha: &str,
    versions: usize,
) -> anyhow::Result<()> {
    copy_tree(&source.join("tiles"), &staging.join("tiles"))?;
    for name in COPIED_FILES {
        copy_synced(&source.join(name), &staging.join(name))?;
    }
    atomic_write(&staging.join(WARNINGS_FILE), warnings, ".replay-", ".tmp")?;
    let warnings_meta =
        json!({"status": "complete", "file": WARNINGS_FILE, "sha256": sha, "versions": versions});
    let mut manifest = revision::build_manifest(case, target_name, &reviewed.progress)?;
    manifest.extra.insert("public_ready".into(), json!(true));
    manifest
        .extra
        .insert("warnings".into(), warnings_meta.clone());
    let rendered = radar_core::manifest::render_manifest(&manifest)?;
    atomic_write(
        &staging.join(revision::MANIFEST_FILE),
        format!("{rendered}\n").as_bytes(),
        ".replay-",
        ".tmp",
    )?;
    let mut metadata = reviewed.metadata.clone();
    let object = metadata
        .as_object_mut()
        .ok_or_else(|| anyhow::anyhow!("revision.json is not an object"))?;
    object.insert("revision".into(), json!(target_name));
    object.insert("public_ready".into(), json!(true));
    object.insert("coverage_reviewed".into(), json!(true));
    object.insert("warnings".into(), warnings_meta);
    object.insert(
        "published_from".into(),
        json!({"revision": text(&reviewed.metadata, "revision"), "review": reviewed.summary}),
    );
    revision::write_json_atomic(&staging.join("revision.json"), &metadata)?;
    Ok(())
}

/// Re-run on an existing public revision: succeed only when it is exactly this publication.
fn confirm_existing(
    dir: &Path,
    case: &CaseDefinition,
    source_name: &str,
    target: &revision::Layout,
    sha: &str,
) -> anyhow::Result<Value> {
    let meta = read_json(&dir.join("revision.json"))?;
    let same = meta.get("public_ready") == Some(&json!(true))
        && meta.get("coverage_reviewed") == Some(&json!(true))
        && meta.get("published_from").and_then(|p| text(p, "revision")) == Some(source_name)
        && meta.get("warnings").and_then(|w| text(w, "sha256")) == Some(sha);
    anyhow::ensure!(
        same,
        "revision {} already exists with different content; it is immutable, so choose another --published-revision",
        dir.display()
    );
    let on_disk = read_regular(
        &dir.join(WARNINGS_FILE),
        radar_alerts::replay::MAX_PUBLIC_BYTES as u64,
    )?;
    anyhow::ensure!(
        warnings_sha(&on_disk) == sha,
        "published warnings.geojson was altered"
    );
    let reviewed = review(dir, case, &target.revision, source_name, false)?;
    Ok(json!({
        "case": case.id, "source_revision": source_name, "published_revision": target.revision,
        "created": false, "published": dir, "warnings_sha256": sha, "review": reviewed.summary,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testkit;

    struct Env {
        _tmp: tempfile::TempDir,
        root: PathBuf,
        warnings: PathBuf,
        case: &'static CaseDefinition,
    }

    fn env(case_id: &str) -> Env {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().join("tiles");
        fs::create_dir_all(&root).unwrap();
        let case = case::find(case_id).unwrap();
        testkit::synthetic_revision(&root, case, "full-1", 3).unwrap();
        let warnings = tmp.path().join("warnings.geojson");
        fs::write(&warnings, testkit::warnings_json(case)).unwrap();
        Env {
            _tmp: tmp,
            root,
            warnings,
            case,
        }
    }

    impl Env {
        fn options(&self) -> Options {
            Options {
                case_id: self.case.id.into(),
                revision: "full-1".into(),
                warnings: self.warnings.clone(),
                tile_root: self.root.clone(),
                published_revision: None,
            }
        }
        fn publish(&self) -> anyhow::Result<Value> {
            run_with_check(self.options(), |_, _| Ok(()))
        }
        fn source(&self) -> PathBuf {
            self.root.join("replays").join(self.case.id).join("full-1")
        }
        fn target(&self) -> PathBuf {
            self.root
                .join("replays")
                .join(self.case.id)
                .join("full-1-public")
        }
        fn staging(&self) -> PathBuf {
            self.root
                .join("replays")
                .join(self.case.id)
                .join(".staging-full-1-public")
        }
        fn edit(&self, file: &str, change: impl FnOnce(&mut Value)) {
            let path = self.source().join(file);
            let mut value = read_json(&path).unwrap();
            change(&mut value);
            fs::write(path, serde_json::to_vec(&value).unwrap()).unwrap();
        }
        fn refused(&self, why: &str) {
            let err = self.publish().unwrap_err().to_string();
            assert!(err.contains(why), "expected {why:?} in {err:?}");
            assert!(!self.target().exists(), "nothing may be published");
            assert!(!self.staging().exists(), "no staging may be left behind");
        }
    }

    fn tree(root: &Path) -> Vec<(PathBuf, String)> {
        walkdir::WalkDir::new(root)
            .sort_by_file_name()
            .into_iter()
            .filter_map(|e| e.ok())
            .filter(|e| e.file_type().is_file())
            .map(|e| {
                (
                    e.path().strip_prefix(root).unwrap().to_path_buf(),
                    warnings_sha(&fs::read(e.path()).unwrap()),
                )
            })
            .collect()
    }

    #[test]
    fn publishes_a_new_revision_and_leaves_the_source_alone() {
        for id in ["rolling-fork-2023", "greenfield-2024"] {
            let env = env(id);
            let before = tree(&env.source());
            let report = env.publish().unwrap();
            assert_eq!(report["created"], true);
            assert_eq!(tree(&env.source()), before, "source revision is immutable");
            let meta = read_json(&env.target().join("revision.json")).unwrap();
            assert_eq!(meta["revision"], "full-1-public");
            assert_eq!(meta["public_ready"], true);
            assert_eq!(meta["coverage_reviewed"], true);
            assert_eq!(meta["scope"], "case");
            assert_eq!(meta["status"], "tiles_complete");
            assert_eq!(meta["warnings"]["status"], "complete");
            assert_eq!(meta["published_from"]["revision"], "full-1");
            let manifest = read_json(&env.target().join("manifest.json")).unwrap();
            assert_eq!(manifest["revision"], "full-1-public");
            assert!(
                manifest["tile_url_template"]
                    .as_str()
                    .unwrap()
                    .contains("/full-1-public/tiles/")
            );
            assert_eq!(
                fs::read(env.target().join(WARNINGS_FILE)).unwrap(),
                fs::read(&env.warnings).unwrap()
            );
            // The copy is a full, valid revision of its own.
            review(&env.target(), env.case, "full-1-public", "full-1", false).unwrap();
            assert!(!env.staging().exists());
        }
    }

    #[test]
    fn rerun_is_idempotent_and_changes_nothing() {
        let env = env("greenfield-2024");
        env.publish().unwrap();
        let before = tree(&env.root.join("replays"));
        let report = env.publish().unwrap();
        assert_eq!(report["created"], false);
        assert_eq!(tree(&env.root.join("replays")), before);
    }

    #[test]
    fn an_existing_public_revision_is_never_replaced() {
        let env = env("greenfield-2024");
        env.publish().unwrap();
        let before = tree(&env.target());
        let other = testkit::warnings_json_with(env.case, "different bulletin text");
        fs::write(&env.warnings, other).unwrap();
        let err = env.publish().unwrap_err().to_string();
        assert!(err.contains("immutable"), "{err}");
        assert_eq!(tree(&env.target()), before);
    }

    #[test]
    fn refuses_pilot_and_incomplete_revisions() {
        let env_a = env("greenfield-2024");
        env_a.edit("revision.json", |v| v["scope"] = json!("pilot"));
        env_a.refused("pilot-scope");
        let env_b = env("rolling-fork-2023");
        env_b.edit("revision.json", |v| v["status"] = json!("rendering"));
        env_b.refused("incomplete revision");
        let env_c = env("greenfield-2024");
        env_c.edit("plan.json", |v| v["max_frames"] = json!(1));
        env_c.refused("whole-case plan");
        let env_d = env("greenfield-2024");
        env_d.edit("revision.json", |v| v["public_ready"] = json!(true));
        env_d.refused("already public");
    }

    #[test]
    fn refuses_a_revision_that_is_still_staging() {
        let env = env("greenfield-2024");
        let source = env.source();
        let staging = source.with_file_name(".staging-full-1");
        fs::rename(&source, &staging).unwrap();
        env.refused("does not exist");
    }

    #[test]
    fn coverage_review_fails_closed() {
        // A missing tile.
        let env_a = env("greenfield-2024");
        let victim = walkdir::WalkDir::new(env_a.source().join("tiles"))
            .into_iter()
            .filter_map(|e| e.ok())
            .find(|e| e.path().extension().is_some_and(|x| x == "png"))
            .unwrap()
            .into_path();
        fs::remove_file(victim).unwrap();
        assert!(env_a.publish().is_err());
        assert!(!env_a.target().exists() && !env_a.staging().exists());
        // A corrupted tile fails its marker digest.
        let env_b = env("greenfield-2024");
        let victim = walkdir::WalkDir::new(env_b.source().join("tiles"))
            .into_iter()
            .filter_map(|e| e.ok())
            .find(|e| e.path().extension().is_some_and(|x| x == "png"))
            .unwrap()
            .into_path();
        fs::write(victim, b"corrupt").unwrap();
        assert!(env_b.publish().is_err());
        assert!(!env_b.target().exists());
        // A frame dropped from the plan: the import did not cover every archive object.
        let env_c = env("greenfield-2024");
        env_c.edit("plan.json", |v| {
            v["frames"].as_array_mut().unwrap().pop();
        });
        env_c.refused("every listed archive object");
        // Selection gaps that do not match the playheads.
        let env_d = env("rolling-fork-2023");
        env_d.edit("selection-gaps.json", |v| {
            v["hail-size"] = json!(["2023-03-25T00:00:00Z"]);
        });
        env_d.refused("selection-gaps.json");
        // Coverage gaps that do not match the object list.
        let env_e = env("rolling-fork-2023");
        env_e.edit("coverage.json", |v| {
            v["layers"][0]["gaps"] = json!([]);
            v["layers"][0]["missing_expected"] = json!(0);
        });
        env_e.refused("coverage.json gaps");
        // An exclusion recorded in only one of the two files.
        let env_g = env("greenfield-2024");
        env_g.edit("plan.json", |v| {
            v["excluded"] = json!([{"key": "k", "reason": "r", "bytes": 1, "sha256": "s"}]);
        });
        env_g.refused("disagree on excluded");
        // A manifest that advertises a frame that was not verified.
        let env_f = env("greenfield-2024");
        env_f.edit("manifest.json", |v| {
            v["layers"]["radar"]["frames"].as_array_mut().unwrap().pop();
        });
        env_f.refused("manifest.json");
    }

    #[test]
    fn refuses_bad_warnings() {
        let env = env("greenfield-2024");
        let good: Value = serde_json::from_slice(&fs::read(&env.warnings).unwrap()).unwrap();
        let cases: Vec<(&str, Box<dyn Fn(&mut Value)>)> = vec![
            (
                "different case",
                Box::new(|v| v["case_id"] = json!("rolling-fork-2023")),
            ),
            ("schema 1", Box::new(|v| v["schema_version"] = json!(2))),
            (
                "no warning versions",
                Box::new(|v| {
                    v["features"] = json!([]);
                    v["seed"] = json!([]);
                }),
            ),
            (
                "hash does not match",
                Box::new(|v| {
                    let id = v["bulletins"]
                        .as_object()
                        .unwrap()
                        .keys()
                        .next()
                        .unwrap()
                        .clone();
                    v["bulletins"][id]["text"] = json!("edited");
                }),
            ),
            ("no sources", Box::new(|v| v["sources"] = json!([]))),
            (
                "disagree with its bulletin",
                Box::new(|v| {
                    v["features"][0]["properties"]["is_pds"] = json!(true);
                    v["seed"][0]["properties"]["is_pds"] = json!(true);
                }),
            ),
            (
                "not an active-at-start",
                Box::new(|v| v["seed"][0]["id"] = json!("x")),
            ),
        ];
        for (why, change) in cases {
            let mut bad = good.clone();
            change(&mut bad);
            fs::write(&env.warnings, serde_json::to_vec(&bad).unwrap()).unwrap();
            let err = env.publish().unwrap_err().to_string();
            assert!(err.contains(why), "expected {why:?} in {err:?}");
            assert!(!env.target().exists(), "{why}");
        }
        fs::write(&env.warnings, b"not json").unwrap();
        assert!(env.publish().is_err());
        let huge = vec![b' '; radar_alerts::replay::MAX_PUBLIC_BYTES + 1];
        fs::write(&env.warnings, huge).unwrap();
        assert!(env.publish().is_err());
        assert!(!env.target().exists());
    }

    #[test]
    fn quota_refusal_publishes_nothing_and_resumes_after_a_stale_staging_dir() {
        let env = env("greenfield-2024");
        let err = run_with_check(env.options(), |_, _| anyhow::bail!("quota denied"));
        assert!(err.is_err());
        assert!(!env.target().exists() && !env.staging().exists());
        fs::create_dir_all(env.staging().join("tiles")).unwrap();
        fs::write(env.staging().join("junk"), b"x").unwrap();
        assert_eq!(env.publish().unwrap()["created"], true);
        assert!(!env.target().join("junk").exists());
    }
}
