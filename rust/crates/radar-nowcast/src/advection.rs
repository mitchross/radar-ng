//! Semi-Lagrangian advection (pysteps `extrapolation.semilagrangian.extrapolate` with
//! `interp_order = 1`, `n_iter = 1`).
//!
//! Two building blocks reproduce pysteps' sampling exactly:
//!
//! - [`sample_bilinear`] mimics `scipy.ndimage.map_coordinates(order=1)`. Coordinates
//!   outside `[0, n-1]` return the constant value (`Edge::Constant`) or the clamped
//!   edge (`Edge::Clamp`, scipy `mode="nearest"`). Inside, the 2x2 corners are combined
//!   with bilinear weights. A corner that scipy would read (including one with zero
//!   weight) propagates NaN into the result, and corners past the last index are skipped.
//! - [`advect_segment`] advances a cumulative backward displacement `D` by one time segment
//!   `dt`, using the midpoint velocity (`v0 = V(x+D) dt`, `D -= V(x + D - v0/2) dt`).
//!   [`warp`] samples a field at `x + D`.
//!
//! Velocity components are `x` (column) and `y` (row). Coordinates are `(x, y)`.

use rayon::prelude::*;

/// Out-of-range behaviour for [`sample_bilinear`].
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Edge {
    /// Return this value for coordinates outside the array (scipy `mode="constant"`).
    Constant(f64),
    /// Clamp coordinates to the array (scipy `mode="nearest"`).
    Clamp,
}

/// Corner indices and weights of one bilinear sample (scipy `order=1` semantics).
#[derive(Clone, Copy)]
struct Corners {
    i00: usize,
    i10: usize,
    i01: usize,
    i11: usize,
    fx: f64,
    fy: f64,
    included_x1: bool,
    included_y1: bool,
}

/// Corners for `(x, y)`, or the constant value when `edge` puts the point outside.
#[inline]
fn corners(w: usize, h: usize, x: f64, y: f64, edge: Edge) -> Result<Corners, f64> {
    let (x, y) = match edge {
        Edge::Clamp => (x.clamp(0.0, (w - 1) as f64), y.clamp(0.0, (h - 1) as f64)),
        Edge::Constant(cval) => {
            if !(0.0..=(w - 1) as f64).contains(&x) || !(0.0..=(h - 1) as f64).contains(&y) {
                return Err(cval);
            }
            (x, y)
        }
    };
    let x0 = x.floor();
    let y0 = y.floor();
    let (x0, y0) = (x0 as usize, y0 as usize);
    // The upper corner exists only inside the array; at the last index its weight is 0.
    let x1 = (x0 + 1).min(w - 1);
    let y1 = (y0 + 1).min(h - 1);
    Ok(Corners {
        i00: y0 * w + x0,
        i10: y0 * w + x1,
        i01: y1 * w + x0,
        i11: y1 * w + x1,
        fx: x - x0 as f64,
        fy: y - y0 as f64,
        included_x1: x0 + 1 < w,
        included_y1: y0 + 1 < h,
    })
}

/// Bilinear combination of the corners of `data` (NaN-propagating like scipy).
#[inline]
fn gather<T: Copy + Into<f64>>(data: &[T], c: &Corners) -> f64 {
    let v00: f64 = data[c.i00].into();
    let v10: f64 = data[c.i10].into();
    let v01: f64 = data[c.i01].into();
    let v11: f64 = data[c.i11].into();
    // NaN in any read corner poisons the result, even with zero weight.
    let mut nan = v00.is_nan();
    if c.included_x1 {
        nan |= v10.is_nan();
    }
    if c.included_y1 {
        nan |= v01.is_nan();
        if c.included_x1 {
            nan |= v11.is_nan();
        }
    }
    if nan {
        return f64::NAN;
    }
    let (fx, fy) = (c.fx, c.fy);
    let top = v00 * (1.0 - fx) + if c.included_x1 { v10 * fx } else { 0.0 };
    let bottom = v01 * (1.0 - fx) + if c.included_x1 { v11 * fx } else { 0.0 };
    if c.included_y1 {
        top * (1.0 - fy) + bottom * fy
    } else {
        top
    }
}

/// Bilinear sample of a row-major `w x h` array at `(x, y)` with scipy `order=1` semantics.
pub fn sample_bilinear<T: Copy + Into<f64>>(
    data: &[T],
    w: usize,
    h: usize,
    x: f64,
    y: f64,
    edge: Edge,
) -> f64 {
    match corners(w, h, x, y, edge) {
        Ok(c) => gather(data, &c),
        Err(cval) => cval,
    }
}

/// [`sample_bilinear`] of two arrays at the same point, sharing the corner arithmetic.
#[inline]
fn sample_bilinear_pair<T: Copy + Into<f64>>(
    a: &[T],
    b: &[T],
    w: usize,
    h: usize,
    x: f64,
    y: f64,
    edge: Edge,
) -> (f64, f64) {
    match corners(w, h, x, y, edge) {
        Ok(c) => (gather(a, &c), gather(b, &c)),
        Err(cval) => (cval, cval),
    }
}

/// Cumulative backward displacement per cell, in pixels.
#[derive(Clone, Debug, PartialEq)]
pub struct Displacement {
    /// Grid width.
    pub width: usize,
    /// Grid height.
    pub height: usize,
    /// Column displacement.
    pub dx: Vec<f64>,
    /// Row displacement.
    pub dy: Vec<f64>,
}

