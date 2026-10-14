//! Dense Lucas-Kanade motion estimation (pysteps `motion.lucaskanade.dense_lucaskanade`)
//! and radar-ng's stride-2 wrapper (`activities._estimate_motion`).
//!
//! Pipeline per consecutive frame pair (spec 6.2): NaN masking, binary morphological
//! opening, Shi-Tomasi corner detection, pyramidal LK. Then all sparse vectors from all
//! pairs are pooled, outliers are removed (kNN Mahalanobis), declustered, and
//! interpolated to a dense grid with inverse-distance weighting.
//!
//! Corner detection and pyramidal LK are our own implementations of the OpenCV
//! routines (see `shitomasi.rs` and `pyrlk.rs` for the approximations).

mod morph;
mod pyrlk;
mod shitomasi;
mod sparse;

use rayon::prelude::*;

use crate::error::NowcastError;
use crate::field::Field2;

/// A dense 2-D velocity field. `x` is the column (east) component and `y` the row
/// (south) component, both in pixels per input step (spec 6.1 `U[0]`, `U[1]`).
#[derive(Clone, Debug, PartialEq)]
pub struct Velocity {
    /// Column component, `x` in pixels per input step.
    pub x: Field2<f32>,
    /// Row component, `y` in pixels per input step.
    pub y: Field2<f32>,
}

impl Velocity {
    /// Build a velocity field. Both components must share a shape and be finite.
    pub fn new(x: Field2<f32>, y: Field2<f32>) -> Result<Self, NowcastError> {
        if x.width() != y.width() || x.height() != y.height() {
            return Err(NowcastError::InvalidInput(format!(
                "velocity components differ in shape: {}x{} vs {}x{}",
                x.width(),
                x.height(),
                y.width(),
                y.height()
            )));
        }
        if x.as_slice()
            .iter()
            .chain(y.as_slice())
            .any(|v| !v.is_finite())
        {
            return Err(NowcastError::NonFiniteVelocity);
        }
        Ok(Self { x, y })
    }

    /// A zero motion field of the given shape.
    pub fn zeros(width: usize, height: usize) -> Self {
        Self {
            x: Field2::filled(width, height, 0.0),
            y: Field2::filled(width, height, 0.0),
        }
    }

    /// Width (columns) of the field.
    pub fn width(&self) -> usize {
        self.x.width()
    }

    /// Height (rows) of the field.
    pub fn height(&self) -> usize {
        self.x.height()
    }
}

/// Shi-Tomasi corner detection settings (pysteps `feature.shitomasi.detection`).
#[derive(Clone, Debug, PartialEq)]
pub struct ShiTomasi {
    /// `maxCorners`.
    pub max_corners: usize,
    /// `qualityLevel`, relative to the strongest response.
    pub quality_level: f64,
    /// `minDistance` in pixels.
    pub min_distance: f64,
    /// `blockSize` of the structure-tensor window.
    pub block_size: usize,
    /// Dilation (square, pixels) applied to the invalid mask before detection.
    pub buffer_mask: usize,
}

/// Pyramidal Lucas-Kanade settings (pysteps `tracking.lucaskanade.track_features`).
#[derive(Clone, Debug, PartialEq)]
pub struct PyrLk {
    /// Search window side length in pixels (`winSize`).
    pub win_size: usize,
    /// Highest pyramid level (`maxLevel`).
    pub max_level: usize,
    /// Newton iterations per level (`TermCriteria` count).
    pub max_iter: usize,
    /// Newton step tolerance (`TermCriteria` epsilon).
    pub epsilon: f64,
    /// `minEigThreshold`, normalised by the window area.
    pub min_eig_threshold: f64,
}

/// Inverse-distance interpolation settings (pysteps `utils.interpolate.idwinterp2d`).
#[derive(Clone, Debug, PartialEq)]
pub struct Idw {
    /// Distance exponent (`power`).
    pub power: f64,
    /// Number of nearest vectors used (`k`).
    pub k: usize,
    /// Offset added to distances before weighting (`dist_offset`).
    pub dist_offset: f64,
}

