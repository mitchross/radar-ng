//! Sampling kernels (spec-tiler section 4).
//!
//! * [`sample_nearest_into`] gathers class indices (never interpolated).
//! * [`sample_bilinear_into`] samples physical values with the exact arithmetic of spec 4.2:
//!   f32 storage, f64 weights, and f32 accumulator stores in the stated order.
//! * [`max_block_factors`], [`footprint_edges`] and the reducers implement the max-pooled
//!   overview of spec 4.4.

use crate::error::{Result, TilerError};
use crate::geometry::{AxisMap, GeometryBody, TileGeometry};

/// Nearest-neighbour gather of a `h x w` class index into a `tile x tile` output.
///
/// `idx` is row-major with width `w`. Invalid coordinates produce class 0.
pub fn sample_nearest_into(
    geom: &TileGeometry,
    idx: &[u8],
    w: usize,
    tile: usize,
    out: &mut [u8],
) -> Result<()> {
    debug_assert_eq!(out.len(), tile * tile);
    match &geom.body {
        GeometryBody::RegularNearest { rows, cols } => {
            for (i, &r) in rows.iter().enumerate() {
                let row_out = &mut out[i * tile..(i + 1) * tile];
                if r < 0 {
                    row_out.fill(0);
                    continue;
                }
                let src_row = &idx[r as usize * w..r as usize * w + w];
                for (&c, dst) in cols.iter().zip(row_out.iter_mut()) {
                    *dst = if c < 0 { 0 } else { src_row[c as usize] };
                }
            }
            Ok(())
        }
        GeometryBody::ProjectedNearest { flat } => {
            for (dst, &k) in out.iter_mut().zip(flat) {
                *dst = if k < 0 { 0 } else { idx[k as usize] };
            }
            Ok(())
        }
        _ => Err(TilerError::Invalid(
            "nearest sampling requires nearest tile geometry".into(),
        )),
    }
}

/// Fractional-index tap for one axis of a bilinear sample (spec 4.2 steps 1-4).
#[derive(Clone, Copy)]
struct Tap {
    lo: usize,
    hi: usize,
    frac: f64,
    in_range: bool,
}

#[inline]
fn tap(coord: f32, size: usize) -> Tap {
    let last = size - 1;
    let in_range = coord >= 0.0 && coord <= last as f32;
    let floor = coord.floor() as i32;
    let frac = f64::from(coord) - f64::from(floor);
    let lo = floor.clamp(0, last as i32) as usize;
    let hi = (floor + 1).clamp(0, last as i32) as usize;
    Tap {
        lo,
        hi,
        frac,
        in_range,
    }
}

/// One output pixel of the bilinear kernel (spec 4.2, steps 5-9).
#[inline]
fn bilinear_pixel(
    values: &[f32],
    w: usize,
    r: Tap,
    c: Tap,
    nodata: Option<f32>,
    threshold: f32,
) -> f32 {
    let (rf, cf) = (r.frac, c.frac);
    let corners = [
        (values[r.lo * w + c.lo], (1.0 - rf) * (1.0 - cf)),
        (values[r.lo * w + c.hi], (1.0 - rf) * cf),
        (values[r.hi * w + c.lo], rf * (1.0 - cf)),
        (values[r.hi * w + c.hi], rf * cf),
    ];
    let mut numerator = 0.0f32;
    let mut valid_weight = 0.0f32;
    for (s, weight) in corners {
        let valid = s.is_finite() && nodata.is_none_or(|nd| s != nd);
        let sv = if valid { f64::from(s) } else { 0.0 };
        numerator = (f64::from(numerator) + sv * weight) as f32;
        let vw = if valid { 1.0 } else { 0.0 };
        valid_weight = (f64::from(valid_weight) + vw * weight) as f32;
    }
    if r.in_range && c.in_range && valid_weight >= threshold {
        numerator / valid_weight
    } else {
        f32::NAN
    }
}

