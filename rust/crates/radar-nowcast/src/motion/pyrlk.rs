//! Pyramidal iterative Lucas-Kanade, our implementation of OpenCV
//! `calcOpticalFlowPyrLK` as radar-ng calls it (`winSize=50`, `maxLevel=3`,
//! `criteria=(COUNT|EPS, 10, 0)`, `minEigThreshold=1e-4`).
//!
//! Approximations versus OpenCV (documented in the crate README):
//! - Pyramids use the 5-tap binomial `pyrDown` with REFLECT_101 borders and derivatives use
//!   3x3 Scharr (1/32 scale), as OpenCV does.
//! - Window samples use bilinear interpolation with clamped coordinates (replicate border),
//!   and template values/gradients come from the previous image only.
//! - The minimum-eigenvalue test runs at level 0 only and is divided by the window area.
//!   A feature failing it, or whose final position leaves the image by more than half a
//!   window, is dropped (status 0). Coarse-level singular systems skip the Newton update.
//! - Floating point is `f64` in the normal equations. OpenCV uses `f32` internally, so
//!   vectors can differ by about 1e-3 px.

use rayon::prelude::*;

use super::reflect101;
use crate::field::Field2;
use crate::motion::PyrLk;

/// `FLT_EPSILON`, the singularity threshold OpenCV uses for the normal matrix.
const FLT_EPS: f64 = 1.192_092_9e-7;

/// Track points from `prev` to `next` (both 8-bit). Returns, per input point, the
/// displacement `(u, v)` or `None` when the feature is lost.
pub(crate) fn track(
    prev: &Field2<u8>,
    next: &Field2<u8>,
    pts: &[[f32; 2]],
    p: &PyrLk,
) -> Vec<Option<(f32, f32)>> {
    let to_f32 = |f: &Field2<u8>| {
        Field2::new(
            f.width(),
            f.height(),
            f.as_slice().iter().map(|&v| v as f32).collect(),
        )
        .expect("same shape")
    };
    let prev_pyr = build_pyramid(to_f32(prev), p.max_level);
    let next_pyr = build_pyramid(to_f32(next), p.max_level);
    let grads: Vec<(Field2<f32>, Field2<f32>)> = prev_pyr.iter().map(scharr).collect();
    let half = (p.win_size as f64 - 1.0) / 2.0;
    let offs: Vec<f64> = (0..p.win_size).map(|j| j as f64 - half).collect();
    let (w, h) = (prev.width() as f64, prev.height() as f64);
    let margin = (p.win_size as f64) / 2.0;
    pts.par_iter()
        .map(|pt| {
            let (u, v) = track_point(&prev_pyr, &grads, &next_pyr, *pt, p, &offs)?;
            let (nx, ny) = (pt[0] as f64 + u, pt[1] as f64 + v);
            if nx < -margin || ny < -margin || nx > w - 1.0 + margin || ny > h - 1.0 + margin {
                return None;
            }
            Some((u as f32, v as f32))
        })
        .collect()
}

