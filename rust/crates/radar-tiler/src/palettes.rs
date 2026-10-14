//! Palette loader and color-table parsing (spec-tiler section 6).
//!
//! The palette JSON files are embedded with `include_str!`, so the crate has no runtime
//! file dependency. Each palette file is an object whose non-underscore keys are color keys
//! (`reflectivity`, `temperature`, ...). Callers select one key and receive a [`ColorTable`].

use std::collections::BTreeMap;

use serde_json::Value;

use crate::error::{Result, TilerError};

const CLASSIC_JSON: &str = include_str!("../assets/classic.json");
const MUTED_JSON: &str = include_str!("../assets/muted.json");
const VIVID_JSON: &str = include_str!("../assets/vivid.json");
/// The legacy `color_tables.json`. Python uses it only as a fallback for `classic` when
/// `classic.json` is missing, which never happens with the shipped tree, so it is exposed
/// through [`legacy_color_tables`] and not used by [`load_palette`].
const LEGACY_JSON: &str = include_str!("../assets/color_tables.json");

/// Shipped palette names with their embedded documents.
pub const EMBEDDED_PALETTES: [(&str, &str); 3] = [
    ("classic", CLASSIC_JSON),
    ("muted", MUTED_JSON),
    ("vivid", VIVID_JSON),
];

/// Default `no_data_below` when a color table omits it (spec 5.1).
pub const DEFAULT_NO_DATA_BELOW: f64 = -999.0;

/// One `{min, max, rgba}` bin. Interval is half-open `[min, max)`.
#[derive(Debug, Clone, PartialEq)]
pub struct ColorRange {
    /// Inclusive lower bound.
    pub min: f64,
    /// Exclusive upper bound.
    pub max: f64,
    /// Non-premultiplied RGBA.
    pub rgba: [u8; 4],
}

/// One color key's table, as the renderer consumes it.
///
/// `raw` keeps the JSON object exactly as parsed (integers stay integers, `_comment` and
/// `unit` strings included), because the palette digest covers all of it (spec H2, H4).
#[derive(Debug, Clone)]
pub struct ColorTable {
    ranges: Option<Vec<ColorRange>>,
    categories: Option<BTreeMap<String, [u8; 4]>>,
    no_data_below: f64,
    raw: Value,
}

impl ColorTable {
    /// Parse one color-key object (`{"ranges": ..., "no_data_below": ...}` or `{"categories": ...}`).
    pub fn from_value(value: &Value) -> Result<ColorTable> {
        let obj = value
            .as_object()
            .ok_or_else(|| TilerError::Invalid("color table must be a JSON object".into()))?;
        let ranges = match obj.get("ranges") {
            None => None,
            Some(arr) => {
                let items = arr.as_array().ok_or_else(|| {
                    TilerError::Invalid("color table 'ranges' must be an array".into())
                })?;
                let mut out = Vec::with_capacity(items.len());
                for item in items {
                    let min = number(item.get("min"), "range min")?;
                    let max = number(item.get("max"), "range max")?;
                    out.push(ColorRange {
                        min,
                        max,
                        rgba: rgba(item.get("rgba"))?,
                    });
                }
                Some(out)
            }
        };
        let categories = match obj.get("categories") {
            None => None,
            Some(map) => {
                let entries = map.as_object().ok_or_else(|| {
                    TilerError::Invalid("color table 'categories' must be an object".into())
                })?;
                let mut out = BTreeMap::new();
                for (name, color) in entries {
                    out.insert(name.clone(), rgba(Some(color))?);
                }
                Some(out)
            }
        };
        let no_data_below = match obj.get("no_data_below") {
            None => DEFAULT_NO_DATA_BELOW,
            Some(v) => number(Some(v), "no_data_below")?,
        };
        Ok(ColorTable {
            ranges,
            categories,
            no_data_below,
            raw: value.clone(),
        })
    }

    /// Range bins in declaration order, or `None` for a categorical table.
    pub fn ranges(&self) -> Option<&[ColorRange]> {
        self.ranges.as_deref()
    }

    /// Category colors, or `None` for a range table.
    pub fn categories(&self) -> Option<&BTreeMap<String, [u8; 4]>> {
        self.categories.as_ref()
    }

    /// Values below this are transparent even if a range covers them.
    pub fn no_data_below(&self) -> f64 {
        self.no_data_below
    }

    /// The per-key JSON object exactly as stored in the palette file.
    pub fn raw(&self) -> &Value {
        &self.raw
    }
}

fn number(v: Option<&Value>, what: &str) -> Result<f64> {
    v.and_then(Value::as_f64)
        .ok_or_else(|| TilerError::Invalid(format!("{what} must be a number")))
}

fn rgba(v: Option<&Value>) -> Result<[u8; 4]> {
    let arr = v
        .and_then(Value::as_array)
        .filter(|a| a.len() == 4)
        .ok_or_else(|| TilerError::Invalid("rgba must be an array of four numbers".into()))?;
    let mut out = [0u8; 4];
    for (slot, channel) in out.iter_mut().zip(arr) {
        // Python truncates floats toward zero and rejects values outside 0..=255 (OverflowError).
        let t = channel
            .as_f64()
            .map(f64::trunc)
            .filter(|t| (0.0..=255.0).contains(t))
            .ok_or_else(|| {
                TilerError::Invalid(format!("rgba channel {channel} outside 0..=255"))
            })?;
        *slot = t as u8;
    }
    Ok(out)
}

