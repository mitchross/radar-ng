//! Row-major 2-D buffers and NaN-aware helpers.
//!
//! A [`Field2`] is a `width x height` grid stored row-major (`index = y * width + x`).
//! Rows are `y` (latitude index, north-first in the radar-ng convention) and columns
//! are `x`. The numerics in this crate never assume a geographic meaning; they only
//! need the shape and the values.

use thiserror::Error;

/// Errors raised while building a [`Field2`].
#[derive(Debug, Error, Clone, PartialEq, Eq)]
pub enum FieldError {
    /// The data length does not equal `width * height`.
    #[error("field data length {len} does not match {width}x{height}")]
    LengthMismatch {
        /// Requested width.
        width: usize,
        /// Requested height.
        height: usize,
        /// Length of the supplied buffer.
        len: usize,
    },
}

/// A row-major `width x height` grid.
#[derive(Clone, Debug, PartialEq)]
pub struct Field2<T = f32> {
    width: usize,
    height: usize,
    data: Vec<T>,
}

impl<T: Copy> Field2<T> {
    /// Wrap an existing buffer. `data.len()` must equal `width * height`.
    pub fn new(width: usize, height: usize, data: Vec<T>) -> Result<Self, FieldError> {
        if data.len() != width * height {
            return Err(FieldError::LengthMismatch {
                width,
                height,
                len: data.len(),
            });
        }
        Ok(Self {
            width,
            height,
            data,
        })
    }

    /// A field with every cell set to `value`.
    pub fn filled(width: usize, height: usize, value: T) -> Self {
        Self {
            width,
            height,
            data: vec![value; width * height],
        }
    }

    /// Number of columns.
    #[inline]
    pub fn width(&self) -> usize {
        self.width
    }

    /// Number of rows.
    #[inline]
    pub fn height(&self) -> usize {
        self.height
    }

    /// Total number of cells (`width * height`).
    #[inline]
    pub fn len(&self) -> usize {
        self.data.len()
    }

    /// True when the field has no cells.
    #[inline]
    pub fn is_empty(&self) -> bool {
        self.data.is_empty()
    }

    /// Row-major view of the values.
    #[inline]
    pub fn as_slice(&self) -> &[T] {
        &self.data
    }

    /// Mutable row-major view of the values.
    #[inline]
    pub fn as_mut_slice(&mut self) -> &mut [T] {
        &mut self.data
    }

    /// Consume the field and return its buffer.
    pub fn into_vec(self) -> Vec<T> {
        self.data
    }

    /// Value at column `x`, row `y`. Panics when out of range.
    #[inline]
    pub fn get(&self, x: usize, y: usize) -> T {
        self.data[y * self.width + x]
    }

    /// Row `y` as a slice.
    #[inline]
    pub fn row(&self, y: usize) -> &[T] {
        &self.data[y * self.width..(y + 1) * self.width]
    }
}

impl Field2<f32> {
    /// Convert to `f64` storage (exact).
    pub fn to_f64(&self) -> Field2<f64> {
        Field2 {
            width: self.width,
            height: self.height,
            data: self.data.iter().map(|&v| v as f64).collect(),
        }
    }

    /// Replace every value strictly below `threshold` with NaN, as radar-ng does for
    /// the fill and MRMS sentinel values (`stack < -100 -> NaN`). NaN stays NaN.
    pub fn mask_below(&mut self, threshold: f32) {
        for v in &mut self.data {
            if *v < threshold {
                *v = f32::NAN;
            }
        }
    }

    /// Smallest non-NaN value, or `None` when every value is NaN.
    pub fn nan_min(&self) -> Option<f32> {
        nan_min_f32(&self.data)
    }

    /// Count of cells that are finite (neither NaN nor infinite).
    pub fn count_finite(&self) -> usize {
        self.data.iter().filter(|v| v.is_finite()).count()
    }
}

impl Field2<f64> {
    /// Narrow to `f32` storage, as numpy's `astype(float32)` does (round to nearest).
    pub fn to_f32(&self) -> Field2<f32> {
        Field2 {
            width: self.width,
            height: self.height,
            data: self.data.iter().map(|&v| v as f32).collect(),
        }
    }
}

/// Smallest non-NaN value of a slice, or `None` if there is none (`np.nanmin`).
pub fn nan_min_f64(values: &[f64]) -> Option<f64> {
    let mut best: Option<f64> = None;
    for &v in values {
        if v.is_nan() {
            continue;
        }
        best = Some(match best {
            Some(b) if b <= v => b,
            _ => v,
        });
    }
    best
}

/// Smallest non-NaN value of a slice, or `None` if there is none.
pub fn nan_min_f32(values: &[f32]) -> Option<f32> {
    let mut best: Option<f32> = None;
    for &v in values {
        if v.is_nan() {
            continue;
        }
        best = Some(match best {
            Some(b) if b <= v => b,
            _ => v,
        });
    }
    best
}

/// NaN-propagating maximum, matching `np.maximum` (any NaN operand gives NaN).
#[inline]
pub fn nan_max(a: f64, b: f64) -> f64 {
    if a.is_nan() || b.is_nan() {
        f64::NAN
    } else if a >= b {
        a
    } else {
        b
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn length_is_checked() {
        assert!(Field2::<f32>::new(2, 3, vec![0.0; 6]).is_ok());
        assert_eq!(
            Field2::<f32>::new(2, 3, vec![0.0; 5]),
            Err(FieldError::LengthMismatch {
                width: 2,
                height: 3,
                len: 5
            })
        );
    }

    #[test]
    fn mask_below_is_strict_and_keeps_nan() {
        let mut f = Field2::new(4, 1, vec![-100.0f32, -100.5, f32::NAN, 7.0]).unwrap();
        f.mask_below(-100.0);
        let s = f.as_slice();
        assert_eq!(s[0], -100.0); // not strictly below
        assert!(s[1].is_nan());
        assert!(s[2].is_nan());
        assert_eq!(s[3], 7.0);
    }

    #[test]
    fn nan_min_skips_nan_and_handles_all_nan() {
        assert_eq!(nan_min_f64(&[f64::NAN, 3.0, -1.0, f64::NAN]), Some(-1.0));
        assert_eq!(nan_min_f64(&[f64::NAN]), None);
        assert_eq!(nan_min_f32(&[2.0, f32::NAN]), Some(2.0));
    }

    #[test]
    fn nan_max_propagates() {
        assert!(nan_max(f64::NAN, 1.0).is_nan());
        assert!(nan_max(1.0, f64::NAN).is_nan());
        assert_eq!(nan_max(1.0, 2.0), 2.0);
    }
}
