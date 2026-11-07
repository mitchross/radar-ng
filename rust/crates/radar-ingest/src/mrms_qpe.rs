//! Single-field MRMS products rendered through one path: the 24-hour multi-sensor QPE (`precip-24h`,
//! rainfall totals in inches) and the severe-weather layers (`hail-size`, `rotation-track`,
//! `echo-top`).
//!
//! Each product has its own schedule running `IngestMrmsWorkflow`. Listing, mark-processed and
//! cleanup are the reflectivity activities; [`crate::mrms::process_frame`] hands the frame here.
//! The products differ only in the [`Product`] table row (prefix, unit conversion, palette key).

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

/// Layer name of the 60-minute maximum hail size (MESH).
pub const HAIL_LAYER: &str = "hail-size";
/// S3 prefix of the MRMS Maximum Estimated Size of Hail, past 60 minutes.
pub const HAIL_PREFIX: &str = "CONUS/MESH_Max_60min_00.50";
/// Layer name of the 60-minute low-level rotation track.
pub const ROTATION_LAYER: &str = "rotation-track";
/// S3 prefix of the MRMS low-level (0-2 km) rotation track, past 60 minutes.
pub const ROTATION_PREFIX: &str = "CONUS/RotationTrack60min_00.50";
/// Layer name of the 18 dBZ echo-top height.
pub const ECHO_TOP_LAYER: &str = "echo-top";
/// S3 prefix of the MRMS 18 dBZ echo-top height.
pub const ECHO_TOP_PREFIX: &str = "CONUS/EchoTop_18_00.50";

/// Which config toggle gates a product.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Toggle {
    /// `MRMS_QPE_24H_ENABLED`.
    Qpe24h,
    /// `MRMS_SEVERE_ENABLED`.
    Severe,
}

/// One single-field MRMS product: everything that differs between the layers this module renders.
#[derive(Debug, Clone, Copy)]
pub struct Product {
    /// Layer name (tile directory and manifest key).
    pub layer: &'static str,
    /// S3 prefix in `noaa-mrms-pds`.
    pub prefix: &'static str,
    /// Palette color key in the palette JSON files.
    pub color_key: &'static str,
    /// MRMS product name for the manifest.
    pub product: &'static str,
    /// Layer title for the manifest.
    pub title: &'static str,
    /// Unit of the rendered values.
    pub unit: &'static str,
    /// Accumulation or tracking window in minutes, when the product has one.
    pub window_minutes: Option<u32>,
    /// Converts a GRIB value to the rendered unit; missing codes become NaN.
    pub convert: fn(f32) -> f32,
    /// Config toggle that gates the layer.
    pub toggle: Toggle,
}

/// Every product this module renders.
pub const PRODUCTS: [Product; 4] = [
    Product {
        layer: QPE_24H_LAYER,
        prefix: QPE_24H_PREFIX,
        color_key: QPE_COLOR_KEY,
        product: "MultiSensor_QPE_24H_Pass1",
        title: "MRMS observed precipitation, past 24 hours",
        unit: "in",
        window_minutes: Some(24 * 60),
        convert: qpe_mm_to_inches,
        toggle: Toggle::Qpe24h,
    },
    Product {
        layer: HAIL_LAYER,
        prefix: HAIL_PREFIX,
        color_key: "hail_size",
        product: "MESH_Max_60min",
        title: "MRMS estimated hail size, past hour",
        unit: "in",
        window_minutes: Some(60),
        convert: qpe_mm_to_inches,
        toggle: Toggle::Severe,
    },
    Product {
        layer: ROTATION_LAYER,
        prefix: ROTATION_PREFIX,
        color_key: "rotation_track",
        product: "RotationTrack60min",
        title: "MRMS low-level rotation tracks, past hour",
        unit: "1/s",
        window_minutes: Some(60),
        convert: rotation_value,
        toggle: Toggle::Severe,
    },
    Product {
        layer: ECHO_TOP_LAYER,
        prefix: ECHO_TOP_PREFIX,
        color_key: "echo_top",
        product: "EchoTop_18",
        title: "MRMS storm top height (18 dBZ)",
        unit: "kft",
        window_minutes: None,
        convert: echo_top_km_to_kft,
        toggle: Toggle::Severe,
    },
];

/// The product rendered for `layer`, if this module owns it.
pub fn product_for(layer: &str) -> Option<&'static Product> {
    PRODUCTS.iter().find(|p| p.layer == layer)
}

/// Whether `layer` is rendered by this module.
pub fn is_qpe_layer(layer: &str) -> bool {
    product_for(layer).is_some()
}

/// Whether the listing offers keys for `layer`. Each product follows its toggle
/// (`MRMS_QPE_24H_ENABLED`, `MRMS_SEVERE_ENABLED`); reflectivity layers are always on.
pub fn layer_enabled(cfg: &IngestConfig, layer: &str) -> bool {
    match product_for(layer).map(|p| p.toggle) {
        None => true,
        Some(Toggle::Qpe24h) => cfg.mrms_qpe_24h,
        Some(Toggle::Severe) => cfg.mrms_severe,
    }
}

/// MESH or QPE-style millimetres to inches is [`qpe_mm_to_inches`]. MRMS stores rotation track in
/// 0.001 s^-1 (NSSL GRIB2 table); scale to s^-1. Negative no-coverage codes become NaN.
pub fn rotation_value(v: f32) -> f32 {
    if v < 0.0 { f32::NAN } else { v / 1000.0 }
}

/// Echo-top kilometres to kilofeet; negative missing codes become NaN.
pub fn echo_top_km_to_kft(km: f32) -> f32 {
    if km < 0.0 {
        f32::NAN
    } else {
        units::km_to_kft(km)
    }
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
    load_tables(names, QPE_COLOR_KEY)
}

