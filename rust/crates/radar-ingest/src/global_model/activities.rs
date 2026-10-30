//! Global-model activity bodies: find the latest run, process one forecast hour, publish, mark processed,
//! cleanup. The heavy lifting (palettes, tile pyramids, manifest, grid dumps) is shared with HRRR.

use std::path::PathBuf;
use std::time::Instant;

use chrono::{DateTime, Duration as ChronoDuration, Utc};
use radar_core::grid_dump::{self, GridInput};
use radar_core::manifest::{ReplaceOptions, read_manifest, replace_layer};
use radar_geo::Grid;
use radar_grib::decode::find_message;
use radar_grib::hrrr::{Fetched, parse_run_id};
use radar_grib::mrms::ensure_south_up;
use radar_tiler::identity::FrameKind;
use radar_tiler::palettes::Palette;
use radar_tiler::{
    ColorTable, FrameOptions, complete_pyramid_identity, frame_identity_is_compatible,
    render_frame_palettes,
};
use radar_types::payloads::global_model::GlobalModelFindResult;
use radar_types::payloads::hrrr::{ForecastHourResult, HrrrCleanupResult};

use super::state::{self, CONUS_BBOX};
use super::{PROCESSED_MAX, Product, ZOOMS};
use crate::common::{self, child_dirs, elapsed_s, processed_set};
use crate::config::IngestConfig;
use crate::ctx::IngestCtx;
use crate::heartbeat::{self, Heartbeat};
use crate::hrrr::render::{load_hrrr_palettes, tables_for_key};
use crate::hrrr::{HRRR_BEAT, run_init, valid_time};

const KIND: FrameKind = FrameKind::Continuous {
    nodata_value: None,
    min_valid_weight: 1.0,
};

/// Newest run with every planned hour posted, or none when the product is off.
pub async fn find_latest_run(
    ctx: &IngestCtx,
    product: Product,
    now: DateTime<Utc>,
) -> anyhow::Result<GlobalModelFindResult> {
    let hours = state::forecast_hours(product, &ctx.cfg);
    let (true, Some(&probe)) = (state::enabled(product, &ctx.cfg), hours.last()) else {
        return Ok(GlobalModelFindResult::default());
    };
    let run = match product {
        Product::GfsRadar => radar_grib::gfs::find_latest_run(&ctx.gfs_s3, now, probe).await?,
        Product::Uv => radar_grib::uvi::find_latest_run(&ctx.uv_http, now, probe).await?,
    };
    let Some(run) = run else {
        return Ok(GlobalModelFindResult::default());
    };
    let cfg = ctx.cfg.clone();
    let check = run.clone();
    let already_processed = common::offload(move || -> anyhow::Result<bool> {
        Ok(processed_set(&cfg, product.processed_file(), PROCESSED_MAX)?.contains(&check))
    })
    .await??;
    Ok(GlobalModelFindResult {
        run_id: Some(run),
        already_processed,
        hours: hours.into_iter().map(i64::from).collect(),
    })
}

fn source_id(product: Product, rel: &str) -> String {
    format!("{}:{}:{rel}", product.source(), product.layer())
}

fn rel_path(run_id: &str, valid: &str) -> String {
    format!("runs/{run_id}/{valid}")
}

/// Whether every palette with the layer's key already has a complete pyramid of this renderer.
pub fn already_rendered(
    cfg: &IngestConfig,
    product: Product,
    palettes: &[Palette],
    run_id: &str,
    valid: &str,
) -> bool {
    let tables = tables_for_key(palettes, product.color_key());
    let rel = rel_path(run_id, valid);
    let png_level = FrameOptions::default().png_compress_level;
    !tables.is_empty()
        && tables.iter().all(|(name, _)| {
            let path = cfg
                .paths
                .tile_layer_dir(product.layer())
                .join(name)
                .join(&rel);
            complete_pyramid_identity(&path).is_some_and(|stored| {
                frame_identity_is_compatible(
                    &stored,
                    &source_id(product, &rel),
                    &tables,
                    name,
                    &ZOOMS,
                    256,
                    &KIND,
                    png_level,
                    None,
                )
                .unwrap_or(false)
            })
        })
}