/// Track one point through the pyramid. Returns the full-resolution displacement.
fn track_point(
    prev_pyr: &[Field2<f32>],
    grads: &[(Field2<f32>, Field2<f32>)],
    next_pyr: &[Field2<f32>],
    p0: [f32; 2],
    p: &PyrLk,
    offs: &[f64],
) -> Option<(f64, f64)> {
    let area = (offs.len() * offs.len()) as f64;
    let mut d = (0.0f64, 0.0f64); // displacement in the current level's pixels
    let levels = p.max_level + 1;
    let mut tpl = vec![0.0f64; offs.len() * offs.len()];
    let mut gxs = vec![0.0f64; offs.len() * offs.len()];
    let mut gys = vec![0.0f64; offs.len() * offs.len()];
    for lvl in (0..levels).rev() {
        let scale = 1.0 / (1u64 << lvl) as f64;
        let q = (p0[0] as f64 * scale, p0[1] as f64 * scale);
        let (gx_img, gy_img) = (&grads[lvl].0, &grads[lvl].1);
        let img = &prev_pyr[lvl];
        // Template samples and gradients (prev image) over the window.
        let (mut g11, mut g12, mut g22) = (0.0f64, 0.0f64, 0.0f64);
        let mut k = 0;
        for &oy in offs {
            for &ox in offs {
                let (x, y) = (q.0 + ox, q.1 + oy);
                tpl[k] = sample(img, x, y) as f64;
                let gx = sample(gx_img, x, y) as f64;
                let gy = sample(gy_img, x, y) as f64;
                gxs[k] = gx;
                gys[k] = gy;
                g11 += gx * gx;
                g12 += gx * gy;
                g22 += gy * gy;
                k += 1;
            }
        }
        let det = g11 * g22 - g12 * g12;
        if lvl == 0 {
            let disc = ((g11 - g22) * (g11 - g22) + 4.0 * g12 * g12).sqrt();
            let min_eig = 0.5 * (g11 + g22 - disc) / area;
            if min_eig < p.min_eig_threshold || det < FLT_EPS {
                return None;
            }
        }
        if det >= FLT_EPS {
            for _ in 0..p.max_iter {
                let (nx, ny) = (q.0 + d.0, q.1 + d.1);
                let (mut b1, mut b2) = (0.0f64, 0.0f64);
                let mut k = 0;
                for &oy in offs {
                    for &ox in offs {
                        let j = sample(&next_pyr[lvl], nx + ox, ny + oy) as f64;
                        let diff = tpl[k] - j;
                        b1 += diff * gxs[k];
                        b2 += diff * gys[k];
                        k += 1;
                    }
                }
                let delta = ((g22 * b1 - g12 * b2) / det, (g11 * b2 - g12 * b1) / det);
                d.0 += delta.0;
                d.1 += delta.1;
                if delta.0.abs() < p.epsilon && delta.1.abs() < p.epsilon {
                    break;
                }
            }
        }
        if lvl > 0 {
            d = (2.0 * d.0, 2.0 * d.1);
        }
    }
    Some(d)
}

/// Bilinear sample with clamped coordinates (replicate border).
#[inline]
fn sample(img: &Field2<f32>, x: f64, y: f64) -> f32 {
    let (w, h) = (img.width(), img.height());
    let x = x.clamp(0.0, (w - 1) as f64);
    let y = y.clamp(0.0, (h - 1) as f64);
    let x0 = x.floor();
    let y0 = y.floor();
    let fx = (x - x0) as f32;
    let fy = (y - y0) as f32;
    let (x0, y0) = (x0 as usize, y0 as usize);
    let x1 = (x0 + 1).min(w - 1);
    let y1 = (y0 + 1).min(h - 1);
    let a = img.get(x0, y0);
    let b = img.get(x1, y0);
    let c = img.get(x0, y1);
    let d = img.get(x1, y1);
    let top = a + (b - a) * fx;
    let bottom = c + (d - c) * fx;
    top + (bottom - top) * fy
}

/// Build `levels + 1` images: level 0 is the input, each next level is `pyrDown`.
fn build_pyramid(base: Field2<f32>, levels: usize) -> Vec<Field2<f32>> {
    let mut out = vec![base];
    for _ in 0..levels {
        let next = pyr_down(out.last().expect("non-empty"));
        out.push(next);
    }
    out
}

