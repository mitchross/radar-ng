//! HRRR activities against the fake bucket and the Lambert GRIB fixture (spec §14.2 H1, H2, H10 to H19;
//! PLAN D3 and D5).
//!
//! The fixture's REFC field is a uniform -10 dBZ, below every bin of the classic reflectivity palette,
//! so the radar layer only produces tiles from a varied grid (`refc_grid`). Tests that go through the
//! GRIB decode path assert on layers that the fixture's data can fill (temperature, wind).

mod common;

use std::path::{Path, PathBuf};

use radar_core::env::Paths;
use radar_grib::decode::find_message;
use radar_grib::hrrr::HrrrVar;
use radar_ingest::heartbeat;
use radar_ingest::hrrr::activities as act;
use radar_ingest::hrrr::render::{
    existing_rendered_layers, load_hrrr_palettes, process_hour_sync, render_layer,
};
use radar_ingest::scratch::ActivityInfo;
use radar_ingest::{IngestConfig, IngestCtx};
use radar_tiler::marker::{PYRAMID_COMPLETE_FILE, list_tile_paths, write_completion_marker};
use radar_tiler::{PyramidIdentity, complete_pyramid_identity};
use tempfile::TempDir;

const RUN: &str = "20260715_12";
const VALID_F01: &str = "2026-07-15T13:00:00+00:00";

struct Harness {
    /// Held so the directory outlives the test.
    _root: TempDir,
    paths: Paths,
    s3: common::FakeS3,
    ctx: IngestCtx,
}

async fn harness(palettes: &[&str], enabled: &[&str]) -> Harness {
    let root = tempfile::tempdir().unwrap();
    let paths = Paths::new(
        root.path().join("state"),
        root.path().join("grids"),
        root.path().join("tiles"),
    );
    let s3 = common::start().await;
    let mut cfg = IngestConfig::with_roots(paths.clone(), root.path().join("work")).unwrap();
    cfg.hrrr_base_url = s3.base.clone();
    cfg.palettes = palettes.iter().map(|s| s.to_string()).collect();
    cfg.hrrr_enabled_layers = enabled.iter().map(|s| s.to_string()).collect();
    let ctx = IngestCtx::new(cfg, None).unwrap();
    Harness {
        _root: root,
        paths,
        s3,
        ctx,
    }
}

fn info() -> ActivityInfo {
    ActivityInfo {
        workflow_id: "sched/ingest:hrrr".into(),
        run_id: "run-1".into(),
        activity_id: "3".into(),
        attempt: 1,
    }
}

fn work_root(h: &Harness) -> PathBuf {
    h.ctx.cfg.hrrr_tmp_root.clone()
}

fn is_empty_or_missing(p: &Path) -> bool {
    !p.exists() || std::fs::read_dir(p).unwrap().next().is_none()
}

fn key_for(run: &str, fhr: u32) -> String {
    let date = &run[..8];
    let hour = &run[9..11];
    format!("hrrr.{date}/conus/hrrr.t{hour}z.wrfsfcf{fhr:02}.grib2")
}

/// Whether a request log entry is a GET for the object `key` (not its index).
fn got_object(requests: &[String], key: &str) -> bool {
    requests
        .iter()
        .any(|q| q.starts_with("GET") && q.ends_with(key))
}

/// The refc field from the fixture, with finite values spread over 10 to 49 dBZ so the radar palette
/// has data to colour.
fn refc_grid() -> radar_geo::Grid {
    let bytes = common::hrrr_grib();
    let mut grid = find_message(&bytes, &HrrrVar::Refc.selector())
        .unwrap()
        .unwrap()
        .grid;
    for (i, v) in grid.values.iter_mut().enumerate() {
        *v = 10.0 + (i % 40) as f32;
    }
    grid
}

fn layer_dir(h: &Harness, layer: &str, palette: &str, valid: &str) -> PathBuf {
    h.paths
        .tile_layer_dir(layer)
        .join(palette)
        .join("runs")
        .join(RUN)
        .join(valid)
}

