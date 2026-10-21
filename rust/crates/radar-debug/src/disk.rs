//! On-disk checks: run these where the PVCs are mounted (a `radar api` or worker pod, or the compose
//! host against the named volumes).
//!
//! Port of `tools/debug_harness/disk_checks.py`. The data layout comes from `radar-core`: the
//! `Paths` roots from the environment, and the manifest through `read_manifest`, which fails closed
//! on a malformed file (the Python read it raw and reported it the same way).

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

use radar_core::env::{Paths, path_var};
use radar_core::manifest::{self, ManifestError};
use radar_core::pycompat::Json;
use walkdir::WalkDir;

use crate::model::{Check, Status, age_str, py_repr_counts, py_repr_list};

/// A `.tmp` staging dir or leaked work dir older than this is an orphan (30 minutes).
pub const ORPHAN_TMP_AGE_S: f64 = 30.0 * 60.0;

/// Directories inspected by the disk checks.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DiskRoots {
    /// Rendered tile pyramid (`TILE_DIR`).
    pub tile: PathBuf,
    /// Grid dumps (`GRID_DIR`). Only headroom is checked, as in the Python.
    pub grid: PathBuf,
    /// State directory (`STATE_DIR`): manifest and state files.
    pub state: PathBuf,
    /// Ingest scratch roots (`MRMS_TMP_ROOT`, `HRRR_TMP_ROOT`).
    pub tmp_roots: Vec<PathBuf>,
}

impl DiskRoots {
    /// Resolve from the environment with the same defaults as the Python.
    pub fn from_env() -> Self {
        let paths = Paths::from_env();
        Self {
            tile: paths.tile_dir,
            grid: paths.grid_dir,
            state: paths.state_dir,
            tmp_roots: vec![
                path_var("MRMS_TMP_ROOT", "/tmp/mrms_work"),
                path_var("HRRR_TMP_ROOT", "/tmp/hrrr_work"),
            ],
        }
    }

    /// Whether any data root exists, which `doctor` uses to decide whether to run disk checks at all.
    pub fn any_present(&self) -> bool {
        [&self.tile, &self.state].iter().any(|p| p.is_dir())
    }
}

/// Seconds from `mtime` to `now`, negative when `mtime` is in the future.
fn age_secs(now: SystemTime, mtime: SystemTime) -> f64 {
    match now.duration_since(mtime) {
        Ok(d) => d.as_secs_f64(),
        Err(e) => -e.duration().as_secs_f64(),
    }
}

fn mtime_of(path: &Path) -> std::io::Result<SystemTime> {
    fs::metadata(path)?.modified()
}

/// Filesystem usage the way `shutil.disk_usage` reports it: `(total, free, used)` in bytes, where
/// `used` excludes blocks that are free for root only.
fn disk_usage(path: &Path) -> std::io::Result<(u64, u64, u64)> {
    let st = rustix::fs::statvfs(path)?;
    let frsize = st.f_frsize;
    let total = st.f_blocks * frsize;
    let free = st.f_bavail * frsize;
    let used = st.f_blocks.saturating_sub(st.f_bfree) * frsize;
    Ok((total, free, used))
}

