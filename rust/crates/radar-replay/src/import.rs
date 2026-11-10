//! Operator import orchestration; no live ingest context or manifest writer is constructed.

use crate::{case, objects, plan, quota, revision};
use chrono::{DateTime, Duration, Utc};
use radar_core::fsutil::{FlockGuard, durable_mkdir, fsync_dir};
use radar_geo::{Grid, GridGeo};
use radar_grib::{mrms as gmrms, s3::S3Client};
use radar_tiler::render::SourceWindow;
use radar_tiler::{FrameKind, FrameOptions, Overview, TypePlane};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::Instant;

pub struct Options {
    pub case_id: String,
    pub revision: Option<String>,
    pub tile_root: PathBuf,
    pub dry_run: bool,
    pub max_frames: Option<usize>,
    pub layers: Vec<String>,
    pub base_url: String,
    pub verify_crop: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
struct PlannedFrame {
    layer: String,
    timestamp: String,
    key: String,
    flag_key: Option<String>,
}

pub async fn run(options: Options) -> anyhow::Result<Value> {
    run_with_check(options, quota::check).await
}

async fn run_with_check(
    options: Options,
    check: fn(&Path, u64) -> anyhow::Result<()>,
) -> anyhow::Result<Value> {
    let case = *case::find(&options.case_id)?;
    anyhow::ensure!(
        options.max_frames != Some(0),
        "--max-frames must be positive"
    );
    for layer in &options.layers {
        anyhow::ensure!(case::layer(layer).is_some(), "unknown replay layer {layer}");
    }
    let revision_name = options
        .revision
        .clone()
        .unwrap_or_else(|| Utc::now().format("%Y%m%dt%H%M%Sz").to_string());
    revision::layout(&options.tile_root, &case, &revision_name)?;
    let s3 = S3Client::new(radar_grib::http::client()?, options.base_url.clone());
    let coverage = objects::check_case(&s3, case.id).await?;
    let (start, end) = case.window()?;
    let flags = objects::list_objects(
        &s3,
        case::PRECIP_FLAG_PREFIX,
        start - Duration::minutes(5),
        end,
    )
    .await?;
    let mut frames = Vec::new();
    let mut selection_gaps = BTreeMap::new();
    let base_times: Vec<DateTime<Utc>> = coverage.layers[0]
        .keys
        .iter()
        .map(|k| gmrms::key_time(k))
        .collect::<Result<_, _>>()?;
    for coverage_layer in &coverage.layers {
        if !options.layers.is_empty() && !options.layers.contains(&coverage_layer.layer) {
            continue;
        }
        let spec = case::layer(&coverage_layer.layer).unwrap();
        let listed: Vec<_> = coverage_layer
            .keys
            .iter()
            .map(|k| Ok((gmrms::key_time(k)?, k.clone())))
            .collect::<anyhow::Result<_>>()?;
        let (_, missing) = plan::choose_frames(&base_times, &listed, plan::staleness_for(spec));
        selection_gaps.insert(spec.layer, missing);
        for (timestamp, key) in listed
            .into_iter()
            .take(options.max_frames.unwrap_or(usize::MAX))
        {
            let flag_key = if matches!(spec.layer, case::RADAR_LAYER | case::COMPOSITE_LAYER) {
                plan::choose_frames(&[timestamp], &flags, Duration::minutes(5))
                    .0
                    .first()
                    .map(|f| f.key.clone())
            } else {
                None
            };
            frames.push(PlannedFrame {
                layer: spec.layer.into(),
                timestamp: gmrms::format_valid_time(timestamp),
                key,
                flag_key,
            });
        }
    }
    anyhow::ensure!(
        !frames.is_empty(),
        "no archive objects available for selected layers"
    );
    let tiles = plan::case_tiles(&case.bounds, &case::CASE_ZOOMS);
    let pinned_plan = json!({
        "schema_version": 1, "case": case.id, "revision": revision_name,
        "base_url": options.base_url, "max_frames": options.max_frames,
        "layers": options.layers, "frames": frames, "tiles": tiles,
        "renderer": radar_tiler::identity::RENDERER,
        "renderer_source_sha256": env!("REPLAY_RENDERER_SOURCE_SHA256"),
        "verify_crop": options.verify_crop,
        "png_compress_level": radar_tiler::env::DEFAULT_PNG_COMPRESS_LEVEL,
    });
    if options.dry_run {
        return Ok(
            json!({"plan": pinned_plan, "coverage": coverage, "selection_gaps": selection_gaps}),
        );
    }
    // One lock covers every case, staging write and reservation against the shared quota.
    anyhow::ensure!(
        options.tile_root.is_dir(),
        "--tile-root must be an existing directory"
    );
    let tile_root = fs::canonicalize(&options.tile_root)?;
    let replay_root = quota::replay_root(&tile_root);
    quota::dir_usage(&replay_root)?;
    check(&tile_root, 4 * 1024 * 1024)?;
    durable_mkdir(&replay_root, &fsync_dir)?;
    let _lock = FlockGuard::exclusive(&replay_root.join(".import.lock"))?;
    check(&tile_root, 4 * 1024 * 1024)?;
    let layout = revision::layout(&tile_root, &case, &revision_name)?;
    anyhow::ensure!(
        !layout.final_dir.try_exists()?,
        "revision already published; choose another --revision"
    );
    durable_mkdir(&layout.staging, &fsync_dir)?;
    let plan_file = layout.staging.join("plan.json");
    if plan_file.try_exists()? {
        let prior: Value = serde_json::from_slice(&fs::read(&plan_file)?)?;
        anyhow::ensure!(
            prior == pinned_plan,
            "resume plan differs; use original options/build or a new revision"
        );
    } else {
        revision::write_json_atomic(&plan_file, &pinned_plan)?;
    }
    revision::write_json_atomic(&layout.staging.join(revision::COVERAGE_FILE), &coverage)?;
    revision::write_json_atomic(&layout.staging.join("selection-gaps.json"), &selection_gaps)?;
    let progress_file = layout.staging.join(revision::PROGRESS_FILE);
    let mut progress = revision::Progress::load(&progress_file, case.id, &revision_name)?;
    let started = Instant::now();
    let mut verified_tiles = 0;
    for frame in &frames {
        let id = format!("{}@{}", frame.layer, frame.timestamp);
        let output = frame_dir(&layout.staging, &frame.layer, &frame.timestamp);
        if let Some(done) = progress.frames.get(&id) {
            anyhow::ensure!(
                done.key == frame.key && done.flag_key == frame.flag_key,
                "resume provenance differs from pinned plan"
            );
            verify_frame(&output, done, &tiles)?;
            continue;
        }
        // An indexed 256px PNG is smaller than 256 KiB, including chunks and palette.
        let stat = rustix::fs::statvfs(&tile_root)?;
        let unit = stat.f_bsize.max(stat.f_frsize).max(4096);
        let per_tile = (256 * 1024_u64).div_ceil(unit) * unit + 4 * unit;
        let reserve = (tiles.len() as u64 * per_tile + 4 * 1024 * 1024 + 64 * unit)
            * if options.verify_crop { 2 } else { 1 };
        check(&tile_root, reserve)?;
        let (_, bytes) = gmrms::fetch_gz(&s3, &frame.key).await?;
        let flag = match &frame.flag_key {
            Some(key) => Some(gmrms::fetch_gz(&s3, key).await?.1),
            None => None,
        };
        let frame = frame.clone();
        let staging = layout.staging.clone();
        let tiles = tiles.clone();
        let verify_crop = options.verify_crop;
        let (done, verified) = std::thread::spawn(move || {
            render(
                &frame,
                &staging,
                &tiles,
                &bytes,
                flag.as_deref(),
                verify_crop,
            )
        })
        .join()
        .map_err(|_| anyhow::anyhow!("replay render thread panicked"))??;
        verified_tiles += verified;
        check(&tile_root, 4 * 1024 * 1024)?;
        progress.record(&progress_file, done)?;
    }
    anyhow::ensure!(
        progress.frames.len() == frames.len(),
        "progress contains unexpected frames"
    );
    for frame in progress.frames.values() {
        verify_frame(
            &frame_dir(&layout.staging, &frame.layer, &frame.timestamp),
            frame,
            &tiles,
        )?;
    }
    check(&tile_root, 4 * 1024 * 1024)?;
    let metadata = json!({
        "schema_version": 1, "case": case.id, "revision": revision_name,
        "status": "tiles_complete", "public_ready": false,
        "warnings": {"status": "missing", "issue": 282},
        "scope": if options.max_frames.is_some() || !options.layers.is_empty() { "pilot" } else { "case" },
        "start": case.start, "end": case.end, "bounds": case.bounds, "timezone": case.timezone,
        "renderer": radar_tiler::identity::RENDERER,
        "renderer_source_sha256": env!("REPLAY_RENDERER_SOURCE_SHA256"),
        "sources": [{"name": "NOAA MRMS", "base_url": options.base_url}],
        "manifest": revision::MANIFEST_FILE, "coverage": revision::COVERAGE_FILE,
        "provenance": revision::PROGRESS_FILE, "plan": "plan.json",
    });
    revision::write_json_atomic(&layout.staging.join("revision.json"), &metadata)?;
    let report = json!({
        "case": case.id, "revision": revision_name, "frames": progress.frames.len(),
        "render_ms": progress.frames.values().map(|f| f.render_ms).sum::<u64>(),
        "elapsed_ms": started.elapsed().as_millis(), "boundary_tiles_verified": verified_tiles,
        "usage_before_report": quota::dir_usage(&layout.staging)?,
        "replay_usage_before_report": quota::dir_usage(&replay_root)?,
        "free_bytes": quota::free_bytes(&tile_root)?,
    });
    revision::write_json_atomic(&layout.staging.join(revision::REPORT_FILE), &report)?;
    revision::publish(&layout, &case, &progress)?;
    check(&tile_root, 0)?;
    Ok(
        json!({"report": report, "published": layout.final_dir, "usage": quota::dir_usage(&layout.final_dir)?}),
    )
}

fn frame_dir(root: &Path, layer: &str, timestamp: &str) -> PathBuf {
    root.join("tiles")
        .join(layer)
        .join(case::CASE_PALETTE)
        .join(timestamp)
}

fn verify_frame(
    path: &Path,
    frame: &revision::FrameProgress,
    tiles: &[(u32, u32, u32)],
) -> anyhow::Result<()> {
    let inspection = radar_tiler::marker::inspect_complete_pyramid(path, None)?;
    anyhow::ensure!(
        inspection.identity.source_id == frame.source_id,
        "resume source identity mismatch"
    );
    anyhow::ensure!(
        inspection.tile_count as u64 == frame.tiles && inspection.tile_count == tiles.len(),
        "resume tile count mismatch"
    );
    for rel in &inspection.tile_paths {
        anyhow::ensure!(
            tiles
                .iter()
                .any(|(z, x, y)| *rel == format!("{z}/{x}/{y}.png")),
            "tile outside case allowlist"
        );
    }
    Ok(())
}

fn render(
    frame: &PlannedFrame,
    staging: &Path,
    tiles: &[(u32, u32, u32)],
    bytes: &[u8],
    flag: Option<&[u8]>,
    verify_crop: bool,
) -> anyhow::Result<(revision::FrameProgress, u64)> {
    let mut grid = gmrms::decode_gz(bytes)?;
    gmrms::ensure_south_up(&mut grid);
    let spec = case::layer(&frame.layer).unwrap();
    for v in &mut grid.values {
        *v = (spec.convert)(*v);
    }
    let types = flag
        .map(|gz| radar_ingest::precip_type::type_plane(gz, &grid))
        .transpose()?;
    let window = plan::crop_window(&grid, tiles, plan::CROP_HALO_CELLS)?;
    let cropped = plan::crop_grid(&grid, window)?;
    let cropped_types = types
        .as_ref()
        .map(|t| crop_types(t, grid.width, window))
        .transpose()?;
    let GridGeo::Regular { lats, lons } = &grid.geo else {
        anyhow::bail!("MRMS axes must be regular");
    };
    let source_id = format!(
        "replay:{}:{}:{}",
        frame.layer,
        frame.key,
        radar_tiler::identity::sha256_hex(bytes)
    );
    let mut options = FrameOptions {
        source_id: source_id.clone(),
        png_compress_level: radar_tiler::env::DEFAULT_PNG_COMPRESS_LEVEL as u32,
        overview: Some(Overview::Max),
        keep_empty: true,
        publication_lock_root: Some(staging.to_path_buf()),
        types: cropped_types,
        tile_allowlist: Some(tiles.to_vec()),
        source_window: Some(SourceWindow {
            lats: lats.clone(),
            lons: lons.clone(),
            row: window.0.0,
            col: window.1.0,
        }),
        ..FrameOptions::default()
    };
    let tables = radar_ingest::mrms_qpe::load_tables(&[case::CASE_PALETTE.into()], spec.color_key);
    let kind = FrameKind::Continuous {
        nodata_value: None,
        min_valid_weight: 1.0,
    };
    let output = frame_dir(staging, &frame.layer, &frame.timestamp);
    let full = if verify_crop {
        Some(grid)
    } else {
        drop(grid);
        None
    };
    let full_types = if verify_crop {
        types
    } else {
        drop(types);
        None
    };
    let started = Instant::now();
    let result = radar_tiler::render_frame_palettes(
        &cropped,
        &kind,
        &tables,
        &[(case::CASE_PALETTE.into(), output.clone())],
        &case::CASE_ZOOMS,
        &options,
    )?;
    let render_ms = started.elapsed().as_millis() as u64;
    let outcome = result
        .outcome(case::CASE_PALETTE)
        .ok_or_else(|| anyhow::anyhow!("missing classic render outcome"))?;
    anyhow::ensure!(
        matches!(
            outcome.status,
            radar_tiler::PublishStatus::Created | radar_tiler::PublishStatus::ExistingValid
        ),
        "incomplete or foreign frame"
    );
    let mut verified = 0;
    if verify_crop {
        // Parity staging stays inside the replay quota and disappears after verified comparison.
        let parity = staging.join(".parity");
        options.source_window = None;
        options.types = full_types;
        let full = full.unwrap();
        radar_tiler::render_frame_palettes(
            &full,
            &kind,
            &tables,
            &[(case::CASE_PALETTE.into(), parity.clone())],
            &case::CASE_ZOOMS,
            &options,
        )?;
        let full_paths = radar_tiler::marker::list_tile_paths(&parity)?;
        let crop_paths = radar_tiler::marker::list_tile_paths(&output)?;
        anyhow::ensure!(full_paths == crop_paths, "cropping changed tile set");
        for path in full_paths {
            anyhow::ensure!(
                fs::read(parity.join(&path))? == fs::read(output.join(&path))?,
                "crop byte mismatch: {path}"
            );
            verified += 1;
        }
        fs::remove_dir_all(parity)?;
        fsync_dir(staging)?;
    }
    Ok((
        revision::FrameProgress {
            layer: frame.layer.clone(),
            playhead: frame.timestamp.clone(),
            timestamp: frame.timestamp.clone(),
            key: frame.key.clone(),
            source_id,
            source_sha256: radar_tiler::identity::sha256_hex(bytes),
            flag_key: frame.flag_key.clone(),
            flag_sha256: flag.map(radar_tiler::identity::sha256_hex),
            tiles: outcome.tile_count,
            bytes: quota::dir_usage(&output)?.bytes,
            render_ms,
        },
        verified,
    ))
}

fn crop_types(
    types: &TypePlane,
    width: usize,
    ((r0, r1), (c0, c1)): ((usize, usize), (usize, usize)),
) -> anyhow::Result<TypePlane> {
    let mut classes = Vec::with_capacity((r1 - r0 + 1) * (c1 - c0 + 1));
    for r in r0..=r1 {
        classes.extend_from_slice(&types.classes()[r * width + c0..=r * width + c1]);
    }
    Ok(TypePlane::new(classes)?)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mrms_boundary_tiles_match_full_grid_at_native_and_overview_zooms() {
        let (height, width) = (3500, 7000);
        let grid = Grid {
            height,
            width,
            values: (0..height * width)
                .map(|k| 10.0 + ((k / width * 17 + k % width * 31) % 500) as f32 * 0.1)
                .collect(),
            geo: GridGeo::Regular {
                lats: (0..height).map(|i| 20.005 + i as f64 * 0.01).collect(),
                lons: (0..width).map(|i| -129.995 + i as f64 * 0.01).collect(),
            },
        };
        let GridGeo::Regular { lats, lons } = &grid.geo else {
            unreachable!()
        };
        let tables = radar_ingest::mrms_qpe::load_tables(&["classic".into()], "reflectivity");
        let kind = FrameKind::Continuous {
            nodata_value: None,
            min_valid_weight: 1.0,
        };
        for case in case::CASES {
            let tmp = tempfile::tempdir().unwrap();
            let tiles = plan::case_tiles(&case.bounds, &case::CASE_ZOOMS);
            let window = plan::crop_window(&grid, &tiles, plan::CROP_HALO_CELLS).unwrap();
            let cropped = plan::crop_grid(&grid, window).unwrap();
            assert!(cropped.values.len() < grid.values.len() / 2);
            let mut options = FrameOptions {
                tile_allowlist: Some(tiles.clone()),
                overview: Some(Overview::Max),
                keep_empty: true,
                ..FrameOptions::default()
            };
            let full = tmp.path().join("full");
            radar_tiler::render_frame_palettes(
                &grid,
                &kind,
                &tables,
                &[("classic".into(), full.clone())],
                &case::CASE_ZOOMS,
                &options,
            )
            .unwrap();
            options.source_window = Some(SourceWindow {
                lats: lats.clone(),
                lons: lons.clone(),
                row: window.0.0,
                col: window.1.0,
            });
            let crop = tmp.path().join("crop");
            radar_tiler::render_frame_palettes(
                &cropped,
                &kind,
                &tables,
                &[("classic".into(), crop.clone())],
                &case::CASE_ZOOMS,
                &options,
            )
            .unwrap();
            let paths = radar_tiler::marker::list_tile_paths(&full).unwrap();
            assert_eq!(paths.len(), tiles.len());
            assert_eq!(paths, radar_tiler::marker::list_tile_paths(&crop).unwrap());
            for path in paths {
                assert_eq!(
                    fs::read(full.join(&path)).unwrap(),
                    fs::read(crop.join(&path)).unwrap(),
                    "{} {path}",
                    case.id
                );
            }
        }
    }

    #[test]
    fn live_cleanup_ignores_old_replay_revisions_and_staging() {
        use radar_aux::tile_cleanup::{TileCleanupConfig, tile_cleanup_sweep_at};
        let tmp = tempfile::tempdir().unwrap();
        let tiles = tmp.path().join("tiles");
        let state = tmp.path().join("state");
        let grid = tmp.path().join("grids");
        fs::create_dir_all(&state).unwrap();
        let old = tiles.join(
            "replays/greenfield-2024/r1/tiles/radar/classic/2024-05-21T19:00:17+00:00/4/3/6.png",
        );
        let staging = tiles.join("replays/greenfield-2024/.staging-r2/tiles");
        fs::create_dir_all(old.parent().unwrap()).unwrap();
        fs::create_dir_all(&staging).unwrap();
        fs::write(&old, b"preserve").unwrap();
        fs::write(staging.join("progress.json"), b"{}").unwrap();
        let before = quota::dir_usage(&tiles.join("replays")).unwrap();
        let result = tile_cleanup_sweep_at(
            &TileCleanupConfig::new(&tiles, &state, &grid),
            std::time::SystemTime::now(),
        )
        .unwrap();
        assert_eq!(result.tile_dirs_removed, 0);
        assert_eq!(before, quota::dir_usage(&tiles.join("replays")).unwrap());
        assert_eq!(fs::read(old).unwrap(), b"preserve");
        assert!(!state.join("manifest.json").exists());
    }
}

#[cfg(test)]
mod resume_tests {
    use super::*;
    use axum::{
        Router,
        extract::{Path as UrlPath, Query, State},
        http::StatusCode,
        response::IntoResponse,
        routing::get,
    };
    use std::sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    };

