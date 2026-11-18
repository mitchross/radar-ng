//! Per-lead rendering and point grids (spec-nowcast sections 8.1, 8.2, 8.7, 8.8).
//!
//! One lead is one forecast field. It is classified against the palettes, rendered into one pyramid
//! per palette by `radar-tiler` (renderer identity `indexed-rs1`), and written as a point grid for the
//! API. A lead is accepted only when every expected palette rendered tiles. A rejected lead has its tile directories removed
//! and produces no manifest frame.

use std::collections::BTreeSet;
use std::path::PathBuf;

use anyhow::Context;
use chrono::{DateTime, Utc};
use radar_core::grid_dump::{GridInput, write_grid};
use radar_core::grid_read::read_grid;
use radar_core::ice::{classes_on, nearest_observed_types};
use radar_core::manifest::FrameSpec;
use radar_core::time::{isoformat_utc, round_digits};
use radar_tiler::typed::TYPE_ICE;
use radar_tiler::{
    ColorTable, FrameKind, FrameOptions, Grid, GridGeo, Overview, TypePlane, render_frame_palettes,
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
    /// Precipitation classes of the newest observed frame, held still on every lead.
    pub types: Option<&'a TypePlane>,
}

/// The newest observed `radar-type` dump near `anchor`, as classes on the science grid, or `None`
/// when there is none (precip type off) or every cell is rain. Type boundaries move slowly over the
/// one-hour horizon, so every lead holds these classes still.
pub fn held_types(
    cfg: &NowcastConfig,
    anchor: DateTime<Utc>,
    lats: &[f64],
    lons: &[f64],
) -> Option<TypePlane> {
    let path = nearest_observed_types(&cfg.grid.grid_dir, anchor)?;
    let dump = read_grid(&path)
        .inspect_err(
            |err| tracing::warn!(path = %path.display(), err = %err, "nowcast_types_failed"),
        )
        .ok()?;
    let classes = classes_on(&dump, lats, lons, TYPE_ICE);
    if classes.iter().all(|&c| c == 0) {
        return None;
    }
    TypePlane::new(classes)
        .inspect_err(|err| tracing::warn!(err = %err, "nowcast_types_failed"))
        .ok()
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
        types: ctx.types.cloned(),
        tile_allowlist: None,
        source_window: None,
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

#[cfg(test)]
mod tests {
    use super::*;
    use radar_core::env::Paths;
    use radar_core::grid_dump::GridConfig;
    use radar_core::ice::OBSERVED_TYPE_GRID;
    use radar_tiler::packed::read_bucket;

    use crate::inputs::linspace;

    fn cfg_in(dir: &std::path::Path) -> NowcastConfig {
        let mut cfg = NowcastConfig::with_paths(Paths::new(
            dir.join("state"),
            dir.join("grids"),
            dir.join("tiles"),
        ));
        cfg.grid = GridConfig::with_dir(dir.join("grids"));
        cfg
    }

    #[test]
    fn leads_hold_the_newest_observed_types() {
        let dir = tempfile::tempdir().unwrap();
        let cfg = cfg_in(dir.path());
        let lats = linspace(45.0, 40.0, 16);
        let lons = linspace(-90.0, -80.0, 16);
        let anchor: DateTime<Utc> = "2026-11-17T19:00:00Z".parse().unwrap();
        assert!(
            held_types(&cfg, anchor, &lats, &lons).is_none(),
            "no dump, no plane"
        );

        // The observed dump: snow on the north half, ice in the south-east corner.
        let mut classes = vec![0.0f32; 16 * 16];
        for (i, c) in classes.iter_mut().enumerate() {
            let (r, col) = (i / 16, i % 16);
            *c = if r < 8 {
                1.0
            } else if col >= 12 {
                3.0
            } else {
                0.0
            };
        }
        let ts = "2026-11-17T18:58:00+00:00";
        let input = GridInput::new(&classes, &lats, &lons, "class");
        write_grid(&cfg.grid, OBSERVED_TYPE_GRID, ts, &input).unwrap();
        let plane = held_types(&cfg, anchor, &lats, &lons).unwrap();
        assert_eq!(plane.classes()[0], 1);
        assert_eq!(plane.classes()[16 * 15 + 15], 3);
        assert_eq!(plane.classes()[16 * 15], 0);
        // Older than the hold window: rain colours, never stale snow.
        let late = anchor + chrono::TimeDelta::minutes(45);
        assert!(held_types(&cfg, late, &lats, &lons).is_none());

        // The run packs typed codes, and the manifest layout says so.
        let fields = vec![crate::field::Field2::filled(16, 16, 30.0f32); 2];
        let stamps = vec![
            "2026-11-17T19:05:00+00:00".to_string(),
            "2026-11-17T19:10:00+00:00".to_string(),
        ];
        let anchor_s = "2026-11-17T19:00:00+00:00";
        crate::packed::pack_run(&cfg, anchor_s, &lats, &lons, &fields, &stamps, Some(&plane))
            .unwrap();
        let on_disk = read_bucket(&crate::packed::packed_layer_dir(&cfg).join(anchor_s))
            .unwrap()
            .unwrap();
        assert!(on_disk.typed);
        assert!(crate::packed::layout_typed(true).types.is_some());
        assert!(crate::packed::layout().types.is_none());
    }
}
