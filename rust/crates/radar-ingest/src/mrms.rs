//! MRMS activities (spec §11.1, §12.1): list, process one frame, mark processed, cleanup.

use std::collections::BTreeSet;
use std::path::PathBuf;
use std::time::Instant;

use chrono::{Duration as ChronoDuration, Utc};
use radar_core::grid_dump::{self, GridInput};
use radar_core::manifest::{AddOptions, FrameSpec};
use radar_geo::Grid;
use radar_grib::mrms as gmrms;
use radar_storms::Grid as StormGrid;
use radar_tiler::{ColorTable, FrameKind, FrameOptions, Overview};
use radar_types::payloads::mrms::{
    CleanupInput, CleanupResult, ListKeysResult, MarkProcessedInput, ProcessFrameInput,
    ProcessFrameResult,
};

use crate::common::{self, child_dirs, elapsed_s, processed_set, remove_timestamp_dir};
use crate::ctx::{self, IngestCtx};
use crate::heartbeat::{self, Heartbeat};
use crate::scratch::{ActivityInfo, ScratchDir};

/// `ProcessedSet` capacity for MRMS keys.
pub const MRMS_PROCESSED_MAX: usize = 2000;
/// Heartbeat cadence for the MRMS download and decode.
pub const DOWNLOAD_BEAT: std::time::Duration = std::time::Duration::from_secs(20);
/// Heartbeat cadence for the MRMS render and grid work.
pub const RENDER_BEAT: std::time::Duration = std::time::Duration::from_secs(30);
/// Zooms rendered for MRMS (spec §12.1 step 7).
pub const MRMS_ZOOMS: [u32; 4] = [4, 5, 6, 7];
/// Storm layer that writes `storms.json` and the nowcast input.
pub const RADAR_LAYER: &str = "radar";
/// Nowcast input layer name (spec §7.4).
pub const NOWCAST_INPUT_LAYER: &str = "radar-nowcast-input";

/// Palette tables for MRMS (spec §6.1 `_load_palette_tables`): the `reflectivity` key of each name in
/// `PALETTES`, skipping missing names with a `palette_missing` warning. Falls back to `classic` when
/// nothing loads. Duplicate names are dropped (first occurrence wins) because the publisher needs
/// distinct output directories.
pub fn load_mrms_palettes(names: &[String]) -> Vec<(String, ColorTable)> {
    load_reflectivity_tables(names)
}

pub(crate) fn load_reflectivity_tables(names: &[String]) -> Vec<(String, ColorTable)> {
    let mut seen = BTreeSet::new();
    let mut out = Vec::new();
    for name in names {
        if !seen.insert(name.clone()) {
            continue;
        }
        let table = radar_tiler::load_palette(name).and_then(|p| p.color_table("reflectivity"));
        match table {
            Ok(t) => out.push((name.clone(), t)),
            Err(err) => tracing::warn!(palette = %name, err = %err, "palette_missing"),
        }
    }
    if out.is_empty()
        && let Ok(t) =
            radar_tiler::load_palette("classic").and_then(|p| p.color_table("reflectivity"))
    {
        out.push(("classic".to_string(), t));
    }
    out
}

/// `mrms_list_unprocessed_keys` (spec §12.1). Read-only.
pub async fn list_unprocessed_keys(
    ctx: &IngestCtx,
    args: &radar_types::payloads::mrms::IngestMrmsArgs,
) -> anyhow::Result<ListKeysResult> {
    let now = Utc::now();
    let keys = gmrms::list_recent_keys(&ctx.mrms_s3, &args.mrms_prefix, now).await?;
    let state_file = format!("ingest-mrms-{}.json", args.layer_name);
    let cfg = ctx.cfg.clone();
    let new_keys: Vec<String> = common::offload(move || -> anyhow::Result<Vec<String>> {
        let processed = processed_set(&cfg, &state_file, MRMS_PROCESSED_MAX)?;
        Ok(keys
            .into_iter()
            .filter(|k| !processed.contains(k))
            .collect())
    })
    .await??;
    let backlog_total = new_keys.len() as i64;
    let ordered: Vec<String> = new_keys
        .iter()
        .rev()
        .take(ctx.cfg.backlog_per_cycle)
        .cloned()
        .collect();
    Ok(ListKeysResult {
        keys: ordered,
        backlog_total,
    })
}

