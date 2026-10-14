//! FFT cascade decomposition with the Gaussian bandpass (pysteps
//! `cascade.bandpass_filters.filter_gaussian` and `cascade.decomposition.decomposition_fft`
//! with `normalize=True`, `output_domain="spatial"`).
//!
//! Layout: a spectrum of a real `h x w` field has `wh = w/2 + 1` columns (frequency `j`)
//! and `h` rows (frequency `i`). Storage is column-major (`index = j * h + i`) so that the
//! column transforms run on contiguous memory. The transform is `rfft2` in numpy's
//! convention: real FFT along the row (width) axis, then complex FFT along the column
//! (height) axis. Inverse normalisation is `1 / (h w)`, as in `numpy.fft.irfft2`.
//!
//! Frequency radius uses the signed-index convention (`|i|` folded at `h/2`), so the
//! Nyquist row is treated like pysteps' `np.roll` layout. Arithmetic is `f64` (DEC-6).

use std::sync::Arc;

use rayon::prelude::*;
use realfft::{ComplexToReal, RealFftPlanner, RealToComplex};
use rustfft::num_complex::Complex64;
use rustfft::{Fft, FftPlanner};

use crate::error::NowcastError;

/// Spectrum of a real field in column-major layout (`index = j * h + i`).
#[derive(Clone, Debug)]
pub struct Spectrum {
    /// Field height (number of rows, FFT length along the column axis).
    pub h: usize,
    /// Number of non-negative width frequencies, `w / 2 + 1`.
    pub wh: usize,
    /// Column-major data, length `h * wh`.
    pub data: Vec<Complex64>,
}

/// Plans and scratch-free FFT machinery for one `h x w` shape.
pub struct Fft2 {
    h: usize,
    w: usize,
    r2c: Arc<dyn RealToComplex<f64>>,
    c2r: Arc<dyn ComplexToReal<f64>>,
    fwd: Arc<dyn Fft<f64>>,
    inv: Arc<dyn Fft<f64>>,
}

impl Fft2 {
    /// Create plans for `h x w` real fields.
    pub fn new(h: usize, w: usize) -> Self {
        let mut rp = RealFftPlanner::<f64>::new();
        let mut cp = FftPlanner::<f64>::new();
        Self {
            h,
            w,
            r2c: rp.plan_fft_forward(w),
            c2r: rp.plan_fft_inverse(w),
            fwd: cp.plan_fft_forward(h),
            inv: cp.plan_fft_inverse(h),
        }
    }

    /// Number of width frequencies, `w / 2 + 1`.
    pub fn wh(&self) -> usize {
        self.w / 2 + 1
    }

    /// `numpy.fft.rfft2` of a row-major `h x w` real field.
    pub fn rfft2(&self, field: &[f64]) -> Spectrum {
        let (h, w, wh) = (self.h, self.w, self.wh());
        assert_eq!(field.len(), h * w, "rfft2 shape");
        // Row transforms, row-major output.
        let mut rows = vec![Complex64::new(0.0, 0.0); h * wh];
        rows.par_chunks_mut(wh)
            .zip(field.par_chunks(w))
            .for_each_init(
                || (self.r2c.make_input_vec(), self.r2c.make_scratch_vec()),
                |(input, scratch), (out, src)| {
                    input.copy_from_slice(src);
                    self.r2c
                        .process_with_scratch(input, out, scratch)
                        .expect("r2c buffers sized by planner");
                },
            );
        // Transpose to column-major and transform the columns. Blocks of `TRANSPOSE_BLOCK`
        // columns keep the reads from `rows` contiguous.
        let mut data = vec![Complex64::new(0.0, 0.0); h * wh];
        data.par_chunks_mut(TRANSPOSE_BLOCK * h)
            .enumerate()
            .for_each(|(blk, chunk)| {
                let j0 = blk * TRANSPOSE_BLOCK;
                let cols = chunk.len() / h;
                for i in 0..h {
                    let src = &rows[i * wh + j0..i * wh + j0 + cols];
                    for (jj, &c) in src.iter().enumerate() {
                        chunk[jj * h + i] = c;
                    }
                }
            });
        drop(rows);
        self.column_transform(&mut data, &self.fwd);
        Spectrum { h, wh, data }
    }

    /// `numpy.fft.irfft2` of a column-major spectrum, returning a row-major real field.
    pub fn irfft2(&self, spec: &Spectrum) -> Vec<f64> {
        assert_eq!(spec.data.len(), self.h * self.wh(), "irfft2 shape");
        self.inverse_from(spec.data.clone())
    }

    /// `irfft2(spec * weights)` for a real per-frequency weight, without an intermediate
    /// weighted spectrum (same arithmetic as [`apply_weights`] followed by [`Fft2::irfft2`]).
    pub fn irfft2_weighted(&self, spec: &Spectrum, weights: &[f64]) -> Vec<f64> {
        assert_eq!(spec.data.len(), self.h * self.wh(), "irfft2 shape");
        assert_eq!(weights.len(), spec.data.len(), "weights shape");
        let cols: Vec<Complex64> = spec
            .data
            .par_iter()
            .zip(weights.par_iter())
            .map(|(c, &w)| c * w)
            .collect();
        self.inverse_from(cols)
    }