/// OpenCV `pyrDown`: separable `[1 4 6 4 1]/16` with REFLECT_101, then keep even samples.
fn pyr_down(src: &Field2<f32>) -> Field2<f32> {
    const K: [f32; 5] = [1.0 / 16.0, 4.0 / 16.0, 6.0 / 16.0, 4.0 / 16.0, 1.0 / 16.0];
    let (w, h) = (src.width(), src.height());
    let (nw, nh) = (w.div_ceil(2), h.div_ceil(2));
    let s = src.as_slice();
    // Horizontal pass on every row.
    let mut tmp = vec![0.0f32; w * h];
    for y in 0..h {
        for x in 0..w {
            let mut acc = 0.0f32;
            for (k, &kv) in K.iter().enumerate() {
                let xx = reflect101(x as i64 + k as i64 - 2, w as i64) as usize;
                acc += kv * s[y * w + xx];
            }
            tmp[y * w + x] = acc;
        }
    }
    // Vertical pass, decimated.
    let mut out = vec![0.0f32; nw * nh];
    for oy in 0..nh {
        for ox in 0..nw {
            let x = (2 * ox).min(w - 1);
            let mut acc = 0.0f32;
            for (k, &kv) in K.iter().enumerate() {
                let yy = reflect101((2 * oy) as i64 + k as i64 - 2, h as i64) as usize;
                acc += kv * tmp[yy * w + x];
            }
            out[oy * nw + ox] = acc;
        }
    }
    Field2::new(nw, nh, out).expect("pyramid shape")
}

/// Scharr derivatives (3x3, scale 1/32, REFLECT_101): `(dI/dx, dI/dy)`.
fn scharr(img: &Field2<f32>) -> (Field2<f32>, Field2<f32>) {
    let (w, h) = (img.width(), img.height());
    let s = img.as_slice();
    let at = |x: i64, y: i64| -> f32 {
        let xx = reflect101(x, w as i64) as usize;
        let yy = reflect101(y, h as i64) as usize;
        s[yy * w + xx]
    };
    let mut dx = vec![0.0f32; w * h];
    let mut dy = vec![0.0f32; w * h];
    for y in 0..h as i64 {
        for x in 0..w as i64 {
            let gx = 3.0 * (at(x + 1, y - 1) - at(x - 1, y - 1))
                + 10.0 * (at(x + 1, y) - at(x - 1, y))
                + 3.0 * (at(x + 1, y + 1) - at(x - 1, y + 1));
            let gy = 3.0 * (at(x - 1, y + 1) - at(x - 1, y - 1))
                + 10.0 * (at(x, y + 1) - at(x, y - 1))
                + 3.0 * (at(x + 1, y + 1) - at(x + 1, y - 1));
            let i = y as usize * w + x as usize;
            dx[i] = gx / 32.0;
            dy[i] = gy / 32.0;
        }
    }
    (
        Field2::new(w, h, dx).expect("shape"),
        Field2::new(w, h, dy).expect("shape"),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn blob(w: usize, h: usize, cx: f64, cy: f64) -> Field2<u8> {
        let mut v = vec![0u8; w * h];
        for y in 0..h {
            for x in 0..w {
                let r2 = (x as f64 - cx).powi(2) + (y as f64 - cy).powi(2);
                v[y * w + x] = (200.0 * (-r2 / (2.0 * 12.0 * 12.0)).exp()) as u8;
            }
        }
        Field2::new(w, h, v).unwrap()
    }

    #[test]
    fn recovers_integer_shift_of_a_blob() {
        let p = PyrLk {
            win_size: 50,
            max_level: 3,
            max_iter: 10,
            epsilon: 0.0,
            min_eig_threshold: 1e-4,
        };
        let a = blob(120, 120, 60.0, 60.0);
        let b = blob(120, 120, 63.0, 58.0);
        let pts = vec![[60.0f32, 60.0], [55.0, 62.0]];
        let tr = track(&a, &b, &pts, &p);
        for t in tr.into_iter().flatten() {
            assert!((t.0 - 3.0).abs() < 0.15, "u={}", t.0);
            assert!((t.1 + 2.0).abs() < 0.15, "v={}", t.1);
        }
    }

    #[test]
    fn flat_images_lose_all_points() {
        let p = PyrLk {
            win_size: 50,
            max_level: 3,
            max_iter: 10,
            epsilon: 0.0,
            min_eig_threshold: 1e-4,
        };
        let a = Field2::filled(64, 64, 7u8);
        let tr = track(&a, &a, &[[30.0, 30.0]], &p);
        assert!(tr[0].is_none());
    }
}