    #[derive(Clone)]
    struct Bucket {
        fail: Arc<AtomicBool>,
        keys: Vec<String>,
    }

    async fn listing(
        State(bucket): State<Bucket>,
        Query(query): Query<BTreeMap<String, String>>,
    ) -> String {
        let prefix = query.get("prefix").unwrap();
        let keys: String = bucket
            .keys
            .iter()
            .filter(|k| k.starts_with(prefix))
            .map(|k| format!("<Contents><Key>{k}</Key></Contents>"))
            .collect();
        format!("<ListBucketResult><IsTruncated>false</IsTruncated>{keys}</ListBucketResult>")
    }

    async fn object(
        State(bucket): State<Bucket>,
        UrlPath(key): UrlPath<String>,
    ) -> impl IntoResponse {
        if key.contains("190217") && bucket.fail.load(Ordering::SeqCst) {
            return (StatusCode::SERVICE_UNAVAILABLE, Vec::new());
        }
        (
            StatusCode::OK,
            include_bytes!("../../radar-grib/testdata/mrms_regular_ll_png.grib2.gz").to_vec(),
        )
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn interrupted_import_resumes_and_preserves_live_and_prior_revision() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path();
        let old = root.join("replays/greenfield-2024/previous/manifest.json");
        fs::create_dir_all(old.parent().unwrap()).unwrap();
        fs::write(&old, b"previous revision").unwrap();
        fs::write(root.join("manifest.json"), b"live sentinel").unwrap();
        fs::create_dir_all(root.join("nowcast")).unwrap();
        fs::write(root.join("nowcast/input"), b"nowcast sentinel").unwrap();
        let bucket = Bucket { fail: Arc::new(AtomicBool::new(true)), keys: ["190017", "190217"].iter().map(|time| format!("CONUS/MergedBaseReflectivityQC_00.50/20240521/MRMS_MergedBaseReflectivityQC_00.50_20240521-{time}.grib2.gz")).collect() };
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        let router = Router::new()
            .route("/", get(listing))
            .route("/{*key}", get(object))
            .with_state(bucket.clone());
        let server = tokio::spawn(async move { axum::serve(listener, router).await.unwrap() });
        let options = || Options {
            case_id: "greenfield-2024".into(),
            revision: Some("resume-test".into()),
            tile_root: root.into(),
            dry_run: false,
            max_frames: Some(2),
            layers: vec!["radar".into()],
            base_url: base.clone(),
            verify_crop: false,
        };
        let replay_before = quota::dir_usage(&root.join("replays")).unwrap();
        assert!(
            run_with_check(options(), |_, _| anyhow::bail!("quota denied"))
                .await
                .is_err()
        );
        assert_eq!(
            replay_before,
            quota::dir_usage(&root.join("replays")).unwrap()
        );
        assert!(run_with_check(options(), |_, _| Ok(())).await.is_err());
        let stage = root.join("replays/greenfield-2024/.staging-resume-test");
        let before = fs::read(stage.join("progress.json")).unwrap();
        let progress: revision::Progress = serde_json::from_slice(&before).unwrap();
        assert_eq!(progress.frames.len(), 1);
        assert!(!root.join("replays/greenfield-2024/resume-test").exists());
        let first_dir = frame_dir(
            &stage,
            "radar",
            &progress.frames.values().next().unwrap().timestamp,
        );
        let first_marker = first_dir.join(radar_tiler::marker::PYRAMID_COMPLETE_FILE);
        let marker_before = fs::metadata(&first_marker).unwrap().modified().unwrap();
        let tile_path = radar_tiler::marker::list_tile_paths(&first_dir)
            .unwrap()
            .into_iter()
            .next()
            .unwrap();
        let tile_path = first_dir.join(tile_path);
        let tile_bytes = fs::read(&tile_path).unwrap();
        fs::write(&tile_path, b"corrupt").unwrap();
        assert!(run_with_check(options(), |_, _| Ok(())).await.is_err());
        fs::write(&tile_path, tile_bytes).unwrap();
        bucket.fail.store(false, Ordering::SeqCst);
        let report = run_with_check(options(), |_, _| Ok(())).await.unwrap();
        let published = PathBuf::from(report["published"].as_str().unwrap());
        let done: revision::Progress =
            serde_json::from_slice(&fs::read(published.join("progress.json")).unwrap()).unwrap();
        assert_eq!(done.frames.len(), 2);
        let first_after = published.join(first_marker.strip_prefix(stage).unwrap());
        assert_eq!(
            marker_before,
            fs::metadata(first_after).unwrap().modified().unwrap()
        );
        assert_eq!(fs::read(old).unwrap(), b"previous revision");
        assert_eq!(
            fs::read(root.join("manifest.json")).unwrap(),
            b"live sentinel"
        );
        assert_eq!(
            fs::read(root.join("nowcast/input")).unwrap(),
            b"nowcast sentinel"
        );
        let metadata: Value =
            serde_json::from_slice(&fs::read(published.join("revision.json")).unwrap()).unwrap();
        assert_eq!(metadata["public_ready"], false);
        assert_eq!(metadata["warnings"]["status"], "missing");
        assert!(run_with_check(options(), |_, _| Ok(())).await.is_err());
        server.abort();
    }
}