/// Bilinear physical sampling of a `h x w` f32 grid into a `tile x tile` f32 output (spec 4.2).
///
/// `nodata` is applied only when finite. A pixel is emitted (not NaN) when the valid
/// weight reaches `min_valid_weight` (default 1.0: all non-zero support must be valid).
#[allow(clippy::too_many_arguments)] // hot path: plain arguments avoid a struct per tile
pub fn sample_bilinear_into(
    geom: &TileGeometry,
    values: &[f32],
    h: usize,
    w: usize,
    tile: usize,
    nodata: Option<f64>,
    min_valid_weight: f64,
    out: &mut [f32],
) -> Result<()> {
    if !(min_valid_weight > 0.0 && min_valid_weight <= 1.0) {
        return Err(TilerError::Invalid(
            "min_valid_weight must be in (0, 1]".into(),
        ));
    }
    debug_assert_eq!(values.len(), h * w);
    debug_assert_eq!(out.len(), tile * tile);
    let threshold = (min_valid_weight - 1e-6) as f32;
    let nd = nodata.filter(|v| v.is_finite()).map(|v| v as f32);
    match &geom.body {
        GeometryBody::RegularBilinear { rows, cols } => {
            let col_taps: Vec<Tap> = cols.iter().map(|&c| tap(c, w)).collect();
            for (i, &r) in rows.iter().enumerate() {
                let rt = tap(r, h);
                for (dst, &ct) in out[i * tile..(i + 1) * tile].iter_mut().zip(&col_taps) {
                    *dst = bilinear_pixel(values, w, rt, ct, nd, threshold);
                }
            }
            Ok(())
        }
        GeometryBody::ProjectedBilinear { rows, cols } => {
            for ((dst, &r), &c) in out.iter_mut().zip(rows).zip(cols) {
                *dst = bilinear_pixel(values, w, tap(r, h), tap(c, w), nd, threshold);
            }
            Ok(())
        }
        _ => Err(TilerError::Invalid(
            "bilinear sampling requires bilinear tile geometry".into(),
        )),
    }
}

/// `_region_transparent`: is the inclusive source region `[r0..=r1] x [c0..=c1]` all zero?
///
/// Large regions (over 4 Mi cells) are probed with a `[::8, ::8]` stride first; a nonzero
/// probe proves the region is not transparent without scanning it.
pub fn region_transparent(
    idx: &[u8],
    width: usize,
    rows: (usize, usize),
    cols: (usize, usize),
) -> bool {
    let (r0, r1) = rows;
    let (c0, c1) = cols;
    let size = (r1 - r0 + 1) * (c1 - c0 + 1);
    if size > 1 << 22 {
        let probe_hit = (r0..=r1).step_by(8).any(|r| {
            idx[r * width + c0..=r * width + c1]
                .iter()
                .step_by(8)
                .any(|&v| v != 0)
        });
        if probe_hit {
            return false;
        }
    }
    (r0..=r1).all(|r| idx[r * width + c0..=r * width + c1].iter().all(|&v| v == 0))
}

/// Pooled-zoom test (spec 4.4, `_max_block_factors`): the source block that fits in one output
/// pixel everywhere in the grid. A zoom is pooled iff the block is not `(1, 1)`.
pub fn max_block_factors(lats: &[f64], lons: &[f64], zoom: u32, tile: usize) -> (usize, usize) {
    let px_deg = 360.0 / (tile as f64 * f64::from(1u32 << zoom));
    let dlat = (lats[1] - lats[0]).abs();
    let dlon = (lons[1] - lons[0]).abs();
    let lat_min = lats.iter().copied().fold(f64::INFINITY, f64::min);
    let lat_max = lats.iter().copied().fold(f64::NEG_INFINITY, f64::max);
    // Mercator pixels are shortest in latitude at the poleward edge.
    let min_cos = 89.0f64
        .min(lat_min.abs().max(lat_max.abs()))
        .to_radians()
        .cos();
    let block_r = ((px_deg * min_cos / dlat) as usize).max(1);
    let block_c = ((px_deg / dlon) as usize).max(1);
    (block_r, block_c)
}

/// NaN-propagating minimum, as `np.minimum`.
fn np_min(a: f64, b: f64) -> f64 {
    if a.is_nan() || b.is_nan() {
        f64::NAN
    } else {
        a.min(b)
    }
}

/// NaN-propagating maximum, as `np.maximum`.
fn np_max(a: f64, b: f64) -> f64 {
    if a.is_nan() || b.is_nan() {
        f64::NAN
    } else {
        a.max(b)
    }
}

