//! Winter layers, one `IngestWinterWorkflow` schedule per product:
//!
//! - `nbm-snow` (schedule `ingest-nbm-snow`): `snowfall-forecast` and `ice-forecast`, the running NBM
//!   snow and flat-ice totals from the run start, one frame every 6 h out to +72 h.
//! - `snow-depth` (schedule `ingest-snow-depth`): SNODAS snow on the ground, one frame a day.
//! - `snowfall-24h` (schedule `ingest-snowfall-24h`): the NOHRSC 24 h snowfall analysis, newest only.
//! - `snow-chance` (schedule `ingest-snow-chance`): `snow-chance-1`, `-4` and `-8`, the WPC chance of at
//!   least 1, 4 or 8 in of snow in the 24 h ending at each frame, one frame every 6 h. Its hours come
//!   from the cycle's file list, not a fixed table.
//!
//! Same steps as the global-model ingest: find the run, process its hours, publish the coherent prefix,
//! mark processed, clean up. A product can render several layers; an hour counts only when all of them
//! rendered. The observed products are a one-hour "run" (hour 0) named after their valid time; when
//! today's file is missing, the last frame stays published. New products add a [`Product`] variant and
//! its [`WinterLayer`] rows.

pub mod activities;
pub mod nohrsc;
pub mod snodas;
pub mod state;
pub mod wpc;

use radar_grib::nbm::NbmVar;

/// Where a layer's values come from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Field {
    /// Running sum of NBM 6 hour records from the run start, in inches.
    NbmTotal(NbmVar),
    /// SNODAS snow depth, in inches.
    SnodasDepth,
    /// NOHRSC 24 hour snowfall analysis, in inches.
    Nohrsc24h,
    /// WPC chance of at least this many inches of snow in 24 h, in percent.
    WpcChance(u32),
}

/// One rendered winter layer.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct WinterLayer {
    /// Manifest key, tile subtree and grid-dump layer.
    pub id: &'static str,
    /// Palette color key.
    pub color_key: &'static str,
    /// Grid-dump unit.
    pub unit: &'static str,
    /// Manifest layer title.
    pub title: &'static str,
    /// Values.
    pub field: Field,
}

/// NBM snow and ice layers.
pub const NBM_SNOW_LAYERS: [WinterLayer; 2] = [
    WinterLayer {
        id: "snowfall-forecast",
        color_key: "snowfall",
        unit: "in",
        title: "Snowfall forecast (NOAA NBM)",
        field: Field::NbmTotal(NbmVar::Asnow),
    },
    WinterLayer {
        id: "ice-forecast",
        color_key: "ice_accum",
        unit: "in",
        title: "Ice forecast (NOAA NBM)",
        field: Field::NbmTotal(NbmVar::Ficeac),
    },
];

/// SNODAS snow depth layer.
pub const SNOW_DEPTH_LAYERS: [WinterLayer; 1] = [WinterLayer {
    id: "snow-depth",
    color_key: "snow_depth",
    unit: "in",
    title: "Snow depth (NOAA SNODAS)",
    field: Field::SnodasDepth,
}];

/// NOHRSC 24 hour snowfall layer.
pub const SNOWFALL_24H_LAYERS: [WinterLayer; 1] = [WinterLayer {
    id: "snowfall-24h",
    color_key: "snowfall",
    unit: "in",
    title: "Snowfall, past 24 h (NOAA NOHRSC)",
    field: Field::Nohrsc24h,
}];

/// WPC snow chance layers, one per threshold.
pub const SNOW_CHANCE_LAYERS: [WinterLayer; 3] = [
    WinterLayer {
        id: "snow-chance-1",
        color_key: "snow_chance",
        unit: "%",
        title: "Chance of 1 in of snow in 24 h (NOAA WPC)",
        field: Field::WpcChance(1),
    },
    WinterLayer {
        id: "snow-chance-4",
        color_key: "snow_chance",
        unit: "%",
        title: "Chance of 4 in of snow in 24 h (NOAA WPC)",
        field: Field::WpcChance(4),
    },
    WinterLayer {
        id: "snow-chance-8",
        color_key: "snow_chance",
        unit: "%",
        title: "Chance of 8 in of snow in 24 h (NOAA WPC)",
        field: Field::WpcChance(8),
    },
];

/// One winter schedule. The workflow argument is [`Product::name`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Product {
    /// NBM snowfall and ice totals.
    NbmSnow,
    /// SNODAS snow depth.
    SnowDepth,
    /// NOHRSC 24 hour snowfall.
    Snowfall24h,
    /// WPC snow chance.
    SnowChance,
}

impl Product {
    /// Every product, in schedule order.
    pub const ALL: [Product; 4] = [
        Product::NbmSnow,
        Product::SnowDepth,
        Product::Snowfall24h,
        Product::SnowChance,
    ];

    /// Product for a workflow argument.
    pub fn from_name(name: &str) -> anyhow::Result<Self> {
        Self::ALL
            .into_iter()
            .find(|p| p.name() == name)
            .ok_or_else(|| anyhow::anyhow!("unknown winter product {name:?}"))
    }