#[cfg(test)]
mod pilot_probe {
    use super::*;

    #[test]
    #[ignore = "requires an operator-created scratch pilot root"]
    fn cleanup_preserves_real_scratch_pilot() {
        let root =
            fs::canonicalize(std::env::var("REPLAY_PILOT_ROOT").expect("set REPLAY_PILOT_ROOT"))
                .unwrap();
        assert!(root.starts_with("/opt/data/cache/scratch"));
        let snapshot = || {
            let mut files = BTreeMap::new();
            for entry in walkdir::WalkDir::new(&root) {
                let entry = entry.unwrap();
                if entry.file_type().is_file() {
                    files.insert(
                        entry.path().strip_prefix(&root).unwrap().to_path_buf(),
                        radar_tiler::identity::sha256_hex(&fs::read(entry.path()).unwrap()),
                    );
                }
            }
            files
        };
        let before = snapshot();
        assert!(before.keys().any(|p| p.starts_with("tiles/replays")));
        let result = radar_aux::tile_cleanup::tile_cleanup_sweep_at(
            &radar_aux::tile_cleanup::TileCleanupConfig::new(
                root.join("tiles"),
                root.join("state"),
                root.join("grids"),
            ),
            std::time::SystemTime::now(),
        )
        .unwrap();
        assert_eq!(result.tile_dirs_removed, 0);
        assert_eq!(result.grid_files_removed, 0);
        assert_eq!(before, snapshot());
        println!(
            "pilot cleanup: {} files unchanged; zero tile or grid deletions",
            before.len()
        );
    }
}
