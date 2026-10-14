//! Serialized shape of `storms.json` (spec-shared 6.10).
//!
//! Field order matches Python's dict insertion order, so the compact writer
//! produces the same key sequence. Python writes a short shape (no threshold,
//! count or tracking vectors) when no component reaches the threshold at all. That
//! shape is kept as-is. `next_cell_id` is the one addition on every payload: it
//! is the monotonic counter that fixes defect D3 and is ignored by existing readers.

use serde::Serialize;

/// The whole `storms.json` payload.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct StormsDoc {
    /// Always `"FeatureCollection"`.
    #[serde(rename = "type")]
    pub kind: String,
    /// Tracked cells, sorted by peak reflectivity (descending).
    pub features: Vec<CellFeature>,
    /// Wall-clock time the payload was produced (epoch seconds).
    pub generated_at: f64,
    /// Frame timestamp as passed in.
    pub timestamp: String,
    /// Threshold used for the mask. Omitted in the short shape (no components at all).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub threshold_dbz: Option<f64>,
    /// Number of features. Omitted in the short shape.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cell_count: Option<usize>,
    /// Prediction leads, always `[0, 5, 10]`.
    pub prediction_minutes: Vec<i64>,
    /// One entry per feature. Omitted in the short shape.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tracking_vectors: Option<Vec<TrackingVectorEntry>>,
    /// Next unused cell id. Never decreases across frames (D3 fix).
    pub next_cell_id: i64,
}

/// One storm cell as a GeoJSON point feature.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct CellFeature {
    /// Always `"Feature"`.
    #[serde(rename = "type")]
    pub kind: String,
    /// Centroid of the component.
    pub geometry: PointGeometry,
    /// Cell attributes and tracking state.
    pub properties: CellProperties,
}

/// GeoJSON `Point` geometry.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct PointGeometry {
    /// Always `"Point"`.
    #[serde(rename = "type")]
    pub kind: String,
    /// `[lon, lat]` in degrees.
    pub coordinates: [f64; 2],
}

/// Attributes of one cell.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct CellProperties {
    /// Maximum reflectivity inside the component (dBZ).
    pub peak_dbz: f64,
    /// Component area using the source grid cell size (km^2).
    pub area_km2: f64,
    /// Number of pixels in the component.
    pub pixel_count: u64,
    /// Padded `[west, south, east, north]` (unrounded).
    pub bbox: [f64; 4],
    /// Stable identifier, never reused.
    pub cell_id: i64,
    /// `[epoch_s (1 dp), lon (5 dp), lat (5 dp)]` samples, oldest first.
    pub track_history: Vec<[f64; 3]>,
    /// Fitted motion.
    pub tracking_vector: TrackingVector,
    /// Confidence in the fitted motion, 0 to 1 (2 dp).
    pub tracking_confidence: f64,
    /// Bboxes at 0, 5 and 10 minutes.
    pub predicted_bboxes: Vec<PredictedBbox>,
}

/// Motion of a cell, rounded as Python rounds it.
#[derive(Debug, Clone, Copy, PartialEq, Serialize)]
pub struct TrackingVector {
    /// Eastward speed (km/h, 3 dp).
    pub east_kmh: f64,
    /// Northward speed (km/h, 3 dp).
    pub north_kmh: f64,
    /// Ground speed (km/h, 3 dp).
    pub speed_kmh: f64,
    /// Clockwise bearing from north (degrees, 2 dp).
    pub bearing_deg: f64,
}

/// A predicted bbox at one lead time.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct PredictedBbox {
    /// Lead in minutes (0, 5 or 10).
    pub lead_minutes: i64,
    /// `[west, south, east, north]`, each rounded to 6 dp.
    pub bbox: [f64; 4],
}

/// Entry of `tracking_vectors`: the cell id followed by the motion fields.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct TrackingVectorEntry {
    /// Cell id.
    pub cell_id: i64,
    /// `east_kmh`, `north_kmh`, `speed_kmh`, `bearing_deg`.
    #[serde(flatten)]
    pub vector: TrackingVector,
}