impl Displacement {
    /// Zero displacement on a `w x h` grid.
    pub fn zeros(w: usize, h: usize) -> Self {
        Self {
            width: w,
            height: h,
            dx: vec![0.0; w * h],
            dy: vec![0.0; w * h],
        }
    }
}

/// Advance the displacement by one segment of length `dt` (pysteps semi-Lagrangian step).
///
/// `vx` and `vy` are the velocity components (any `Into<f64>` storage, typically `f32`).
pub fn advect_segment<T: Copy + Into<f64> + Sync>(
    disp: &mut Displacement,
    vx: &[T],
    vy: &[T],
    dt: f64,
) {
    let (w, h) = (disp.width, disp.height);
    disp.dx
        .par_chunks_mut(w)
        .zip(disp.dy.par_chunks_mut(w))
        .enumerate()
        .for_each(|(row, (dxr, dyr))| {
            let y = row as f64;
            for x in 0..w {
                let xf = x as f64;
                let (cx, cy) = (xf + dxr[x], y + dyr[x]);
                let (v0x, v0y) = sample_bilinear_pair(vx, vy, w, h, cx, cy, Edge::Clamp);
                let (v0x, v0y) = (v0x * dt, v0y * dt);
                let (mx, my) = (cx - 0.5 * v0x, cy - 0.5 * v0y);
                let (mvx, mvy) = sample_bilinear_pair(vx, vy, w, h, mx, my, Edge::Clamp);
                dxr[x] -= mvx * dt;
                dyr[x] -= mvy * dt;
            }
        });
}

/// Sample `field` at `(x + dx, y + dy)` for every cell, with `cval` outside the grid.
pub fn warp<T: Copy + Into<f64> + Sync>(field: &[T], disp: &Displacement, cval: f64) -> Vec<f64> {
    let (w, h) = (disp.width, disp.height);
    let mut out = vec![0.0f64; w * h];
    out.par_chunks_mut(w)
        .enumerate()
        .for_each(|(row, out_row)| {
            let y = row as f64;
            for x in 0..w {
                let i = row * w + x;
                out_row[x] = sample_bilinear(
                    field,
                    w,
                    h,
                    x as f64 + disp.dx[i],
                    y + disp.dy[i],
                    Edge::Constant(cval),
                );
            }
        });
    out
}

/// Pre-advection of an older frame by `n_steps` unit steps, returning the final field
/// (spec 7.2: `extrapolate(P, V, n, outval)[-1]`).
pub fn advect_unit_steps<T: Copy + Into<f64> + Sync, U: Copy + Into<f64> + Sync>(
    field: &[T],
    vx: &[U],
    vy: &[U],
    w: usize,
    h: usize,
    n_steps: usize,
    outval: f64,
) -> Vec<f64> {
    let mut disp = Displacement::zeros(w, h);
    for _ in 0..n_steps {
        advect_segment(&mut disp, vx, vy, 1.0);
    }
    warp(field, &disp, outval)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bilinear_interior_and_edges() {
        let a = [0.0f64, 10.0, 20.0, 30.0]; // 2x2
        assert!((sample_bilinear(&a, 2, 2, 0.5, 0.0, Edge::Constant(-1.0)) - 5.0).abs() < 1e-12);
        assert!((sample_bilinear(&a, 2, 2, 1.0, 1.0, Edge::Constant(-1.0)) - 30.0).abs() < 1e-12);
        assert_eq!(
            sample_bilinear(&a, 2, 2, 1.5, 0.0, Edge::Constant(-99.0)),
            -99.0
        );
        assert_eq!(
            sample_bilinear(&a, 2, 2, -0.2, 0.0, Edge::Constant(-99.0)),
            -99.0
        );
        // Clamp mode: far outside takes the nearest corner.
        assert_eq!(sample_bilinear(&a, 2, 2, 5.0, -3.0, Edge::Clamp), 10.0);
    }

    #[test]
    fn nan_corner_with_zero_weight_poisons_the_sample() {
        // (1.0, 2.0) on a 3x4 grid: the NaN at (x=2, y=2) is read with zero weight.
        let mut v = vec![1.0f64; 12];
        v[2 * 3 + 2] = f64::NAN;
        assert!(sample_bilinear(&v, 3, 4, 1.0, 2.0, Edge::Constant(f64::NAN)).is_nan());
        // Exact last column: the out-of-range corner is skipped, so no NaN from cval.
        let mut u = vec![5.0f64; 12];
        u[3 + 2] = 7.0;
        assert_eq!(
            sample_bilinear(&u, 3, 4, 2.0, 1.0, Edge::Constant(f64::NAN)),
            7.0
        );
    }

    #[test]
    fn constant_velocity_translates_a_field() {
        // Uniform velocity (1, 0.5) for one unit step: sample at x + 1? Backward
        // displacement after one step is -(1, 0.5) so the warped field is shifted.
        let (w, h) = (20usize, 20usize);
        let vx = vec![1.0f32; w * h];
        let vy = vec![0.5f32; w * h];
        let mut disp = Displacement::zeros(w, h);
        advect_segment(&mut disp, &vx, &vy, 1.0);
        assert!((disp.dx[10 * w + 10] + 1.0).abs() < 1e-9);
        assert!((disp.dy[10 * w + 10] + 0.5).abs() < 1e-9);
        let field: Vec<f64> = (0..w * h).map(|i| (i % w) as f64).collect();
        let out = warp(&field, &disp, f64::NAN);
        // Value at (10,10) now equals the input at (9, 9.5) -> column 9.
        assert!((out[10 * w + 10] - 9.0).abs() < 1e-9);
    }
}
