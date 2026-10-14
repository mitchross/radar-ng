//! Tile geometry: XYZ tile math, source-axis mapping, and per-tile source coordinates
//! (spec-tiler sections 3, 9.1).
//!
//! [`SourceGeometry`] is built once per frame. It validates the source grid, fingerprints
//! its coordinates for the cache key, and precomputes the axis maps. Per-tile
//! [`TileGeometry`] values are cached process-wide (see `cache`).

use radar_geo::{Crs, Grid, GridGeo};

use crate::error::{Result, TilerError};
use crate::identity::{CoordinateIdentity, GridIdentity, fingerprint_f64};

/// Highest zoom accepted (spec section 3.6 says Rust should reject `z` outside `0..=30`).
pub const MAX_ZOOM: u32 = 30;

/// Sampling mode of a tile geometry.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Sampling {
    /// Nearest source cell, used for class indices.
    Nearest,
    /// Bilinear fractional coordinates, used for physical values.
    Bilinear,
}

/// Map a lat/lon to tile indices at `zoom` (spec 3.1). Out-of-range values are clamped.
pub fn lat_lon_to_tile(lat: f64, lon: f64, zoom: u32) -> (u32, u32) {
    let n = f64::from(1u32 << zoom);
    let x = ((lon + 180.0) / 360.0 * n) as i64;
    let lat_rad = lat.to_radians();
    let y = ((1.0 - (lat_rad.tan() + 1.0 / lat_rad.cos()).ln() / std::f64::consts::PI) / 2.0 * n)
        as i64;
    let max = (1i64 << zoom) - 1;
    (x.clamp(0, max) as u32, y.clamp(0, max) as u32)
}

/// `(west, south, east, north)` in degrees for tile `(x, y)` at `zoom` (spec 3.2).
pub fn tile_bounds(x: u32, y: u32, zoom: u32) -> (f64, f64, f64, f64) {
    let n = f64::from(1u32 << zoom);
    let (x, y) = (f64::from(x), f64::from(y));
    let west = x / n * 360.0 - 180.0;
    let east = (x + 1.0) / n * 360.0 - 180.0;
    let north = (std::f64::consts::PI * (1.0 - 2.0 * y / n))
        .sinh()
        .atan()
        .to_degrees();
    let south = (std::f64::consts::PI * (1.0 - 2.0 * (y + 1.0) / n))
        .sinh()
        .atan()
        .to_degrees();
    (west, south, east, north)
}

/// Pixel-center longitudes and latitudes of a tile (spec 3.3), `tile` samples each.
///
/// Evaluation order follows the Python reference so the doubles match bit for bit.
pub fn tile_pixel_axes(tx: u32, ty: u32, zoom: u32, tile: usize) -> (Vec<f64>, Vec<f64>) {
    let n = f64::from(1u32 << zoom);
    let t = tile as f64;
    let mut lons = Vec::with_capacity(tile);
    let mut lats = Vec::with_capacity(tile);
    for j in 0..tile {
        let px = (j as f64 + 0.5) / t;
        lons.push((f64::from(tx) + px) / n * 360.0 - 180.0);
    }
    for i in 0..tile {
        let px = (i as f64 + 0.5) / t;
        let v = std::f64::consts::PI * (1.0 - 2.0 * (f64::from(ty) + px) / n);
        lats.push(v.sinh().atan().to_degrees());
    }
    (lons, lats)
}

/// Piecewise-linear or affine map from coordinate values to fractional indices on a
/// monotonic axis (spec 3.4). Values outside the axis extrapolate linearly.
#[derive(Debug, Clone)]
pub struct AxisMap {
    len: usize,
    kind: AxisKind,
}

#[derive(Debug, Clone)]
enum AxisKind {
    Affine {
        start: f64,
        span: f64,
        scale: f64,
    },
    Piecewise {
        ordered: Vec<f64>,
        positions: Vec<f64>,
    },
}