    /// Workflow argument.
    pub fn name(self) -> &'static str {
        match self {
            Product::NbmSnow => "nbm-snow",
            Product::SnowDepth => "snow-depth",
            Product::Snowfall24h => "snowfall-24h",
            Product::SnowChance => "snow-chance",
        }
    }

    /// Layers the product renders.
    pub fn layers(self) -> &'static [WinterLayer] {
        match self {
            Product::NbmSnow => &NBM_SNOW_LAYERS,
            Product::SnowDepth => &SNOW_DEPTH_LAYERS,
            Product::Snowfall24h => &SNOWFALL_24H_LAYERS,
            Product::SnowChance => &SNOW_CHANCE_LAYERS,
        }
    }

    /// `source` written into each frame.
    pub fn source(self) -> &'static str {
        match self {
            Product::NbmSnow => "nbm",
            Product::SnowDepth => "snodas",
            Product::Snowfall24h => "nohrsc",
            Product::SnowChance => "wpc",
        }
    }

    /// `ProcessedSet` file in the state directory.
    pub fn processed_file(self) -> &'static str {
        match self {
            Product::NbmSnow => "ingest-nbm-snow.json",
            Product::SnowDepth => "ingest-snow-depth.json",
            Product::Snowfall24h => "ingest-snowfall-24h.json",
            Product::SnowChance => "ingest-snow-chance.json",
        }
    }

    /// Hours a run keeps its tiles after a newer run replaces it.
    pub fn retention_hours(self) -> i64 {
        match self {
            Product::NbmSnow | Product::SnowChance => 12,
            Product::SnowDepth | Product::Snowfall24h => 48,
        }
    }

    /// Whether the frames are observed analyses rather than a forecast.
    pub fn observed(self) -> bool {
        !matches!(self, Product::NbmSnow | Product::SnowChance)
    }

    /// Whether the run's hours come from the source's file list rather than a fixed table.
    pub fn listed_hours(self) -> bool {
        matches!(self, Product::SnowChance)
    }

    /// `kind` written into each frame and the layer.
    pub fn kind(self) -> &'static str {
        if self.observed() {
            "observation"
        } else {
            "model_guidance"
        }
    }

    /// `spatial_resolution_km` written into each frame.
    pub fn resolution_km(self) -> f64 {
        match self {
            Product::SnowDepth => 1.0,
            Product::NbmSnow | Product::Snowfall24h | Product::SnowChance => SPATIAL_RESOLUTION_KM,
        }
    }

    /// NBM records the product reads.
    pub fn nbm_vars(self) -> Vec<NbmVar> {
        self.layers()
            .iter()
            .filter_map(|l| match l.field {
                Field::NbmTotal(v) => Some(v),
                Field::SnodasDepth | Field::Nohrsc24h | Field::WpcChance(_) => None,
            })
            .collect()
    }
}

/// Layer by id, across every product.
pub fn layer(id: &str) -> Option<&'static WinterLayer> {
    Product::ALL
        .iter()
        .flat_map(|p| p.layers())
        .find(|l| l.id == id)
}

/// `ProcessedSet` capacity per product.
pub const PROCESSED_MAX: usize = 60;
/// Zooms rendered (as HRRR; a 2.5 km cell is about 1 pixel at z6).
pub const ZOOMS: [u32; 3] = [4, 5, 6];
/// `spatial_resolution_km` of the 2.5 km grids (NBM, NOHRSC).
pub const SPATIAL_RESOLUTION_KM: f64 = 2.5;
/// `max_zoom` written into each frame.
pub const MAX_ZOOM: i64 = 6;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_round_trip_and_layers_are_unique() {
        for p in Product::ALL {
            assert_eq!(Product::from_name(p.name()).unwrap(), p);
        }
        assert!(Product::from_name("uv").is_err());
        let mut ids: Vec<&str> = Product::ALL
            .iter()
            .flat_map(|p| p.layers())
            .map(|l| l.id)
            .collect();
        let n = ids.len();
        ids.sort();
        ids.dedup();
        assert_eq!(ids.len(), n);
        assert_eq!(layer("ice-forecast").unwrap().color_key, "ice_accum");
        assert_eq!(
            Product::NbmSnow.nbm_vars(),
            vec![NbmVar::Asnow, NbmVar::Ficeac]
        );
        assert!(Product::SnowDepth.nbm_vars().is_empty());
        assert_eq!(layer("snowfall-24h").unwrap().color_key, "snowfall");
        assert_eq!(
            Product::from_name("snow-depth").unwrap().retention_hours(),
            48
        );
        assert_eq!(
            (Product::Snowfall24h.kind(), Product::NbmSnow.kind()),
            ("observation", "model_guidance")
        );
        let chance = Product::from_name("snow-chance").unwrap();
        assert_eq!(chance.kind(), "model_guidance");
        assert_eq!(chance.retention_hours(), 12);
        assert!(chance.listed_hours() && !Product::NbmSnow.listed_hours());
        assert_eq!(
            chance.layers().iter().map(|l| l.field).collect::<Vec<_>>(),
            wpc::THRESHOLDS.map(Field::WpcChance)
        );
        assert_eq!(layer("snow-chance-8").unwrap().unit, "%");
    }
}