/// All motion parameters. [`MotionParams::radar_ng`] is the production configuration.
#[derive(Clone, Debug, PartialEq)]
pub struct MotionParams {
    /// Morphological opening size (`size_opening`); 0 disables it.
    pub size_opening: usize,
    /// Outlier threshold in standard deviations (`nr_std_outlier`).
    pub nr_std_outlier: f64,
    /// Neighbours used by the outlier test (`k_outlier`).
    pub k_outlier: usize,
    /// Declustering scale in pixels (`decl_scale`); values <= 1 disable it.
    pub decl_scale: f64,
    /// Corner detection.
    pub shitomasi: ShiTomasi,
    /// Pyramidal LK.
    pub lk: PyrLk,
    /// Interpolation.
    pub idw: Idw,
}

impl MotionParams {
    /// The parameters radar-ng passes to pysteps (spec 6.2).
    pub fn radar_ng() -> Self {
        Self {
            size_opening: 3,
            nr_std_outlier: 3.0,
            k_outlier: 30,
            decl_scale: 20.0,
            shitomasi: ShiTomasi {
                max_corners: 1000,
                quality_level: 0.01,
                min_distance: 10.0,
                block_size: 5,
                buffer_mask: 5,
            },
            lk: PyrLk {
                win_size: 50,
                max_level: 3,
                max_iter: 10,
                epsilon: 0.0,
                min_eig_threshold: 1e-4,
            },
            idw: Idw {
                power: 0.5,
                k: 20,
                dist_offset: 0.5,
            },
        }
    }
}

impl Default for MotionParams {
    fn default() -> Self {
        Self::radar_ng()
    }
}

/// One sparse vector: position `(x, y)` in the coarse grid and displacement `(u, v)`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct SparseVec {
    pub x: f32,
    pub y: f32,
    pub u: f32,
    pub v: f32,
}

/// Dense Lucas-Kanade on a stack of frames (`T >= 2`), spec 6.2 and 6.3.
///
/// NaN marks missing cells. Returns a velocity field of the frames' shape in pixels
/// per input step. Returns zeros when no vectors survive.
pub fn dense_lucaskanade(
    frames: &[Field2<f32>],
    params: &MotionParams,
) -> Result<Velocity, NowcastError> {
    if frames.len() < 2 {
        return Err(NowcastError::InvalidInput(
            "dense LK needs at least 2 frames".into(),
        ));
    }
    let (w, h) = (frames[0].width(), frames[0].height());
    if frames.iter().any(|f| f.width() != w || f.height() != h) {
        return Err(NowcastError::InvalidInput("frames differ in shape".into()));
    }

    let mut sparse: Vec<SparseVec> = Vec::new();
    for pair in frames.windows(2) {
        sparse.extend(pair_vectors(&pair[0], &pair[1], params)?);
    }
    if sparse.is_empty() {
        return Ok(Velocity::zeros(w, h));
    }

    let xy: Vec<[f32; 2]> = sparse.iter().map(|s| [s.x, s.y]).collect();
    let uv: Vec<[f32; 2]> = sparse.iter().map(|s| [s.u, s.v]).collect();
    let outliers = sparse::detect_outliers(&uv, &xy, params.nr_std_outlier, params.k_outlier);
    let (mut xy, mut uv): (Vec<[f32; 2]>, Vec<[f32; 2]>) = xy
        .into_iter()
        .zip(uv)
        .zip(outliers)
        .filter(|(_, out)| !out)
        .map(|(p, _)| p)
        .unzip();

    if params.decl_scale > 1.0 {
        let (dxy, duv) = sparse::decluster(&xy, &uv, params.decl_scale);
        xy = dxy;
        uv = duv;
    }
    if xy.is_empty() {
        return Ok(Velocity::zeros(w, h));
    }
    Ok(sparse::idw_interpolate(&xy, &uv, w, h, &params.idw))
}