impl AxisMap {
    /// Build the map for `axis` (length at least 2, strictly monotonic).
    pub fn new(axis: &[f64]) -> Result<AxisMap> {
        let n = axis.len();
        if n < 2 {
            return Err(TilerError::Invalid(
                "source axes must be 1D arrays with at least two points".into(),
            ));
        }
        let steps: Vec<f64> = axis.windows(2).map(|w| w[1] - w[0]).collect();
        let ascending = steps.iter().all(|&s| s > 0.0);
        let descending = steps.iter().all(|&s| s < 0.0);
        if !ascending && !descending {
            return Err(TilerError::Invalid(
                "source axes must be strictly monotonic".into(),
            ));
        }
        let s0 = steps[0];
        let atol = s0.abs() * 1e-10;
        let rtol = 1e-7;
        // numpy.allclose(steps, steps[0], rtol, atol): |a - b| <= atol + rtol * |b|
        if steps
            .iter()
            .all(|&s| (s - s0).abs() <= atol + rtol * s0.abs())
        {
            let start = axis[0];
            let span = axis[n - 1] - start;
            return Ok(AxisMap {
                len: n,
                kind: AxisKind::Affine {
                    start,
                    span,
                    scale: (n - 1) as f64,
                },
            });
        }
        let (ordered, positions): (Vec<f64>, Vec<f64>) = if ascending {
            (axis.to_vec(), (0..n).map(|i| i as f64).collect())
        } else {
            (
                axis.iter().rev().copied().collect(),
                (0..n).rev().map(|i| i as f64).collect(),
            )
        };
        Ok(AxisMap {
            len: n,
            kind: AxisKind::Piecewise { ordered, positions },
        })
    }

    /// Number of axis samples.
    pub fn len(&self) -> usize {
        self.len
    }

    /// `true` when the axis is empty (never true for a constructed map).
    pub fn is_empty(&self) -> bool {
        self.len == 0
    }

    /// Fractional index of `v` (NaN in gives NaN out).
    #[inline]
    pub fn map(&self, v: f64) -> f64 {
        match &self.kind {
            AxisKind::Affine { start, span, scale } => (v - start) / span * scale,
            AxisKind::Piecewise { ordered, positions } => {
                let n = ordered.len();
                let seg = (ordered.partition_point(|&e| e <= v) as isize - 1)
                    .clamp(0, n as isize - 2) as usize;
                let frac = (v - ordered[seg]) / (ordered[seg + 1] - ordered[seg]);
                positions[seg] + frac * (positions[seg + 1] - positions[seg])
            }
        }
    }
}

/// Finite index bounds `(start, end)` (inclusive) for fractional indices on an axis of
/// `size` samples (spec 3.5). `None` when nothing finite overlaps the grid, or when the span
/// collapses to a single integer (hazard H9).
pub fn finite_index_bounds(values: &[f64], size: usize) -> Option<(usize, usize)> {
    let mut lo = f64::INFINITY;
    let mut hi = f64::NEG_INFINITY;
    for &v in values {
        if v.is_finite() {
            lo = lo.min(v);
            hi = hi.max(v);
        }
    }
    if lo > hi {
        return None;
    }
    let last = (size - 1) as f64;
    if hi < 0.0 || lo > last {
        return None;
    }
    let start = lo.floor().clamp(0.0, last) as usize;
    let end = hi.ceil().clamp(0.0, last) as usize;
    (end > start).then_some((start, end))
}

/// Round-half-up nearest index on `[0, n-1]`, or `-1` when the coordinate is outside (spec 4.1).
#[inline]
pub fn nearest_index(coord: f64, n: usize) -> i32 {
    if !(coord >= 0.0 && coord <= (n - 1) as f64) {
        return -1;
    }
    let v = (coord + 0.5).floor() as i64;
    v.min(n as i64 - 1) as i32
}

