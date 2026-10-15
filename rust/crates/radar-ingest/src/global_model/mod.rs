//! Forecast layers cut from global 0.25 degree models, one hourly frame per forecast hour:
//!
//! - `radar-gfs`: GFS composite reflectivity. Clients play it after the last `radar-hrrr` frame, which
//!   extends future radar from HRRR's 18 to 48 h out to 72 h.
//! - `uv`: the NOAA/CPC UV Index forecast (TUV model on the 12z GFS).
//!
//! `IngestGlobalModelWorkflow` runs one product per schedule: find run, process hours, publish the
//! coherent prefix, mark processed, clean up. Same shape as HRRR, split into `state` (pure rules) and
//! `activities`.

pub mod activities;
pub mod state;

use radar_grib::decode::MessageSelector;

/// One global-model layer. The workflow argument is the layer name.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Product {
    /// GFS simulated composite reflectivity (`radar-gfs`).
    GfsRadar,
    /// NOAA/CPC UV Index (`uv`).
    Uv,
}

impl Product {
    /// Both products, in schedule order.
    pub const ALL: [Product; 2] = [Product::GfsRadar, Product::Uv];

    /// Product for a layer name.
    pub fn from_layer(layer: &str) -> anyhow::Result<Self> {
        Self::ALL
            .into_iter()
            .find(|p| p.layer() == layer)
            .ok_or_else(|| anyhow::anyhow!("unknown global-model layer {layer:?}"))
    }

    /// Manifest key, tile subtree and grid-dump layer.
    pub fn layer(self) -> &'static str {
        match self {
            Product::GfsRadar => "radar-gfs",
            Product::Uv => "uv",
        }
    }

    /// Palette color key.
    pub fn color_key(self) -> &'static str {
        match self {
            Product::GfsRadar => "reflectivity",
            Product::Uv => "uv_index",
        }
    }

    /// Grid-dump unit.
    pub fn unit(self) -> &'static str {
        match self {
            Product::GfsRadar => "dBZ",
            Product::Uv => "UVI",
        }
    }

    /// `source` written into each frame.
    pub fn source(self) -> &'static str {
        match self {
            Product::GfsRadar => "gfs",
            Product::Uv => "cpc-uvi",
        }
    }

    /// Manifest layer title.
    pub fn title(self) -> &'static str {
        match self {
            Product::GfsRadar => "GFS simulated reflectivity",
            Product::Uv => "UV Index (NOAA/CPC)",
        }
    }

    /// `ProcessedSet` file in the state directory.
    pub fn processed_file(self) -> &'static str {
        match self {
            Product::GfsRadar => "ingest-gfs-radar.json",
            Product::Uv => "ingest-uv.json",
        }
    }

    /// GRIB2 message to decode.
    pub fn selector(self) -> MessageSelector {
        match self {
            Product::GfsRadar => radar_grib::gfs::refc_selector(),
            Product::Uv => radar_grib::uvi::selector(),
        }
    }

    /// Hours a run keeps its tiles after a newer run replaces it. UV runs once a day.
    pub fn retention_hours(self) -> i64 {
        match self {
            Product::GfsRadar => 12,
            Product::Uv => 36,
        }
    }

    /// Source value to layer value (`W/m2` erythemal irradiance to UV Index; dBZ unchanged).
    pub fn convert(self, v: f32) -> f32 {
        match self {
            Product::GfsRadar => v,
            Product::Uv => v * radar_grib::uvi::UVI_PER_W_M2,
        }
    }
}

/// `ProcessedSet` capacity per product.
pub const PROCESSED_MAX: usize = 60;
/// Zooms rendered (as HRRR; a 0.25 degree cell is about 1 pixel at z6).
pub const ZOOMS: [u32; 3] = [4, 5, 6];
/// `spatial_resolution_km` written into each frame (0.25 degree at mid-latitudes).
pub const SPATIAL_RESOLUTION_KM: f64 = 25.0;
/// `max_zoom` written into each frame.
pub const MAX_ZOOM: i64 = 6;
