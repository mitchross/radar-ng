//! Value-to-class models shared by every palette (spec-tiler section 5).
//!
//! A [`ClassModel`] maps physical values (or category ids) to one `u8` class index. Class 0
//! is transparent in every palette, so `idx.any()` means "has visible content". The per-palette
//! RGBA lookup tables (LUTs) turn class indices into colors.

use std::collections::BTreeMap;

use crate::error::{Result, TilerError};
use crate::palettes::ColorTable;

/// One palette's lookup table over the shared class indices.
#[derive(Debug, Clone)]
pub struct PaletteLut {
    name: String,
    rgba: Vec<[u8; 4]>,
    plte: Vec<u8>,
    trns: Vec<u8>,
}

impl PaletteLut {
    fn new(name: &str, rgba: Vec<[u8; 4]>) -> PaletteLut {
        let plte = rgba.iter().flat_map(|c| [c[0], c[1], c[2]]).collect();
        let trns = rgba.iter().map(|c| c[3]).collect();
        PaletteLut {
            name: name.to_string(),
            rgba,
            plte,
            trns,
        }
    }

    /// Palette name.
    pub fn name(&self) -> &str {
        &self.name
    }

    /// RGBA rows, row 0 = class 0 (transparent).
    pub fn rgba(&self) -> &[[u8; 4]] {
        &self.rgba
    }

    /// PNG PLTE payload: 3 bytes per class, not padded to 256 entries.
    pub fn plte(&self) -> &[u8] {
        &self.plte
    }

    /// PNG tRNS payload: one alpha byte per class.
    pub fn trns(&self) -> &[u8] {
        &self.trns
    }
}

/// Shared class index plus per-palette LUTs.
///
/// `edges` is `None` for categorical models. Continuous models store the strictly increasing
/// bin edges (f64) and, when the f32 cast is still strictly increasing, an f32 copy used for
/// comparisons against f32 samples (spec 5.3 precision rule).
#[derive(Debug, Clone)]
pub struct ClassModel {
    edges: Option<Vec<f64>>,
    edges_f32: Option<Vec<f32>>,
    remap: Vec<u8>,
    luts: Vec<PaletteLut>,
}

impl ClassModel {
    /// Bin edges for continuous models; `None` for categorical models.
    pub fn edges(&self) -> Option<&[f64]> {
        self.edges.as_deref()
    }

    /// Raw index (edge count or category value) to class index.
    pub fn remap(&self) -> &[u8] {
        &self.remap
    }

    /// Number of classes including the transparent class 0 (`K + 1`).
    pub fn n_classes(&self) -> usize {
        self.luts.first().map_or(1, |l| l.rgba.len())
    }

    /// `true` for category models (`edges` is absent).
    pub fn is_categorical(&self) -> bool {
        self.edges.is_none()
    }

    /// Palette names in model order.
    pub fn palette_names(&self) -> impl Iterator<Item = &str> {
        self.luts.iter().map(PaletteLut::name)
    }

    /// Lookup table for `name`.
    pub fn lut(&self, name: &str) -> Option<&PaletteLut> {
        self.luts.iter().find(|l| l.name == name)
    }

    /// Classify continuous values into `dst` (same length). NaN and +/-inf map to class 0.
    ///
    /// Errors when the model is categorical.
    pub fn classify_f32_into(&self, src: &[f32], dst: &mut [u8]) -> Result<()> {
        let Some(edges) = self.edges.as_deref() else {
            return Err(TilerError::Invalid(
                "continuous classify needs a range class model".into(),
            ));
        };
        debug_assert_eq!(src.len(), dst.len());
        if let Some(e32) = self.edges_f32.as_deref() {
            for (s, d) in src.iter().zip(dst.iter_mut()) {
                *d = if s.is_finite() {
                    self.remap[e32.partition_point(|&e| e <= *s)]
                } else {
                    0
                };
            }
        } else {
            for (s, d) in src.iter().zip(dst.iter_mut()) {
                *d = if s.is_finite() {
                    let v = f64::from(*s);
                    self.remap[edges.partition_point(|&e| e <= v)]
                } else {
                    0
                };
            }
        }
        Ok(())
    }

