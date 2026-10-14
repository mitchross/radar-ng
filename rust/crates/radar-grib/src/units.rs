//! Unit conversions and the precip-type category grid used by the HRRR and NAQFC paths
//! (spec-ingest-mrms-hrrr §5.5, `_process_forecast_hour_sync`).
//!
//! Every function performs its arithmetic in `f32`, in the same operation order as the numpy
//! expression in the Python source. The operation order is part of the contract (spec §5.5).

/// Categorical precip-type codes (`PRECIP_TYPE_CATEGORIES`).
pub const PRECIP_RAIN: i32 = 1;
/// Snow.
pub const PRECIP_SNOW: i32 = 2;
/// Freezing rain.
pub const PRECIP_FREEZING_RAIN: i32 = 3;
/// Ice pellets.
pub const PRECIP_ICE_PELLETS: i32 = 4;

/// Kelvin to degrees Fahrenheit: `(k - 273.15) * 9 / 5 + 32`, in f32.
#[inline]
pub fn kelvin_to_f(k: f32) -> f32 {
    (k - 273.15_f32) * 9.0_f32 / 5.0_f32 + 32.0_f32
}

/// Metres per second to miles per hour: `ms * 2.237`, in f32.
#[inline]
pub fn ms_to_mph(ms: f32) -> f32 {
    ms * 2.237_f32
}

/// Wind speed from mph components: `sqrt(u**2 + v**2)`, in f32.
#[inline]
pub fn wind_speed_mph(u_mph: f32, v_mph: f32) -> f32 {
    (u_mph * u_mph + v_mph * v_mph).sqrt()
}

/// Kilograms per square metre (mm) to inches: `d / 25.4`, in f32.
#[inline]
pub fn mm_to_inches(mm: f32) -> f32 {
    mm / 25.4_f32
}

/// Builds the precip-type category grid from the four categorical flags, in the Python order:
/// `cat[crain > 0] = 1`, then snow, then freezing rain, then ice pellets. Later assignments win, so
/// the effective precedence is ice pellets > freezing rain > snow > rain. NaN compares false.
///
/// All four slices must have the same length; the result has that length. Missing (absent) inputs
/// are passed as `None` and contribute nothing, matching "each optional" in the Python.
pub fn precip_type_category(
    crain: Option<&[f32]>,
    csnow: Option<&[f32]>,
    cfrzr: Option<&[f32]>,
    cicep: Option<&[f32]>,
    len: usize,
) -> Vec<i32> {
    let mut cat = vec![0_i32; len];
    for (flag, code) in [
        (crain, PRECIP_RAIN),
        (csnow, PRECIP_SNOW),
        (cfrzr, PRECIP_FREEZING_RAIN),
        (cicep, PRECIP_ICE_PELLETS),
    ] {
        if let Some(values) = flag {
            debug_assert_eq!(values.len(), len);
            for (c, v) in cat.iter_mut().zip(values) {
                if *v > 0.0 {
                    *c = code;
                }
            }
        }
    }
    cat
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn kelvin_to_f_matches_numpy_f32_order() {
        // numpy: ((np.float32(k) - 273.15) * 9 / 5 + 32) evaluated in float32
        let k = 273.15_f32;
        assert_eq!(kelvin_to_f(k), 32.0);
        let v = kelvin_to_f(300.0);
        assert!((v - 80.33).abs() < 0.01, "{v}");
    }

    #[test]
    fn mph_and_inches() {
        assert!((ms_to_mph(10.0) - 22.37).abs() < 1e-4);
        assert_eq!(mm_to_inches(25.4), 1.0);
        assert_eq!(wind_speed_mph(3.0, 4.0), 5.0);
    }

    #[test]
    fn precip_type_precedence_and_nan() {
        let rain = [1.0_f32, 1.0, f32::NAN, 0.0, 1.0];
        let snow = [0.0_f32, 1.0, 1.0, 0.0, 0.0];
        let frzr = [0.0_f32, 0.0, 1.0, 0.0, 0.0];
        let pell = [0.0_f32, 0.0, 0.0, 1.0, 0.0];
        let cat = precip_type_category(Some(&rain), Some(&snow), Some(&frzr), Some(&pell), 5);
        // idx0: rain only -> 1; idx1: rain+snow -> snow 2; idx2: NaN rain, snow+frzr -> 3;
        // idx3: ice pellets -> 4; idx4: rain only -> 1
        assert_eq!(cat, vec![1, 2, 3, 4, 1]);
        // Absent categorical fields contribute nothing.
        assert_eq!(precip_type_category(None, None, None, None, 2), vec![0, 0]);
    }
}
