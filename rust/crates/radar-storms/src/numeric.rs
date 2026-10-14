//! Python- and numpy-compatible numeric helpers (spec-shared section 15.2).
//!
//! The storm math must reproduce Python's `round`, `format`, `math.radians`
//! and numpy's pairwise summation, because rounded outputs and tie-breaks are
//! compared exactly against the Python golden fixtures.

use serde_json::Value;

/// Python `math.radians`: `x * (pi / 180)`.
#[must_use]
pub fn radians(deg: f64) -> f64 {
    deg * (std::f64::consts::PI / 180.0)
}

/// Python `math.degrees`: `x * (180 / pi)`.
#[must_use]
pub fn degrees(rad: f64) -> f64 {
    rad * (180.0 / std::f64::consts::PI)
}

/// Python `round(x)` for a float: round half to even.
#[must_use]
pub fn round_half_even(x: f64) -> f64 {
    x.round_ties_even()
}

/// Python `round(x, ndigits)` for `ndigits >= 1`.
///
/// Python rounds the exact binary value half to even. Formatting with a
/// precision and parsing back gives the same result, which a multiply-and-divide
/// does not (spec-shared 15.2).
#[must_use]
pub fn round_digits(x: f64, ndigits: usize) -> f64 {
    if !x.is_finite() {
        return x;
    }
    format!("{x:.ndigits$}").parse::<f64>().unwrap_or(x)
}

/// Python `float(value)` for a JSON value. Returns `None` where Python would raise.
#[must_use]
pub fn py_float(value: &Value) -> Option<f64> {
    match value {
        Value::Number(n) => n.as_f64(),
        Value::Bool(b) => Some(if *b { 1.0 } else { 0.0 }),
        Value::String(s) => s.trim().parse::<f64>().ok(),
        _ => None,
    }
}

/// Python `int(value)` for a JSON value. Floats truncate toward zero.
#[must_use]
pub fn py_int(value: &Value) -> Option<i64> {
    match value {
        Value::Number(n) => n.as_i64().or_else(|| {
            n.as_f64()
                .filter(|f| f.is_finite())
                .map(|f| f.trunc() as i64)
        }),
        Value::Bool(b) => Some(i64::from(*b)),
        Value::String(s) => s.trim().parse::<i64>().ok(),
        _ => None,
    }
}

macro_rules! numpy_pairwise_sum {
    ($name:ident, $t:ty) => {
        /// Sum with the same association order as numpy's `add.reduce` on a
        /// contiguous array (`pairwise_sum`, 8-way unrolled blocks of 128).
        #[must_use]
        pub fn $name(a: &[$t]) -> $t {
            let n = a.len();
            if n < 8 {
                let mut res: $t = 0.0;
                for &x in a {
                    res += x;
                }
                return res;
            }
            if n <= 128 {
                let mut r: [$t; 8] = [a[0], a[1], a[2], a[3], a[4], a[5], a[6], a[7]];
                let limit = n - (n % 8);
                let mut i = 8;
                while i < limit {
                    for (j, acc) in r.iter_mut().enumerate() {
                        *acc += a[i + j];
                    }
                    i += 8;
                }
                let mut res = ((r[0] + r[1]) + (r[2] + r[3])) + ((r[4] + r[5]) + (r[6] + r[7]));
                while i < n {
                    res += a[i];
                    i += 1;
                }
                return res;
            }
            let mut n2 = n / 2;
            n2 -= n2 % 8;
            $name(&a[..n2]) + $name(&a[n2..])
        }
    };
}

numpy_pairwise_sum!(np_sum_f64, f64);
numpy_pairwise_sum!(np_sum_f32, f32);

/// numpy `mean()` for float64: pairwise sum divided by the count.
#[must_use]
pub fn np_mean_f64(a: &[f64]) -> f64 {
    np_sum_f64(a) / (a.len() as f64)
}

/// numpy `mean()` for float32: the reduction stays in float32 (spec-shared 8.4).
#[must_use]
pub fn np_mean_f32(a: &[f32]) -> f32 {
    np_sum_f32(a) / (a.len() as f32)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn round_half_even_matches_python() {
        // Python: round(0.5)=0, round(1.5)=2, round(2.5)=2, round(-0.5)=0
        assert_eq!(round_half_even(0.5), 0.0);
        assert_eq!(round_half_even(1.5), 2.0);
        assert_eq!(round_half_even(2.5), 2.0);
        assert_eq!(round_half_even(-0.5), -0.0);
    }

    #[test]
    fn round_digits_uses_exact_binary_value() {
        // Python: round(2.675, 2) == 2.67 (2.675 is slightly below the tie)
        assert_eq!(round_digits(2.675, 2), 2.67);
        assert_eq!(round_digits(1.005, 2), 1.0);
        assert_eq!(round_digits(0.125, 2), 0.12);
        assert_eq!(round_digits(-84.123_456_789, 5), -84.12346);
        assert!(round_digits(f64::NAN, 3).is_nan());
    }

    #[test]
    fn format_zero_decimals_is_half_even() {
        // Python: f"{2.5:.0f}" == "2", f"{0.5:.0f}" == "0", f"{1.5:.0f}" == "2"
        assert_eq!(format!("{:.0}", 2.5_f64), "2");
        assert_eq!(format!("{:.0}", 0.5_f64), "0");
        assert_eq!(format!("{:.0}", 1.5_f64), "2");
        assert_eq!(format!("{:.0}", 42.5_f64), "42");
    }

    #[test]
    fn pairwise_sum_uses_numpy_block_association() {
        // n = 8 is one unrolled block: ((a0+a1)+(a2+a3)) + ((a4+a5)+(a6+a7)).
        let a: Vec<f64> = (0..8).map(|k| 0.1 * f64::from(k) + 1e-3).collect();
        let expected = ((a[0] + a[1]) + (a[2] + a[3])) + ((a[4] + a[5]) + (a[6] + a[7]));
        assert_eq!(np_sum_f64(&a), expected);
        // n < 8 is a plain left-to-right sum starting from 0.
        assert_eq!(np_sum_f64(&a[..3]), ((0.0 + a[0]) + a[1]) + a[2]);
        // Bit-exact agreement with numpy for the longer golden lengths is checked in tests/golden.rs.
    }

    #[test]
    fn py_coercions() {
        assert_eq!(py_int(&json!(7)), Some(7));
        assert_eq!(py_int(&json!(7.9)), Some(7));
        assert_eq!(py_int(&json!("  12 ")), Some(12));
        assert_eq!(py_int(&json!(null)), None);
        assert_eq!(py_int(&json!("x")), None);
        assert_eq!(py_float(&json!("1.5")), Some(1.5));
        assert_eq!(py_float(&json!(true)), Some(1.0));
        assert_eq!(py_float(&json!([1])), None);
    }
}