/// Output of the blocking decode step.
struct Decoded {
    grid: Grid,
}

/// `mrms_process_frame` (spec §12.1). Returns `rendered == false` for incomplete frames (D2 replicated).
pub async fn process_frame(
    ctx: &IngestCtx,
    info: &ActivityInfo,
    input: &ProcessFrameInput,
    hb: &Heartbeat,
) -> anyhow::Result<ProcessFrameResult> {
    let started = Instant::now();
    let cfg = ctx.cfg.clone();
    let layer = input.layer_name.clone();
    let key = input.key.clone();
    let tables = load_mrms_palettes(&cfg.palettes);
    let all_palettes: BTreeSet<String> = tables.iter().map(|(n, _)| n.clone()).collect();

    let hint = gmrms::timestamp_hint(&key).to_string();
    let scratch = {
        let root = cfg.mrms_tmp_root.clone();
        let info = info.clone();
        let parts = [layer.clone(), hint.clone()];
        common::offload(move || {
            ScratchDir::create(
                &root,
                "mrms",
                &info,
                &[parts[0].as_str(), parts[1].as_str()],
            )
        })
        .await??
    };

    hb(serde_json::json!({"phase": "download", "key": key}));
    // Fetch and parse the key first (D21): a malformed key fails before any download.
    let (ts_fetch, gz) = heartbeat::during(
        DOWNLOAD_BEAT,
        hb,
        serde_json::json!({"phase": "download", "key": key}),
        gmrms::fetch_gz(&ctx.mrms_s3, &key),
    )
    .await?;
    let gz_path: PathBuf = scratch.path().join("mrms.grib2.gz");
    let decode_path = gz_path.clone();
    let decoded: Decoded = heartbeat::blocking(
        DOWNLOAD_BEAT,
        hb,
        serde_json::json!({"phase": "download", "key": key}),
        move || -> anyhow::Result<Decoded> {
            std::fs::write(&decode_path, &gz)?;
            let grid = gmrms::decode_gz(&gz)?;
            Ok(Decoded { grid })
        },
    )
    .await?;
    let mut grid = decoded.grid;
    gmrms::ensure_south_up(&mut grid);
    let timestamp = ts_fetch;

    hb(
        serde_json::json!({"phase": "render", "timestamp": timestamp, "palettes": tables.iter().map(|(n, _)| n).collect::<Vec<_>>()}),
    );
    let tile_layer = cfg.paths.tile_layer_dir(&layer);
    let output_dirs: Vec<(String, PathBuf)> = tables
        .iter()
        .map(|(name, _)| (name.clone(), tile_layer.join(name).join(&timestamp)))
        .collect();
    let source_id = format!("mrms:{layer}:{timestamp}");
    let render_tables = tables.clone();
    let render_dirs = output_dirs.clone();
    let render_tile_layer = tile_layer.clone();
    let (grid, rendered) = heartbeat::blocking(
        RENDER_BEAT,
        hb,
        serde_json::json!({"phase": "render", "timestamp": timestamp}),
        move || -> anyhow::Result<(Grid, Vec<String>)> {
            let kind = FrameKind::Continuous {
                nodata_value: None,
                min_valid_weight: 1.0,
            };
            let options = FrameOptions {
                source_id,
                publication_lock_root: Some(render_tile_layer),
                overview: Some(Overview::Max),
                ..FrameOptions::default()
            };
            let res = radar_tiler::render_frame_palettes(
                &grid,
                &kind,
                &render_tables,
                &render_dirs,
                &MRMS_ZOOMS,
                &options,
            )?;
            let rendered: Vec<String> = res
                .rendered_palettes()
                .iter()
                .map(|s| s.to_string())
                .collect();
            Ok((grid, rendered))
        },
    )
    .await?;
    // Tiles are done; the scratch download is no longer needed (Python removes it in `finally`).
    scratch.remove().await;

    let rendered_set: BTreeSet<String> = rendered.iter().cloned().collect();
    if rendered_set != all_palettes {
        tracing::error!(
            layer = %layer,
            timestamp = %timestamp,
            expected_palettes = ?all_palettes,
            rendered_palettes = ?rendered_set,
            "frame_incomplete"
        );
        return Ok(ProcessFrameResult {
            key,
            timestamp,
            rendered: false,
            palettes: rendered,
            duration_s: elapsed_s(started),
        });
    }

    // Packed buckets are best effort: a failure leaves the frame on the PNG path only.
    let (grid, packed) = if cfg.packed_tiles {
        let paths = cfg.paths.clone();
        let layer = layer.clone();
        let timestamp = timestamp.clone();
        heartbeat::blocking(
            RENDER_BEAT,
            hb,
            serde_json::json!({"phase": "packed", "timestamp": timestamp}),
            move || {
                let packed = crate::packed::pack_observation(&paths, &layer, &timestamp, &grid)
                    .unwrap_or_else(|err| {
                        tracing::warn!(layer = %layer, timestamp = %timestamp, err = %format!("{err:#}"), "packed_failed");
                        None
                    });
                (grid, packed)
            },
        )
        .await
    } else {
        (grid, None)
    };

    hb(serde_json::json!({"phase": "grid", "timestamp": timestamp}));
    let state_dir = cfg.paths.state_dir.clone();
    let storms_cfg = cfg.storms.clone();
    let grid = heartbeat::blocking(
        RENDER_BEAT,
        hb,
        serde_json::json!({"phase": "grid", "timestamp": timestamp}),
        {
            let cfg = cfg.clone();
            let layer = layer.clone();
            let timestamp = timestamp.clone();
            move || {
                common::dump_grid_logged(&cfg, &layer, &timestamp, &grid, "dBZ");
                if layer == RADAR_LAYER
                    && let Ok((lats, lons)) = common::regular_axes(&grid)
                {
                    match StormGrid::new(&grid.values, grid.height, grid.width, lats, lons) {
                        Ok(view) => {
                            if let Err(err) = radar_storms::write_storms_json(
                                &state_dir,
                                &view,
                                &timestamp,
                                &storms_cfg,
                            ) {
                                tracing::warn!(err = %err, "storm_detect_failed");
                            }
                        }
                        Err(err) => tracing::warn!(err = %err, "storm_detect_failed"),
                    }
                }
                grid
            }
        },
    )
    .await;

    if layer == RADAR_LAYER {
        let grid_for_input = grid.clone();
        let cfg_for_input = cfg.clone();
        let ts_for_input = timestamp.clone();
        // Not guarded (D12): a failure here fails the frame activity. Retry converges.
        heartbeat::blocking(
            RENDER_BEAT,
            hb,
            serde_json::json!({"phase": "grid", "timestamp": timestamp}),
            move || -> anyhow::Result<()> {
                let (lats, lons) = common::regular_axes(&grid_for_input)?;
                let input = GridInput {
                    max_cells: Some(cfg_for_input.nowcast_science_grid_max_cells),
                    ..GridInput::new(&grid_for_input.values, lats, lons, "dBZ")
                };
                grid_dump::write_grid(
                    &cfg_for_input.grid,
                    NOWCAST_INPUT_LAYER,
                    &ts_for_input,
                    &input,
                )?;
                grid_dump::prune_grid_layer(&cfg_for_input.grid, NOWCAST_INPUT_LAYER, None);
                Ok(())
            },
        )
        .await?;
        ctx::kick_nowcast(ctx, &timestamp).await;
    }

    let frame = FrameSpec {
        timestamp: timestamp.clone(),
        path: Some(timestamp.clone()),
        palettes: Vec::new(),
        extra: serde_json::json!({
            "source": "mrms",
            "kind": "observation",
            "issued_at": timestamp,
            "lead_minutes": 0,
            "spatial_resolution_km": 1.0,
            "max_zoom": 7,
        })
        .as_object()
        .cloned()
        .unwrap_or_default(),
    };
    let options = AddOptions {
        palettes: rendered.clone(),
        frame: Some(frame),
        layer_metadata:
            serde_json::json!({"title": "MRMS observed reflectivity", "kind": "observation"})
                .as_object()
                .cloned(),
    };
    // The manifest lock is an flock wait, so the write runs on the blocking pool with heartbeats (F5).
    heartbeat::blocking(
        RENDER_BEAT,
        hb,
        serde_json::json!({"phase": "manifest", "timestamp": timestamp}),
        {
            let state_dir = cfg.paths.state_dir.clone();
            let layer = layer.clone();
            let timestamp = timestamp.clone();
            move || radar_core::manifest::add_frame(&state_dir, &layer, &timestamp, &options)
        },
    )
    .await?;
    if let Some(bucket) = packed {
        // After the frame and the bucket file, so the manifest never lists a slot it cannot serve.
        let state_dir = cfg.paths.state_dir.clone();
        let layer_name = layer.clone();
        let written = heartbeat::blocking(
            RENDER_BEAT,
            hb,
            serde_json::json!({"phase": "manifest", "timestamp": timestamp}),
            move || {
                radar_core::manifest::packed::upsert_packed_bucket(
                    &state_dir,
                    &layer_name,
                    &crate::packed::mrms_layout(),
                    &bucket,
                )
            },
        )
        .await;
        if let Err(err) = written {
            tracing::warn!(layer = %layer, timestamp = %timestamp, err = %err, "packed_manifest_failed");
        }
    }
    let duration_s = elapsed_s(started);
    tracing::info!(layer = %layer, timestamp = %timestamp, duration_s, "frame_done");
    Ok(ProcessFrameResult {
        key,
        timestamp,
        rendered: true,
        palettes: rendered,
        duration_s,
    })
}