/// Map `T + 1` pixel edges to half-open source-cell ranges `[start, stop)` on `axis`
/// (spec 4.4, `_footprint_edges`). `stop <= start` means off-grid.
pub fn footprint_edges(edge_coords: &[f64], axis: &AxisMap) -> Vec<(usize, usize)> {
    let n = axis.len();
    let nf = n as f64;
    let frac: Vec<f64> = edge_coords.iter().map(|&e| axis.map(e)).collect();
    frac.windows(2)
        .map(|pair| {
            let lo = np_min(pair[0], pair[1]);
            let hi = np_max(pair[0], pair[1]);
            let start = (lo + 0.5).floor().clamp(0.0, nf) as usize;
            let stop = (hi + 0.5).floor().clamp(0.0, nf) as usize;
            // A pixel narrower than a cell still takes the cell under it.
            let thin = stop <= start && hi >= -0.5 && lo <= nf - 0.5;
            if thin {
                (if start >= n { n - 1 } else { start }, (start + 1).min(n))
            } else {
                (start, stop)
            }
        })
        .collect()
}

/// Column reduction for overviews: for each row of `values` (`h x w`) and each
/// `[start, stop)` range, the NaN-ignoring maximum (`fmax`). Empty ranges give NaN.
/// Output is row-major `h x ranges.len()`.
pub fn reduce_column_ranges(
    values: &[f32],
    h: usize,
    w: usize,
    ranges: &[(usize, usize)],
) -> Vec<f32> {
    let mut out = vec![f32::NAN; h * ranges.len()];
    for r in 0..h {
        let row = &values[r * w..(r + 1) * w];
        for (j, &(start, stop)) in ranges.iter().enumerate() {
            let mut acc = f32::NAN;
            if stop > start {
                for &v in &row[start..stop] {
                    acc = acc.max(v);
                }
            }
            out[r * ranges.len() + j] = acc;
        }
    }
    out
}

/// Row reduction for overviews: for each `[start, stop)` row range and each of `t` columns of
/// `cols` (`h x t`), the NaN-ignoring maximum. Output is `ranges.len() x t`.
pub fn reduce_row_ranges(cols: &[f32], t: usize, ranges: &[(usize, usize)]) -> Vec<f32> {
    let mut out = vec![f32::NAN; ranges.len() * t];
    for (i, &(start, stop)) in ranges.iter().enumerate() {
        let dst = &mut out[i * t..(i + 1) * t];
        for r in start..stop {
            for (d, &v) in dst.iter_mut().zip(&cols[r * t..(r + 1) * t]) {
                *d = d.max(v);
            }
        }
    }
    out
}

/// [`reduce_column_ranges`] for class planes: the largest class per range, 0 for an empty range.
pub fn reduce_column_ranges_u8(
    values: &[u8],
    h: usize,
    w: usize,
    ranges: &[(usize, usize)],
) -> Vec<u8> {
    let mut out = vec![0u8; h * ranges.len()];
    for r in 0..h {
        let row = &values[r * w..(r + 1) * w];
        for (j, &(start, stop)) in ranges.iter().enumerate() {
            if stop > start {
                out[r * ranges.len() + j] = row[start..stop].iter().copied().max().unwrap_or(0);
            }
        }
    }
    out
}

