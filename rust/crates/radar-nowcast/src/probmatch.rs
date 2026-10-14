//! Percentile masking and empirical-CDF probability matching (spec 7.7c and 7.7d;
//! pysteps `nowcasts.utils.compute_percentile_mask` and
//! `postprocessing.probmatching.nonparam_match_empirical_cdf`).
//!
//! Selection replaces the full sorts of the Python code. Only order statistics are needed:
//! one for the threshold, one for the percentile, and the top-k values for the CDF map.
//!
//! D1 fix (spec 1.4): pysteps indexes `precip_s[i+1]` after choosing `i`. When the wet
//! fraction is below `1.5/N` the chosen index is `N-1` and that lookup raises. The tie
//! rule only changes which index holds the threshold value, never the value itself, so
//! the threshold is `s[i]` and no lookahead is needed. The mask is identical whenever
//! Python succeeds.

use rayon::prelude::*;

use crate::error::NowcastError;

/// The `i`-th smallest value of `values` (0-based), by selection. Panics if `i >= len`.
pub fn order_statistic(values: &[f64], i: usize) -> f64 {
    let mut v = values.to_vec();
    *v.select_nth_unstable_by(i, cmp_nan_last).1
}

/// Order-preserving `u64` key for an `f64`: NaN last, `-0.0` equal to `0.0`.
#[inline]
fn order_key(v: f64) -> u64 {
    if v.is_nan() {
        return u64::MAX;
    }
    let v = if v == 0.0 { 0.0 } else { v };
    let bits = v.to_bits();
    if bits >> 63 == 1 {
        !bits
    } else {
        bits | (1 << 63)
    }
}

/// Total order that sorts NaN last, like numpy.
fn cmp_nan_last(a: &f64, b: &f64) -> std::cmp::Ordering {
    a.partial_cmp(b)
        .unwrap_or_else(|| a.is_nan().cmp(&b.is_nan()))
}

/// Threshold for the wet-area percentile mask (spec 7.7c).
///
/// `x_j = (N - j) / N` for `j = 0..N-1`, `i` is the first index with `|x_i - war|` minimal,
/// and the threshold is the `i`-th smallest value. Cells `>=` the threshold are kept.
pub fn percentile_threshold(values: &[f64], war: f64) -> Result<f64, NowcastError> {
    let n = values.len();
    if n == 0 {
        return Err(NowcastError::InvalidInput(
            "empty field in percentile mask".into(),
        ));
    }
    let nf = n as f64;
    let target = (1.0 - war) * nf;
    let centre = target.round().clamp(0.0, (n - 1) as f64) as isize;
    let mut best_j = 0usize;
    let mut best = f64::INFINITY;
    for j in (centre - 2).max(0)..=(centre + 2).min(n as isize - 1) {
        let x = (n - j as usize) as f64 / nf;
        let d = (x - war).abs();
        if d < best {
            best = d;
            best_j = j as usize;
        }
    }
    Ok(order_statistic(values, best_j))
}

/// Keep cells `>= threshold` and set the others to `fill` (spec 7.7c, in place).
pub fn apply_threshold(values: &mut [f64], threshold: f64, fill: f64) -> usize {
    let mut kept = 0usize;
    for v in values.iter_mut() {
        if *v >= threshold {
            kept += 1;
        } else {
            *v = fill;
        }
    }
    kept
}

/// Linear-interpolation percentile of `values` at quantile `q` in `[0, 1]`
/// (numpy `percentile(method="linear")`, including its `_lerp` branch).
fn percentile_linear(values: &[f64], q: f64) -> f64 {
    let n = values.len();
    let vi = q * (n as f64 - 1.0);
    let lo = vi.floor().max(0.0) as usize;
    let t = vi - lo as f64;
    let mut v = values.to_vec();
    let (_, a, right) = v.select_nth_unstable_by(lo, cmp_nan_last);
    let a = *a;
    if lo + 1 >= n || t == 0.0 {
        return a;
    }
    let b = right
        .iter()
        .copied()
        .min_by(cmp_nan_last)
        .expect("right side non-empty");
    let diff = b - a;
    if t >= 0.5 {
        b - diff * (1.0 - t)
    } else {
        a + diff * t
    }
}

