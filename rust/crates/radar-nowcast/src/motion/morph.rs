//! Morphological clean-up of one frame (pysteps `utils.images.morph_opening`).
//!
//! Binary image `b = (img > min)` (NaN counts as 0), opened with an elliptical
//! structuring element of side `n`. Pixels that opening removes take the minimum valid
//! value. OpenCV's default border rule applies: erosion ignores pixels outside the
//! image, dilation sees them as 0.

use crate::error::NowcastError;
use crate::field::Field2;

/// Offsets `(dx, dy)` of an `n x n` ellipse as produced by `cv2.getStructuringElement`.
/// For `n = 3` this is the cross shape, which is the only size radar-ng uses.
fn ellipse_offsets(n: usize) -> Vec<(i64, i64)> {
    let r = (n / 2) as i64;
    let rr = (r as f64).max(0.5);
    let mut out = Vec::new();
    for dy in -r..=r {
        for dx in -r..=r {
            let fx = dx as f64 / rr;
            let fy = dy as f64 / rr;
            if fx * fx + fy * fy <= 1.0 + 1e-9 {
                out.push((dx, dy));
            }
        }
    }
    out
}

/// Binary erosion with out-of-image pixels ignored (they never erode).
fn erode(src: &[bool], w: usize, h: usize, se: &[(i64, i64)]) -> Vec<bool> {
    let mut out = vec![false; w * h];
    for y in 0..h {
        for x in 0..w {
            let mut all = true;
            for &(dx, dy) in se {
                let (xx, yy) = (x as i64 + dx, y as i64 + dy);
                if xx < 0 || yy < 0 || xx >= w as i64 || yy >= h as i64 {
                    continue;
                }
                if !src[yy as usize * w + xx as usize] {
                    all = false;
                    break;
                }
            }
            out[y * w + x] = all;
        }
    }
    out
}

/// Binary dilation with out-of-image pixels counted as 0.
fn dilate(src: &[bool], w: usize, h: usize, se: &[(i64, i64)]) -> Vec<bool> {
    let mut out = vec![false; w * h];
    for y in 0..h {
        for x in 0..w {
            let mut any = false;
            for &(dx, dy) in se {
                let (xx, yy) = (x as i64 + dx, y as i64 + dy);
                if xx < 0 || yy < 0 || xx >= w as i64 || yy >= h as i64 {
                    continue;
                }
                if src[yy as usize * w + xx as usize] {
                    any = true;
                    break;
                }
            }
            out[y * w + x] = any;
        }
    }
    out
}

/// Apply `morph_opening(img, min(img), n)` (spec 6.2 step 2) and return the cleaned frame.
///
/// NaN cells stay NaN. `n == 0` returns a copy unchanged.
pub(crate) fn prepare(img: &Field2<f32>, n: usize) -> Result<Field2<f32>, NowcastError> {
    if n == 0 {
        return Ok(img.clone());
    }
    let (w, h) = (img.width(), img.height());
    let src = img.as_slice();
    let Some(thr) = crate::field::nan_min_f32(src) else {
        return Err(NowcastError::InvalidInput(
            "frame has no valid (non-NaN) values".into(),
        ));
    };
    let binary: Vec<bool> = src.iter().map(|&v| !v.is_nan() && v > thr).collect();
    let se = ellipse_offsets(n);
    let opened = dilate(&erode(&binary, w, h, &se), w, h, &se);
    let mut out = src.to_vec();
    for (i, (&b, &o)) in binary.iter().zip(&opened).enumerate() {
        if b && !o {
            out[i] = thr;
        }
    }
    Field2::new(w, h, out).map_err(|e| NowcastError::InvalidInput(e.to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn three_by_three_ellipse_is_a_cross() {
        let mut se = ellipse_offsets(3);
        se.sort();
        assert_eq!(se, vec![(-1, 0), (0, -1), (0, 0), (0, 1), (1, 0)]);
    }

    #[test]
    fn isolated_pixel_is_removed_and_block_kept() {
        // 0 everywhere except a 1-pixel speck at (0,0) and a 3x3 block at (3..6, 3..6).
        let (w, h) = (8, 8);
        let mut v = vec![0.0f32; w * h];
        v[0] = 5.0;
        for y in 3..6 {
            for x in 3..6 {
                v[y * w + x] = 5.0;
            }
        }
        // Make the background the minimum and the speck/block the foreground.
        let img = Field2::new(w, h, v).unwrap();
        let out = prepare(&img, 3).unwrap();
        assert_eq!(out.as_slice()[0], 0.0, "isolated speck removed");
        // A cross-shaped element erodes the corners of a 3x3 square away, so only the
        // centre survives the opening (and the corners are removed, as in OpenCV).
        assert_eq!(out.as_slice()[4 * w + 4], 5.0, "block centre kept");
        assert_eq!(
            out.as_slice()[3 * w + 3],
            0.0,
            "square corner removed by cross opening"
        );
    }

    #[test]
    fn nan_cells_are_preserved() {
        let img =
            Field2::new(3, 3, vec![f32::NAN, 1.0, 2.0, 1.0, 1.0, 1.0, 0.0, 1.0, 1.0]).unwrap();
        let out = prepare(&img, 3).unwrap();
        assert!(out.as_slice()[0].is_nan());
    }
}