fn has_grid_dump(h: &Harness, layer: &str) -> bool {
    std::fs::read_dir(h.paths.grid_layer_dir(layer))
        .map(|rd| {
            rd.flatten()
                .any(|e| e.file_name().to_string_lossy().ends_with(".meta.json"))
        })
        .unwrap_or(false)
}

/// Render the radar-hrrr layer for `valid` across `names`. The return value is ignored: a complete
/// pyramid that is skipped reports no tiles, and callers check the outcome with
/// `existing_rendered_layers`.
fn render_radar(ctx: &IngestCtx, valid: &str, names: &[&str]) {
    let pals = load_hrrr_palettes(&names.iter().map(|s| s.to_string()).collect::<Vec<_>>());
    render_layer(
        &ctx.cfg,
        "radar-hrrr",
        RUN,
        valid,
        &refc_grid(),
        &pals,
        "reflectivity",
        false,
    )
    .unwrap();
}

/// A harness whose radar-hrrr pyramid for F01 is rendered over `names`.
async fn radar_harness(palettes: &[&str]) -> Harness {
    let h = harness(palettes, &["radar-hrrr"]).await;
    render_radar(&h.ctx, VALID_F01, palettes);
    h
}

/// Rewrite a pyramid's completion marker with an edited identity (tests only).
fn restamp(dir: &Path, edit: impl FnOnce(&mut PyramidIdentity)) {
    let mut id = complete_pyramid_identity(dir).expect("complete pyramid");
    edit(&mut id);
    std::fs::remove_file(dir.join(PYRAMID_COMPLETE_FILE)).unwrap();
    let paths = list_tile_paths(dir).unwrap();
    write_completion_marker(dir, &paths, &id).unwrap();
}

/// H1 and H2: a GRIB with refc renders the hour (temperature here, since the fixture's refc has no
/// visible colour at the default palette); a GRIB without refc fails with the spec message.
#[tokio::test]
async fn h1_h2_refc_required_and_hour_renders() {
    let h = harness(&["classic"], &["radar-hrrr", "temperature"]).await;
    let pals = load_hrrr_palettes(&["classic".into()]);
    let rendered =
        process_hour_sync(&h.ctx.cfg, &common::hrrr_grib(), RUN, VALID_F01, &pals).unwrap();
    assert!(
        rendered.iter().any(|l| l == "temperature"),
        "rendered: {rendered:?}"
    );
    assert!(
        layer_dir(&h, "temperature", "classic", VALID_F01)
            .join(PYRAMID_COMPLETE_FILE)
            .is_file()
    );

    // Message 1 (REFC) is 519 bytes; from offset 519 the file starts at the TMP message.
    let err = process_hour_sync(
        &h.ctx.cfg,
        &common::hrrr_grib()[519..],
        RUN,
        VALID_F01,
        &pals,
    )
    .unwrap_err();
    assert!(
        format!("{err:#}").contains("required HRRR variable refc"),
        "{err:#}"
    );
}

/// The fixture's temperature and wind layers render, and temperature leaves a grid dump.
#[tokio::test]
async fn full_pipeline_renders_fixture_layers_and_dumps() {
    let h = harness(&["classic"], &["radar-hrrr", "temperature", "wind"]).await;
    let pals = load_hrrr_palettes(&["classic".into()]);
    let rendered =
        process_hour_sync(&h.ctx.cfg, &common::hrrr_grib(), RUN, VALID_F01, &pals).unwrap();
    assert!(rendered.iter().any(|l| l == "temperature"), "{rendered:?}");
    assert!(rendered.iter().any(|l| l == "wind"), "{rendered:?}");
    assert!(has_grid_dump(&h, "temperature"), "temperature grid dump");
}

/// H10 and H19: index 404 and full-file 404 mean the hour is pending: empty result, no scratch left.
#[tokio::test]
async fn h19_pending_hour_returns_empty_and_cleans_scratch() {
    let h = harness(&["classic"], &["radar-hrrr"]).await;
    let r = act::process_forecast_hour(&h.ctx, &info(), "20260715_18", 42, &heartbeat::noop())
        .await
        .unwrap();
    assert_eq!(r.fhr, 42);
    assert!(r.rendered_layers.is_empty());
    assert!(is_empty_or_missing(&work_root(&h)));
}