/// Run every on-disk check. `now` is injected so tests can pin the clock.
pub fn check_disk(roots: &DiskRoots, now: SystemTime) -> Vec<Check> {
    let mut checks = Vec::new();
    let named = [
        ("tiles", &roots.tile),
        ("grids", &roots.grid),
        ("state", &roots.state),
    ];
    let present: Vec<(&str, &PathBuf)> = named
        .iter()
        .filter(|(_, p)| p.is_dir())
        .map(|(n, p)| (*n, *p))
        .collect();
    if present.is_empty() {
        let all: Vec<String> = named.iter().map(|(_, p)| p.display().to_string()).collect();
        return vec![Check::new(
            "disk.mounts",
            Status::Fail,
            format!(
                "none of {} exist — run this inside a container that mounts the data volumes (or set TILE_DIR/GRID_DIR/STATE_DIR)",
                py_repr_list(&all)
            ),
        )];
    }

    // -- filesystem headroom ----------------------------------------------------
    for (name, p) in &present {
        match disk_usage(p) {
            Err(e) => checks.push(Check::new(
                format!("disk.{name}.usage"),
                Status::Fail,
                format!("disk_usage failed: {e}"),
            )),
            Ok((total, free, used)) => {
                let pct = if total == 0 {
                    0.0
                } else {
                    used as f64 / total as f64 * 100.0
                };
                let status = if pct < 85.0 {
                    Status::Ok
                } else if pct < 95.0 {
                    Status::Warn
                } else {
                    Status::Fail
                };
                let free_gb = free as f64 / 1e9;
                checks.push(
                    Check::new(
                        format!("disk.{name}.usage"),
                        status,
                        format!(
                            "{}: {:.0}% used ({:.1} GB free of {:.1} GB)",
                            p.display(),
                            pct,
                            free_gb,
                            total as f64 / 1e9
                        ),
                    )
                    .with_data(Json::Object(vec![
                        (
                            "pct".into(),
                            Json::Float(radar_core::time::round_digits(pct, 1)),
                        ),
                        (
                            "free_gb".into(),
                            Json::Float(radar_core::time::round_digits(free_gb, 1)),
                        ),
                    ])),
                );
            }
        }
    }

    // -- tile tree: per-layer frame counts and orphaned .tmp staging dirs --------
    let tile_root = &roots.tile;
    let mut disk_layers: BTreeMap<String, BTreeMap<String, BTreeSet<String>>> = BTreeMap::new();
    let mut orphans: Vec<String> = Vec::new();
    if tile_root.is_dir() {
        for layer_dir in sorted_children(tile_root) {
            let Some(layer_name) = file_name(&layer_dir) else {
                continue;
            };
            if !layer_dir.is_dir() || !layer_name.chars().next().is_some_and(char::is_alphabetic) {
                continue;
            }
            let mut palettes: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
            for palette_dir in sorted_children(&layer_dir) {
                if !palette_dir.is_dir() {
                    continue;
                }
                let Some(palette_name) = file_name(&palette_dir) else {
                    continue;
                };
                // Skip dot-dirs such as the tiler's `.render-publish-locks`.
                if palette_name.starts_with('.') {
                    continue;
                }
                let mut stamps = BTreeSet::new();
                for ts_dir in sorted_children(&palette_dir) {
                    if !ts_dir.is_dir() {
                        continue;
                    }
                    let Some(ts_name) = file_name(&ts_dir) else {
                        continue;
                    };
                    if ts_name.ends_with(".tmp") {
                        if let Ok(mtime) = mtime_of(&ts_dir) {
                            let age = age_secs(now, mtime);
                            if age > ORPHAN_TMP_AGE_S {
                                orphans.push(format!(
                                    "{} ({} old)",
                                    ts_dir.display(),
                                    age_str(age)
                                ));
                            }
                        }
                        continue;
                    }
                    stamps.insert(ts_name);
                }
                palettes.insert(palette_name, stamps);
            }
            disk_layers.insert(layer_name, palettes);
        }

        for (layer, palettes) in &disk_layers {
            let counts: Vec<(String, usize)> =
                palettes.iter().map(|(p, s)| (p.clone(), s.len())).collect();
            let distinct: BTreeSet<usize> = counts.iter().map(|(_, n)| *n).collect();
            // Palettes of one layer render from one grid in one activity, so a lopsided count usually
            // means partial palette failures. Palettes with different minimum thresholds can also skip
            // fully transparent frames legitimately.
            let lopsided = distinct.len() > 1;
            let mut detail = format!("frames per palette: {}", py_repr_counts(&counts));
            if lopsided {
                detail.push_str(
                    " — uneven counts: palette render failures, or palettes with different transparency thresholds",
                );
            }
            let data = Json::Object(
                counts
                    .iter()
                    .map(|(p, n)| (p.clone(), Json::Int(*n as i64)))
                    .collect(),
            );
            checks.push(
                Check::new(
                    format!("disk.tiles.{layer}"),
                    if lopsided { Status::Warn } else { Status::Ok },
                    detail,
                )
                .with_data(Json::Object(vec![("counts".into(), data)])),
            );
        }
    }

    if !orphans.is_empty() {
        checks.push(Check::new(
            "disk.tiles.orphans",
            Status::Warn,
            format!(
                "{} stale .tmp staging dir(s) from crashed/cancelled renders: {}",
                orphans.len(),
                orphans
                    .iter()
                    .take(5)
                    .cloned()
                    .collect::<Vec<_>>()
                    .join("; ")
            ),
        ));
    } else if tile_root.is_dir() {
        checks.push(Check::new(
            "disk.tiles.orphans",
            Status::Ok,
            "no stale .tmp staging dirs",
        ));
    }

    // -- manifest vs disk drift ---------------------------------------------------
    let manifest_file = manifest::manifest_path(&roots.state);
    if manifest_file.is_file() && !disk_layers.is_empty() {
        let layers = match manifest::read_manifest(&roots.state) {
            Ok(m) => m.layers,
            Err(err) => {
                checks.push(Check::new(
                    "disk.manifest",
                    Status::Fail,
                    format!("manifest unreadable: {}", describe(&err)),
                ));
                Default::default()
            }
        };
        for (layer_name, layer) in &layers {
            let advertised: BTreeSet<&str> = layer.timestamps.iter().map(String::as_str).collect();
            let palettes: Vec<String> = if layer.palettes.is_empty() {
                vec!["classic".to_string()]
            } else {
                layer.palettes.clone()
            };
            let mut on_disk: BTreeSet<String> = BTreeSet::new();
            for pal in &palettes {
                if let Some(stamps) = disk_layers.get(layer_name).and_then(|p| p.get(pal)) {
                    on_disk.extend(stamps.iter().cloned());
                }
            }
            // Advertised but no tiles: every client 404s those frames.
            let ghost: Vec<String> = advertised
                .iter()
                .filter(|t| !on_disk.contains(**t))
                .map(|t| t.to_string())
                .collect();
            // Rendered but not advertised: invisible to clients.
            let hidden: Vec<String> = on_disk
                .iter()
                .filter(|t| !advertised.contains(t.as_str()))
                .cloned()
                .collect();
            let name = format!("disk.manifest.{layer_name}");
            if !ghost.is_empty() {
                let head: Vec<String> = ghost.iter().take(3).cloned().collect();
                checks.push(
                    Check::new(
                        name,
                        Status::Fail,
                        format!(
                            "{} manifest timestamp(s) have NO tiles on disk (clients 404 these frames): {}...",
                            ghost.len(),
                            py_repr_list(&head)
                        ),
                    )
                    .with_data(Json::Object(vec![("ghost".into(), str_array(&ghost))])),
                );
            } else if hidden.len() > 2 {
                // One or two is normal mid-publish skew.
                checks.push(
                    Check::new(
                        name,
                        Status::Warn,
                        format!(
                            "{} rendered timestamp(s) missing from manifest (invisible to clients)",
                            hidden.len()
                        ),
                    )
                    .with_data(Json::Object(vec![("hidden".into(), str_array(&hidden))])),
                );
            } else {
                checks.push(Check::new(
                    name,
                    Status::Ok,
                    format!("{} advertised frames all present on disk", advertised.len()),
                ));
            }
        }
    }

    // -- state files: freshness and size -------------------------------------------
    if roots.state.is_dir() {
        let expectations: [(&str, Option<f64>); 5] = [
            ("manifest.json", Some(600.0)),
            ("nowcast-status.json", Some(600.0)),
            ("lightning.json", Some(300.0)),
            ("storms.json", Some(600.0)),
            ("tropical.json", None),
        ];
        for (fname, max_age) in expectations {
            let f = roots.state.join(fname);
            let name = format!("disk.state.{fname}");
            if !f.is_file() {
                checks.push(Check::new(name, Status::Warn, "missing"));
                continue;
            }
            let meta = match fs::metadata(&f) {
                Ok(m) => m,
                Err(e) => {
                    checks.push(Check::new(name, Status::Fail, e.to_string()));
                    continue;
                }
            };
            let age = match meta.modified() {
                Ok(m) => age_secs(now, m),
                Err(e) => {
                    checks.push(Check::new(name, Status::Fail, e.to_string()));
                    continue;
                }
            };
            let status = match max_age {
                Some(max) if age > max => Status::Warn,
                _ => Status::Ok,
            };
            checks.push(
                Check::new(
                    name,
                    status,
                    format!("written {} ago · {} KB", age_str(age), meta.len() / 1024),
                )
                .with_data(Json::Object(vec![
                    ("age_s".into(), Json::Int(age as i64)),
                    ("bytes".into(), Json::Int(meta.len() as i64)),
                ])),
            );
        }
    }

    // -- ingest scratch dirs: leaked attempt dirs ---------------------------------
    for root in &roots.tmp_roots {
        if !root.is_dir() {
            continue;
        }
        let root_name = file_name(root).unwrap_or_default();
        let mut stale = 0usize;
        let mut total_bytes: u64 = 0;
        for child in sorted_children(root) {
            let Ok(mtime) = mtime_of(&child) else {
                continue;
            };
            if age_secs(now, mtime) > ORPHAN_TMP_AGE_S {
                stale += 1;
                total_bytes += dir_bytes(&child);
            }
        }
        let name = format!("disk.tmp.{root_name}");
        if stale > 0 {
            checks.push(Check::new(
                name,
                Status::Warn,
                format!(
                    "{stale} leaked work dir(s), {:.0} MB (cancelled activities skip their cleanup handler)",
                    total_bytes as f64 / 1e6
                ),
            ));
        } else {
            checks.push(Check::new(name, Status::Ok, "no leaked work dirs"));
        }
    }

    checks
}

