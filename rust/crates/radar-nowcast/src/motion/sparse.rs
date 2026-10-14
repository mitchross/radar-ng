//! Sparse-vector cleansing and dense interpolation (pysteps `utils.cleansing` and
//! `utils.interpolate.idwinterp2d`).
//!
//! - [`detect_outliers`]: local Mahalanobis test over the `k` nearest neighbours. pysteps
//!   uses a KD-tree; we use an exact brute-force selection (O(N^2), fine for the few
//!   thousand vectors a radar pair yields). Tie order can differ from the KD-tree.
//! - [`decluster`]: one median vector per `scale x scale` bin.
//! - [`idw_interpolate`]: exact k-nearest inverse-distance interpolation over a regular
//!   grid, using a bucket index instead of a KD-tree.

use std::collections::BTreeMap;

use rayon::prelude::*;

use crate::field::Field2;
use crate::motion::{Idw, Velocity};

/// Mahalanobis outlier flags for 2-D vectors `uv` at positions `xy` (spec 6.3 step 1).
///
/// A sample is an outlier when its distance from the mean of its `k` nearest neighbours
/// (self excluded), measured with the neighbours' covariance (ddof 1), exceeds `thr`.
/// Singular covariance gives distance 0. Fewer than 2 samples yields no outliers.
pub(crate) fn detect_outliers(uv: &[[f32; 2]], xy: &[[f32; 2]], thr: f64, k: usize) -> Vec<bool> {
    let n = uv.len();
    if n < 2 {
        return vec![false; n];
    }
    let kk = (k + 1).min(n);
    (0..n)
        .into_par_iter()
        .map(|i| {
            let mut dist: Vec<(f64, usize)> = (0..n)
                .map(|j| {
                    let dx = xy[i][0] as f64 - xy[j][0] as f64;
                    let dy = xy[i][1] as f64 - xy[j][1] as f64;
                    (dx * dx + dy * dy, j)
                })
                .collect();
            let pivot = kk - 1;
            dist.select_nth_unstable_by(pivot, |a, b| a.partial_cmp(b).unwrap());
            let mut near: Vec<(f64, usize)> = dist[..kk].to_vec();
            near.sort_by(|a, b| a.partial_cmp(b).unwrap());
            // Drop the first entry (normally self), as `inds[i, 1:]` does.
            let nb: Vec<[f64; 2]> = near[1..]
                .iter()
                .map(|&(_, j)| [uv[j][0] as f64, uv[j][1] as f64])
                .collect();
            if nb.len() < 2 {
                return false;
            }
            let m = nb.len() as f64;
            let mean = [
                nb.iter().map(|p| p[0]).sum::<f64>() / m,
                nb.iter().map(|p| p[1]).sum::<f64>() / m,
            ];
            let (mut c11, mut c12, mut c22) = (0.0, 0.0, 0.0);
            for p in &nb {
                let (a, b) = (p[0] - mean[0], p[1] - mean[1]);
                c11 += a * a;
                c12 += a * b;
                c22 += b * b;
            }
            let denom = m - 1.0;
            let (c11, c12, c22) = (c11 / denom, c12 / denom, c22 / denom);
            let det = c11 * c22 - c12 * c12;
            if !det.is_finite() || det.abs() <= 8.0 * f64::EPSILON * (c11 * c22).abs() {
                return false; // singular: distance 0
            }
            let z = [uv[i][0] as f64 - mean[0], uv[i][1] as f64 - mean[1]];
            // z^T C^{-1} z with C^{-1} = [[c22, -c12], [-c12, c11]] / det.
            let q = (c22 * z[0] * z[0] - 2.0 * c12 * z[0] * z[1] + c11 * z[1] * z[1]) / det;
            q.sqrt() > thr
        })
        .collect()
}

/// Median-collapse vectors that fall into the same `scale x scale` bin (spec 6.3 step 2).
pub(crate) fn decluster(
    xy: &[[f32; 2]],
    uv: &[[f32; 2]],
    scale: f64,
) -> (Vec<[f32; 2]>, Vec<[f32; 2]>) {
    let mut bins: BTreeMap<(i64, i64), Vec<usize>> = BTreeMap::new();
    for (i, p) in xy.iter().enumerate() {
        let key = (
            (p[0] as f64 / scale).floor() as i64,
            (p[1] as f64 / scale).floor() as i64,
        );
        bins.entry(key).or_default().push(i);
    }
    let mut out_xy = Vec::with_capacity(bins.len());
    let mut out_uv = Vec::with_capacity(bins.len());
    for idx in bins.values() {
        let med = |f: &dyn Fn(usize) -> f64| -> f64 {
            let mut vals: Vec<f64> = idx.iter().map(|&i| f(i)).collect();
            vals.sort_by(|a, b| a.partial_cmp(b).unwrap());
            let m = vals.len();
            if m % 2 == 1 {
                vals[m / 2]
            } else {
                0.5 * (vals[m / 2 - 1] + vals[m / 2])
            }
        };
        out_xy.push([
            med(&|i| xy[i][0] as f64) as f32,
            med(&|i| xy[i][1] as f64) as f32,
        ]);
        out_uv.push([
            med(&|i| uv[i][0] as f64) as f32,
            med(&|i| uv[i][1] as f64) as f32,
        ]);
    }
    (out_xy, out_uv)
}