/// Finite `(min, max)` of a coordinate array, ignoring NaN and infinities.
pub fn finite_bounds(values: &[f64], name: &str) -> Result<(f64, f64)> {
    let mut lo = f64::INFINITY;
    let mut hi = f64::NEG_INFINITY;
    for &v in values {
        if v.is_finite() {
            lo = lo.min(v);
            hi = hi.max(v);
        }
    }
    if lo > hi {
        return Err(TilerError::Invalid(format!(
            "{name} has no finite coordinates"
        )));
    }
    Ok((lo, hi))
}

/// Cached source coordinates for one tile and one sampling mode.
#[derive(Debug, Clone)]
pub struct TileGeometry {
    /// Coordinates by kind.
    pub body: GeometryBody,
    /// Finite source row range `(start, end)` inclusive, used for transparency prechecks.
    pub row_bounds: (usize, usize),
    /// Finite source column range `(start, end)` inclusive.
    pub col_bounds: (usize, usize),
}

/// Coordinate arrays of a tile.
#[derive(Debug, Clone)]
pub enum GeometryBody {
    /// Regular grid, nearest: per-pixel-row and per-pixel-column source indices (`-1` invalid).
    RegularNearest {
        /// Length `T`.
        rows: Vec<i32>,
        /// Length `T`.
        cols: Vec<i32>,
    },
    /// Regular grid, bilinear: fractional coordinates.
    RegularBilinear {
        /// Length `T`.
        rows: Vec<f32>,
        /// Length `T`.
        cols: Vec<f32>,
    },
    /// Projected grid, nearest: `row * W + col` per output pixel (`-1` invalid).
    ProjectedNearest {
        /// Length `T * T`, row-major.
        flat: Vec<i32>,
    },
    /// Projected grid, bilinear: fractional coordinates per output pixel.
    ProjectedBilinear {
        /// Length `T * T`.
        rows: Vec<f32>,
        /// Length `T * T`.
        cols: Vec<f32>,
    },
}

impl TileGeometry {
    /// Heap bytes held by the coordinate arrays (cache accounting).
    pub fn nbytes(&self) -> usize {
        match &self.body {
            GeometryBody::RegularNearest { rows, cols } => (rows.len() + cols.len()) * 4,
            GeometryBody::RegularBilinear { rows, cols }
            | GeometryBody::ProjectedBilinear { rows, cols } => (rows.len() + cols.len()) * 4,
            GeometryBody::ProjectedNearest { flat } => flat.len() * 4,
        }
    }

    /// Sampling mode of this geometry.
    pub fn sampling(&self) -> Sampling {
        match self.body {
            GeometryBody::RegularNearest { .. } | GeometryBody::ProjectedNearest { .. } => {
                Sampling::Nearest
            }
            _ => Sampling::Bilinear,
        }
    }
}

enum Projection {
    Regular { lats: AxisMap, lons: AxisMap },
    Projected { crs: Crs, x: AxisMap, y: AxisMap },
}

/// A validated source grid, ready to compute tile geometry.
pub struct SourceGeometry {
    height: usize,
    width: usize,
    identity: GridIdentity,
    projection: Projection,
}