    /// Classify category ids (stored as f32) into `dst`. Negative, unknown and NaN ids map to 0.
    ///
    /// Errors when the model is continuous.
    pub fn classify_category_into(&self, src: &[f32], dst: &mut [u8]) -> Result<()> {
        if !self.is_categorical() {
            return Err(TilerError::Invalid(
                "categorical classify needs a category class model".into(),
            ));
        }
        debug_assert_eq!(src.len(), dst.len());
        let top = self.remap.len() as i64 - 1;
        for (s, d) in src.iter().zip(dst.iter_mut()) {
            let v = if s.is_nan() { 0 } else { *s as i64 };
            *d = if v < 0 || v > top {
                0
            } else {
                self.remap[v as usize]
            };
        }
        Ok(())
    }
}

/// Collapse raw per-palette LUTs so class 0 is the only class transparent everywhere
/// (spec 5.1 step 4). Visible raw indices keep ascending order and are renumbered `1..=K`.
fn collapse(
    names: &[&str],
    raw: &[Vec<[u8; 4]>],
    raw_n: usize,
) -> Result<(Vec<u8>, Vec<PaletteLut>)> {
    let mut visible = vec![false; raw_n];
    for lut in raw {
        for (k, c) in lut.iter().enumerate() {
            visible[k] |= c[3] > 0;
        }
    }
    let keep: Vec<usize> = (0..raw_n).filter(|&k| visible[k]).collect();
    if keep.len() > 255 {
        return Err(TilerError::Invalid(format!(
            "{} visible classes exceed the uint8 palette budget",
            keep.len()
        )));
    }
    let mut remap = vec![0u8; raw_n];
    for (class, &k) in keep.iter().enumerate() {
        remap[k] = (class + 1) as u8;
    }
    let luts = names
        .iter()
        .zip(raw)
        .map(|(name, lut)| {
            let mut rgba = vec![[0u8; 4]; keep.len() + 1];
            for (class, &k) in keep.iter().enumerate() {
                rgba[class + 1] = lut[k];
            }
            PaletteLut::new(name, rgba)
        })
        .collect();
    Ok((remap, luts))
}

/// Build one shared continuous class model from `(palette, table)` pairs (spec 5.1).
///
/// Edges are the sorted unique set of every range `min`/`max` and every `no_data_below`.
/// Overlapping ranges: the later one wins. Values below `no_data_below` stay transparent.
pub fn build_class_model(tables: &[(String, ColorTable)]) -> Result<ClassModel> {
    if tables.is_empty() {
        return Err(TilerError::Invalid(
            "at least one color table is required".into(),
        ));
    }
    let mut edges: Vec<f64> = Vec::new();
    for (name, table) in tables {
        let ranges = table.ranges().ok_or_else(|| {
            TilerError::Invalid(format!(
                "palette {name} has no 'ranges' for a continuous model"
            ))
        })?;
        for r in ranges {
            edges.push(r.min);
            edges.push(r.max);
        }
        edges.push(table.no_data_below());
    }
    if edges.iter().any(|e| e.is_nan()) {
        return Err(TilerError::Invalid(
            "color table edges must not be NaN".into(),
        ));
    }
    edges.sort_by(|a, b| a.partial_cmp(b).expect("NaN rejected above"));
    edges.dedup();
    let n_e = edges.len();
    let raw_n = n_e + 1;

    let mut raw: Vec<Vec<[u8; 4]>> = Vec::with_capacity(tables.len());
    let names: Vec<&str> = tables.iter().map(|(n, _)| n.as_str()).collect();
    for (_, table) in tables {
        let mut lut = vec![[0u8; 4]; raw_n];
        let no_data = table.no_data_below();
        let ranges = table.ranges().unwrap_or(&[]);
        for i in 1..n_e {
            let (lo, hi) = (edges[i - 1], edges[i]);
            if lo < no_data {
                continue;
            }
            for r in ranges {
                if r.min <= lo && hi <= r.max {
                    lut[i] = r.rgba;
                }
            }
        }
        raw.push(lut);
    }
    let (remap, luts) = collapse(&names, &raw, raw_n)?;
    let edges_f32: Vec<f32> = edges.iter().map(|&e| e as f32).collect();
    let edges_f32 = edges_f32
        .windows(2)
        .all(|w| w[0] < w[1])
        .then_some(edges_f32);
    Ok(ClassModel {
        edges: Some(edges),
        edges_f32,
        remap,
        luts,
    })
}

