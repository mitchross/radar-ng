//! AR(2) machinery per cascade level (pysteps `timeseries.correlation` and
//! `timeseries.autoregression`).
//!
//! - [`pearson`]: `np.corrcoef` of two equal-length arrays, clipped to `[-1, 1]`.
//! - [`adjust_lag2`]: the lag-2 stationarity adjustment (pysteps `adjust_lag2_corrcoef2`).
//! - [`yule_walker_ar2`]: Yule-Walker AR(2) with the innovation coefficient, and the
//!   stationarity test (pysteps `estimate_ar_params_yw`).
//! - [`Ar2State`]: the deterministic AR(2) iteration on two stored states (pysteps
//!   `iterate_ar_model` with no innovation term).
//!
//! Deliberate deviations and corrections are recorded in the crate README:
//! `adjust_lag2` follows the pysteps code (`2*g1*g2 - 1`), not the spec text
//! (`2*g1^2 - 1`), since the golden values come from pysteps.

use thiserror::Error;

use crate::field::nan_max;

/// Failure of the Yule-Walker AR(2) fit.
#[derive(Debug, Error, Clone, Copy, PartialEq, Eq)]
pub enum ArError {
    /// An autocorrelation is NaN or infinite (numpy `LinAlgError`: "must not contain NaN").
    #[error("non-finite autocorrelation")]
    NonFinite,
    /// The Yule-Walker system is exactly singular (numpy `LinAlgError`).
    #[error("singular Yule-Walker system")]
    Singular,
    /// A root of `z^2 - phi1 z - phi2` lies on or outside the unit circle.
    #[error("non-stationary AR(2) process")]
    NonStationary,
}

/// `np.corrcoef(x, y)[0, 1]`: Pearson correlation clipped to `[-1, 1]`.
///
/// Zero variance gives NaN (numpy `0/0`).
pub fn pearson(x: &[f64], y: &[f64]) -> f64 {
    debug_assert_eq!(x.len(), y.len());
    let n = x.len() as f64;
    let mx = x.iter().sum::<f64>() / n;
    let my = y.iter().sum::<f64>() / n;
    let (mut sxy, mut sxx, mut syy) = (0.0f64, 0.0f64, 0.0f64);
    for (a, b) in x.iter().zip(y) {
        let (da, db) = (a - mx, b - my);
        sxy += da * db;
        sxx += da * da;
        syy += db * db;
    }
    let r = sxy / (sxx.sqrt() * syy.sqrt());
    r.clamp(-1.0, 1.0)
}

/// Lag-2 adjustment (pysteps `adjust_lag2_corrcoef2`):
/// `g2' = max(g2, 2 g1 g2 - 1, (3 g1^2 - 2 + 2 (1 - g1^2)^1.5) / g1^2)` with NaN propagation.
pub fn adjust_lag2(gamma1: f64, gamma2: f64) -> f64 {
    let g2 = nan_max(gamma2, 2.0 * gamma1 * gamma2 - 1.0);
    let g1s = gamma1 * gamma1;
    let third = (3.0 * g1s - 2.0 + 2.0 * (1.0 - g1s).powf(1.5)) / g1s;
    nan_max(g2, third)
}

/// Yule-Walker AR(2) from `(gamma1, gamma2)` (spec 7.6).
///
/// Solves `[[1, g1], [g1, 1]] [phi1, phi2]^T = [g1, g2]^T` with partial pivoting, then the
/// innovation coefficient `phi_pert = sqrt(max(0, 1 - g1 phi1 - g2 phi2))`. Returns
/// `[phi1, phi2, phi_pert]`. The fit fails on NaN input, on an exactly singular system and
/// on non-stationary roots.
pub fn yule_walker_ar2(gamma1: f64, gamma2: f64) -> Result<[f64; 3], ArError> {
    if !gamma1.is_finite() || !gamma2.is_finite() {
        return Err(ArError::NonFinite);
    }
    // LU with partial pivoting on [[1, g1], [g1, 1]].
    let (a11, a12, a21, a22): (f64, f64, f64, f64) = (1.0, gamma1, gamma1, 1.0);
    let (b1, b2) = (gamma1, gamma2);
    let (phi1, phi2) = if a21.abs() > a11.abs() {
        // Pivot on the second row.
        let l = a11 / a21;
        let u22 = a12 - l * a22;
        if u22 == 0.0 {
            return Err(ArError::Singular);
        }
        let x2 = (b1 - l * b2) / u22;
        let x1 = (b2 - a22 * x2) / a21;
        (x1, x2)
    } else {
        let l = a21 / a11;
        let u22 = a22 - l * a12;
        if u22 == 0.0 {
            return Err(ArError::Singular);
        }
        let x2 = (b2 - l * b1) / u22;
        let x1 = (b1 - a12 * x2) / a11;
        (x1, x2)
    };
    if !phi1.is_finite() || !phi2.is_finite() {
        return Err(ArError::NonFinite);
    }
    if !is_stationary(phi1, phi2) {
        return Err(ArError::NonStationary);
    }
    let c = 1.0 - gamma1 * phi1 - gamma2 * phi2;
    let phi_pert = if c < 0.0 { 0.0 } else { c.sqrt() };
    Ok([phi1, phi2, phi_pert])
}