/// Edge length (pixels) of the blocks that share one IDW candidate set.
const IDW_BLOCK: usize = 16;

/// `d^power` with the common exponents special-cased (`0.5` is the default).
#[inline]
fn idw_power(d: f64, power: f64) -> f64 {
    if power == 0.5 {
        d.sqrt()
    } else if power == 1.0 {
        d
    } else if power == 2.0 {
        d * d
    } else {
        d.powf(power)
    }
}

/// Inverse-distance interpolation of `uv` at `xy` to every pixel of a `w x h` grid
/// (spec 6.3 step 3): `k` nearest vectors, weights `1 / (d + offset)^power`.
///
/// The `k` nearest points of every pixel in an `IDW_BLOCK` square lie within
/// `reach = d_k(centre) + 2 * half_diagonal` of the block centre, because the k-th
/// nearest distance is 1-Lipschitz in the query position. So each block gathers one
/// candidate set and every pixel selects its exact `k` nearest from that set.
pub(crate) fn idw_interpolate(
    xy: &[[f32; 2]],
    uv: &[[f32; 2]],
    w: usize,
    h: usize,
    p: &Idw,
) -> Velocity {
    let n = xy.len();
    if n == 0 || w == 0 || h == 0 {
        return Velocity::zeros(w, h);
    }
    let k = p.k.min(n).max(1);
    let mut ux = vec![0.0f32; w * h];
    let mut uy = vec![0.0f32; w * h];
    let cmp =
        |a: &(f64, usize), b: &(f64, usize)| a.0.partial_cmp(&b.0).unwrap().then(a.1.cmp(&b.1));
    ux.par_chunks_mut(IDW_BLOCK * w)
        .zip(uy.par_chunks_mut(IDW_BLOCK * w))
        .enumerate()
        .for_each(|(block_row, (rx, ry))| {
            let y0 = block_row * IDW_BLOCK;
            let y1 = y0 + rx.len() / w - 1;
            let mut scratch: Vec<f64> = Vec::with_capacity(n);
            let mut cand: Vec<usize> = Vec::new();
            let mut scored: Vec<(f64, usize)> = Vec::new();
            for bx in (0..w).step_by(IDW_BLOCK) {
                let x1 = (bx + IDW_BLOCK - 1).min(w - 1);
                let cx = (bx + x1) as f64 / 2.0;
                let cy = (y0 + y1) as f64 / 2.0;
                let half_diag =
                    ((((x1 - bx) as f64).powi(2) + ((y1 - y0) as f64).powi(2)).sqrt()) / 2.0;
                scratch.clear();
                scratch.extend(xy.iter().map(|q| {
                    let dx = q[0] as f64 - cx;
                    let dy = q[1] as f64 - cy;
                    dx * dx + dy * dy
                }));
                let (_, kth, _) =
                    scratch.select_nth_unstable_by(k - 1, |a, b| a.partial_cmp(b).unwrap());
                let reach = kth.sqrt() + 2.0 * half_diag + 1e-9;
                let reach2 = reach * reach;
                cand.clear();
                cand.extend(
                    xy.iter()
                        .enumerate()
                        .filter(|(_, q)| {
                            let dx = q[0] as f64 - cx;
                            let dy = q[1] as f64 - cy;
                            dx * dx + dy * dy <= reach2
                        })
                        .map(|(j, _)| j),
                );
                for y in y0..=y1 {
                    for x in bx..=x1 {
                        let (qx, qy) = (x as f64, y as f64);
                        scored.clear();
                        scored.extend(cand.iter().map(|&j| {
                            let dx = xy[j][0] as f64 - qx;
                            let dy = xy[j][1] as f64 - qy;
                            (dx * dx + dy * dy, j)
                        }));
                        if scored.len() > k {
                            scored.select_nth_unstable_by(k - 1, cmp);
                            scored.truncate(k);
                        }
                        scored.sort_by(cmp);
                        let (mut sw, mut su, mut sv) = (0.0f64, 0.0f64, 0.0f64);
                        for &(d2, j) in &scored {
                            let wgt = 1.0 / idw_power(d2.sqrt() + p.dist_offset, p.power);
                            sw += wgt;
                            su += wgt * uv[j][0] as f64;
                            sv += wgt * uv[j][1] as f64;
                        }
                        let i = (y - y0) * w + x;
                        rx[i] = (su / sw) as f32;
                        ry[i] = (sv / sw) as f32;
                    }
                }
            }
        });
    Velocity {
        x: Field2::new(w, h, ux).expect("shape"),
        y: Field2::new(w, h, uy).expect("shape"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn outlier_is_flagged_among_consistent_vectors() {
        // 40 vectors on a grid with a common motion (1, 0.5) and one wild vector.
        let mut xy = Vec::new();
        let mut uv = Vec::new();
        for i in 0..40 {
            xy.push([(i % 8) as f32 * 3.0, (i / 8) as f32 * 3.0]);
            uv.push([1.0 + 0.01 * (i % 3) as f32, 0.5 - 0.01 * (i % 4) as f32]);
        }
        xy.push([12.0, 9.0]);
        uv.push([30.0, -20.0]);
        let flags = detect_outliers(&uv, &xy, 3.0, 30);
        assert!(flags[40], "wild vector flagged");
        assert!(flags[..40].iter().all(|&f| !f), "consistent vectors kept");
    }

    #[test]
    fn decluster_takes_bin_medians() {
        let xy = [[1.0f32, 1.0], [2.0, 2.0], [3.0, 3.0], [50.0, 50.0]];
        let uv = [[1.0f32, 0.0], [5.0, 0.0], [2.0, 0.0], [9.0, 9.0]];
        let (dxy, duv) = decluster(&xy, &uv, 20.0);
        assert_eq!(dxy.len(), 2);
        // First bin holds the first three: median u = 2, median position = (2, 2).
        assert_eq!(duv[0], [2.0, 0.0]);
        assert_eq!(dxy[0], [2.0, 2.0]);
    }

    #[test]
    fn idw_reproduces_a_single_vector_everywhere() {
        let xy = [[5.0f32, 5.0]];
        let uv = [[2.0f32, -1.0]];
        let v = idw_interpolate(
            &xy,
            &uv,
            10,
            8,
            &Idw {
                power: 0.5,
                k: 20,
                dist_offset: 0.5,
            },
        );
        assert!(v.x.as_slice().iter().all(|&a| (a - 2.0).abs() < 1e-6));
        assert!(v.y.as_slice().iter().all(|&a| (a + 1.0).abs() < 1e-6));
    }

    #[test]
    fn idw_matches_brute_force() {
        let xy: Vec<[f32; 2]> = (0..200)
            .map(|i| [((i * 53) % 97) as f32 + 0.25, ((i * 31) % 89) as f32])
            .collect();
        let uv: Vec<[f32; 2]> = (0..200)
            .map(|i| [(i % 7) as f32 * 0.3 - 1.0, (i % 5) as f32 * 0.2])
            .collect();
        let (w, h) = (101, 75); // not multiples of the block size
        let p = Idw {
            power: 0.5,
            k: 7,
            dist_offset: 0.5,
        };
        let got = idw_interpolate(&xy, &uv, w, h, &p);
        for y in 0..h {
            for x in 0..w {
                let mut all: Vec<(f64, usize)> = xy
                    .iter()
                    .enumerate()
                    .map(|(j, q)| {
                        let dx = q[0] as f64 - x as f64;
                        let dy = q[1] as f64 - y as f64;
                        (dx * dx + dy * dy, j)
                    })
                    .collect();
                all.sort_by(|a, b| a.0.partial_cmp(&b.0).unwrap().then(a.1.cmp(&b.1)));
                let (mut sw, mut su, mut sv) = (0.0f64, 0.0f64, 0.0f64);
                for &(d2, j) in &all[..7] {
                    let wgt = 1.0 / (d2.sqrt() + 0.5).powf(0.5);
                    sw += wgt;
                    su += wgt * uv[j][0] as f64;
                    sv += wgt * uv[j][1] as f64;
                }
                let i = y * w + x;
                assert!(
                    (got.x.as_slice()[i] as f64 - su / sw).abs() < 1e-4,
                    "u at ({x},{y})"
                );
                assert!(
                    (got.y.as_slice()[i] as f64 - sv / sw).abs() < 1e-4,
                    "v at ({x},{y})"
                );
            }
        }
    }
}