/// Build a categorical class model: `category_map` maps integer ids to names, and each table
/// is `{"categories": {name: rgba}}` (spec 5.2). Unknown names are dropped silently.
pub fn build_categorical_class_model(
    tables: &[(String, ColorTable)],
    category_map: &BTreeMap<i64, String>,
) -> Result<ClassModel> {
    if tables.is_empty() {
        return Err(TilerError::Invalid(
            "at least one color table is required".into(),
        ));
    }
    let max_id = category_map.keys().copied().filter(|&v| v >= 0).max();
    let raw_n = max_id.map_or(0, |m| m as usize) + 1;
    let names: Vec<&str> = tables.iter().map(|(n, _)| n.as_str()).collect();
    let mut raw = Vec::with_capacity(tables.len());
    for (name, table) in tables {
        let categories = table
            .categories()
            .ok_or_else(|| TilerError::Invalid(format!("palette {name} has no 'categories'")))?;
        let mut lut = vec![[0u8; 4]; raw_n];
        for (&value, cat) in category_map {
            if value >= 0
                && let Some(color) = categories.get(cat)
            {
                lut[value as usize] = *color;
            }
        }
        raw.push(lut);
    }
    let (remap, luts) = collapse(&names, &raw, raw_n)?;
    Ok(ClassModel {
        edges: None,
        edges_f32: None,
        remap,
        luts,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn table(v: serde_json::Value) -> ColorTable {
        ColorTable::from_value(&v).unwrap()
    }

    #[test]
    fn apply_color_table_semantics_via_model() {
        // Spec test_apply_color_table: [5,20) green, [20,40) red, no_data_below=5.
        let t = table(json!({"ranges": [
            {"min": 5, "max": 20, "rgba": [0, 255, 0, 255]},
            {"min": 20, "max": 40, "rgba": [255, 0, 0, 255]}], "no_data_below": 5}));
        let m = build_class_model(&[("p".into(), t)]).unwrap();
        let vals = [3.0f32, 10.0, 25.0, 50.0];
        let mut idx = [0u8; 4];
        m.classify_f32_into(&vals, &mut idx).unwrap();
        let lut = m.lut("p").unwrap().rgba();
        let colors: Vec<[u8; 4]> = idx.iter().map(|&i| lut[i as usize]).collect();
        assert_eq!(
            colors,
            vec![
                [0, 0, 0, 0],
                [0, 255, 0, 255],
                [255, 0, 0, 255],
                [0, 0, 0, 0]
            ]
        );
    }

    #[test]
    fn later_overlapping_range_wins_and_gaps_are_transparent() {
        let t = table(json!({"ranges": [
            {"min": 0, "max": 20, "rgba": [1, 0, 0, 255]},
            {"min": 10, "max": 30, "rgba": [0, 1, 0, 255]}], "no_data_below": -999}));
        let m = build_class_model(&[("p".into(), t)]).unwrap();
        let vals = [5.0f32, 15.0, 25.0, 35.0];
        let mut idx = [0u8; 4];
        m.classify_f32_into(&vals, &mut idx).unwrap();
        let lut = m.lut("p").unwrap().rgba();
        let colors: Vec<[u8; 4]> = idx.iter().map(|&i| lut[i as usize]).collect();
        assert_eq!(colors[0], [1, 0, 0, 255]);
        assert_eq!(colors[1], [0, 1, 0, 255]);
        assert_eq!(colors[2], [0, 1, 0, 255]);
        assert_eq!(colors[3], [0, 0, 0, 0]);
    }

    #[test]
    fn palettes_with_different_edges_share_one_index() {
        let a = table(json!({"ranges": [
            {"min": 0, "max": 10, "rgba": [1, 1, 1, 255]},
            {"min": 10, "max": 20, "rgba": [2, 2, 2, 255]}], "no_data_below": 0}));
        let b = table(
            json!({"ranges": [{"min": 5, "max": 15, "rgba": [3, 3, 3, 255]}], "no_data_below": 5}),
        );
        let m = build_class_model(&[("a".into(), a), ("b".into(), b)]).unwrap();
        let vals = [-1.0f32, 0.0, 4.9, 5.0, 9.9, 10.0, 14.9, 15.0, 19.9, 20.0];
        let mut idx = [0u8; 10];
        m.classify_f32_into(&vals, &mut idx).unwrap();
        assert_eq!(idx[0], 0);
        assert_eq!(idx[9], 0);
        let la = m.lut("a").unwrap().rgba();
        let lb = m.lut("b").unwrap().rgba();
        assert_eq!(la[idx[4] as usize], [1, 1, 1, 255]);
        assert_eq!(lb[idx[4] as usize], [3, 3, 3, 255]);
        assert_eq!(lb[idx[1] as usize], [0, 0, 0, 0]); // below b's no_data_below
    }

    #[test]
    fn float32_edges_follow_the_precision_rule() {
        // 0.1 is not exact in f32; the edge comparison must still match `v >= 0.1` in f32.
        let t = table(
            json!({"ranges": [{"min": 0.1, "max": 0.2, "rgba": [9, 9, 9, 255]}], "no_data_below": -1}),
        );
        let m = build_class_model(&[("p".into(), t)]).unwrap();
        let edge = 0.1f64 as f32;
        let mut idx = [0u8; 2];
        m.classify_f32_into(&[edge, edge.next_down()], &mut idx)
            .unwrap();
        assert_ne!(idx[0], 0);
        assert_eq!(idx[1], 0);
    }

    #[test]
    fn non_finite_values_are_class_zero() {
        let t = table(
            json!({"ranges": [{"min": 0, "max": 1, "rgba": [9, 9, 9, 255]}], "no_data_below": 0}),
        );
        let m = build_class_model(&[("p".into(), t)]).unwrap();
        let mut idx = [7u8; 3];
        m.classify_f32_into(&[f32::NAN, f32::INFINITY, f32::NEG_INFINITY], &mut idx)
            .unwrap();
        assert_eq!(idx, [0, 0, 0]);
    }

    #[test]
    fn categorical_model_drops_unknown_names_and_maps_out_of_range_to_zero() {
        let t = table(json!({"categories": {"rain": [0, 255, 0, 255], "snow": [0, 0, 255, 255]}}));
        let cmap: BTreeMap<i64, String> = [(1, "rain"), (2, "snow"), (3, "sleet")]
            .into_iter()
            .map(|(k, v)| (k, v.to_string()))
            .collect();
        let m = build_categorical_class_model(&[("p".into(), t)], &cmap).unwrap();
        let mut idx = [9u8; 6];
        m.classify_category_into(&[-1.0, 0.0, 1.0, 2.0, 3.0, 99.0], &mut idx)
            .unwrap();
        let lut = m.lut("p").unwrap().rgba();
        assert_eq!(lut[idx[2] as usize], [0, 255, 0, 255]);
        assert_eq!(lut[idx[3] as usize], [0, 0, 255, 255]);
        assert_eq!(idx[0], 0);
        assert_eq!(idx[1], 0);
        assert_eq!(idx[4], 0, "sleet is unknown, so it is transparent");
        assert_eq!(idx[5], 0);
    }

    #[test]
    fn categorical_and_continuous_entry_points_refuse_the_other_kind() {
        let t = table(json!({"ranges": [{"min": 0, "max": 1, "rgba": [9, 9, 9, 255]}]}));
        let m = build_class_model(&[("p".into(), t)]).unwrap();
        assert!(m.classify_category_into(&[0.0], &mut [0]).is_err());
    }

    #[test]
    fn budget_overflow_is_an_error() {
        // 300 distinct visible bins exceed the 255-class budget.
        let ranges: Vec<_> = (0..300)
            .map(|i| json!({"min": i, "max": i + 1, "rgba": [1, 2, 3, 255]}))
            .collect();
        let t = table(json!({"ranges": ranges, "no_data_below": -1}));
        let err = build_class_model(&[("p".into(), t)]).unwrap_err();
        assert!(err.to_string().contains("uint8 palette budget"));
    }
}