/// `mrms_mark_processed` (spec §12.1).
pub async fn mark_processed(ctx: &IngestCtx, input: &MarkProcessedInput) -> anyhow::Result<()> {
    let cfg = ctx.cfg.clone();
    let file = format!("ingest-mrms-{}.json", input.layer_name);
    let key = input.key.clone();
    common::offload(move || -> anyhow::Result<()> {
        let mut set = processed_set(&cfg, &file, MRMS_PROCESSED_MAX)?;
        set.add(&key)?;
        Ok(())
    })
    .await?
}

/// `mrms_cleanup` (spec §12.1). Removes timestamp directories older than the retention window, with
/// their manifest frames, then expired packed buckets (manifest first), then sweeps grids.
pub async fn cleanup(ctx: &IngestCtx, input: &CleanupInput) -> anyhow::Result<CleanupResult> {
    let cfg = ctx.cfg.clone();
    let layer = input.layer_name.clone();
    let cutoff = Utc::now() - ChronoDuration::hours(input.retention_hours);
    tokio::task::spawn_blocking(move || -> anyhow::Result<CleanupResult> {
        let layer_dir = cfg.paths.tile_layer_dir(&layer);
        let mut tile_dirs_removed = 0u64;
        if layer_dir.exists() {
            let mut candidates: Vec<PathBuf> = Vec::new();
            for entry in child_dirs(&layer_dir, |_| true)? {
                let name = entry
                    .file_name()
                    .and_then(|n| n.to_str())
                    .unwrap_or_default()
                    .to_string();
                if name.starts_with(|c: char| c.is_ascii_digit()) {
                    candidates.push(entry);
                } else {
                    candidates.extend(child_dirs(&entry, |_| true)?);
                }
            }
            for dir in candidates {
                let name = dir
                    .file_name()
                    .and_then(|n| n.to_str())
                    .unwrap_or_default()
                    .to_string();
                tile_dirs_removed +=
                    remove_timestamp_dir(&cfg.paths.state_dir, &layer, &dir, &name, cutoff)?;
            }
        }
        let packed_removed = crate::packed::cleanup_observation_buckets(
            &cfg.paths.state_dir,
            &cfg.paths,
            &layer,
            cutoff,
        )?;
        if packed_removed > 0 {
            tracing::info!(layer = %layer, packed_removed, "packed_buckets_removed");
        }
        let grid_files_removed = common::cleanup_grids(&cfg)?;
        Ok(CleanupResult {
            tile_dirs_removed: tile_dirs_removed as i64,
            grid_files_removed: grid_files_removed as i64,
        })
    })
    .await?
}