/// [`reduce_row_ranges`] for class planes: the largest class per range, 0 for an empty range.
pub fn reduce_row_ranges_u8(cols: &[u8], t: usize, ranges: &[(usize, usize)]) -> Vec<u8> {
    let mut out = vec![0u8; ranges.len() * t];
    for (i, &(start, stop)) in ranges.iter().enumerate() {
        let dst = &mut out[i * t..(i + 1) * t];
        for r in start..stop {
            for (d, &v) in dst.iter_mut().zip(&cols[r * t..(r + 1) * t]) {
                *d = (*d).max(v);
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::geometry::nearest_index;

    /// test_footprint_edges_cover_every_cell_once_and_flag_off_grid_pixels
    #[test]
    fn footprint_edges_cover_each_cell_once() {
        let axis = AxisMap::new(&(0..10).map(f64::from).collect::<Vec<_>>()).unwrap();
        let ranges = footprint_edges(&[-3.0, -1.0, 2.5, 5.5, 9.4, 12.0], &axis);
        let starts: Vec<usize> = ranges.iter().map(|r| r.0).collect();
        let stops: Vec<usize> = ranges.iter().map(|r| r.1).collect();
        assert_eq!(starts, [0, 0, 3, 6, 9]);
        assert_eq!(stops, [0, 3, 6, 9, 10]);
        // Descending axis gives the same cells.
        let desc: Vec<f64> = (0..10).rev().map(f64::from).collect();
        let daxis = AxisMap::new(&desc).unwrap();
        let d = footprint_edges(&[2.5, 5.5], &daxis);
        assert_eq!(d[0].1 - d[0].0, 3);
    }

    /// test_footprint_edges_keep_the_cell_under_a_sub_cell_pixel
    #[test]
    fn sub_cell_pixel_keeps_its_cell() {
        let axis = AxisMap::new(&(0..10).map(f64::from).collect::<Vec<_>>()).unwrap();
        let r = footprint_edges(&[4.1, 4.2], &axis);
        assert_eq!(r[0], (4, 5));
    }

    /// test_reduce_ranges_is_nan_aware_and_empty_ranges_are_nan
    #[test]
    fn reduce_ranges_ignores_nan_and_empty_is_nan() {
        let vals = [1.0f32, f32::NAN, 7.0, 2.0, f32::NAN];
        let out = reduce_column_ranges(&vals, 1, 5, &[(0, 2), (1, 4), (4, 5), (5, 5)]);
        assert_eq!(out[0], 1.0);
        assert_eq!(out[1], 7.0);
        assert!(out[2].is_nan());
        assert!(out[3].is_nan());
    }

    #[test]
    fn block_factors_track_mercator_pixel_size() {
        let lats: Vec<f64> = (0..3500).map(|i| 20.0 + 0.01 * f64::from(i)).collect();
        let lons: Vec<f64> = (0..7000).map(|i| -130.0 + 0.01 * f64::from(i)).collect();
        assert_eq!(max_block_factors(&lats, &lons, 4, 256), (5, 8));
        assert_eq!(max_block_factors(&lats, &lons, 5, 256), (2, 4));
        assert_eq!(max_block_factors(&lats, &lons, 7, 256), (1, 1));
    }

    #[test]
    fn bilinear_midcell_and_nodata_policy() {
        // spec test_bilinear_samples_physical_values_with_explicit_nodata_policy
        let geom = TileGeometry {
            body: GeometryBody::RegularBilinear {
                rows: vec![0.5],
                cols: vec![0.5],
            },
            row_bounds: (0, 1),
            col_bounds: (0, 1),
        };
        let mut data = vec![0.0f32, 10.0, 20.0, 30.0];
        let mut out = [0.0f32; 1];
        sample_bilinear_into(&geom, &data, 2, 2, 1, None, 1.0, &mut out).unwrap();
        assert_eq!(out[0], 15.0);
        data[3] = -9999.0;
        sample_bilinear_into(&geom, &data, 2, 2, 1, Some(-9999.0), 1.0, &mut out).unwrap();
        assert!(out[0].is_nan(), "strict policy");
        sample_bilinear_into(&geom, &data, 2, 2, 1, Some(-9999.0), 0.75, &mut out).unwrap();
        assert_eq!(out[0], 10.0, "partial policy");
    }

    #[test]
    fn bilinear_zero_weight_invalid_corner_is_ignored() {
        // H10: an invalid corner with zero weight does not poison the sample.
        let geom = TileGeometry {
            body: GeometryBody::RegularBilinear {
                rows: vec![0.0],
                cols: vec![0.5],
            },
            row_bounds: (0, 0),
            col_bounds: (0, 1),
        };
        let data = [4.0f32, 8.0, f32::NAN, f32::NAN];
        let mut out = [0.0f32; 1];
        sample_bilinear_into(&geom, &data, 2, 2, 1, None, 1.0, &mut out).unwrap();
        assert_eq!(out[0], 6.0);
    }

    #[test]
    fn nearest_gather_zeroes_invalid_rows_and_columns() {
        let geom = TileGeometry {
            body: GeometryBody::RegularNearest {
                rows: vec![-1, 1],
                cols: vec![0, -1],
            },
            row_bounds: (1, 1),
            col_bounds: (0, 0),
        };
        let idx = [1u8, 2, 3, 4];
        let mut out = [9u8; 4];
        sample_nearest_into(&geom, &idx, 2, 2, &mut out).unwrap();
        assert_eq!(out, [0, 0, 3, 0]);
        assert_eq!(nearest_index(0.5, 2), 1);
    }

    #[test]
    fn region_transparency_probe_and_scan() {
        let mut idx = vec![0u8; 16];
        assert!(region_transparent(&idx, 4, (0, 3), (0, 3)));
        idx[5] = 1;
        assert!(!region_transparent(&idx, 4, (0, 3), (0, 3)));
        assert!(region_transparent(&idx, 4, (2, 3), (0, 3)));
    }
}
