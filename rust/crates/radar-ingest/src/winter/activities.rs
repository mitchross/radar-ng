//! Winter activity bodies: find the latest run, process one forecast hour, publish, mark processed,
//! cleanup. Rendering, the manifest and grid dumps are shared with HRRR.
//!
//! A frame at hour F is the sum of the 6 hour records ending at 6..F. Each record is fetched once per
//! run and cached under `HRRR_TMP_ROOT/nbm/<run>`; a worker without the cache fetches it again.

use std::path::{Path, PathBuf};
use std::time::Instant;

use bytes::Bytes;
use chrono::{DateTime, Duration as ChronoDuration, Utc};
use radar_core::fsutil;
use radar_core::manifest::{ReplaceOptions, read_manifest, replace_layer};
use radar_geo::{Crs, Grid, GridGeo};
use radar_grib::decode::find_message;
use radar_grib::hrrr::parse_run_id;
use radar_grib::mrms::ensure_south_up;
use radar_grib::nbm::{self, NbmVar};
use radar_tiler::identity::FrameKind;
use radar_tiler::palettes::Palette;
use radar_tiler::{
    ColorTable, FrameOptions, complete_pyramid_identity, frame_identity_is_compatible,
    render_frame_palettes,
};
use radar_types::payloads::hrrr::{ForecastHourResult, HrrrCleanupResult};
use radar_types::payloads::winter::WinterFindResult;

use super::state;
use super::{Field, PROCESSED_MAX, Product, WinterLayer, ZOOMS};
use crate::common::{self, child_dirs, elapsed_s, processed_set};
use crate::config::IngestConfig;
use crate::ctx::IngestCtx;
use crate::heartbeat::{self, Heartbeat};
use crate::hrrr::dump::{Resampler, safe_dump, target_axes};
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
) -> anyhow::Result<WinterFindResult> {
    let hours = state::forecast_hours(product);
    let (true, Some(&probe)) = (state::enabled(product, &ctx.cfg), hours.last()) else {
        return Ok(WinterFindResult::default());
    };
    let run = match product {
        Product::NbmSnow => nbm::find_latest_run(&ctx.nbm_s3, now, probe).await?,
    };
    let Some(run) = run else {
        return Ok(WinterFindResult::default());
    };
    let cfg = ctx.cfg.clone();
    let check = run.clone();
    let already_processed = common::offload(move || -> anyhow::Result<bool> {
        Ok(processed_set(&cfg, product.processed_file(), PROCESSED_MAX)?.contains(&check))
    })
    .await??;
    Ok(WinterFindResult {
        run_id: Some(run),
        already_processed,
        hours: hours.into_iter().map(i64::from).collect(),
    })
}

fn source_id(product: Product, layer: &str, rel: &str) -> String {
    format!("{}:{layer}:{rel}", product.source())
}

fn rel_path(run_id: &str, valid: &str) -> String {
    format!("runs/{run_id}/{valid}")
}

