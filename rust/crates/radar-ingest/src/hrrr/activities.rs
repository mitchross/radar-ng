//! HRRR activity bodies (spec §12.2): find the latest run, horizon, process one forecast hour, publish,
//! mark processed, cleanup.

use std::path::PathBuf;
use std::time::Instant;

use chrono::{DateTime, Duration as ChronoDuration, Utc};
use radar_core::manifest::read_manifest;
use radar_grib::hrrr::{self, Fetched};
use radar_types::payloads::hrrr::{FindRunResult, ForecastHourResult, HrrrCleanupResult};

use super::render::{RENDER_BEAT, existing_rendered_layers, load_hrrr_palettes, process_hour_sync};
use super::state::{needed_vars, resume_incomplete_run};
use super::{
    HRRR_BEAT, HRRR_PROCESSED_FILE, HRRR_PROCESSED_MAX, horizon_for, run_init, valid_time,
};
use crate::common::{self, child_dirs, elapsed_s, processed_set, remove_timestamp_dir};
use crate::ctx::IngestCtx;
use crate::heartbeat::{self, Heartbeat};
use crate::scratch::{ActivityInfo, ScratchDir};

/// `hrrr_find_latest_run` (spec §12.2). Resumes an incomplete run younger than 3 h, unless no f01 is
/// found at all (D8 replicated).
pub async fn find_latest_run(ctx: &IngestCtx, now: DateTime<Utc>) -> anyhow::Result<FindRunResult> {
    let Some(latest) = hrrr::find_latest_run(&ctx.hrrr_s3, now).await? else {
        return Ok(FindRunResult {
            run_id: None,
            already_processed: false,
        });
    };
    // The manifest read and the processed-set read are file I/O (F5): both run on the blocking pool.
    let cfg = ctx.cfg.clone();
    let (run, already_processed) = common::offload(move || -> anyhow::Result<(String, bool)> {
        let manifest = read_manifest(&cfg.paths.state_dir)?;
        let run = match resume_incomplete_run(&manifest, now) {
            Some(resume) if resume != latest => {
                tracing::info!(run_id = %resume, latest = %latest, "resuming_incomplete_run");
                resume
            }
            _ => latest,
        };
        let processed = processed_set(&cfg, HRRR_PROCESSED_FILE, HRRR_PROCESSED_MAX)?;
        let already_processed = processed.contains(&run);
        Ok((run, already_processed))
    })
    .await??;
    Ok(FindRunResult {
        run_id: Some(run),
        already_processed,
    })
}

/// `hrrr_horizon_for_run` (spec §12.2).
pub fn horizon_for_run(ctx: &IngestCtx, run_id: &str) -> anyhow::Result<i64> {
    Ok(horizon_for(
        run_id,
        ctx.cfg.forecast_hours,
        ctx.cfg.extended_forecast_hours,
    )? as i64)
}

fn pending_result(fhr: i64, started: Instant, valid: String) -> ForecastHourResult {
    ForecastHourResult {
        fhr,
        rendered_layers: Vec::new(),
        duration_s: elapsed_s(started),
        valid_timestamp: valid,
    }
}