    /// Inverse transform of a column-major spectrum that the caller owns.
    fn inverse_from(&self, mut cols: Vec<Complex64>) -> Vec<f64> {
        let (h, w, wh) = (self.h, self.w, self.wh());
        self.column_transform(&mut cols, &self.inv);
        // DC and Nyquist columns: numpy's c2r ignores their imaginary parts.
        // Each block of `TRANSPOSE_BLOCK` output rows is gathered from contiguous column
        // runs, then transformed row by row.
        let nyq = if w % 2 == 0 { Some(wh - 1) } else { None };
        let mut out = vec![0.0f64; h * w];
        let cols = &cols;
        out.par_chunks_mut(TRANSPOSE_BLOCK * w)
            .enumerate()
            .for_each_init(
                || {
                    (
                        self.c2r.make_input_vec(),
                        self.c2r.make_scratch_vec(),
                        vec![Complex64::new(0.0, 0.0); TRANSPOSE_BLOCK * wh],
                    )
                },
                |(input, scratch, block), (blk, dst_block)| {
                    let r0 = blk * TRANSPOSE_BLOCK;
                    let rows = dst_block.len() / w;
                    for j in 0..wh {
                        let src = &cols[j * h + r0..j * h + r0 + rows];
                        for (rr, &c) in src.iter().enumerate() {
                            block[rr * wh + j] = c;
                        }
                    }
                    for (rr, dst) in dst_block.chunks_mut(w).enumerate() {
                        input.copy_from_slice(&block[rr * wh..(rr + 1) * wh]);
                        input[0].im = 0.0;
                        if let Some(n) = nyq {
                            input[n].im = 0.0;
                        }
                        self.c2r
                            .process_with_scratch(input, dst, scratch)
                            .expect("c2r buffers sized by planner");
                    }
                },
            );
        let scale = 1.0 / (h as f64 * w as f64);
        out.par_iter_mut().for_each(|v| *v *= scale);
        out
    }

    /// Apply an in-place FFT to every column (`h`-long contiguous chunks).
    fn column_transform(&self, data: &mut [Complex64], plan: &Arc<dyn Fft<f64>>) {
        let scratch_len = plan.get_inplace_scratch_len();
        data.par_chunks_mut(self.h).for_each_init(
            || vec![Complex64::new(0.0, 0.0); scratch_len],
            |scratch, col| plan.process_with_scratch(col, scratch),
        );
    }
}

/// Columns (forward) or rows (inverse) handled per cache block by the transposes.
const TRANSPOSE_BLOCK: usize = 16;

/// Gaussian bandpass weights per level, normalised over levels (spec 7.4).
pub struct BandpassFilter {
    /// Field height.
    pub h: usize,
    /// Field width.
    pub w: usize,
    /// Central wavenumbers `r_k` (index units), one per level.
    pub central_wavenumbers: Vec<f64>,
    /// Weights per level in column-major spectrum layout (`index = j * h + i`).
    pub weights: Vec<Vec<f64>>,
}

impl BandpassFilter {
    /// Number of cascade levels.
    pub fn n_levels(&self) -> usize {
        self.weights.len()
    }
}

/// Build the Gaussian bandpass for `h x w` fields with `n` levels and width `s` (`gauss_scale`).
///
/// `q = (L/2)^(1/n)` with `L = max(h, w)`, `r_k = (q^(k-1) + q^k)/2`, and in log-`q` space a
/// Gaussian of standard deviation `s` centred on `log_q(r_k)`. Weights are normalised so
/// that they sum to one at every frequency. The DC term belongs to the first level.
pub fn gaussian_bandpass(
    h: usize,
    w: usize,
    n_levels: usize,
    gauss_scale: f64,
) -> Result<BandpassFilter, NowcastError> {
    if n_levels < 3 {
        return Err(NowcastError::InvalidInput(
            "n_cascade_levels must be at least 3".into(),
        ));
    }
    if h == 0 || w == 0 {
        return Err(NowcastError::InvalidInput("empty field".into()));
    }
    let l = h.max(w) as f64;
    let q = (0.5 * l).powf(1.0 / n_levels as f64);
    let log_q = |x: f64| if x == 0.0 { 0.0 } else { x.ln() / q.ln() };
    let r: Vec<f64> = (1..=n_levels)
        .map(|k| 0.5 * (q.powi(k as i32 - 1) + q.powi(k as i32)))
        .collect();
    let centres: Vec<f64> = r.iter().map(|&rk| log_q(rk)).collect();
    let wh = w / 2 + 1;
    let s2 = 2.0 * gauss_scale * gauss_scale;
    let mut weights = vec![vec![0.0f64; h * wh]; n_levels];
    for j in 0..wh {
        for i in 0..h {
            let fi = i.min(h - i) as f64;
            let radius = ((j * j) as f64 + fi * fi).sqrt();
            let lr = log_q(radius);
            let idx = j * h + i;
            let mut sum = 0.0;
            for k in 0..n_levels {
                let d = lr - centres[k];
                let wk = (-(d * d) / s2).exp();
                weights[k][idx] = wk;
                sum += wk;
            }
            for wk in weights.iter_mut() {
                wk[idx] /= sum;
            }
        }
    }
    for (k, wk) in weights.iter_mut().enumerate() {
        wk[0] = if k == 0 { 1.0 } else { 0.0 };
    }
    Ok(BandpassFilter {
        h,
        w,
        central_wavenumbers: r,
        weights,
    })
}

