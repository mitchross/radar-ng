//! Winter layers, one `IngestWinterWorkflow` schedule per product:
//!
//! - `nbm-snow` (schedule `ingest-nbm-snow`): `snowfall-forecast` and `ice-forecast`, the running NBM
//!   snow and flat-ice totals from the run start, one frame every 6 h out to +72 h.
//!
//! Same steps as the global-model ingest: find the run, process its hours, publish the coherent prefix,
//! mark processed, clean up. A product can render several layers; an hour counts only when all of them
//! rendered. New products add a [`Product`] variant and its [`WinterLayer`] rows.

pub mod activities;
pub mod state;

use radar_grib::nbm::NbmVar;

/// Where a layer's values come from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Field {
    /// Running sum of NBM 6 hour records from the run start, in inches.
    NbmTotal(NbmVar),
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

/// One winter schedule. The workflow argument is [`Product::name`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Product {
    /// NBM snowfall and ice totals.
    NbmSnow,
}

impl Product {
    /// Every product, in schedule order.
    pub const ALL: [Product; 1] = [Product::NbmSnow];

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
        }
    }

    /// Layers the product renders.
    pub fn layers(self) -> &'static [WinterLayer] {
        match self {
            Product::NbmSnow => &NBM_SNOW_LAYERS,
        }
    }

    /// `source` written into each frame.
    pub fn source(self) -> &'static str {
        match self {
            Product::NbmSnow => "nbm",
        }
    }

    /// `ProcessedSet` file in the state directory.
    pub fn processed_file(self) -> &'static str {
        match self {
            Product::NbmSnow => "ingest-nbm-snow.json",
        }
    }

    /// Hours a run keeps its tiles after a newer run replaces it.
    pub fn retention_hours(self) -> i64 {
        match self {
            Product::NbmSnow => 12,
        }
    }

    /// NBM records the product reads.
    pub fn nbm_vars(self) -> Vec<NbmVar> {
        self.layers()
            .iter()
            .map(|l| match l.field {
                Field::NbmTotal(v) => v,
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
/// `spatial_resolution_km` written into each frame.
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
        ids.dedup();
        assert_eq!(ids.len(), n);
        assert_eq!(layer("ice-forecast").unwrap().color_key, "ice_accum");
        assert_eq!(
            Product::NbmSnow.nbm_vars(),
            vec![NbmVar::Asnow, NbmVar::Ficeac]
        );
    }
}