/// `hrrr_process_forecast_hour` (spec §12.2). Resumes when every enabled layer is already complete,
/// downloads the needed bytes otherwise, and renders. A missing hour returns an empty result (pending).
pub async fn process_forecast_hour(
    ctx: &IngestCtx,
    info: &ActivityInfo,
    run_id: &str,
    fhr: i64,
    hb: &Heartbeat,
) -> anyhow::Result<ForecastHourResult> {
    let started = Instant::now();
    let cfg = ctx.cfg.clone();
    let init = run_init(run_id)?;
    let fhr_u32 = u32::try_from(fhr).map_err(|_| anyhow::anyhow!("fhr out of range: {fhr}"))?;
    let valid = valid_time(init, fhr_u32);

    let palettes = load_hrrr_palettes(&cfg.palettes);
    // Checks each tile pyramid on disk (F5): blocking pool, not a runtime worker.
    let existing = {
        let (check_cfg, check_palettes) = (cfg.clone(), palettes.clone());
        let (check_run, check_valid) = (run_id.to_string(), valid.clone());
        common::offload(move || {
            let mut layers =
                existing_rendered_layers(&check_cfg, &check_palettes, &check_run, &check_valid);
            // A layer rendered before its packed slot was written is rendered (and packed) again.
            if check_cfg.packed_tiles {
                layers.retain(|l| {
                    super::packed::has_slot(&check_cfg.paths, l, &check_run, &check_valid)
                });
            }
            layers
        })
        .await?
    };
    let required: Vec<String> = cfg
        .hrrr_enabled_layers
        .iter()
        .filter(|l| {
            super::state::layer_color_key(l)
                .is_some_and(|k| palettes.iter().any(|p| p.color_keys().any(|c| c == k)))
        })
        .cloned()
        .collect();
    if required.iter().all(|r| existing.contains(r)) {
        tracing::info!(run_id = %run_id, fhr, rendered = ?existing, "hour_resumed");
        return Ok(ForecastHourResult {
            fhr,
            rendered_layers: existing,
            duration_s: elapsed_s(started),
            valid_timestamp: valid,
        });
    }

    let needed = needed_vars(&cfg.hrrr_enabled_layers);
    let scratch = {
        let root = cfg.hrrr_tmp_root.clone();
        let info = info.clone();
        let (scratch_run, scratch_part) = (run_id.to_string(), format!("f{fhr:02}"));
        common::offload(move || {
            ScratchDir::create(
                &root,
                "hrrr",
                &info,
                &[scratch_run.as_str(), scratch_part.as_str()],
            )
        })
        .await??
    };
    let details = serde_json::json!({"phase": "download", "fhr": fhr, "run_id": run_id});
    hb(details.clone());
    let fetched = heartbeat::during(
        HRRR_BEAT,
        hb,
        details.clone(),
        hrrr::fetch_hour(&ctx.hrrr_s3, init, fhr_u32, &needed),
    )
    .await?;
    let (bytes, full_file) = match fetched {
        Fetched::Pending => {
            tracing::info!(run_id = %run_id, fhr, "hour_pending");
            return Ok(pending_result(fhr, started, valid));
        }
        Fetched::NoMatchingFields => return Ok(pending_result(fhr, started, valid)),
        Fetched::Data { bytes, full_file } => (bytes, full_file),
    };
    let name = if full_file {
        format!("hrrr_f{fhr:02}.grib2")
    } else {
        format!("hrrr_f{fhr:02}_subset.grib2")
    };
    let grib: PathBuf = scratch.path().join(name);
    let write_path = grib.clone();
    heartbeat::blocking(HRRR_BEAT, hb, details.clone(), move || {
        std::fs::write(&write_path, &bytes)
    })
    .await?;

    hb(serde_json::json!({"phase": "render", "fhr": fhr, "run_id": run_id}));
    let render_cfg = cfg.clone();
    let render_run = run_id.to_string();
    let render_valid = valid.clone();
    let render_palettes = palettes.clone();
    let rendered = heartbeat::blocking(
        RENDER_BEAT,
        hb,
        serde_json::json!({"phase": "render", "fhr": fhr, "run_id": run_id}),
        move || {
            process_hour_sync(
                &render_cfg,
                &grib,
                &render_run,
                &render_valid,
                &render_palettes,
            )
        },
    )
    .await?;
    scratch.remove().await;
    let duration_s = elapsed_s(started);
    tracing::info!(run_id = %run_id, fhr, rendered = ?rendered, duration_s, "hour_done");
    Ok(ForecastHourResult {
        fhr,
        rendered_layers: rendered,
        duration_s,
        valid_timestamp: valid,
    })
}