/// Render one cropped field for every palette with the layer's key. True when all of them published.
pub fn render(
    cfg: &IngestConfig,
    product: Product,
    run_id: &str,
    valid: &str,
    grid: &Grid,
    tables: &[(String, ColorTable)],
) -> anyhow::Result<bool> {
    if tables.is_empty() {
        return Ok(false);
    }
    let rel = rel_path(run_id, valid);
    let tile_layer = cfg.paths.tile_layer_dir(product.layer());
    let dirs: Vec<(String, PathBuf)> = tables
        .iter()
        .map(|(n, _)| (n.clone(), tile_layer.join(n).join(&rel)))
        .collect();
    let options = FrameOptions {
        source_id: source_id(product, &rel),
        publication_lock_root: Some(tile_layer.clone()),
        overview: None,
        // UV at night (and a dry GFS hour) has no tiles but is still a frame of the timeline.
        keep_empty: true,
        ..FrameOptions::default()
    };
    let result = render_frame_palettes(grid, &KIND, tables, &dirs, &ZOOMS, &options)?;
    Ok(result.outcomes.len() == tables.len())
}

/// Decode, convert, crop, render and dump one hour's GRIB bytes. Returns the layers rendered.
pub fn process_bytes_sync(
    cfg: &IngestConfig,
    product: Product,
    bytes: &[u8],
    run_id: &str,
    valid: &str,
    palettes: &[Palette],
) -> anyhow::Result<Vec<String>> {
    let mut msg = find_message(bytes, &product.selector())?
        .ok_or_else(|| anyhow::anyhow!("{} message missing from GRIB", product.layer()))?;
    ensure_south_up(&mut msg.grid);
    for v in &mut msg.grid.values {
        *v = product.convert(*v);
    }
    let grid = state::crop(&msg.grid, CONUS_BBOX)?;
    let tables = tables_for_key(palettes, product.color_key());
    if !render(cfg, product, run_id, valid, &grid, &tables)? {
        return Ok(Vec::new());
    }
    let (lats, lons) = common::regular_axes(&grid)?;
    let input = GridInput::new(&grid.values, lats, lons, product.unit());
    if let Err(err) = grid_dump::write_grid(&cfg.grid, product.layer(), valid, &input) {
        tracing::warn!(err = %err, layer = product.layer(), "grid_dump_failed");
    }
    Ok(vec![product.layer().to_string()])
}

/// Render one forecast hour. An hour not posted yet returns no layers (the run stays incomplete).
pub async fn process_hour(
    ctx: &IngestCtx,
    product: Product,
    run_id: &str,
    fhr: i64,
    hb: &Heartbeat,
) -> anyhow::Result<ForecastHourResult> {
    let started = Instant::now();
    let init = run_init(run_id)?;
    let fhr_u32 = u32::try_from(fhr).map_err(|_| anyhow::anyhow!("fhr out of range: {fhr}"))?;
    let valid = valid_time(init, fhr_u32);
    let result = |layers: Vec<String>, valid: String| ForecastHourResult {
        fhr,
        rendered_layers: layers,
        duration_s: elapsed_s(started),
        valid_timestamp: valid,
    };
    let palettes = load_hrrr_palettes(&ctx.cfg.palettes);
    let done = {
        let (cfg, palettes) = (ctx.cfg.clone(), palettes.clone());
        let (run, valid) = (run_id.to_string(), valid.clone());
        common::offload(move || already_rendered(&cfg, product, &palettes, &run, &valid)).await?
    };
    if done {
        return Ok(result(vec![product.layer().to_string()], valid));
    }

    let details = serde_json::json!({"phase": "download", "layer": product.layer(), "fhr": fhr, "run_id": run_id});
    hb(details.clone());
    let fetch = async {
        match product {
            Product::GfsRadar => radar_grib::gfs::fetch_refc(&ctx.gfs_s3, init, fhr_u32).await,
            Product::Uv => radar_grib::uvi::fetch_hour(&ctx.uv_http, init, fhr_u32).await,
        }
    };
    let bytes = match heartbeat::during(HRRR_BEAT, hb, details, fetch).await? {
        Fetched::Data { bytes, .. } => bytes,
        Fetched::Pending | Fetched::NoMatchingFields => {
            tracing::info!(layer = product.layer(), run_id = %run_id, fhr, "hour_pending");
            return Ok(result(Vec::new(), valid));
        }
    };

    let details = serde_json::json!({"phase": "render", "layer": product.layer(), "fhr": fhr, "run_id": run_id});
    let cfg = ctx.cfg.clone();
    let (run, render_valid) = (run_id.to_string(), valid.clone());
    let rendered = heartbeat::blocking(HRRR_BEAT, hb, details, move || {
        process_bytes_sync(&cfg, product, &bytes, &run, &render_valid, &palettes)
    })
    .await?;
    let out = result(rendered, valid);
    tracing::info!(layer = product.layer(), run_id = %run_id, fhr, rendered = ?out.rendered_layers, duration_s = out.duration_s, "hour_done");
    Ok(out)
}