/// A parsed palette file: its color keys, in file order, without top-level `_` keys.
#[derive(Debug, Clone)]
pub struct Palette {
    name: String,
    keys: Vec<(String, Value)>,
}

impl Palette {
    /// Palette name (the file stem).
    pub fn name(&self) -> &str {
        &self.name
    }

    /// Color keys in file order.
    pub fn color_keys(&self) -> impl Iterator<Item = &str> {
        self.keys.iter().map(|(k, _)| k.as_str())
    }

    /// Parse the color table for `key`.
    pub fn color_table(&self, key: &str) -> Result<ColorTable> {
        let value = self
            .keys
            .iter()
            .find(|(k, _)| k == key)
            .map(|(_, v)| v)
            .ok_or_else(|| {
                TilerError::Invalid(format!("palette {} has no color key {key:?}", self.name))
            })?;
        ColorTable::from_value(value)
    }
}

fn parse_palette(name: &str, text: &str) -> Result<Palette> {
    let doc: Value = serde_json::from_str(text)
        .map_err(|e| TilerError::Invalid(format!("palette {name} is not valid JSON: {e}")))?;
    let obj = doc
        .as_object()
        .ok_or_else(|| TilerError::Invalid(format!("palette {name} must be a JSON object")))?;
    let keys = obj
        .iter()
        .filter(|(k, _)| !k.starts_with('_'))
        .map(|(k, v)| (k.clone(), v.clone()))
        .collect();
    Ok(Palette {
        name: name.to_string(),
        keys,
    })
}

/// Reject palette names that could escape a directory (spec H13).
pub fn validate_palette_name(name: &str) -> Result<()> {
    if name.is_empty() || name.contains('/') || name.contains('\\') || name.contains("..") {
        return Err(TilerError::Invalid(format!(
            "invalid palette name {name:?}"
        )));
    }
    Ok(())
}

/// Palette names from a `PALETTES`-style comma-separated value: split on `,`, trim, drop
/// empties, default to `["classic"]`. Duplicates are kept, as in Python.
pub fn parse_palette_names(raw: Option<&str>) -> Vec<String> {
    let names: Vec<String> = raw
        .unwrap_or("classic")
        .split(',')
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string)
        .collect();
    if names.is_empty() {
        vec!["classic".to_string()]
    } else {
        names
    }
}

/// Palette names from the `PALETTES` environment variable (default `classic`).
pub fn palette_names() -> Vec<String> {
    parse_palette_names(std::env::var("PALETTES").ok().as_deref())
}

/// Load a shipped palette by name.
pub fn load_palette(name: &str) -> Result<Palette> {
    validate_palette_name(name)?;
    EMBEDDED_PALETTES
        .iter()
        .find(|(n, _)| *n == name)
        .map(|(n, text)| parse_palette(n, text))
        .unwrap_or_else(|| Err(TilerError::Invalid(format!("palette not found: {name}"))))
}

/// The legacy `color_tables.json` as a [`Palette`] (named `legacy`). Not used by the
/// renderer; it documents the fallback the Python loader would have used.
pub fn legacy_color_tables() -> Result<Palette> {
    parse_palette("legacy", LEGACY_JSON)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn palette_names_follow_python_rules() {
        assert_eq!(parse_palette_names(None), vec!["classic"]);
        assert_eq!(parse_palette_names(Some(" , ")), vec!["classic"]);
        assert_eq!(
            parse_palette_names(Some("muted, vivid,muted")),
            vec!["muted", "vivid", "muted"]
        );
    }

    #[test]
    fn bad_palette_names_are_rejected() {
        for bad in ["", "../classic", "a/b", "a\\b", "x..y"] {
            assert!(load_palette(bad).is_err(), "{bad}");
        }
        assert!(load_palette("nope").is_err());
    }

    #[test]
    fn shipped_palettes_load_and_skip_underscore_keys() {
        for (name, _) in EMBEDDED_PALETTES {
            let p = load_palette(name).unwrap();
            assert!(p.color_keys().all(|k| !k.starts_with('_')));
            assert!(p.color_keys().any(|k| k == "reflectivity"));
        }
        assert!(
            legacy_color_tables()
                .unwrap()
                .color_keys()
                .any(|k| k == "reflectivity")
        );
    }

    #[test]
    fn rgba_truncates_floats_and_rejects_overflow() {
        let t = ColorTable::from_value(&serde_json::json!({
            "ranges": [{"min": 0, "max": 1, "rgba": [127.9, 0, 255, 0.5]}]
        }))
        .unwrap();
        assert_eq!(t.ranges().unwrap()[0].rgba, [127, 0, 255, 0]);
        assert!(
            ColorTable::from_value(&serde_json::json!({
                "ranges": [{"min": 0, "max": 1, "rgba": [256, 0, 0, 0]}]
            }))
            .is_err()
        );
    }

    #[test]
    fn default_no_data_below_is_minus_999() {
        let t = ColorTable::from_value(&serde_json::json!({"ranges": []})).unwrap();
        assert_eq!(t.no_data_below(), -999.0);
    }
}
