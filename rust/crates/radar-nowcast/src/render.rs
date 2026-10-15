//! Per-lead rendering and point grids (spec-nowcast sections 8.1, 8.2, 8.7, 8.8).
//!
//! One lead is one forecast field. It is classified against the palettes, rendered into one pyramid
//! per palette by `radar-tiler` (renderer identity `indexed-rs1`), and written as a point grid for the
//! API. A lead is accepted only when every expected palette rendered tiles (Python `set(palettes) ==
//! expected`). A rejected lead has its tile directories removed and produces no manifest frame.

use std::collections::BTreeSet;
use std::path::PathBuf;

use anyhow::Context;
use chrono::{DateTime, Utc};
use radar_core::grid_dump::{GridInput, write_grid};
use radar_core::manifest::FrameSpec;
use radar_core::time::{isoformat_utc, round_digits};
use radar_tiler::{
    ColorTable, FrameKind, FrameOptions, Grid, GridGeo, Overview, render_frame_palettes,
};
use serde_json::{Map, Value};

use crate::config::{NowcastConfig, ZOOM_LEVELS};
use crate::field::Field2;

/// Cells below this many dBZ are dry for the rendered and stored lead (spec 8.1).
pub const DRY_BELOW_DBZ: f32 = 5.0;
/// No-data sentinel of the stored and rendered frames.
pub const NODATA: f32 = -9999.0;
/// Tile edge in pixels.
const TILE_SIZE: usize = 256;

/// Everything a lead needs that is shared by all leads of one run.
pub struct RenderContext<'a> {
    /// Run configuration.
    pub cfg: &'a NowcastConfig,
    /// Palettes that have a `reflectivity` table, with their tables.
    pub tables: &'a [(String, ColorTable)],
    /// Names of the palettes every accepted lead must render (the keys of `tables`).
    pub expected: &'a BTreeSet<String>,
    /// Anchor (newest input) timestamp, the run id.
    pub anchor: &'a str,
    /// Anchor as a time.
    pub latest_dt: DateTime<Utc>,
    /// Latitudes of the science grid, descending.
    pub lats: &'a [f64],
    /// Longitudes of the science grid, ascending.
    pub lons: &'a [f64],
    /// Method label written to the frame (`pysteps-sprog`).
    pub method: &'a str,
    /// Median input spacing, minutes.
    pub input_interval_min: f64,
    /// Grid resolution in km for the manifest.
    pub resolution_km: f64,
}

/// A lead that passed the acceptance rule.
#[derive(Debug, Clone)]
pub struct AcceptedLead {
    /// Valid time, canonical UTC.
    pub timestamp: String,
    /// Palettes it rendered (equal to the expected set).
    pub palettes: Vec<String>,
    /// Manifest frame (path, grid key and metadata).
    pub frame: FrameSpec,
}

/// Tile-path segment and point-grid key of a lead: `runs/<anchor>/<valid>`.
pub fn run_relative_key(anchor: &str, valid: &str) -> String {
    format!("runs/{anchor}/{valid}")
}