/// Publish the coherent prefix of a run. Returns the number of frames published.
pub fn publish_run_sync(
    cfg: &IngestConfig,
    product: Product,
    run_id: &str,
    results: &[ForecastHourResult],
) -> anyhow::Result<i64> {
    let hours = state::forecast_hours(product, cfg);
    let prefix = state::publishable_prefix(results, &hours, product.layer());
    let mut palette_names: Vec<String> = load_hrrr_palettes(&cfg.palettes)
        .iter()
        .filter(|p| p.color_keys().any(|k| k == product.color_key()))
        .map(|p| p.name().to_string())
        .collect();
    palette_names.sort();
    if prefix.is_empty() || palette_names.is_empty() {
        return Ok(0);
    }
    let frames = state::frames(product, run_id, run_init(run_id)?, &prefix);
    let timestamps: Vec<String> = frames.iter().map(|f| f.timestamp.clone()).collect();
    let metadata = serde_json::json!({
        "title": product.title(),
        "kind": "model_guidance",
        "palette_key": product.color_key(),
        "run_id": run_id,
        "complete": prefix.len() == hours.len(),
    });
    let options = ReplaceOptions {
        palettes: palette_names,
        frames,
        layer_metadata: metadata.as_object().cloned(),
    };
    replace_layer(&cfg.paths.state_dir, product.layer(), &timestamps, &options)?;
    Ok(prefix.len() as i64)
}

/// [`publish_run_sync`] on the blocking pool.
pub async fn publish_run(
    ctx: &IngestCtx,
    product: Product,
    run_id: &str,
    results: &[ForecastHourResult],
) -> anyhow::Result<i64> {
    let cfg = ctx.cfg.clone();
    let (run, results) = (run_id.to_string(), results.to_vec());
    common::offload(move || publish_run_sync(&cfg, product, &run, &results)).await?
}

/// Record a complete run so later fires skip it.
pub async fn mark_processed(ctx: &IngestCtx, product: Product, run_id: &str) -> anyhow::Result<()> {
    let cfg = ctx.cfg.clone();
    let run = run_id.to_string();
    common::offload(move || -> anyhow::Result<()> {
        processed_set(&cfg, product.processed_file(), PROCESSED_MAX)?.add(&run)?;
        Ok(())
    })
    .await?
}

/// Remove run directories the manifest no longer references and that are older than the retention.
pub fn cleanup_sync(
    cfg: &IngestConfig,
    product: Product,
    cutoff: DateTime<Utc>,
) -> anyhow::Result<HrrrCleanupResult> {
    let manifest = read_manifest(&cfg.paths.state_dir)?;
    let live = crate::hrrr::state::referenced_runs(manifest.layers.get(product.layer()));
    let layer_dir = cfg.paths.tile_layer_dir(product.layer());
    let mut removed = 0i64;
    if layer_dir.exists() {
        for palette_dir in child_dirs(&layer_dir, |_| true)? {
            for run_dir in child_dirs(&palette_dir.join("runs"), |_| true).unwrap_or_default() {
                let name = run_dir
                    .file_name()
                    .and_then(|n| n.to_str())
                    .unwrap_or_default();
                if live.contains(name) {
                    continue;
                }
                if parse_run_id(name).is_some_and(|init| init < cutoff) {
                    std::fs::remove_dir_all(&run_dir)?;
                    removed += 1;
                }
            }
        }
    }
    Ok(HrrrCleanupResult {
        tile_dirs_removed: removed,
        grid_files_removed: 0,
    })
}

/// [`cleanup_sync`] with the product's retention window.
pub async fn cleanup(ctx: &IngestCtx, product: Product) -> anyhow::Result<HrrrCleanupResult> {
    let cfg = ctx.cfg.clone();
    let cutoff = Utc::now() - ChronoDuration::hours(product.retention_hours());
    common::offload(move || cleanup_sync(&cfg, product, cutoff)).await?
}