/// Empirical-CDF matching: remap `initial` so its distribution follows `target`
/// (pysteps `nonparam_match_empirical_cdf`, without `ignore_indices`).
///
/// - `z0 = min(initial)`; cells equal to `z0` become `zt = nanmin(target)`.
/// - `k` = number of cells above `z0`. If `target` has more than `k` cells above `zt`,
///   the bottom `100*(1-k/N)` percentile of `target` is set to `zt`.
/// - The top `k` target values, sorted, are assigned to the wet cells in ascending order
///   of `initial`. Ties in `initial` are broken by index. (numpy uses an unstable argsort,
///   so tie order is unspecified there.)
pub fn cdf_match(initial: &[f64], target: &[f64]) -> Result<Vec<f64>, NowcastError> {
    let n = initial.len();
    if n != target.len() {
        return Err(NowcastError::ProbMatch(format!(
            "size mismatch {} vs {}",
            n,
            target.len()
        )));
    }
    if n == 0 {
        return Err(NowcastError::ProbMatch("empty field".into()));
    }
    let z0 = crate::field::nan_min_f64(initial)
        .ok_or_else(|| NowcastError::ProbMatch("initial array contains only NaN".into()))?;
    if initial.iter().any(|v| !v.is_finite()) {
        return Err(NowcastError::ProbMatch(
            "initial array has non-finite values".into(),
        ));
    }
    let zt = crate::field::nan_min_f64(target)
        .ok_or_else(|| NowcastError::ProbMatch("target array contains only NaN".into()))?;
    let mut t: Vec<f64> = target
        .iter()
        .map(|&v| if v.is_nan() { zt } else { v })
        .collect();
    let k = initial.par_iter().filter(|&&v| v > z0).count();
    let target_wet = t.par_iter().filter(|&&v| v > zt).count();
    if target_wet > k {
        let war = k as f64 / n as f64;
        let pct = 100.0 * (1.0 - war);
        let p = percentile_linear(&t, pct / 100.0);
        for v in t.iter_mut() {
            if *v < p {
                *v = zt;
            }
        }
    }
    let mut out = vec![zt; n];
    if k == 0 {
        return Ok(out);
    }
    // Top-k target values, ascending.
    let mut top: Vec<f64> = {
        let mut v = t;
        v.select_nth_unstable_by(n - k, cmp_nan_last);
        v[n - k..].to_vec()
    };
    top.par_sort_unstable_by_key(|&v| order_key(v));
    // Wet cells ordered by initial value (index breaks ties). Integer keys sort in the
    // same order as the values, which is much faster than comparing f64 pairs.
    let mut wet: Vec<(u64, usize)> = initial
        .iter()
        .enumerate()
        .filter(|(_, v)| **v > z0)
        .map(|(i, &v)| (order_key(v), i))
        .collect();
    wet.par_sort_unstable();
    for (r, &(_, idx)) in wet.iter().enumerate() {
        out[idx] = top[r];
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn percentile_mask_matches_appendix_a2() {
        let field = [1.0, 5.0, 3.0, 2.0, 4.0, 3.0];
        let mask_for = |war: f64| -> Vec<u8> {
            let thr = percentile_threshold(&field, war).unwrap();
            field.iter().map(|&v| u8::from(v >= thr)).collect()
        };
        assert_eq!(mask_for(0.5), vec![0, 1, 1, 0, 1, 1]);
        assert_eq!(mask_for(0.34), vec![0, 1, 0, 0, 1, 0]);
        assert_eq!(mask_for(0.25), vec![0, 1, 0, 0, 1, 0]);
    }

    #[test]
    fn low_wet_fraction_does_not_error() {
        // D1: pysteps raises IndexError for war < 1.5/N. Here we keep only the maximum.
        let field = [1.0, 5.0, 3.0, 2.0, 4.0, 3.0];
        for war in [0.2, 0.0] {
            let thr = percentile_threshold(&field, war).unwrap();
            assert_eq!(thr, 5.0);
        }
    }

    #[test]
    fn ties_follow_appendix_a2() {
        let field = [1.0, 2.0, 2.0, 2.0, 3.0, 4.0];
        let thr = percentile_threshold(&field, 0.5).unwrap();
        let mask: Vec<u8> = field.iter().map(|&v| u8::from(v >= thr)).collect();
        assert_eq!(mask, vec![0, 1, 1, 1, 1, 1]);
    }

    #[test]
    fn cdf_match_reproduces_hand_example() {
        // Hand-checked against pysteps: init [[1,5,3],[2,4,3]], target [[0.5,NaN,2],[7,1,3]].
        let init = [1.0, 5.0, 3.0, 2.0, 4.0, 3.0];
        let target = [0.5, f64::NAN, 2.0, 7.0, 1.0, 3.0];
        let out = cdf_match(&init, &target).unwrap();
        let want = [0.5, 7.0, 1.0, 0.5, 3.0, 2.0];
        assert_eq!(out, want);
    }

    #[test]
    fn cdf_match_preserves_sum_of_wet_values() {
        let init: Vec<f64> = (0..50).map(|i| ((i * 37) % 23) as f64 - 2.0).collect();
        let target: Vec<f64> = (0..50).map(|i| ((i * 11) % 29) as f64 * 0.7).collect();
        let out = cdf_match(&init, &target).unwrap();
        let z0 = init.iter().cloned().fold(f64::INFINITY, f64::min);
        let k = init.iter().filter(|&&v| v > z0).count();
        let zt = target.iter().cloned().fold(f64::INFINITY, f64::min);
        let wet_out: f64 = out.iter().filter(|&&v| v > zt).sum();
        let mut tsorted = target.clone();
        tsorted.sort_by(|a, b| a.partial_cmp(b).unwrap());
        let top_sum: f64 = tsorted[50 - k..].iter().sum();
        assert!((wet_out - top_sum).abs() < 1e-9 || k == 0);
    }

    #[test]
    fn percentile_linear_matches_numpy_examples() {
        // np.percentile([1,2,3,4], 50) = 2.5 ; np.percentile([1,2,3,4], 25) = 1.75
        let v = [1.0, 2.0, 3.0, 4.0];
        assert!((percentile_linear(&v, 0.5) - 2.5).abs() < 1e-12);
        assert!((percentile_linear(&v, 0.25) - 1.75).abs() < 1e-12);
    }
}