impl SourceGeometry {
    /// Validate `geo` against a `height x width` grid and precompute its axis maps.
    pub fn new(height: usize, width: usize, geo: &GridGeo) -> Result<SourceGeometry> {
        if height == 0 || width == 0 {
            return Err(TilerError::Invalid(
                "grid must have at least one row and column".into(),
            ));
        }
        match geo {
            GridGeo::Regular { lats, lons } => {
                if lats.len() != height || lons.len() != width {
                    return Err(TilerError::Invalid(
                        "lats/lons lengths must match the grid height/width".into(),
                    ));
                }
                let lat_bounds = finite_bounds(lats, "lats")?;
                let lon_bounds = finite_bounds(lons, "lons")?;
                let identity = GridIdentity {
                    kind: "regular",
                    height,
                    width,
                    lat_bounds,
                    lon_bounds,
                    coordinates: CoordinateIdentity::Regular {
                        lats: fingerprint_f64(lats),
                        lons: fingerprint_f64(lons),
                    },
                };
                Ok(SourceGeometry {
                    height,
                    width,
                    identity,
                    projection: Projection::Regular {
                        lats: AxisMap::new(lats)?,
                        lons: AxisMap::new(lons)?,
                    },
                })
            }
            GridGeo::Projected {
                crs,
                x,
                y,
                lat_mesh,
                lon_mesh,
            } => {
                if x.len() != width || y.len() != height {
                    return Err(TilerError::Invalid(
                        "source_x/source_y lengths must match the grid width/height".into(),
                    ));
                }
                if lat_mesh.len() != height * width || lon_mesh.len() != height * width {
                    return Err(TilerError::Invalid(
                        "projected lat/lon meshes must be height x width".into(),
                    ));
                }
                let lat_bounds = finite_bounds(lat_mesh, "lats")?;
                let lon_bounds = finite_bounds(lon_mesh, "lons")?;
                let identity = GridIdentity {
                    kind: "projected",
                    height,
                    width,
                    lat_bounds,
                    lon_bounds,
                    coordinates: CoordinateIdentity::Projected {
                        crs: crs.identity_string(),
                        x: fingerprint_f64(x),
                        y: fingerprint_f64(y),
                    },
                };
                Ok(SourceGeometry {
                    height,
                    width,
                    identity,
                    projection: Projection::Projected {
                        crs: crs.clone(),
                        x: AxisMap::new(x)?,
                        y: AxisMap::new(y)?,
                    },
                })
            }
        }
    }

    /// Convenience constructor from a [`Grid`].
    pub fn from_grid(grid: &Grid) -> Result<SourceGeometry> {
        SourceGeometry::new(grid.height, grid.width, &grid.geo)
    }

    /// Grid height (rows).
    pub fn height(&self) -> usize {
        self.height
    }

    /// Grid width (columns).
    pub fn width(&self) -> usize {
        self.width
    }

    /// Coordinate and bounds identity (cache key and digest input).
    pub fn identity(&self) -> &GridIdentity {
        &self.identity
    }

    /// Tile jobs `(z, tx, ty)` for every zoom, in spec order: zooms as given, `tx` ascending,
    /// `ty` ascending. Tiles outside the grid bbox are not listed.
    pub fn tile_jobs(&self, zooms: &[u32]) -> Vec<(u32, u32, u32)> {
        let (lat_min, lat_max) = self.identity.lat_bounds;
        let (lon_min, lon_max) = self.identity.lon_bounds;
        let mut jobs = Vec::new();
        for &z in zooms {
            let (tx_min, ty_min) = lat_lon_to_tile(lat_max, lon_min, z);
            let (tx_max, ty_max) = lat_lon_to_tile(lat_min, lon_max, z);
            for tx in tx_min..=tx_max {
                for ty in ty_min..=ty_max {
                    jobs.push((z, tx, ty));
                }
            }
        }
        jobs
    }

    /// Compute the geometry of tile `(z, tx, ty)` (spec 9.1). `None` when the tile does not
    /// overlap the source grid (cached as a negative entry by the caller).
    pub fn tile_geometry(
        &self,
        sampling: Sampling,
        tile: usize,
        z: u32,
        tx: u32,
        ty: u32,
    ) -> Result<Option<TileGeometry>> {
        let (lons_tile, lats_tile) = tile_pixel_axes(tx, ty, z, tile);
        match &self.projection {
            Projection::Regular { lats, lons } => Ok(regular_geometry(
                self.height,
                self.width,
                lats,
                lons,
                &lats_tile,
                &lons_tile,
                sampling,
            )),
            Projection::Projected { crs, x, y } => {
                let (lon_min, lon_max) = self.identity.lon_bounds;
                let (lat_min, lat_max) = self.identity.lat_bounds;
                let (west, south, east, north) = tile_bounds(tx, ty, z);
                if east < lon_min || west > lon_max || north < lat_min || south > lat_max {
                    return Ok(None);
                }
                Ok(projected_geometry(
                    self.height,
                    self.width,
                    crs,
                    x,
                    y,
                    &lats_tile,
                    &lons_tile,
                    sampling,
                ))
            }
        }
    }
}