/// Motion field at full resolution from a stride-`k` coarse Lucas-Kanade (spec 6.1).
///
/// The stack is subsampled with `[::k, ::k]`, dense LK runs on the coarse stack,
/// NaN in the coarse field becomes 0, the field is bilinearly upsampled with
/// align-corners geometry (scipy `zoom(order=1)`), cropped to `H x W` and multiplied by `k`.
/// `stride <= 1` runs LK at full resolution.
pub fn estimate_motion(
    stack: &[Field2<f32>],
    stride: usize,
    params: &MotionParams,
) -> Result<Velocity, NowcastError> {
    if stride <= 1 {
        return dense_lucaskanade(stack, params);
    }
    let (h, w) = (stack[0].height(), stack[0].width());
    let coarse: Vec<Field2<f32>> = stack.iter().map(|f| subsample(f, stride)).collect();
    let uv = dense_lucaskanade(&coarse, params)?;
    let k = stride as f32;
    let x = upsample_align_corners(&nan_to_zero(&uv.x), stride, h, w, k);
    let y = upsample_align_corners(&nan_to_zero(&uv.y), stride, h, w, k);
    Velocity::new(x, y)
}

/// `field[::k, ::k]`, shape `ceil(H/k) x ceil(W/k)`.
fn subsample(f: &Field2<f32>, k: usize) -> Field2<f32> {
    let (w, h) = (f.width(), f.height());
    let (cw, ch) = (w.div_ceil(k), h.div_ceil(k));
    let mut out = Vec::with_capacity(cw * ch);
    for cy in 0..ch {
        let row = f.row(cy * k);
        for cx in 0..cw {
            out.push(row[cx * k]);
        }
    }
    Field2::new(cw, ch, out).expect("subsample shape")
}

fn nan_to_zero(f: &Field2<f32>) -> Field2<f32> {
    let data = f
        .as_slice()
        .iter()
        .map(|&v| if v.is_nan() { 0.0 } else { v })
        .collect();
    Field2::new(f.width(), f.height(), data).expect("same shape")
}

/// `scipy.ndimage.zoom(src, k, order=1)[:h, :w] * k` with `grid_mode=False`
/// (output index `o` maps to input `o * (n_in - 1) / (n_out - 1)`), edge-padded to `h x w`.
fn upsample_align_corners(
    src: &Field2<f32>,
    k: usize,
    h: usize,
    w: usize,
    scale: f32,
) -> Field2<f32> {
    let (sw, sh) = (src.width(), src.height());
    let out_w = sw * k;
    let out_h = sh * k;
    let map = |o: usize, n_out: usize, n_in: usize| -> f64 {
        if n_out <= 1 || n_in <= 1 {
            0.0
        } else {
            o as f64 * (n_in as f64 - 1.0) / (n_out as f64 - 1.0)
        }
    };
    let xs: Vec<(usize, usize, f64)> = (0..w)
        .map(|o| {
            let o = o.min(out_w - 1);
            let c = map(o, out_w, sw);
            let i0 = (c.floor() as usize).min(sw - 1);
            let i1 = (i0 + 1).min(sw - 1);
            (i0, i1, c - i0 as f64)
        })
        .collect();
    let mut data = vec![0.0f32; w * h];
    data.par_chunks_mut(w).enumerate().for_each(|(y, row)| {
        let oy = y.min(out_h - 1);
        let cy = map(oy, out_h, sh);
        let j0 = (cy.floor() as usize).min(sh - 1);
        let j1 = (j0 + 1).min(sh - 1);
        let fy = cy - j0 as f64;
        for (x, cell) in row.iter_mut().enumerate() {
            let (i0, i1, fx) = xs[x];
            let a = src.get(i0, j0) as f64;
            let b = src.get(i1, j0) as f64;
            let c = src.get(i0, j1) as f64;
            let d = src.get(i1, j1) as f64;
            let top = a + (b - a) * fx;
            let bottom = c + (d - c) * fx;
            let v = top + (bottom - top) * fy;
            *cell = (v as f32) * scale;
        }
    });
    Field2::new(w, h, data).expect("upsample shape")
}