/// The `color_key` table of every `PALETTES` name that has one (`classic` when none load).
pub fn load_tables(names: &[String], color_key: &str) -> Vec<(String, ColorTable)> {
    tables_for_key(&load_hrrr_palettes(names), color_key)
}

/// The manifest frame for a QPE file valid at `timestamp` (the end of the 24-hour window).
pub fn frame_spec(timestamp: &str) -> FrameSpec {
    product_frame_spec(&PRODUCTS[0], timestamp)
}

/// Layer metadata written with each QPE frame.
pub fn layer_metadata() -> Map<String, Value> {
    product_layer_metadata(&PRODUCTS[0])
}

/// The manifest frame for a `product` file valid at `timestamp`.
pub fn product_frame_spec(product: &Product, timestamp: &str) -> FrameSpec {
    let mut extra = object(json!({
        "source": "mrms",
        "kind": "observation",
        "product": product.product,
        "unit": product.unit,
        "issued_at": timestamp,
        "lead_minutes": 0,
        "spatial_resolution_km": 1.0,
        "max_zoom": 7,
    }));
    if let Some(minutes) = product.window_minutes {
        if minutes % 60 == 0 && product.layer == QPE_24H_LAYER {
            extra.insert("accumulation_hours".into(), json!(minutes / 60));
        } else {
            extra.insert("window_minutes".into(), json!(minutes));
        }
    }
    FrameSpec {
        timestamp: timestamp.to_string(),
        path: Some(timestamp.to_string()),
        palettes: Vec::new(),
        extra,
    }
}

/// Layer metadata written with each frame of `product`.
pub fn product_layer_metadata(product: &Product) -> Map<String, Value> {
    object(json!({
        "title": product.title,
        "kind": "observation",
        "unit": product.unit,
        "palette_key": product.color_key,
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
    let product = product_for(&layer)
        .ok_or_else(|| anyhow::anyhow!("{layer} is not a single-field MRMS product"))?;
    let tables = load_tables(&cfg.palettes, product.color_key);
    anyhow::ensure!(!tables.is_empty(), "no palette has {}", product.color_key);
    let convert = product.convert;

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
            *v = convert(*v);
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
        frame: Some(product_frame_spec(product, &timestamp)),
        layer_metadata: Some(product_layer_metadata(product)),
    };
    heartbeat::blocking(
        RENDER_BEAT,
        hb,
        json!({"phase": "manifest", "timestamp": timestamp}),
        {
            let layer = layer.clone();
            let timestamp = timestamp.clone();
            move || -> anyhow::Result<()> {
                dump_grid(&cfg, &layer, product.unit, &timestamp, &grid);
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

/// Write the point grid for the inspector in the layer's unit, then prune the layer to [`QPE_GRID_KEEP`].
/// A failure is logged; the tiles and the frame are still published.
fn dump_grid(cfg: &IngestConfig, layer: &str, unit: &str, timestamp: &str, grid: &Grid) {
    let Ok((lats, lons)) = common::regular_axes(grid) else {
        return;
    };
    let input = GridInput {
        max_cells: Some(QPE_GRID_MAX_CELLS),
        ..GridInput::new(&grid.values, lats, lons, unit)
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
        assert_eq!(layer_metadata()["palette_key"], "precip_accum");
    }

    #[test]
    fn severe_conversions_use_inches_s_inverse_and_kilofeet() {
        assert_eq!(product_for(HAIL_LAYER).unwrap().prefix, HAIL_PREFIX);
        assert!((qpe_mm_to_inches(44.45) - 1.75).abs() < 1e-6, "golf ball");
        assert_eq!(rotation_value(12.0), 0.012);
        assert!(rotation_value(-3.0).is_nan());
        assert!((echo_top_km_to_kft(10.0) - 32.8084).abs() < 1e-3);
        assert!(echo_top_km_to_kft(-3.0).is_nan());
        assert_eq!(echo_top_km_to_kft(0.0), 0.0);
    }

    #[test]
    fn severe_layers_follow_their_own_toggle() {
        let dir = tempfile::tempdir().unwrap();
        let paths = radar_core::env::Paths::new(
            dir.path().join("s"),
            dir.path().join("g"),
            dir.path().join("t"),
        );
        let mut cfg = IngestConfig::with_roots(paths, dir.path().join("w")).unwrap();
        for layer in [HAIL_LAYER, ROTATION_LAYER, ECHO_TOP_LAYER] {
            assert!(!layer_enabled(&cfg, layer));
        }
        cfg.mrms_severe = true;
        for layer in [HAIL_LAYER, ROTATION_LAYER, ECHO_TOP_LAYER] {
            assert!(layer_enabled(&cfg, layer));
        }
        assert!(!layer_enabled(&cfg, QPE_24H_LAYER));
    }

    #[test]
    fn every_built_in_palette_colors_every_product() {
        let names: Vec<String> = ["classic", "muted", "vivid"].map(String::from).into();
        for p in PRODUCTS {
            assert_eq!(load_tables(&names, p.color_key).len(), 3, "{}", p.layer);
        }
    }

    #[test]
    fn hourly_window_products_say_so_in_the_manifest() {
        let frame = product_frame_spec(
            product_for(HAIL_LAYER).unwrap(),
            "2026-10-15T22:00:00+00:00",
        );
        assert_eq!(frame.extra["window_minutes"], 60);
        assert_eq!(frame.extra["unit"], "in");
        let top = product_frame_spec(
            product_for(ECHO_TOP_LAYER).unwrap(),
            "2026-10-15T22:00:00+00:00",
        );
        assert_eq!(top.extra["unit"], "kft");
        assert!(top.extra.get("window_minutes").is_none());
    }
}
