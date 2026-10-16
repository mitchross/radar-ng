//! Precipitation-type coloring: a per-cell class plane (rain, snow, mix) that picks one of three
//! reflectivity ramps.
//!
//! The indexed renderer folds the class into the value before it classifies:
//! `typed = dBZ + class * TYPED_VALUE_STRIDE`, against a table whose snow and mix ranges are shifted
//! by the same stride ([`typed_color_table`]). Packed tiles store `class * TYPE_CODE_STRIDE + q`
//! instead (`docs/packed-tiles.md`, "Typed codes").

use std::fmt;
use std::sync::Arc;

use serde_json::{Value, json};

use crate::error::{Result, TilerError};
use crate::palettes::ColorTable;

/// Display classes in class-index order. A higher index wins when overview pixels are max-pooled.
pub const TYPE_CLASSES: [&str; 3] = ["rain", "snow", "mix"];
/// Rain, and every cell without a known type.
pub const TYPE_RAIN: u8 = 0;
/// Snow.
pub const TYPE_SNOW: u8 = 1;
/// Mixed precipitation.
pub const TYPE_MIX: u8 = 2;
/// Offset between the class ramps in the typed value space. Larger than any reflectivity.
pub const TYPED_VALUE_STRIDE: f32 = 100.0;
/// Codes per class in a typed packed slot; `q` is clamped to `TYPE_CODE_STRIDE - 1`.
pub const TYPE_CODE_STRIDE: u8 = 32;

/// One class index per grid cell, row-major, the same shape as the frame's values.
#[derive(Clone, PartialEq, Eq)]
pub struct TypePlane {
    classes: Arc<[u8]>,
}

impl TypePlane {
    /// Wrap per-cell classes. Values above [`TYPE_MIX`] are rejected.
    pub fn new(classes: Vec<u8>) -> Result<Self> {
        if classes.iter().any(|&c| c > TYPE_MIX) {
            return Err(TilerError::Invalid(format!(
                "type classes must be 0..={TYPE_MIX}"
            )));
        }
        Ok(Self {
            classes: classes.into(),
        })
    }

    /// The per-cell classes.
    pub fn classes(&self) -> &[u8] {
        &self.classes
    }

    /// Number of cells.
    pub fn len(&self) -> usize {
        self.classes.len()
    }

    /// `true` when the plane has no cells.
    pub fn is_empty(&self) -> bool {
        self.classes.is_empty()
    }
}

impl fmt::Debug for TypePlane {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "TypePlane({} cells)", self.classes.len())
    }
}

/// Fold per-pixel classes into sampled values: `v + class * stride` for finite `v`.
pub fn apply_types(values: &mut [f32], classes: &[u8]) {
    for (v, &c) in values.iter_mut().zip(classes) {
        if v.is_finite() {
            *v += f32::from(c) * TYPED_VALUE_STRIDE;
        }
    }
}

/// The ranges of `class` in a reflectivity table: `ranges` for rain, `types.<class>.ranges` for the
/// others. A table without that class reuses its rain ranges.
pub fn class_ranges(table: &ColorTable, class: u8) -> Value {
    let raw = table.raw();
    let rain = raw.get("ranges").cloned().unwrap_or_else(|| json!([]));
    if class == TYPE_RAIN {
        return rain;
    }
    raw.get("types")
        .and_then(|t| t.get(TYPE_CLASSES[usize::from(class)]))
        .and_then(|t| t.get("ranges"))
        .cloned()
        .unwrap_or(rain)
}

/// The color table the indexed renderer uses for typed frames: rain, snow and mix ranges, each
/// shifted by `class * TYPED_VALUE_STRIDE`. `no_data_below` applies to the rain ramp only.
pub fn typed_color_table(table: &ColorTable) -> Result<ColorTable> {
    let mut ranges = Vec::new();
    for class in [TYPE_RAIN, TYPE_SNOW, TYPE_MIX] {
        let shift = f64::from(class) * f64::from(TYPED_VALUE_STRIDE);
        let items = class_ranges(table, class);
        let items = items
            .as_array()
            .ok_or_else(|| TilerError::Invalid("color table ranges must be an array".into()))?;
        for r in items {
            let bound = |k: &str| {
                r.get(k)
                    .and_then(Value::as_f64)
                    .ok_or_else(|| TilerError::Invalid(format!("range {k} must be a number")))
            };
            ranges.push(json!({
                "min": bound("min")? + shift,
                "max": bound("max")? + shift,
                "rgba": r.get("rgba").cloned().unwrap_or(Value::Null),
            }));
        }
    }
    let mut doc = json!({"typed": TYPE_CLASSES, "ranges": ranges});
    if let Some(nd) = table.raw().get("no_data_below") {
        doc["no_data_below"] = nd.clone();
    }
    if let Some(unit) = table.raw().get("unit") {
        doc["unit"] = unit.clone();
    }
    ColorTable::from_value(&doc)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn table() -> ColorTable {
        ColorTable::from_value(&json!({
            "unit": "dBZ",
            "ranges": [{"min": 5, "max": 40, "rgba": [0, 200, 0, 200]}],
            "no_data_below": 5,
            "types": {
                "snow": {"ranges": [{"min": 5, "max": 40, "rgba": [0, 0, 255, 200]}]},
                "mix": {"ranges": [{"min": 5, "max": 40, "rgba": [255, 0, 255, 200]}]}
            }
        }))
        .unwrap()
    }

    #[test]
    fn typed_table_shifts_each_ramp_by_the_stride() {
        let t = typed_color_table(&table()).unwrap();
        let mins: Vec<f64> = t.ranges().unwrap().iter().map(|r| r.min).collect();
        assert_eq!(mins, [5.0, 105.0, 205.0]);
        assert_eq!(t.ranges().unwrap()[1].rgba, [0, 0, 255, 200]);
        assert_eq!(t.no_data_below(), 5.0);
    }

    #[test]
    fn missing_type_ramps_reuse_rain() {
        let plain = ColorTable::from_value(&json!({
            "ranges": [{"min": 5, "max": 40, "rgba": [0, 200, 0, 200]}]
        }))
        .unwrap();
        let t = typed_color_table(&plain).unwrap();
        assert_eq!(t.ranges().unwrap()[2].rgba, [0, 200, 0, 200]);
    }

    #[test]
    fn apply_types_leaves_missing_values_alone() {
        let mut v = [20.0, f32::NAN, 30.0];
        apply_types(&mut v, &[TYPE_SNOW, TYPE_MIX, TYPE_RAIN]);
        assert_eq!(v[0], 120.0);
        assert!(v[1].is_nan());
        assert_eq!(v[2], 30.0);
        assert!(TypePlane::new(vec![3]).is_err());
    }
}