/// Sparse vectors for one consecutive pair (spec 6.2 steps 1 to 4).
fn pair_vectors(
    prev: &Field2<f32>,
    next: &Field2<f32>,
    params: &MotionParams,
) -> Result<Vec<SparseVec>, NowcastError> {
    let prvs = morph::prepare(prev, params.size_opening)?;
    let nxt = morph::prepare(next, params.size_opening)?;
    let points = shitomasi::good_features(&prvs, &params.shitomasi);
    if points.is_empty() {
        return Ok(Vec::new());
    }
    let a = scale_to_u8(&prvs);
    let b = scale_to_u8(&nxt);
    let tracks = pyrlk::track(&a, &b, &points, &params.lk);
    Ok(points
        .iter()
        .zip(tracks)
        .filter_map(|(p, t)| {
            t.map(|(u, v)| SparseVec {
                x: p[0],
                y: p[1],
                u,
                v,
            })
        })
        .collect())
}

/// `track_features` scaling: map the valid min..max to 0..255, NaN to 0 (its fill value
/// is the minimum, which maps to 0).
fn scale_to_u8(img: &Field2<f32>) -> Field2<u8> {
    let (lo, hi) = valid_range(img.as_slice());
    let range = hi - lo;
    let data = img
        .as_slice()
        .iter()
        .map(|&v| {
            let v = if v.is_nan() { lo } else { v };
            let s = if range > 1e-8 {
                (v - lo) / range * 255.0
            } else {
                v - lo
            };
            s.clamp(0.0, 255.0) as u8
        })
        .collect();
    Field2::new(img.width(), img.height(), data).expect("same shape")
}

/// REFLECT_101 index mapping (`dcb|abcd|cba`), OpenCV's default border for filters.
pub(crate) fn reflect101(i: i64, n: i64) -> i64 {
    if n == 1 {
        return 0;
    }
    let mut i = i;
    if i < 0 {
        i = -i;
    }
    if i >= n {
        i = 2 * (n - 1) - i;
    }
    i.clamp(0, n - 1)
}

/// Min and max over non-NaN values (0, 0 when there are none).
pub(crate) fn valid_range(values: &[f32]) -> (f32, f32) {
    let mut lo = f32::INFINITY;
    let mut hi = f32::NEG_INFINITY;
    for &v in values {
        if !v.is_nan() {
            lo = lo.min(v);
            hi = hi.max(v);
        }
    }
    if lo > hi { (0.0, 0.0) } else { (lo, hi) }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn constant_field_gives_zero_motion() {
        let f = Field2::filled(40, 30, 10.0f32);
        let frames = vec![f.clone(), f.clone(), f];
        let v = dense_lucaskanade(&frames, &MotionParams::radar_ng()).unwrap();
        assert!(v.x.as_slice().iter().all(|&a| a == 0.0));
        assert!(v.y.as_slice().iter().all(|&a| a == 0.0));
    }

    #[test]
    fn upsample_matches_scipy_zoom_mapping() {
        // zoom([0, 10], 2, order=1) = [0, 3.333, 6.667, 10]; with scale 1 the first row of
        // a 2x1 field becomes 4 samples. Use a 1-row field and read row 0.
        let src = Field2::new(2, 1, vec![0.0f32, 10.0]).unwrap();
        let out = upsample_align_corners(&src, 2, 1, 4, 1.0);
        let row = out.row(0);
        let want = [0.0, 10.0 / 3.0, 20.0 / 3.0, 10.0];
        for (a, b) in row.iter().zip(want) {
            assert!((*a as f64 - b).abs() < 1e-5, "{a} vs {b}");
        }
    }

    #[test]
    fn reflect101_matches_opencv_convention() {
        assert_eq!(reflect101(-1, 5), 1);
        assert_eq!(reflect101(5, 5), 3);
        assert_eq!(reflect101(2, 5), 2);
    }

    #[test]
    fn stride_wrapper_scales_by_stride() {
        // Constant-valued frames have no motion; check the wrapper keeps the shape and
        // finiteness for an odd-sized field (H, W not multiples of the stride).
        let f = Field2::filled(37, 21, 3.0f32);
        let v = estimate_motion(&[f.clone(), f.clone(), f], 2, &MotionParams::radar_ng()).unwrap();
        assert_eq!((v.width(), v.height()), (37, 21));
        assert!(v.x.as_slice().iter().all(|a| a.is_finite()));
    }
}