/// Both roots of `z^2 - phi1 z - phi2` strictly inside the unit circle.
fn is_stationary(phi1: f64, phi2: f64) -> bool {
    let disc = phi1 * phi1 + 4.0 * phi2;
    if disc >= 0.0 {
        let s = disc.sqrt();
        let r1 = ((phi1 + s) / 2.0).abs();
        let r2 = ((phi1 - s) / 2.0).abs();
        r1 < 1.0 && r2 < 1.0
    } else {
        // Complex pair with |z|^2 = -phi2.
        -phi2 < 1.0
    }
}

/// Two stored AR(2) states for one level: `older` (t-1) and `newer` (t), flattened.
#[derive(Clone, Debug)]
pub struct Ar2State {
    older: Vec<f64>,
    newer: Vec<f64>,
}

impl Ar2State {
    /// Create a state from `(older, newer)` buffers of equal length.
    pub fn new(older: Vec<f64>, newer: Vec<f64>) -> Self {
        assert_eq!(older.len(), newer.len(), "AR state shapes differ");
        Self { older, newer }
    }

    /// Advance one step: `x_new = phi1 * newer + phi2 * older` (pysteps `iterate_ar_model`).
    pub fn step(&mut self, phi1: f64, phi2: f64) {
        for (o, n) in self.older.iter_mut().zip(&self.newer) {
            *o = phi1 * *n + phi2 * *o;
        }
        std::mem::swap(&mut self.older, &mut self.newer);
    }

    /// The most recent state.
    pub fn newest(&self) -> &[f64] {
        &self.newer
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lag2_and_yw_match_appendix_a2() {
        let cases = [
            ((0.95, 0.85), 0.851400, [1.447895, -0.524100, 0.265930]),
            ((0.80, 0.50), 0.550000, [1.000000, -0.250000, 0.580948]),
            ((0.60, 0.30), 0.300000, [0.656250, -0.093750, 0.796477]),
        ];
        for ((g1, g2), g2_adj, phi) in cases {
            let adj = adjust_lag2(g1, g2);
            assert!((adj - g2_adj).abs() < 1e-6, "adj {adj} vs {g2_adj}");
            let got = yule_walker_ar2(g1, adj).unwrap();
            for (a, b) in got.iter().zip(phi) {
                assert!((a - b).abs() < 1e-5, "{a} vs {b}");
            }
        }
    }

    #[test]
    fn lag2_with_zero_gamma1_is_nan_and_yw_fails() {
        let adj = adjust_lag2(0.0, 0.5);
        assert!(adj.is_nan());
        assert_eq!(yule_walker_ar2(0.0, adj), Err(ArError::NonFinite));
    }

    #[test]
    fn yw_rejects_singular_and_nonstationary() {
        assert_eq!(yule_walker_ar2(1.0, 0.99), Err(ArError::Singular));
        assert_eq!(yule_walker_ar2(0.99, 0.2), Err(ArError::NonStationary));
    }

    #[test]
    fn ar_iteration_matches_appendix_a2() {
        // iterate_ar_model([1, 2], phi=(1.2, -0.3, 0.1)) = [2.0, 2.1]
        let mut s = Ar2State::new(vec![1.0], vec![2.0]);
        s.step(1.2, -0.3);
        assert!((s.newest()[0] - 2.1).abs() < 1e-12);
        // older is now 2.0 (the previous newest)
        assert_eq!(s.older, vec![2.0]);
    }

    #[test]
    fn pearson_is_exact_for_linear_relations() {
        let x = [1.0, 2.0, 3.0, 4.0];
        let y: Vec<f64> = x.iter().map(|v| 3.0 * v - 2.0).collect();
        assert!((pearson(&x, &y) - 1.0).abs() < 1e-12);
        let z: Vec<f64> = x.iter().map(|v| -v).collect();
        assert!((pearson(&x, &z) + 1.0).abs() < 1e-12);
        assert!(pearson(&[1.0, 1.0], &[2.0, 3.0]).is_nan());
    }
}
