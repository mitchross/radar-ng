//! Borrowed view of a regular lat/lon reflectivity grid.

use crate::error::StormsError;

/// A row-major `height x width` f32 grid with its 1-D latitude and longitude axes.
///
/// `lats[0]` is the position of row 0 and `lats[height - 1]` the position of the
/// last row. The axes may be ascending or descending. Longitudes above 180 are
/// normalized inside the detector (spec-shared 6.3 step 5).
#[derive(Debug, Clone, Copy)]
pub struct Grid<'a> {
    values: &'a [f32],
    height: usize,
    width: usize,
    lats: &'a [f64],
    lons: &'a [f64],
}

impl<'a> Grid<'a> {
    /// Validate the buffer lengths and build the view.
    ///
    /// # Errors
    /// Returns [`StormsError::InvalidGrid`] when `values.len() != height * width`,
    /// `lats.len() != height` or `lons.len() != width`.
    pub fn new(
        values: &'a [f32],
        height: usize,
        width: usize,
        lats: &'a [f64],
        lons: &'a [f64],
    ) -> Result<Self, StormsError> {
        let expected = height
            .checked_mul(width)
            .ok_or_else(|| StormsError::InvalidGrid("height * width overflows".into()))?;
        if values.len() != expected {
            return Err(StormsError::InvalidGrid(format!(
                "values has {} elements, expected {height} x {width}",
                values.len()
            )));
        }
        if lats.len() != height {
            return Err(StormsError::InvalidGrid(format!(
                "lats has {} entries, expected {height}",
                lats.len()
            )));
        }
        if lons.len() != width {
            return Err(StormsError::InvalidGrid(format!(
                "lons has {} entries, expected {width}",
                lons.len()
            )));
        }
        Ok(Self {
            values,
            height,
            width,
            lats,
            lons,
        })
    }

    /// Number of rows.
    #[must_use]
    pub fn height(&self) -> usize {
        self.height
    }

    /// Number of columns.
    #[must_use]
    pub fn width(&self) -> usize {
        self.width
    }

    /// Row-major values.
    #[must_use]
    pub fn values(&self) -> &'a [f32] {
        self.values
    }

    /// Row latitudes in degrees.
    #[must_use]
    pub fn lats(&self) -> &'a [f64] {
        self.lats
    }

    /// Column longitudes in degrees (as given, before normalization).
    #[must_use]
    pub fn lons(&self) -> &'a [f64] {
        self.lons
    }
}
