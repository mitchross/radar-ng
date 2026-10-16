//! MRMS 24-hour multi-sensor QPE: the `precip-24h` layer (observed rainfall totals in inches).
//!
//! The `ingest-mrms-qpe-24h` schedule runs `IngestMrmsWorkflow` for this layer. Listing, mark-processed
//! and cleanup are the reflectivity activities; [`crate::mrms::process_frame`] hands the frame here.

use std::path::PathBuf;
use std::time::Instant;

use radar_core::grid_dump::{self, GridInput};
use radar_core::manifest::{AddOptions, FrameSpec};
use radar_geo::Grid;
use radar_grib::mrms as gmrms;
use radar_grib::units;
use radar_tiler::{ColorTable, FrameKind, FrameOptions, Overview};
use radar_types::payloads::mrms::{ProcessFrameInput, ProcessFrameResult};
use serde_json::{Map, Value, json};

use crate::common::{self, elapsed_s};
use crate::config::IngestConfig;
use crate::ctx::IngestCtx;
use crate::heartbeat::{self, Heartbeat};
use crate::hrrr::render::{load_hrrr_palettes, tables_for_key};
use crate::mrms::{DOWNLOAD_BEAT, MRMS_ZOOMS, RENDER_BEAT};

/// Layer name of the observed 24-hour precipitation total.
pub const QPE_24H_LAYER: &str = "precip-24h";
/// S3 prefix of the product: MultiSensor Pass 1 is gauge-corrected and lands about 16 minutes after
/// the hour; Pass 2 adds gauges but lands about an hour later.
pub const QPE_24H_PREFIX: &str = "CONUS/MultiSensor_QPE_24H_Pass1_00.00";
/// Palette color key shared with the HRRR `precip-accum` layer.
pub const QPE_COLOR_KEY: &str = "precip_accum";
/// Grid dump cell cap: stride 2 (2 km) on the 7000 x 3500 CONUS grid. The default stride 8 smooths
/// small heavy-rain cores so far that the readout disagrees with the tile color.
pub const QPE_GRID_MAX_CELLS: usize = 3500 * 1750;
/// Point grids kept for the layer (about 24 MB each), matching the 4-hour tile retention.
pub const QPE_GRID_KEEP: u64 = 5;

/// Whether `layer` is rendered by this module.
pub fn is_qpe_layer(layer: &str) -> bool {
    layer == QPE_24H_LAYER
}

/// Whether the listing offers keys for `layer`. The QPE layer follows `MRMS_QPE_24H_ENABLED`.
pub fn layer_enabled(cfg: &IngestConfig, layer: &str) -> bool {
    !is_qpe_layer(layer) || cfg.mrms_qpe_24h
}

/// QPE millimetres to inches. MRMS writes -3 (and -1) where there is no coverage; those become NaN.
pub fn qpe_mm_to_inches(mm: f32) -> f32 {
    if mm < 0.0 {
        f32::NAN
    } else {
        units::mm_to_inches(mm)
    }
}

/// The `precip_accum` table of every `PALETTES` name that has one (`classic` when none load).
pub fn load_qpe_tables(names: &[String]) -> Vec<(String, ColorTable)> {
    tables_for_key(&load_hrrr_palettes(names), QPE_COLOR_KEY)
}

/// The manifest frame for a QPE file valid at `timestamp` (the end of the 24-hour window).
pub fn frame_spec(timestamp: &str) -> FrameSpec {
    FrameSpec {
        timestamp: timestamp.to_string(),
        path: Some(timestamp.to_string()),
        palettes: Vec::new(),
        extra: object(json!({
            "source": "mrms",
            "kind": "observation",
            "product": "MultiSensor_QPE_24H_Pass1",
            "accumulation_hours": 24,
            "unit": "in",
            "issued_at": timestamp,
            "lead_minutes": 0,
            "spatial_resolution_km": 1.0,
            "max_zoom": 7,
        })),
    }
}

/// Layer metadata written with each frame.
pub fn layer_metadata() -> Map<String, Value> {
    object(json!({
        "title": "MRMS observed precipitation, past 24 hours",
        "kind": "observation",
        "unit": "in",
    }))
}

fn object(value: Value) -> Map<String, Value> {
    value.as_object().cloned().unwrap_or_default()
}