fn regular_geometry(
    h: usize,
    w: usize,
    lats: &AxisMap,
    lons: &AxisMap,
    lats_tile: &[f64],
    lons_tile: &[f64],
    sampling: Sampling,
) -> Option<TileGeometry> {
    let cols_1d: Vec<f64> = lons_tile.iter().map(|&v| lons.map(v)).collect();
    let rows_1d: Vec<f64> = lats_tile.iter().map(|&v| lats.map(v)).collect();
    let all_below = |v: &[f64]| v.iter().all(|&c| c < 0.0);
    let all_above = |v: &[f64], size: usize| v.iter().all(|&c| c > (size - 1) as f64);
    if all_below(&cols_1d)
        || all_above(&cols_1d, w)
        || all_below(&rows_1d)
        || all_above(&rows_1d, h)
    {
        return None;
    }
    let row_bounds = finite_index_bounds(&rows_1d, h)?;
    let col_bounds = finite_index_bounds(&cols_1d, w)?;
    let body = match sampling {
        Sampling::Nearest => {
            let rows: Vec<i32> = rows_1d.iter().map(|&r| nearest_index(r, h)).collect();
            let cols: Vec<i32> = cols_1d.iter().map(|&c| nearest_index(c, w)).collect();
            if !rows.iter().any(|&r| r >= 0) || !cols.iter().any(|&c| c >= 0) {
                return None;
            }
            GeometryBody::RegularNearest { rows, cols }
        }
        Sampling::Bilinear => GeometryBody::RegularBilinear {
            rows: rows_1d.iter().map(|&r| r as f32).collect(),
            cols: cols_1d.iter().map(|&c| c as f32).collect(),
        },
    };
    Some(TileGeometry {
        body,
        row_bounds,
        col_bounds,
    })
}