/// Multiply a spectrum by a real per-frequency weight (column-major, same layout).
pub fn apply_weights(spec: &Spectrum, weights: &[f64]) -> Spectrum {
    let data = spec.data.iter().zip(weights).map(|(c, &w)| c * w).collect();
    Spectrum {
        h: spec.h,
        wh: spec.wh,
        data,
    }
}

/// Population mean and standard deviation (numpy `mean`, `std` with ddof 0), two-pass.
pub fn mean_std(values: &[f64]) -> (f64, f64) {
    let n = values.len() as f64;
    let mu = values.iter().sum::<f64>() / n;
    let var = values.iter().map(|v| (v - mu) * (v - mu)).sum::<f64>() / n;
    (mu, var.sqrt())
}

/// Normalised cascade level: `(field - mu) / sigma` plus the statistics (spec 7.5).
pub fn normalise(field: &[f64]) -> (Vec<f64>, f64, f64) {
    let (mu, sigma) = mean_std(field);
    let z = field.iter().map(|v| (v - mu) / sigma).collect();
    (z, mu, sigma)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn centre_wavenumbers_match_appendix_a1() {
        let f = gaussian_bandpass(16, 32, 6, 0.5).unwrap();
        let want = [1.293701, 2.053622, 3.259921, 5.174802, 8.214486, 13.039684];
        for (a, b) in f.central_wavenumbers.iter().zip(want) {
            assert!((a - b).abs() < 1e-6, "{a} vs {b}");
        }
        let big = gaussian_bandpass(3500, 3500, 6, 0.5).unwrap();
        let want_big = [
            2.235707,
            7.761062,
            26.941856,
            93.526322,
            324.668533,
            1127.058711,
        ];
        for (a, b) in big.central_wavenumbers.iter().zip(want_big) {
            assert!((a - b).abs() < 1e-5 * b, "{a} vs {b}");
        }
    }

    #[test]
    fn weights_at_origin_and_first_bins_match_appendix_a1() {
        let f = gaussian_bandpass(16, 32, 6, 0.5).unwrap();
        // (i, j) = (0, 0): [1, 0, 0, 0, 0, 0]
        for k in 0..6 {
            let want = if k == 0 { 1.0 } else { 0.0 };
            assert_eq!(f.weights[k][0], want);
        }
        // (i, j) = (0, 1): [0.985639, 0.014357, 0.000004, 0, 0, 0]
        let idx = 16; // j = 1, i = 0
        let want = [0.985639, 0.014357, 0.000004, 0.0, 0.0, 0.0];
        for k in 0..6 {
            assert!(
                (f.weights[k][idx] - want[k]).abs() < 1e-6,
                "level {k}: {}",
                f.weights[k][idx]
            );
        }
    }

    #[test]
    fn weights_sum_to_one_everywhere() {
        let f = gaussian_bandpass(24, 40, 6, 0.5).unwrap();
        for idx in 0..f.weights[0].len() {
            let s: f64 = f.weights.iter().map(|wk| wk[idx]).sum();
            assert!((s - 1.0).abs() < 1e-12, "sum {s} at {idx}");
        }
    }

    #[test]
    fn fft_round_trip_is_identity() {
        let (h, w) = (12usize, 20usize);
        let plan = Fft2::new(h, w);
        let field: Vec<f64> = (0..h * w)
            .map(|i| ((i * 7919) % 97) as f64 / 13.0 - 3.0)
            .collect();
        let back = plan.irfft2(&plan.rfft2(&field));
        for (a, b) in field.iter().zip(&back) {
            assert!((a - b).abs() < 1e-10);
        }
    }

    #[test]
    fn levels_sum_back_to_the_field() {
        // With normalised weights summing to one, sum_k irfft2(F * W_k) = field (exact).
        let (h, w) = (16usize, 32usize);
        let plan = Fft2::new(h, w);
        let bp = gaussian_bandpass(h, w, 6, 0.5).unwrap();
        let field: Vec<f64> = (0..h * w).map(|i| ((i * 31) % 17) as f64).collect();
        let spec = plan.rfft2(&field);
        let mut sum = vec![0.0; h * w];
        for wk in &bp.weights {
            let part = plan.irfft2(&apply_weights(&spec, wk));
            for (s, p) in sum.iter_mut().zip(part) {
                *s += p;
            }
        }
        for (a, b) in field.iter().zip(&sum) {
            assert!((a - b).abs() < 1e-9);
        }
    }
}