/// Whether every palette with the layer's key already has a complete pyramid of this renderer.
pub fn already_rendered(
    cfg: &IngestConfig,
    product: Product,
    layer: &WinterLayer,
    palettes: &[Palette],
    run_id: &str,
    valid: &str,
) -> bool {
    let tables = tables_for_key(palettes, layer.color_key);
    let rel = rel_path(run_id, valid);
    let png_level = FrameOptions::default().png_compress_level;
    !tables.is_empty()
        && tables.iter().all(|(name, _)| {
            let path = cfg.paths.tile_layer_dir(layer.id).join(name).join(&rel);
            complete_pyramid_identity(&path).is_some_and(|stored| {
                frame_identity_is_compatible(
                    &stored,
                    &source_id(product, layer.id, &rel),
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

/// Render one field for every palette with the layer's key. True when all of them published.
pub fn render(
    cfg: &IngestConfig,
    product: Product,
    layer: &WinterLayer,
    run_id: &str,
    valid: &str,
    grid: &Grid,
    tables: &[(String, ColorTable)],
) -> anyhow::Result<bool> {
    if tables.is_empty() {
        return Ok(false);
    }
    let rel = rel_path(run_id, valid);
    let tile_layer = cfg.paths.tile_layer_dir(layer.id);
    let dirs: Vec<(String, PathBuf)> = tables
        .iter()
        .map(|(n, _)| (n.clone(), tile_layer.join(n).join(&rel)))
        .collect();
    let options = FrameOptions {
        source_id: source_id(product, layer.id, &rel),
        publication_lock_root: Some(tile_layer.clone()),
        overview: None,
        // A dry run has no tiles but is still a frame of the timeline.
        keep_empty: true,
        ..FrameOptions::default()
    };
    let result = render_frame_palettes(grid, &KIND, tables, &dirs, &ZOOMS, &options)?;
    Ok(result.outcomes.len() == tables.len())
}

/// Sum NBM records (GRIB bytes, in hour order) into one grid in inches.
pub fn nbm_total(var: NbmVar, records: &[Bytes]) -> anyhow::Result<Grid> {
    let mut total: Option<Grid> = None;
    for bytes in records {
        let mut msg = find_message(bytes, &var.selector())?
            .ok_or_else(|| anyhow::anyhow!("{} message missing from GRIB", var.idx_name()))?;
        ensure_south_up(&mut msg.grid);
        for v in &mut msg.grid.values {
            *v = var.to_inches(*v);
        }
        match &mut total {
            None => total = Some(msg.grid),
            Some(t) => state::accumulate(&mut t.values, &msg.grid.values)?,
        }
    }
    total.ok_or_else(|| anyhow::anyhow!("no {} records", var.idx_name()))
}

/// Render and dump every layer of one hour from its summed grids. Returns the layers rendered.
pub fn render_hour_sync(
    cfg: &IngestConfig,
    product: Product,
    run_id: &str,
    valid: &str,
    grids: &[(&'static WinterLayer, Grid)],
    palettes: &[Palette],
) -> anyhow::Result<Vec<String>> {
    let mut rendered = Vec::new();
    let mut resampler: Option<(Resampler, Vec<f64>, Vec<f64>)> = None;
    for (layer, grid) in grids {
        let tables = tables_for_key(palettes, layer.color_key);
        if !render(cfg, product, layer, run_id, valid, grid, &tables)? {
            continue;
        }
        if resampler.is_none() {
            let GridGeo::Projected { crs, x, y, .. } = &grid.geo else {
                anyhow::bail!("NBM field has no projected geometry");
            };
            let crs: &Crs = crs;
            let (lats, lons) = target_axes(grid, cfg.hrrr_grid_dump_step_deg)?;
            let r = Resampler::new(crs, x, y, grid.height, grid.width, &lats, &lons)?;
            resampler = Some((r, lats, lons));
        }
        if let Some((r, lats, lons)) = &resampler {
            let data = r.resample(&grid.values);
            safe_dump(cfg, layer.id, valid, &data, lats, lons, layer.unit);
        }
        rendered.push(layer.id.to_string());
    }
    Ok(rendered)
}

fn cache_dir(cfg: &IngestConfig, run_id: &str) -> PathBuf {
    cfg.hrrr_tmp_root.join("nbm").join(run_id)
}

fn cache_path(dir: &Path, var: NbmVar, fhr: u32) -> PathBuf {
    dir.join(format!("{}_f{fhr:03}.grib2", var.idx_name()))
}

/// The 6 hour records of every var for each hour in `hours`, from the cache or NBM. `None` when an
/// hour is not posted yet.
async fn records(
    ctx: &IngestCtx,
    run_id: &str,
    init: DateTime<Utc>,
    vars: &[NbmVar],
    hours: &[u32],
) -> anyhow::Result<Option<Vec<Vec<Bytes>>>> {
    let dir = cache_dir(&ctx.cfg, run_id);
    let mut out: Vec<Vec<Bytes>> = vec![Vec::with_capacity(hours.len()); vars.len()];
    for &h in hours {
        let cached = {
            let dir = dir.clone();
            let vars = vars.to_vec();
            common::offload(move || {
                vars.iter()
                    .map(|&v| std::fs::read(cache_path(&dir, v, h)).ok().map(Bytes::from))
                    .collect::<Option<Vec<Bytes>>>()
            })
            .await?
        };
        let got = match cached {
            Some(got) => got,
            None => {
                let Some(fetched) = nbm::fetch_hour(&ctx.nbm_s3, init, h, vars).await? else {
                    return Ok(None);
                };
                if fetched.len() != vars.len() {
                    anyhow::bail!("NBM {run_id} f{h:03} lacks a 6 hour record");
                }
                let (dir, store) = (dir.clone(), fetched.clone());
                if let Err(err) = common::offload(move || -> anyhow::Result<()> {
                    std::fs::create_dir_all(&dir)?;
                    for (v, b) in &store {
                        fsutil::atomic_write(&cache_path(&dir, *v, h), b, ".nbm-", ".tmp")?;
                    }
                    Ok(())
                })
                .await?
                {
                    tracing::warn!(run_id, fhr = h, err = %format!("{err:#}"), "nbm_cache_write_failed");
                }
                fetched.into_iter().map(|(_, b)| b).collect()
            }
        };
        for (slot, b) in out.iter_mut().zip(got) {
            slot.push(b);
        }
    }
    Ok(Some(out))
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
        common::offload(move || {
            product
                .layers()
                .iter()
                .all(|l| already_rendered(&cfg, product, l, &palettes, &run, &valid))
        })
        .await?
    };
    let all_layers = || product.layers().iter().map(|l| l.id.to_string()).collect();
    if done {
        return Ok(result(all_layers(), valid));
    }

    let details = serde_json::json!({"phase": "download", "product": product.name(), "fhr": fhr, "run_id": run_id});
    hb(details.clone());
    let vars = product.nbm_vars();
    let hours = state::record_hours(fhr_u32);
    let fetch = records(ctx, run_id, init, &vars, &hours);
    let Some(records) = heartbeat::during(HRRR_BEAT, hb, details, fetch).await? else {
        tracing::info!(product = product.name(), run_id = %run_id, fhr, "hour_pending");
        return Ok(result(Vec::new(), valid));
    };

    let details = serde_json::json!({"phase": "render", "product": product.name(), "fhr": fhr, "run_id": run_id});
    let cfg = ctx.cfg.clone();
    let (run, render_valid) = (run_id.to_string(), valid.clone());
    let rendered = heartbeat::blocking(HRRR_BEAT, hb, details, move || {
        let mut grids = Vec::new();
        for layer in product.layers() {
            let Field::NbmTotal(var) = layer.field;
            let i = vars.iter().position(|v| *v == var).unwrap_or_default();
            grids.push((layer, nbm_total(var, &records[i])?));
        }
        render_hour_sync(&cfg, product, &run, &render_valid, &grids, &palettes)
    })
    .await?;
    let out = result(rendered, valid);
    tracing::info!(product = product.name(), run_id = %run_id, fhr, rendered = ?out.rendered_layers, duration_s = out.duration_s, "hour_done");
    Ok(out)
}

/// Publish the coherent prefix of a run for every layer. Returns the number of hours published.
pub fn publish_run_sync(
    cfg: &IngestConfig,
    product: Product,
    run_id: &str,
    results: &[ForecastHourResult],
) -> anyhow::Result<i64> {
    let hours = state::forecast_hours(product);
    let prefix = state::publishable_prefix(results, &hours, product);
    if prefix.is_empty() {
        return Ok(0);
    }
    let palettes = load_hrrr_palettes(&cfg.palettes);
    let frames = state::frames(product, run_id, run_init(run_id)?, &prefix);
    let timestamps: Vec<String> = frames.iter().map(|f| f.timestamp.clone()).collect();
    for layer in product.layers() {
        let mut palette_names: Vec<String> = palettes
            .iter()
            .filter(|p| p.color_keys().any(|k| k == layer.color_key))
            .map(|p| p.name().to_string())
            .collect();
        palette_names.sort();
        if palette_names.is_empty() {
            return Ok(0);
        }
        let metadata = serde_json::json!({
            "title": layer.title,
            "kind": "model_guidance",
            "palette_key": layer.color_key,
            "unit": layer.unit,
            "run_id": run_id,
            "complete": prefix.len() == hours.len(),
        });
        let options = ReplaceOptions {
            palettes: palette_names,
            frames: frames.clone(),
            layer_metadata: metadata.as_object().cloned(),
        };
        replace_layer(&cfg.paths.state_dir, layer.id, &timestamps, &options)?;
    }
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

fn expired_run(name: &str, cutoff: DateTime<Utc>) -> bool {
    parse_run_id(name).is_some_and(|init| init < cutoff)
}

/// Remove run directories the manifest no longer references and that are older than the retention,
/// and record caches of runs past it.
pub fn cleanup_sync(
    cfg: &IngestConfig,
    product: Product,
    cutoff: DateTime<Utc>,
) -> anyhow::Result<HrrrCleanupResult> {
    let manifest = read_manifest(&cfg.paths.state_dir)?;
    let mut removed = 0i64;
    for layer in product.layers() {
        let live = crate::hrrr::state::referenced_runs(manifest.layers.get(layer.id));
        let layer_dir = cfg.paths.tile_layer_dir(layer.id);
        if !layer_dir.exists() {
            continue;
        }
        for palette_dir in child_dirs(&layer_dir, |_| true)? {
            for run_dir in child_dirs(&palette_dir.join("runs"), |_| true).unwrap_or_default() {
                let name = run_dir
                    .file_name()
                    .and_then(|n| n.to_str())
                    .unwrap_or_default();
                if !live.contains(name) && expired_run(name, cutoff) {
                    std::fs::remove_dir_all(&run_dir)?;
                    removed += 1;
                }
            }
        }
    }
    let cache = cfg.hrrr_tmp_root.join("nbm");
    if cache.exists() {
        for run_dir in child_dirs(&cache, |n| expired_run(n, cutoff))? {
            std::fs::remove_dir_all(&run_dir)?;
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

#[cfg(test)]
mod tests {
    use super::*;
    use radar_core::env::Paths;

    fn fixture(name: &str) -> Bytes {
        let p = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../radar-grib/testdata")
            .join(name);
        Bytes::from(std::fs::read(p).unwrap())
    }

    /// Full live run against NBM, printing tile bytes per layer:
    /// `cargo test --release -p radar-ingest --lib live_nbm_run -- --ignored --nocapture`.
    #[tokio::test(flavor = "multi_thread")]
    #[ignore = "network, renders a whole run"]
    async fn live_nbm_run() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        let paths = Paths::new(root.join("state"), root.join("grids"), root.join("tiles"));
        let cfg = IngestConfig::with_roots(paths, root.join("tmp")).unwrap();
        let ctx = IngestCtx::new(cfg.clone(), None).unwrap();
        let product = Product::NbmSnow;
        let find = find_latest_run(&ctx, product, Utc::now()).await.unwrap();
        let run = find.run_id.expect("a posted run");
        let hb = heartbeat::noop();
        let mut results = Vec::new();
        for &fhr in &find.hours {
            let r = process_hour(&ctx, product, &run, fhr, &hb).await.unwrap();
            println!(
                "{run} f{fhr:03}: {:?} in {:.1}s",
                r.rendered_layers, r.duration_s
            );
            results.push(r);
        }
        let published = publish_run(&ctx, product, &run, &results).await.unwrap();
        assert_eq!(published, find.hours.len() as i64);
        for l in product.layers() {
            let du = |p: &Path| -> u64 {
                walk(p)
                    .iter()
                    .map(|f| f.metadata().map(|m| m.len()).unwrap_or(0))
                    .sum()
            };
            println!(
                "{}: tiles {} bytes, grids {} bytes",
                l.id,
                du(&cfg.paths.tile_layer_dir(l.id)),
                du(&cfg.grid.grid_dir.join(l.id))
            );
        }
        println!(
            "cache {} bytes",
            walk(&cfg.hrrr_tmp_root)
                .iter()
                .map(|f| f.metadata().unwrap().len())
                .sum::<u64>()
        );
    }

    fn walk(p: &Path) -> Vec<PathBuf> {
        let mut out = Vec::new();
        let Ok(rd) = std::fs::read_dir(p) else {
            return out;
        };
        for e in rd.flatten() {
            let path = e.path();
            if path.is_dir() {
                out.extend(walk(&path));
            } else {
                out.push(path);
            }
        }
        out
    }

    #[test]
    fn totals_render_tiles_dumps_and_manifest_from_real_records() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        let paths = Paths::new(root.join("state"), root.join("grids"), root.join("tiles"));
        let cfg = IngestConfig::with_roots(paths, root.join("tmp")).unwrap();
        let palettes = load_hrrr_palettes(&cfg.palettes);

        // Two copies of the same 6 hour record make a 12 hour total of twice the value.
        let snow = fixture("nbm_asnow_f006_utah.grib2");
        let once = nbm_total(NbmVar::Asnow, std::slice::from_ref(&snow)).unwrap();
        let twice = nbm_total(NbmVar::Asnow, &[snow.clone(), snow]).unwrap();
        let k = once
            .values
            .iter()
            .position(|v| *v > 1.0)
            .expect("Utah cut has over an inch of snow");
        assert!((twice.values[k] - 2.0 * once.values[k]).abs() < 1e-4);
        let ice = nbm_total(NbmVar::Ficeac, &[fixture("nbm_ficeac_f006_utah.grib2")]).unwrap();

        let (run, valid) = ("20261117_12", "2026-11-17T18:00:00+00:00");
        let layers = Product::NbmSnow.layers();
        let grids = vec![(&layers[0], twice), (&layers[1], ice)];
        let rendered =
            render_hour_sync(&cfg, Product::NbmSnow, run, valid, &grids, &palettes).unwrap();
        assert_eq!(rendered, ["snowfall-forecast", "ice-forecast"]);
        for l in layers {
            assert!(already_rendered(
                &cfg,
                Product::NbmSnow,
                l,
                &palettes,
                run,
                valid
            ));
            assert!(cfg.grid.grid_dir.join(l.id).is_dir(), "{} dump", l.id);
        }

        let results = [ForecastHourResult {
            fhr: 6,
            rendered_layers: rendered,
            duration_s: 0.0,
            valid_timestamp: valid.into(),
        }];
        assert_eq!(
            publish_run_sync(&cfg, Product::NbmSnow, run, &results).unwrap(),
            1
        );
        let manifest = read_manifest(&cfg.paths.state_dir).unwrap();
        for l in layers {
            assert_eq!(manifest.layers[l.id].frames.len(), 1, "{}", l.id);
        }
    }
}
