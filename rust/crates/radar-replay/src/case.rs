//! Validated case definitions (design §5) and their MRMS product table.

use chrono::{DateTime, Utc};
use thiserror::Error;

/// Zooms rendered for a case, matching MRMS ingest.
pub const CASE_ZOOMS: [u32; 4] = [4, 5, 6, 7];
/// Tile edge in pixels, matching live ingest.
pub const CASE_TILE_SIZE: usize = 256;
/// Only the classic PNG pyramid exists for a replay revision.
pub const CASE_PALETTE: &str = "classic";
/// A fast layer older than this at a playhead frame is reported missing, not substituted.
pub const FAST_STALENESS_S: i64 = 300;

/// One MRMS product rendered as a replay layer.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct LayerSpec {
    /// Layer name used in the manifest and the tile path.
    pub layer: &'static str,
    /// S3 prefix under `CONUS/`.
    pub prefix: &'static str,
    /// MRMS product name, for provenance.
    pub product: &'static str,
    /// Palette color key.
    pub color_key: &'static str,
    /// Rendered unit.
    pub unit: &'static str,
    /// Nominal seconds between objects.
    pub cadence_s: i64,
    /// GRIB value to rendered value; negative missing codes become NaN.
    pub convert: fn(f32) -> f32,
}

/// Base reflectivity; the master playhead for a case.
pub const RADAR_LAYER: &str = "radar";
/// Composite reflectivity.
pub const COMPOSITE_LAYER: &str = "radar-composite";
/// 60-minute MESH hail size.
pub const HAIL_LAYER: &str = "hail-size";
/// 60-minute low-level rotation track.
pub const ROTATION_LAYER: &str = "rotation-track";
/// 18 dBZ echo top.
pub const ECHO_TOP_LAYER: &str = "echo-top";
/// 24-hour multi-sensor QPE.
pub const QPE_LAYER: &str = "precip-24h";

/// PrecipFlag colors reflectivity; it is checked, never rendered as a layer.
pub const PRECIP_FLAG_PREFIX: &str = "CONUS/PrecipFlag_00.00";

/// The six layers of a first-release case.
pub const CASE_LAYERS: [LayerSpec; 6] = [
    LayerSpec {
        layer: RADAR_LAYER,
        prefix: "CONUS/MergedBaseReflectivityQC_00.50",
        product: "MergedBaseReflectivityQC_00.50",
        color_key: "reflectivity",
        unit: "dBZ",
        cadence_s: 120,
        convert: identity,
    },
    LayerSpec {
        layer: COMPOSITE_LAYER,
        prefix: "CONUS/MergedReflectivityComposite_00.50",
        product: "MergedReflectivityComposite_00.50",
        color_key: "reflectivity",
        unit: "dBZ",
        cadence_s: 120,
        convert: identity,
    },
    LayerSpec {
        layer: HAIL_LAYER,
        prefix: "CONUS/MESH_Max_60min_00.50",
        product: "MESH_Max_60min_00.50",
        color_key: "hail_size",
        unit: "in",
        cadence_s: 120,
        convert: radar_ingest::mrms_qpe::qpe_mm_to_inches,
    },
    LayerSpec {
        layer: ROTATION_LAYER,
        prefix: "CONUS/RotationTrack60min_00.50",
        product: "RotationTrack60min_00.50",
        color_key: "rotation_track",
        unit: "1/s",
        cadence_s: 120,
        convert: radar_ingest::mrms_qpe::rotation_value,
    },
    LayerSpec {
        layer: ECHO_TOP_LAYER,
        prefix: "CONUS/EchoTop_18_00.50",
        product: "EchoTop_18_00.50",
        color_key: "echo_top",
        unit: "kft",
        cadence_s: 120,
        convert: radar_ingest::mrms_qpe::echo_top_km_to_kft,
    },
    LayerSpec {
        layer: QPE_LAYER,
        prefix: "CONUS/MultiSensor_QPE_24H_Pass1_00.00",
        product: "MultiSensor_QPE_24H_Pass1_00.00",
        color_key: "precip_accum",
        unit: "in",
        cadence_s: 3600,
        convert: radar_ingest::mrms_qpe::qpe_mm_to_inches,
    },
];

fn identity(v: f32) -> f32 {
    v
}

/// A storm case: a UTC half-open window and the bounds a replay may show.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct CaseDefinition {
    /// Stable slug used in the URL and the directory name.
    pub id: &'static str,
    /// Public title.
    pub title: &'static str,
    /// Factual one-line description.
    pub summary: &'static str,
    /// Inclusive UTC start, `%Y-%m-%dT%H:%M:%SZ`.
    pub start: &'static str,
    /// Exclusive UTC end, `%Y-%m-%dT%H:%M:%SZ`.
    pub end: &'static str,
    /// `[west, south, east, north]` in degrees.
    pub bounds: [f64; 4],
    /// Display timezone.
    pub timezone: &'static str,
}

/// Both first-release cases.
pub const CASES: [CaseDefinition; 2] = [
    CaseDefinition {
        id: "rolling-fork-2023",
        title: "Rolling Fork, Mississippi - March 24, 2023",
        summary: "EF4 tornado near Rolling Fork, Mississippi, overnight into March 25, 2023.",
        start: "2023-03-24T23:00:00Z",
        end: "2023-03-25T03:00:00Z",
        bounds: [-92.0, 31.0, -88.0, 35.0],
        timezone: "America/Chicago",
    },
    CaseDefinition {
        id: "greenfield-2024",
        title: "Greenfield, Iowa - May 21, 2024",
        summary: "EF4 tornado at Greenfield, Iowa, during the afternoon of May 21, 2024.",
        start: "2024-05-21T19:00:00Z",
        end: "2024-05-21T22:00:00Z",
        bounds: [-97.0, 40.0, -92.0, 44.0],
        timezone: "America/Chicago",
    },
];

