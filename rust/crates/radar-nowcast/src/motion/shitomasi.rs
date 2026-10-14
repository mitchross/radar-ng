//! Shi-Tomasi corner detection, our implementation of OpenCV `goodFeaturesToTrack`
//! with `useHarrisDetector=False` (pysteps `feature.shitomasi.detection`).
//!
//! Steps: buffer the invalid mask (square dilation), scale to 8 bits as pysteps does,
//! compute the minimum eigenvalue of the `blockSize` structure tensor from 3x3 Sobel
//! gradients, keep 3x3 local maxima above `qualityLevel * max`, sort by response, then
//! greedily accept corners at least `minDistance` apart, up to `maxCorners`.
//!
//! Approximations versus OpenCV: all arithmetic is `f32` where OpenCV uses `f32`, borders
//! are REFLECT_101 for Sobel and REPLICATE for the box sum, and ties are broken by
//! `(y, x)` order instead of OpenCV's sort order. Corner sets agree closely but not
//! bit-for-bit (spec DEC-5).

use crate::field::Field2;
use crate::motion::ShiTomasi;

/// Detect corners in a prepared frame (NaN = invalid). Returns `(x, y)` points.
pub(crate) fn good_features(img: &Field2<f32>, p: &ShiTomasi) -> Vec<[f32; 2]> {
    let (w, h) = (img.width(), img.height());
    let src = img.as_slice();
    let valid: Vec<bool> = src.iter().map(|v| !v.is_nan()).collect();
    if !valid.iter().any(|&v| v) {
        return Vec::new();
    }
    // 1. Invalid mask dilated by a buffer_mask x buffer_mask square.
    let r = (p.buffer_mask / 2) as i64;
    let mut excluded = vec![false; w * h];
    if p.buffer_mask > 0 {
        for y in 0..h {
            for x in 0..w {
                if valid[y * w + x] {
                    continue;
                }
                for dy in -r..=r {
                    for dx in -r..=r {
                        let (xx, yy) = (x as i64 + dx, y as i64 + dy);
                        if xx >= 0 && yy >= 0 && xx < w as i64 && yy < h as i64 {
                            excluded[yy as usize * w + xx as usize] = true;
                        }
                    }
                }
            }
        }
    } else {
        for i in 0..w * h {
            excluded[i] = !valid[i];
        }
    }
    let usable: Vec<bool> = (0..w * h).map(|i| valid[i] && !excluded[i]).collect();
    if !usable.iter().any(|&v| v) {
        return Vec::new();
    }

    // 2. Scale to uint8 with the min/max of usable cells. Cells outside the usable set
    //    take the fill value (the minimum of all valid cells) and are then cast as numpy
    //    does (truncate, wrap modulo 256).
    let fill = src
        .iter()
        .filter(|v| !v.is_nan())
        .fold(f32::INFINITY, |a, &b| a.min(b));
    let (mut lo, mut hi) = (f32::INFINITY, f32::NEG_INFINITY);
    for i in 0..w * h {
        if usable[i] {
            lo = lo.min(src[i]);
            hi = hi.max(src[i]);
        }
    }
    let range = hi - lo;
    let to_u8 = |v: f32| -> u8 {
        let t = v.trunc() as i64;
        t.rem_euclid(256) as u8
    };
    let img8: Vec<f32> = (0..w * h)
        .map(|i| {
            let v = if usable[i] { src[i] } else { fill };
            let s = if range > 1e-8 {
                (v - lo) / range * 255.0
            } else {
                v - lo
            };
            to_u8(s) as f32
        })
        .collect();

    // 3. Structure tensor from Sobel gradients (scale factors do not change the result
    //    because the threshold is relative to the maximum).
    let at = |x: i64, y: i64| -> f32 {
        let xx = super::reflect101(x, w as i64);
        let yy = super::reflect101(y, h as i64);
        img8[yy as usize * w + xx as usize]
    };
    let mut ix = vec![0.0f32; w * h];
    let mut iy = vec![0.0f32; w * h];
    for y in 0..h as i64 {
        for x in 0..w as i64 {
            let gx = (at(x + 1, y - 1) + 2.0 * at(x + 1, y) + at(x + 1, y + 1))
                - (at(x - 1, y - 1) + 2.0 * at(x - 1, y) + at(x - 1, y + 1));
            let gy = (at(x - 1, y + 1) + 2.0 * at(x, y + 1) + at(x + 1, y + 1))
                - (at(x - 1, y - 1) + 2.0 * at(x, y - 1) + at(x + 1, y - 1));
            let idx = y as usize * w + x as usize;
            ix[idx] = gx;
            iy[idx] = gy;
        }
    }
    let b = p.block_size as i64;
    let br = b / 2;
    // Box sums with REPLICATE border of the three tensor components.
    let (sxx, syy, sxy) = box_sums3(&ix, &iy, w, h, br);
    let mut eig = vec![0.0f32; w * h];
    for i in 0..w * h {
        let (a, c, bb) = (sxx[i] as f64, syy[i] as f64, sxy[i] as f64);
        let half_tr = 0.5 * (a + c);
        let disc = (0.5 * (a - c)).powi(2) + bb * bb;
        eig[i] = (half_tr - disc.sqrt()) as f32;
    }

    // 4. Threshold on the maximum over usable cells.
    let max_val = (0..w * h)
        .filter(|&i| usable[i])
        .map(|i| eig[i])
        .fold(f32::NEG_INFINITY, f32::max);
    let thr = max_val as f64 * p.quality_level;

    // 5. Local maxima (3x3) above threshold.
    let mut cands: Vec<(f32, usize, usize)> = Vec::new();
    for y in 0..h {
        for x in 0..w {
            let i = y * w + x;
            if !usable[i] || (eig[i] as f64) <= thr {
                continue;
            }
            let mut is_max = true;
            'nb: for dy in -1i64..=1 {
                for dx in -1i64..=1 {
                    let (xx, yy) = (x as i64 + dx, y as i64 + dy);
                    if xx < 0 || yy < 0 || xx >= w as i64 || yy >= h as i64 {
                        continue;
                    }
                    if eig[yy as usize * w + xx as usize] > eig[i] {
                        is_max = false;
                        break 'nb;
                    }
                }
            }
            if is_max {
                cands.push((eig[i], x, y));
            }
        }
    }
    // 6. Strongest first, ties by (y, x).
    cands.sort_by(|a, b| {
        b.0.partial_cmp(&a.0)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then(a.2.cmp(&b.2))
            .then(a.1.cmp(&b.1))
    });
    // 7. Greedy minimum-distance selection.
    let md2 = p.min_distance * p.min_distance;
    let mut out: Vec<[f32; 2]> = Vec::new();
    for &(_, x, y) in &cands {
        if out.len() >= p.max_corners {
            break;
        }
        let (fx, fy) = (x as f64, y as f64);
        let far = out.iter().all(|q| {
            let dx = q[0] as f64 - fx;
            let dy = q[1] as f64 - fy;
            dx * dx + dy * dy >= md2
        });
        if far {
            out.push([x as f32, y as f32]);
        }
    }
    out
}