#[allow(clippy::too_many_arguments)]
fn projected_geometry(
    h: usize,
    w: usize,
    crs: &Crs,
    x_map: &AxisMap,
    y_map: &AxisMap,
    lats_tile: &[f64],
    lons_tile: &[f64],
    sampling: Sampling,
) -> Option<TileGeometry> {
    let t = lons_tile.len();
    let mut rows_m = Vec::with_capacity(t * t);
    let mut cols_m = Vec::with_capacity(t * t);
    for &lat in lats_tile {
        for &lon in lons_tile {
            let (x, y) = crs.forward(lon, lat);
            let (c, r) = (x_map.map(x), y_map.map(y));
            if r.is_finite() && c.is_finite() {
                rows_m.push(r);
                cols_m.push(c);
            } else {
                rows_m.push(-1.0);
                cols_m.push(-1.0);
            }
        }
    }
    let row_bounds = finite_index_bounds(&rows_m, h)?;
    let col_bounds = finite_index_bounds(&cols_m, w)?;
    let body = match sampling {
        Sampling::Nearest => {
            let mut flat = Vec::with_capacity(t * t);
            let mut any = false;
            for (&r, &c) in rows_m.iter().zip(&cols_m) {
                let (ri, ci) = (nearest_index(r, h), nearest_index(c, w));
                if ri >= 0 && ci >= 0 {
                    any = true;
                    flat.push((ri as i64 * w as i64 + ci as i64) as i32);
                } else {
                    flat.push(-1);
                }
            }
            if !any {
                return None;
            }
            GeometryBody::ProjectedNearest { flat }
        }
        Sampling::Bilinear => GeometryBody::ProjectedBilinear {
            rows: rows_m.iter().map(|&r| r as f32).collect(),
            cols: cols_m.iter().map(|&c| c as f32).collect(),
        },
    };
    Some(TileGeometry {
        body,
        row_bounds,
        col_bounds,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn axis_affine_path_matches_formula() {
        let axis: Vec<f64> = (0..10).map(|i| 0.5 + i as f64 * 0.25).collect();
        let m = AxisMap::new(&axis).unwrap();
        let expect = (1.0 - axis[0]) / (axis[9] - axis[0]) * 9.0;
        assert_eq!(m.map(1.0), expect);
    }

    /// test_fractional_indices_support_nonuniform_monotonic_axes
    #[test]
    fn nonuniform_monotonic_axes_map_piecewise() {
        let values = [-1.0, 0.5, 2.0, 4.0];
        let asc = AxisMap::new(&[0.0, 1.0, 3.0]).unwrap();
        let desc = AxisMap::new(&[3.0, 1.0, 0.0]).unwrap();
        let a: Vec<f64> = values.iter().map(|&v| asc.map(v)).collect();
        let d: Vec<f64> = values.iter().map(|&v| desc.map(v)).collect();
        for (got, want) in a.iter().zip([-1.0, 0.5, 1.5, 2.5]) {
            assert!((got - want).abs() < 1e-12, "{a:?}");
        }
        for (got, want) in d.iter().zip([3.0, 1.5, 0.5, -0.5]) {
            assert!((got - want).abs() < 1e-12, "{d:?}");
        }
        let err = AxisMap::new(&[0.0, 2.0, 1.0]).unwrap_err();
        assert!(err.to_string().contains("strictly monotonic"));
    }

    #[test]
    fn nearest_index_is_closed_interval_round_half_up() {
        assert_eq!(nearest_index(-0.01, 5), -1);
        assert_eq!(nearest_index(0.0, 5), 0);
        assert_eq!(nearest_index(0.5, 5), 1);
        assert_eq!(nearest_index(3.49, 5), 3);
        assert_eq!(nearest_index(4.0, 5), 4);
        assert_eq!(nearest_index(4.01, 5), -1);
        assert_eq!(nearest_index(f64::NAN, 5), -1);
    }

    #[test]
    fn finite_index_bounds_rules() {
        assert_eq!(finite_index_bounds(&[f64::NAN, -2.0], 10), None);
        assert_eq!(finite_index_bounds(&[1.2, 3.7], 10), Some((1, 4)));
        assert_eq!(finite_index_bounds(&[-3.0, 20.0], 10), Some((0, 9)));
        // H9: a single-integer span is dropped.
        assert_eq!(finite_index_bounds(&[3.0, 3.0], 10), None);
    }

    #[test]
    fn tile_bounds_and_lookup_agree() {
        let (w, s, e, n) = tile_bounds(4, 5, 4);
        assert!(w < e && s < n);
        let (tx, ty) = lat_lon_to_tile((n + s) / 2.0, (w + e) / 2.0, 4);
        assert_eq!((tx, ty), (4, 5));
        // Clamping at the poles and antimeridian.
        assert_eq!(lat_lon_to_tile(90.0, 180.0, 3), (7, 0));
        assert_eq!(lat_lon_to_tile(-90.0, -180.0, 3), (0, 7));
    }

    #[test]
    fn regular_tile_rejects_tiles_outside_the_grid() {
        let lats: Vec<f64> = (0..20).map(|i| 40.0 + i as f64 * 0.1).collect();
        let lons: Vec<f64> = (0..30).map(|i| -100.0 + i as f64 * 0.1).collect();
        let src = SourceGeometry::new(20, 30, &GridGeo::Regular { lats, lons }).unwrap();
        // Tile far to the east of the grid is not produced as a job at all.
        assert!(src.tile_jobs(&[4]).iter().all(|&(_, tx, _)| tx < 8));
        assert!(
            src.tile_geometry(Sampling::Bilinear, 256, 4, 0, 0)
                .unwrap()
                .is_none()
        );
    }
}