/// Why a case definition was rejected.
#[derive(Debug, Error, PartialEq, Eq)]
pub enum CaseError {
    /// No case has this id.
    #[error("unknown case {0:?}; known cases: rolling-fork-2023, greenfield-2024")]
    Unknown(String),
    /// A field is outside its documented range.
    #[error("{case}: {field} {value} is invalid")]
    Field {
        /// Case id or `unknown`.
        case: String,
        /// Field name.
        field: &'static str,
        /// Rendered value.
        value: String,
    },
}

impl CaseDefinition {
    /// Parsed `[start, end)` UTC window. The end is exclusive.
    pub fn window(&self) -> Result<(DateTime<Utc>, DateTime<Utc>), CaseError> {
        let parse = |s: &str| {
            DateTime::parse_from_rfc3339(s)
                .map(|t| t.with_timezone(&Utc))
                .map_err(|_| CaseError::Field {
                    case: self.id.to_string(),
                    field: "window",
                    value: s.to_string(),
                })
        };
        Ok((parse(self.start)?, parse(self.end)?))
    }

    /// Validate the case before any network or disk work.
    pub fn validate(&self) -> Result<(), CaseError> {
        let bad = |field: &'static str, value: String| CaseError::Field {
            case: self.id.to_string(),
            field,
            value,
        };
        let slug_ok = !self.id.is_empty()
            && self
                .id
                .chars()
                .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
            && self.id.starts_with(|c: char| c.is_ascii_lowercase());
        if !slug_ok {
            return Err(bad("id", self.id.to_string()));
        }
        if self.timezone.is_empty() {
            return Err(bad("timezone", self.timezone.to_string()));
        }
        let [west, south, east, north] = self.bounds;
        if !(-180.0..=180.0).contains(&west) || !(-180.0..=180.0).contains(&east) {
            return Err(bad("bounds", format!("{self:?}")));
        }
        if !(-85.0..=85.0).contains(&south) || !(-85.0..=85.0).contains(&north) {
            return Err(bad("bounds", format!("{self:?}")));
        }
        if west >= east || south >= north {
            return Err(bad("bounds", format!("{self:?}")));
        }
        let (start, end) = self.window()?;
        let hours = (end - start).num_seconds();
        if hours <= 0 || hours > 24 * 3600 {
            return Err(bad("window", format!("{start}..{end}")));
        }
        Ok(())
    }

    /// The layer specifications a case imports.
    pub fn layers(&self) -> &'static [LayerSpec] {
        &CASE_LAYERS
    }
}

/// Look up and validate a case by id.
pub fn find(id: &str) -> Result<&'static CaseDefinition, CaseError> {
    let case = CASES
        .iter()
        .find(|c| c.id == id)
        .ok_or_else(|| CaseError::Unknown(id.to_string()))?;
    case.validate()?;
    Ok(case)
}

/// Layer spec by name.
pub fn layer(name: &str) -> Option<&'static LayerSpec> {
    CASE_LAYERS.iter().find(|l| l.layer == name)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_case_validates() {
        for case in &CASES {
            case.validate().unwrap_or_else(|e| panic!("{e}"));
        }
    }

    #[test]
    fn windows_are_utc_half_open_and_match_the_design() {
        let rf = find("rolling-fork-2023").unwrap();
        let (start, end) = rf.window().unwrap();
        assert_eq!(start.to_string(), "2023-03-24 23:00:00 UTC");
        assert_eq!(end.to_string(), "2023-03-25 03:00:00 UTC");
        assert_eq!(end - start, chrono::Duration::hours(4));
        let gf = find("greenfield-2024").unwrap();
        assert_eq!((gf.bounds, gf.timezone), ([-97.0, 40.0, -92.0, 44.0], "America/Chicago"));
    }

    #[test]
    fn invalid_definitions_are_rejected() {
        let mut case = CASES[0];
        case.bounds = [-88.0, 31.0, -92.0, 35.0];
        assert!(matches!(case.validate(), Err(CaseError::Field { field: "bounds", .. })));
        let mut case = CASES[0];
        case.end = "2023-03-24T23:00:00Z";
        assert!(matches!(case.validate(), Err(CaseError::Field { field: "window", .. })));
        let mut case = CASES[0];
        case.id = "Rolling Fork";
        assert!(matches!(case.validate(), Err(CaseError::Field { field: "id", .. })));
        assert!(matches!(find("joplin-2011"), Err(CaseError::Unknown(_))));
    }

    #[test]
    fn conversions_match_live_ingest() {
        let rot = layer(ROTATION_LAYER).unwrap();
        assert_eq!((rot.convert)(2500.0), 2.5);
        assert!((rot.convert)(-1.0).is_nan());
        let hail = layer(HAIL_LAYER).unwrap();
        assert!(((hail.convert)(25.4) - 1.0).abs() < 1e-6);
        let top = layer(ECHO_TOP_LAYER).unwrap();
        assert!((top.convert)(10.0) - 32.8084).abs() < 1e-3);
        let radar = layer(RADAR_LAYER).unwrap();
        assert_eq!((radar.convert)(35.0), 35.0);
        assert_eq!(CASE_LAYERS.len(), 6);
    }
}