/// Sum of regular-file sizes under `dir` (`rglob` + `is_file`).
fn dir_bytes(dir: &Path) -> u64 {
    WalkDir::new(dir)
        .into_iter()
        .filter_map(Result::ok)
        .filter(|e| e.file_type().is_file())
        .filter_map(|e| e.metadata().ok())
        .map(|m| m.len())
        .sum()
}

/// Directory entries sorted by path, unreadable entries skipped.
fn sorted_children(dir: &Path) -> Vec<PathBuf> {
    let mut out: Vec<PathBuf> = match fs::read_dir(dir) {
        Ok(rd) => rd.filter_map(|e| e.ok().map(|e| e.path())).collect(),
        Err(_) => Vec::new(),
    };
    out.sort();
    out
}

fn file_name(path: &Path) -> Option<String> {
    path.file_name()
        .and_then(|n| n.to_str())
        .map(str::to_string)
}

fn str_array(items: &[String]) -> Json {
    Json::Array(items.iter().map(|s| Json::Str(s.clone())).collect())
}

/// Short text for a manifest error, as the Python `str(exc)` would show.
fn describe(err: &ManifestError) -> String {
    err.to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::time::Duration;
    use tempfile::TempDir;

    /// Everything under one tempdir: `tiles/`, `grids/`, `state/`, and two empty scratch roots.
    struct Fixture {
        _dir: TempDir,
        roots: DiskRoots,
    }

    fn fixture() -> Fixture {
        let dir = TempDir::new().unwrap();
        let base = dir.path();
        let roots = DiskRoots {
            tile: base.join("tiles"),
            grid: base.join("grids"),
            state: base.join("state"),
            tmp_roots: vec![base.join("mrms_work"), base.join("hrrr_work")],
        };
        fs::create_dir_all(&roots.tile).unwrap();
        fs::create_dir_all(&roots.grid).unwrap();
        fs::create_dir_all(&roots.state).unwrap();
        for t in &roots.tmp_roots {
            fs::create_dir_all(t).unwrap();
        }
        Fixture { _dir: dir, roots }
    }

    fn mkdirs(base: &Path, parts: &[&str]) {
        let mut p = base.to_path_buf();
        for part in parts {
            p.push(part);
        }
        fs::create_dir_all(p).unwrap();
    }

    fn set_old(path: &Path, age: Duration) {
        let when = SystemTime::now() - age;
        fs::File::open(path).unwrap().set_modified(when).unwrap();
    }

    fn find<'a>(checks: &'a [Check], name: &str) -> &'a Check {
        checks
            .iter()
            .find(|c| c.name == name)
            .unwrap_or_else(|| panic!("no check {name}: {checks:#?}"))
    }

    #[test]
    fn no_mounts_is_a_single_fail() {
        let dir = TempDir::new().unwrap();
        let roots = DiskRoots {
            tile: dir.path().join("a"),
            grid: dir.path().join("b"),
            state: dir.path().join("c"),
            tmp_roots: vec![],
        };
        let checks = check_disk(&roots, SystemTime::now());
        assert_eq!(checks.len(), 1);
        assert_eq!(checks[0].name, "disk.mounts");
        assert_eq!(checks[0].status, Status::Fail);
        assert!(!roots.any_present());
    }

    #[test]
    fn usage_is_reported_for_each_present_root() {
        let f = fixture();
        let checks = check_disk(&f.roots, SystemTime::now());
        for name in ["disk.tiles.usage", "disk.grids.usage", "disk.state.usage"] {
            let c = find(&checks, name);
            assert!(c.detail.contains("% used"), "{}", c.detail);
            assert!(matches!(c.data, Json::Object(_)));
        }
    }

    #[test]
    fn ghost_frames_fail_and_hidden_frames_warn_past_two() {
        let f = fixture();
        let t1 = "2026-10-14T12:00:00+00:00";
        let t2 = "2026-10-14T12:05:00+00:00";
        let t3 = "2026-10-14T12:10:00+00:00";
        // Rendered on disk: t1 and t2 (classic and dbz), plus four extra stamps that are not advertised.
        for pal in ["classic", "dbz"] {
            for ts in [
                t1,
                t2,
                "2026-10-14T11:00:00+00:00",
                "2026-10-14T11:05:00+00:00",
                "2026-10-14T11:10:00+00:00",
                "2026-10-14T11:15:00+00:00",
            ] {
                mkdirs(&f.roots.tile, &["radar", pal, ts]);
            }
        }
        // Advertised: t1, t2, t3. t3 has no tiles, so it is a ghost.
        let manifest = json!({
            "schema_version": 2,
            "updated_at": "2026-10-14T12:10:00+00:00",
            "layers": {
                // Shaped like the Python writer's output: layer-level `timestamps`, `latest` and
                // `palettes` are written alongside `frames`, and the reader trusts the stored keys.
                "radar": {
                    "timestamps": [t1, t2, t3],
                    "latest": t3,
                    "palettes": ["classic", "dbz"],
                    "frames": [
                        {"timestamp": t1, "path": t1, "palettes": ["classic", "dbz"]},
                        {"timestamp": t2, "path": t2, "palettes": ["classic", "dbz"]},
                        {"timestamp": t3, "path": t3, "palettes": ["classic", "dbz"]},
                    ]
                }
            }
        });
        fs::write(f.roots.state.join("manifest.json"), manifest.to_string()).unwrap();
        let checks = check_disk(&f.roots, SystemTime::now());
        let c = find(&checks, "disk.manifest.radar");
        assert_eq!(c.status, Status::Fail, "{}", c.detail);
        assert!(
            c.detail
                .starts_with("1 manifest timestamp(s) have NO tiles on disk")
        );
        assert!(c.detail.contains(&format!("['{t3}']...")), "{}", c.detail);
    }

    #[test]
    fn uneven_palette_counts_warn() {
        let f = fixture();
        mkdirs(
            &f.roots.tile,
            &["radar", "classic", "2026-10-14T12:00:00+00:00"],
        );
        mkdirs(&f.roots.tile, &["radar", "dbz"]);
        let checks = check_disk(&f.roots, SystemTime::now());
        let c = find(&checks, "disk.tiles.radar");
        assert_eq!(c.status, Status::Warn);
        assert!(
            c.detail
                .starts_with("frames per palette: {'classic': 1, 'dbz': 0}"),
            "{}",
            c.detail
        );
        assert!(c.detail.contains("uneven counts"));
    }

    #[test]
    fn lock_dir_is_not_a_palette() {
        let f = fixture();
        mkdirs(
            &f.roots.tile,
            &["radar", "classic", "2026-10-14T12:00:00+00:00"],
        );
        mkdirs(&f.roots.tile, &["radar", ".render-publish-locks"]);
        let checks = check_disk(&f.roots, SystemTime::now());
        let c = find(&checks, "disk.tiles.radar");
        assert_eq!(c.status, Status::Ok, "{}", c.detail);
        assert!(!c.detail.contains("render-publish-locks"), "{}", c.detail);
    }

    #[test]
    fn stale_tmp_staging_dirs_are_orphans_fresh_ones_are_not() {
        let f = fixture();
        let stale = f
            .roots
            .tile
            .join("radar/classic/2026-10-14T12:00:00+00:00.tmp");
        let fresh = f
            .roots
            .tile
            .join("radar/classic/2026-10-14T12:05:00+00:00.tmp");
        mkdirs(
            &f.roots.tile,
            &["radar", "classic", "2026-10-14T12:00:00+00:00.tmp"],
        );
        mkdirs(
            &f.roots.tile,
            &["radar", "classic", "2026-10-14T12:05:00+00:00.tmp"],
        );
        set_old(&stale, Duration::from_secs(3 * 3600));
        let checks = check_disk(&f.roots, SystemTime::now());
        let c = find(&checks, "disk.tiles.orphans");
        assert_eq!(c.status, Status::Warn);
        assert!(
            c.detail.starts_with("1 stale .tmp staging dir(s)"),
            "{}",
            c.detail
        );
        assert!(!c.detail.contains(&fresh.display().to_string()));
    }

    #[test]
    fn no_orphans_reports_ok_when_tiles_exist() {
        let f = fixture();
        mkdirs(
            &f.roots.tile,
            &["radar", "classic", "2026-10-14T12:00:00+00:00"],
        );
        let checks = check_disk(&f.roots, SystemTime::now());
        assert_eq!(find(&checks, "disk.tiles.orphans").status, Status::Ok);
    }

    #[test]
    fn malformed_manifest_fails_closed() {
        let f = fixture();
        mkdirs(
            &f.roots.tile,
            &["radar", "classic", "2026-10-14T12:00:00+00:00"],
        );
        fs::write(f.roots.state.join("manifest.json"), "{not json").unwrap();
        let checks = check_disk(&f.roots, SystemTime::now());
        let c = find(&checks, "disk.manifest");
        assert_eq!(c.status, Status::Fail);
        assert!(c.detail.starts_with("manifest unreadable: "));
        // No per-layer drift checks when the manifest cannot be read.
        assert!(!checks.iter().any(|c| c.name == "disk.manifest.radar"));
    }

    #[test]
    fn state_files_missing_stale_and_fresh() {
        let f = fixture();
        let now = SystemTime::now();
        fs::write(f.roots.state.join("storms.json"), "[]").unwrap();
        set_old(
            &f.roots.state.join("storms.json"),
            Duration::from_secs(3600),
        );
        fs::write(f.roots.state.join("tropical.json"), "{}").unwrap();
        let checks = check_disk(&f.roots, now);
        assert_eq!(find(&checks, "disk.state.manifest.json").detail, "missing");
        assert_eq!(
            find(&checks, "disk.state.manifest.json").status,
            Status::Warn
        );
        assert_eq!(find(&checks, "disk.state.storms.json").status, Status::Warn);
        assert_eq!(find(&checks, "disk.state.tropical.json").status, Status::Ok);
    }

    #[test]
    fn leaked_scratch_dirs_warn_with_size() {
        let f = fixture();
        let leak = f.roots.tmp_roots[0].join("attempt-1");
        fs::create_dir_all(&leak).unwrap();
        fs::write(leak.join("grib.bin"), vec![0u8; 2_000_000]).unwrap();
        set_old(&leak, Duration::from_secs(3 * 3600));
        let checks = check_disk(&f.roots, SystemTime::now());
        let c = find(&checks, "disk.tmp.mrms_work");
        assert_eq!(c.status, Status::Warn);
        assert_eq!(
            c.detail,
            "1 leaked work dir(s), 2 MB (cancelled activities skip their cleanup handler)"
        );
        assert_eq!(find(&checks, "disk.tmp.hrrr_work").status, Status::Ok);
    }

    #[test]
    fn negative_age_is_signed() {
        let now = SystemTime::UNIX_EPOCH + Duration::from_secs(1000);
        assert_eq!(age_secs(now, now + Duration::from_secs(30)), -30.0);
        assert_eq!(age_secs(now, now - Duration::from_secs(30)), 30.0);
    }
}