/// Sums of `ix*ix`, `iy*iy`, `ix*iy` over a `(2r+1)^2` window with REPLICATE borders.
fn box_sums3(ix: &[f32], iy: &[f32], w: usize, h: usize, r: i64) -> (Vec<f32>, Vec<f32>, Vec<f32>) {
    let n = w * h;
    let mut pxx = vec![0.0f32; n];
    let mut pyy = vec![0.0f32; n];
    let mut pxy = vec![0.0f32; n];
    for i in 0..n {
        pxx[i] = ix[i] * ix[i];
        pyy[i] = iy[i] * iy[i];
        pxy[i] = ix[i] * iy[i];
    }
    let sep = |src: &[f32]| -> Vec<f32> {
        // horizontal then vertical box sums
        let mut tmp = vec![0.0f32; n];
        for y in 0..h {
            for x in 0..w {
                let mut s = 0.0f32;
                for dx in -r..=r {
                    let xx = (x as i64 + dx).clamp(0, w as i64 - 1) as usize;
                    s += src[y * w + xx];
                }
                tmp[y * w + x] = s;
            }
        }
        let mut out = vec![0.0f32; n];
        for y in 0..h {
            for x in 0..w {
                let mut s = 0.0f32;
                for dy in -r..=r {
                    let yy = (y as i64 + dy).clamp(0, h as i64 - 1) as usize;
                    s += tmp[yy * w + x];
                }
                out[y * w + x] = s;
            }
        }
        out
    };
    (sep(&pxx), sep(&pyy), sep(&pxy))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finds_corners_of_a_bright_square() {
        let (w, h) = (60usize, 60usize);
        let mut v = vec![0.0f32; w * h];
        for y in 20..40 {
            for x in 20..40 {
                v[y * w + x] = 100.0;
            }
        }
        let img = Field2::new(w, h, v).unwrap();
        let p = ShiTomasi {
            max_corners: 100,
            quality_level: 0.01,
            min_distance: 5.0,
            block_size: 5,
            buffer_mask: 0,
        };
        let pts = good_features(&img, &p);
        assert!(!pts.is_empty());
        // Every detected point lies near the square's boundary.
        for q in &pts {
            let near_edge = [19.0f32, 20.0, 21.0, 38.0, 39.0, 40.0]
                .iter()
                .any(|&e| (q[0] - e).abs() <= 3.0)
                || [19.0f32, 20.0, 21.0, 38.0, 39.0, 40.0]
                    .iter()
                    .any(|&e| (q[1] - e).abs() <= 3.0);
            assert!(near_edge, "corner {q:?} not near the square");
        }
    }

    #[test]
    fn min_distance_is_enforced() {
        let (w, h) = (50usize, 50usize);
        let v: Vec<f32> = (0..w * h).map(|i| ((i * 37) % 101) as f32).collect();
        let img = Field2::new(w, h, v).unwrap();
        let p = ShiTomasi {
            max_corners: 1000,
            quality_level: 0.01,
            min_distance: 10.0,
            block_size: 5,
            buffer_mask: 0,
        };
        let pts = good_features(&img, &p);
        for i in 0..pts.len() {
            for j in i + 1..pts.len() {
                let dx = pts[i][0] - pts[j][0];
                let dy = pts[i][1] - pts[j][1];
                assert!((dx * dx + dy * dy).sqrt() >= 10.0 - 1e-4);
            }
        }
    }
}