/// `hrrr_publish_run` (spec §12.2): publishes the coherent prefix to the manifest. Returns the layers.
pub async fn publish_run(
    ctx: &IngestCtx,
    run_id: &str,
    results: &[ForecastHourResult],
) -> anyhow::Result<Vec<String>> {
    let cfg = ctx.cfg.clone();
    let run = run_id.to_string();
    let results = results.to_vec();
    tokio::task::spawn_blocking(move || {
        let palettes = load_hrrr_palettes(&cfg.palettes);
        let horizon = horizon_for(&run, cfg.forecast_hours, cfg.extended_forecast_hours)?;
        let packed = cfg.packed_tiles.then_some(&cfg.paths);
        super::publish::publish_run(
            &cfg.paths.state_dir,
            packed,
            &run,
            &results,
            &palettes,
            horizon,
        )
    })
    .await?
}

/// `hrrr_mark_processed` (spec §12.2).
pub async fn mark_processed(ctx: &IngestCtx, run_id: &str) -> anyhow::Result<()> {
    let cfg = ctx.cfg.clone();
    let run = run_id.to_string();
    common::offload(move || -> anyhow::Result<()> {
        let mut set = processed_set(&cfg, HRRR_PROCESSED_FILE, HRRR_PROCESSED_MAX)?;
        set.add(&run)?;
        Ok(())
    })
    .await?
}

/// `hrrr_cleanup` (spec §12.2). Removes run directories not referenced by the manifest and older than the
/// retention window, plus legacy timestamp directories, then sweeps grids.
pub async fn cleanup(ctx: &IngestCtx, retention_hours: i64) -> anyhow::Result<HrrrCleanupResult> {
    let cfg = ctx.cfg.clone();
    let cutoff = Utc::now() - ChronoDuration::hours(retention_hours);
    tokio::task::spawn_blocking(move || -> anyhow::Result<HrrrCleanupResult> {
        let manifest = read_manifest(&cfg.paths.state_dir)?;
        let mut tile_dirs_removed = 0u64;
        for layer in crate::config::HRRR_TILE_LAYERS {
            let live = super::state::referenced_runs(manifest.layers.get(layer));
            tile_dirs_removed +=
                super::packed::cleanup_run_buckets(&cfg.paths, layer, &live, cutoff)?;
            let layer_dir = cfg.paths.tile_layer_dir(layer);
            if !layer_dir.exists() {
                continue;
            }
            let mut legacy: Vec<PathBuf> = Vec::new();
            for palette_dir in child_dirs(&layer_dir, |_| true)? {
                let name = palette_dir
                    .file_name()
                    .and_then(|n| n.to_str())
                    .unwrap_or_default()
                    .to_string();
                if name.starts_with(|c: char| c.is_ascii_digit()) {
                    legacy.push(palette_dir);
                    continue;
                }
                for run_dir in child_dirs(&palette_dir.join("runs"), |_| true).unwrap_or_default() {
                    let run = run_dir
                        .file_name()
                        .and_then(|n| n.to_str())
                        .unwrap_or_default()
                        .to_string();
                    if live.contains(&run) {
                        continue;
                    }
                    let Some(init) = hrrr::parse_run_id(&run) else {
                        continue;
                    };
                    if init < cutoff {
                        std::fs::remove_dir_all(&run_dir)?;
                        tile_dirs_removed += 1;
                    }
                }
                for child in child_dirs(&palette_dir, |n| n != "runs")? {
                    legacy.push(child);
                }
            }
            for dir in legacy {
                let name = dir
                    .file_name()
                    .and_then(|n| n.to_str())
                    .unwrap_or_default()
                    .to_string();
                tile_dirs_removed +=
                    remove_timestamp_dir(&cfg.paths.state_dir, layer, &dir, &name, cutoff)?;
            }
        }
        let grid_files_removed = common::cleanup_grids(&cfg)?;
        Ok(HrrrCleanupResult {
            tile_dirs_removed: tile_dirs_removed as i64,
            grid_files_removed: grid_files_removed as i64,
        })
    })
    .await?
}