/// H11: index 404 with a 200 on the full file falls back to the whole object and renders it.
#[tokio::test]
async fn h11_index_404_falls_back_to_full_file() {
    let h = harness(&["classic"], &["radar-hrrr", "temperature"]).await;
    let key = key_for(RUN, 7);
    h.s3.put(&key, common::hrrr_grib());
    let r = act::process_forecast_hour(&h.ctx, &info(), RUN, 7, &heartbeat::noop())
        .await
        .unwrap();
    assert!(
        r.rendered_layers.iter().any(|l| l == "temperature"),
        "{:?}",
        r.rendered_layers
    );
    assert_eq!(r.valid_timestamp, "2026-07-15T19:00:00+00:00");
    assert!(
        got_object(&h.s3.requests(), &key),
        "full-object GET after index 404"
    );
    assert!(is_empty_or_missing(&work_root(&h)));
}

/// H12 and D4: a 503 on the index is an error, and the full file is not fetched.
#[tokio::test]
async fn h12_index_5xx_fails_without_full_file_fallback() {
    let h = harness(&["classic"], &["radar-hrrr"]).await;
    let key = key_for(RUN, 1);
    h.s3.put(&key, common::hrrr_grib());
    h.s3.bucket
        .lock()
        .unwrap()
        .overrides
        .insert(format!("{key}.idx"), 503);
    assert!(
        act::process_forecast_hour(&h.ctx, &info(), RUN, 1, &heartbeat::noop())
            .await
            .is_err()
    );
    assert!(
        !got_object(&h.s3.requests(), &key),
        "no full-file GET after an index 5xx (D4)"
    );
    assert!(
        is_empty_or_missing(&work_root(&h)),
        "scratch removed after failure (D3)"
    );
}

/// D5: a server that ignores Range (answers 200) fails the hour instead of writing the whole file per range.
#[tokio::test]
async fn d5_range_not_honoured_fails() {
    let h = harness(&["classic"], &["radar-hrrr"]).await;
    let key = key_for(RUN, 1);
    h.s3.put(&key, common::hrrr_grib());
    h.s3.put(&format!("{key}.idx"), common::hrrr_idx());
    h.s3.bucket.lock().unwrap().ignore_range = true;
    assert!(
        act::process_forecast_hour(&h.ctx, &info(), RUN, 1, &heartbeat::noop())
            .await
            .is_err()
    );
    assert!(is_empty_or_missing(&work_root(&h)));
}

/// With an index present, the hour reads the index, fetches the needed records, renders the
/// temperature layer, and leaves no scratch behind.
#[tokio::test]
async fn indexed_download_renders_and_cleans_scratch() {
    let h = harness(&["classic"], &["radar-hrrr", "temperature"]).await;
    let key = key_for(RUN, 1);
    h.s3.put(&key, common::hrrr_grib());
    h.s3.put(&format!("{key}.idx"), common::hrrr_idx());
    let r = act::process_forecast_hour(&h.ctx, &info(), RUN, 1, &heartbeat::noop())
        .await
        .unwrap();
    assert!(
        r.rendered_layers.iter().any(|l| l == "temperature"),
        "{:?}",
        r.rendered_layers
    );
    assert!(
        h.s3.requests().iter().any(|q| q.ends_with(".idx")),
        "index read"
    );
    assert!(is_empty_or_missing(&work_root(&h)));
}

/// H15: a complete radar pyramid for the hour means no download, no scratch, and the hour resumes.
#[tokio::test]
async fn h15_resume_skips_download() {
    let h = radar_harness(&["classic"]).await;
    let r = act::process_forecast_hour(&h.ctx, &info(), RUN, 1, &heartbeat::noop())
        .await
        .unwrap();
    assert_eq!(r.rendered_layers, vec!["radar-hrrr".to_string()]);
    assert_eq!(r.valid_timestamp, VALID_F01);
    assert!(h.s3.requests().is_empty(), "no download on resume");
    assert!(is_empty_or_missing(&work_root(&h)));
}