/// Render and store lead `index` (zero-based) of the forecast `field`.
///
/// Returns `Ok(None)` when the lead is rejected. Its tile directories are removed in that case.
pub fn render_lead(
    ctx: &RenderContext<'_>,
    index: usize,
    field: &Field2<f32>,
) -> anyhow::Result<Option<AcceptedLead>> {
    let cfg = ctx.cfg;
    let minutes = (index as i64 + 1) * cfg.step_min;
    let valid = ctx.latest_dt + chrono::TimeDelta::minutes(minutes);
    let ts = isoformat_utc(&valid);
    let tile_path = run_relative_key(ctx.anchor, &ts);
    let values: Vec<f32> = field
        .as_slice()
        .iter()
        .map(|&v| {
            if v.is_nan() || v < DRY_BELOW_DBZ {
                NODATA
            } else {
                v
            }
        })
        .collect();
    let grid = Grid {
        width: field.width(),
        height: field.height(),
        values,
        geo: GridGeo::Regular {
            lats: ctx.lats.to_vec(),
            lons: ctx.lons.to_vec(),
        },
    };

    let nowcast_root = cfg.paths.tile_dir.join("nowcast");
    let output_dirs: Vec<(String, PathBuf)> = ctx
        .tables
        .iter()
        .map(|(name, _)| (name.clone(), nowcast_root.join(name).join(&tile_path)))
        .collect();
    let options = FrameOptions {
        tile_size: TILE_SIZE,
        source_id: format!("nowcast:{tile_path}"),
        publication_lock_root: Some(nowcast_root.clone()),
        overview: Some(Overview::Max),
        png_compress_level: cfg.png_compress_level,
        keep_empty: false,
    };
    let kind = FrameKind::Continuous {
        nodata_value: Some(f64::from(NODATA)),
        min_valid_weight: 1.0,
    };
    let result = render_frame_palettes(
        &grid,
        &kind,
        ctx.tables,
        &output_dirs,
        &ZOOM_LEVELS,
        &options,
    )
    .with_context(|| format!("rendering lead {ts}"))?;
    let rendered: Vec<String> = result
        .rendered_palettes()
        .iter()
        .map(|s| s.to_string())
        .collect();
    let rendered_set: BTreeSet<String> = rendered.iter().cloned().collect();

    if rendered_set == *ctx.expected {
        let grid_key = tile_path.clone();
        let written = write_grid(
            &cfg.grid,
            "nowcast",
            &ts,
            &GridInput {
                data: &grid.values,
                lats: ctx.lats,
                lons: ctx.lons,
                unit: "dBZ",
                fill: f64::from(NODATA),
                max_cells: Some(cfg.point_grid_max_cells),
                grid_key: Some(&grid_key),
            },
        )
        .with_context(|| format!("writing point grid {grid_key}"))?;
        if written.is_some() {
            return Ok(Some(AcceptedLead {
                timestamp: ts.clone(),
                palettes: rendered,
                frame: frame_spec(ctx, &ts, &tile_path, minutes),
            }));
        }
    }
    for palette in &rendered {
        let _ = std::fs::remove_dir_all(nowcast_root.join(palette).join(&tile_path));
    }
    Ok(None)
}

/// The manifest frame for an accepted lead. Palettes are supplied by the run (`ReplaceOptions`).
fn frame_spec(ctx: &RenderContext<'_>, ts: &str, tile_path: &str, lead_minutes: i64) -> FrameSpec {
    let mut extra = Map::new();
    extra.insert("grid_key".into(), Value::from(tile_path));
    extra.insert("source".into(), Value::from("mrms-nowcast"));
    extra.insert("kind".into(), Value::from("nowcast"));
    extra.insert(
        "issued_at".into(),
        Value::from(isoformat_utc(&ctx.latest_dt)),
    );
    extra.insert("lead_minutes".into(), Value::from(lead_minutes));
    extra.insert(
        "input_interval_minutes".into(),
        Value::from(round_digits(ctx.input_interval_min, 3)),
    );
    extra.insert("method".into(), Value::from(ctx.method));
    extra.insert(
        "spatial_resolution_km".into(),
        Value::from(ctx.resolution_km),
    );
    extra.insert(
        "max_zoom".into(),
        Value::from(*ZOOM_LEVELS.last().expect("zoom levels") as i64),
    );
    FrameSpec {
        timestamp: ts.to_string(),
        path: Some(tile_path.to_string()),
        palettes: Vec::new(),
        extra,
    }
}

/// Spatial resolution in km of a regular grid: `round(|lat_max - lat_min| / (H - 1) * 111, 2)`.
pub fn resolution_km(lats: &[f64]) -> f64 {
    let max = lats.iter().copied().fold(f64::NEG_INFINITY, f64::max);
    let min = lats.iter().copied().fold(f64::INFINITY, f64::min);
    round_digits(
        (max - min).abs() / (lats.len().max(2) - 1) as f64 * 111.0,
        2,
    )
}