/// `mrms_process_frame` for the QPE layer: download, convert to inches, render, dump the grid, publish.
/// Returns `rendered == false` when a palette failed, so the key is retried.
pub async fn process_frame(
    ctx: &IngestCtx,
    input: &ProcessFrameInput,
    hb: &Heartbeat,
) -> anyhow::Result<ProcessFrameResult> {
    let started = Instant::now();
    let cfg = ctx.cfg.clone();
    let layer = input.layer_name.clone();
    let key = input.key.clone();
    let tables = load_qpe_tables(&cfg.palettes);
    anyhow::ensure!(!tables.is_empty(), "no palette has {QPE_COLOR_KEY}");

    let phase = json!({"phase": "download", "key": key});
    let (timestamp, gz) = heartbeat::during(
        DOWNLOAD_BEAT,
        hb,
        phase.clone(),
        gmrms::fetch_gz(&ctx.mrms_s3, &key),
    )
    .await?;
    let grid = heartbeat::blocking(DOWNLOAD_BEAT, hb, phase, move || -> anyhow::Result<Grid> {
        let mut grid = gmrms::decode_gz(&gz)?;
        gmrms::ensure_south_up(&mut grid);
        for v in &mut grid.values {
            *v = qpe_mm_to_inches(*v);
        }
        Ok(grid)
    })
    .await?;

    let tile_layer = cfg.paths.tile_layer_dir(&layer);
    let dirs: Vec<(String, PathBuf)> = tables
        .iter()
        .map(|(name, _)| (name.clone(), tile_layer.join(name).join(&timestamp)))
        .collect();
    let options = FrameOptions {
        source_id: format!("mrms:{layer}:{timestamp}"),
        publication_lock_root: Some(tile_layer),
        overview: Some(Overview::Max),
        ..FrameOptions::default()
    };
    let expected = tables.len();
    let (grid, rendered) = heartbeat::blocking(
        RENDER_BEAT,
        hb,
        json!({"phase": "render", "timestamp": timestamp}),
        move || -> anyhow::Result<(Grid, Vec<String>)> {
            let kind = FrameKind::Continuous {
                nodata_value: None,
                min_valid_weight: 1.0,
            };
            let res = radar_tiler::render_frame_palettes(
                &grid,
                &kind,
                &tables,
                &dirs,
                &MRMS_ZOOMS,
                &options,
            )?;
            let rendered = res
                .rendered_palettes()
                .iter()
                .map(|s| s.to_string())
                .collect();
            Ok((grid, rendered))
        },
    )
    .await?;
    if rendered.len() != expected {
        tracing::error!(layer = %layer, timestamp = %timestamp, rendered_palettes = ?rendered, "frame_incomplete");
        return Ok(ProcessFrameResult {
            key,
            timestamp,
            rendered: false,
            palettes: rendered,
            duration_s: elapsed_s(started),
        });
    }

    let options = AddOptions {
        palettes: rendered.clone(),
        frame: Some(frame_spec(&timestamp)),
        layer_metadata: Some(layer_metadata()),
    };
    heartbeat::blocking(
        RENDER_BEAT,
        hb,
        json!({"phase": "manifest", "timestamp": timestamp}),
        {
            let layer = layer.clone();
            let timestamp = timestamp.clone();
            move || -> anyhow::Result<()> {
                dump_grid(&cfg, &layer, &timestamp, &grid);
                radar_core::manifest::add_frame(
                    &cfg.paths.state_dir,
                    &layer,
                    &timestamp,
                    &options,
                )?;
                Ok(())
            }
        },
    )
    .await?;
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

/// Write the point grid for the inspector in inches, then prune the layer to [`QPE_GRID_KEEP`].
/// A failure is logged; the tiles and the frame are still published.
fn dump_grid(cfg: &IngestConfig, layer: &str, timestamp: &str, grid: &Grid) {
    let Ok((lats, lons)) = common::regular_axes(grid) else {
        return;
    };
    let input = GridInput {
        max_cells: Some(QPE_GRID_MAX_CELLS),
        ..GridInput::new(&grid.values, lats, lons, "in")
    };
    if let Err(err) = grid_dump::write_grid(&cfg.grid, layer, timestamp, &input) {
        tracing::warn!(layer = %layer, err = %err, "grid_dump_failed");
    }
    grid_dump::prune_grid_layer(&cfg.grid, layer, Some(QPE_GRID_KEEP));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn millimetres_become_inches_and_missing_codes_become_nan() {
        assert_eq!(qpe_mm_to_inches(25.4), 1.0);
        assert_eq!(qpe_mm_to_inches(0.0), 0.0);
        assert!((qpe_mm_to_inches(303.7) - 11.956_693).abs() < 1e-4);
        assert!(qpe_mm_to_inches(-3.0).is_nan());
        assert!(qpe_mm_to_inches(-1.0).is_nan());
        assert!(qpe_mm_to_inches(f32::NAN).is_nan());
    }

    #[test]
    fn product_prefix_is_the_24h_pass1_product() {
        assert_eq!(QPE_24H_PREFIX, "CONUS/MultiSensor_QPE_24H_Pass1_00.00");
        let key = format!(
            "{QPE_24H_PREFIX}/20261015/MRMS_MultiSensor_QPE_24H_Pass1_00.00_20261015-220000.grib2.gz"
        );
        assert_eq!(
            gmrms::extract_timestamp(&key).unwrap(),
            "2026-10-15T22:00:00+00:00"
        );
    }

    #[test]
    fn only_the_qpe_layer_is_gated_by_the_toggle() {
        let dir = tempfile::tempdir().unwrap();
        let paths = radar_core::env::Paths::new(
            dir.path().join("s"),
            dir.path().join("g"),
            dir.path().join("t"),
        );
        let mut cfg = IngestConfig::with_roots(paths, dir.path().join("w")).unwrap();
        assert!(layer_enabled(&cfg, "radar"));
        assert!(!layer_enabled(&cfg, QPE_24H_LAYER));
        cfg.mrms_qpe_24h = true;
        assert!(layer_enabled(&cfg, QPE_24H_LAYER));
    }

    #[test]
    fn every_built_in_palette_colors_the_layer() {
        let names: Vec<String> = ["classic", "muted", "vivid"].map(String::from).into();
        let tables = load_qpe_tables(&names);
        assert_eq!(
            tables.iter().map(|(n, _)| n.as_str()).collect::<Vec<_>>(),
            ["classic", "muted", "vivid"]
        );
    }

    #[test]
    fn manifest_frame_marks_an_hourly_observation_in_inches() {
        let frame = frame_spec("2026-10-15T22:00:00+00:00");
        assert_eq!(frame.path.as_deref(), Some("2026-10-15T22:00:00+00:00"));
        assert_eq!(frame.extra["kind"], "observation");
        assert_eq!(frame.extra["accumulation_hours"], 24);
        assert_eq!(frame.extra["unit"], "in");
        assert_eq!(frame.extra["max_zoom"], 7);
        assert_eq!(layer_metadata()["unit"], "in");
    }
}