/// H13: existing layers need every palette complete; a single complete palette, or an unmarked tree,
/// does not count.
#[tokio::test]
async fn h13_existing_layers_need_every_palette() {
    let h = harness(&["classic", "muted"], &["radar-hrrr"]).await;
    let pals = load_hrrr_palettes(&["classic".into(), "muted".into()]);
    let layers = || existing_rendered_layers(&h.ctx.cfg, &pals, RUN, VALID_F01);

    assert!(layers().is_empty(), "nothing rendered");
    render_radar(&h.ctx, VALID_F01, &["classic"]);
    assert!(layers().is_empty(), "one complete palette is not enough");

    // An unmarked, non-empty muted tree must not count. Remove it again before the full render.
    let muted = layer_dir(&h, "radar-hrrr", "muted", VALID_F01);
    std::fs::create_dir_all(&muted).unwrap();
    std::fs::write(muted.join("stray.png"), b"x").unwrap();
    assert!(layers().is_empty(), "non-empty tree without marker");
    std::fs::remove_dir_all(h.paths.tile_layer_dir("radar-hrrr").join("muted")).unwrap();

    render_radar(&h.ctx, VALID_F01, &["classic", "muted"]);
    assert_eq!(layers(), vec!["radar-hrrr".to_string()]);
}

/// H16: a complete pyramid from a foreign renderer with the same contract is accepted.
#[tokio::test]
async fn h16_foreign_renderer_with_same_contract_is_accepted() {
    let h = radar_harness(&["classic"]).await;
    let pals = load_hrrr_palettes(&["classic".into()]);
    restamp(&layer_dir(&h, "radar-hrrr", "classic", VALID_F01), |id| {
        id.renderer = "indexed".into()
    });
    assert_eq!(
        existing_rendered_layers(&h.ctx.cfg, &pals, RUN, VALID_F01),
        vec!["radar-hrrr".to_string()]
    );
}

/// H17: a legacy-renderer pyramid (different algorithm) on one palette blocks the layer.
#[tokio::test]
async fn h17_mixed_legacy_palette_blocks_layer() {
    let h = radar_harness(&["classic", "muted"]).await;
    let pals = load_hrrr_palettes(&["classic".into(), "muted".into()]);
    restamp(&layer_dir(&h, "radar-hrrr", "muted", VALID_F01), |id| {
        id.renderer = "legacy".into();
        id.algorithm = "rgba-bilinear-v1".into();
    });
    assert!(existing_rendered_layers(&h.ctx.cfg, &pals, RUN, VALID_F01).is_empty());
}

/// H18: identities that no longer match the contract do not resume. Each variant uses a fresh pyramid.
#[tokio::test]
async fn h18_contract_changes_do_not_resume() {
    let pals = load_hrrr_palettes(&["classic".into()]);
    type Mutation = Box<dyn FnOnce(&mut PyramidIdentity)>;
    let variants: Vec<(&str, Mutation)> = vec![
        (
            "zoom/tile spec",
            Box::new(|id: &mut PyramidIdentity| id.tile_spec_digest = "0".repeat(64)),
        ),
        (
            "algorithm",
            Box::new(|id: &mut PyramidIdentity| {
                id.renderer = "indexed".into();
                id.algorithm = "future-render-v9".into();
            }),
        ),
        (
            "palette payload",
            Box::new(|id: &mut PyramidIdentity| {
                id.renderer = "indexed".into();
                id.palette_digest = "1".repeat(64);
            }),
        ),
        (
            "semantics",
            Box::new(|id: &mut PyramidIdentity| id.semantic_digest = "2".repeat(64)),
        ),
    ];
    for (what, edit) in variants {
        let h = radar_harness(&["classic"]).await;
        restamp(&layer_dir(&h, "radar-hrrr", "classic", VALID_F01), edit);
        assert!(
            existing_rendered_layers(&h.ctx.cfg, &pals, RUN, VALID_F01).is_empty(),
            "{what} changed"
        );
    }
}
