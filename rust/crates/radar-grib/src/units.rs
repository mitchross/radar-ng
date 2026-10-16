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

/// NWS apparent temperature in °F: wind chill at or below 50 °F with wind of at least 3 mph, heat index
/// at 80 °F and above (Rothfusz regression with the NWS low/high humidity adjustments), else the air
/// temperature. NaN in any input gives NaN.
pub fn apparent_temperature_f(t_f: f32, rh_pct: f32, wind_mph: f32) -> f32 {
    if t_f.is_nan() || rh_pct.is_nan() || wind_mph.is_nan() {
        return f32::NAN;
    }
    let (t, rh, v) = (
        f64::from(t_f),
        f64::from(rh_pct.clamp(0.0, 100.0)),
        f64::from(wind_mph),
    );
    if t <= 50.0 && v >= 3.0 {
        let v16 = v.powf(0.16);
        return (35.74 + 0.6215 * t - 35.75 * v16 + 0.4275 * t * v16) as f32;
    }
    if t < 80.0 {
        return t_f;
    }
    let simple = 0.5 * (t + 61.0 + (t - 68.0) * 1.2 + rh * 0.094);
    if (simple + t) / 2.0 < 80.0 {
        return simple as f32;
    }
    let mut hi = -42.379 + 2.049_015_23 * t + 10.143_331_27 * rh
        - 0.224_755_41 * t * rh
        - 0.006_837_83 * t * t
        - 0.054_817_17 * rh * rh
        + 0.001_228_74 * t * t * rh
        + 0.000_852_82 * t * rh * rh
        - 0.000_001_99 * t * t * rh * rh;
    if rh < 13.0 && (80.0..=112.0).contains(&t) {
        hi -= ((13.0 - rh) / 4.0) * ((17.0 - (t - 95.0).abs()) / 17.0).sqrt();
    } else if rh > 85.0 && (80.0..=87.0).contains(&t) {
        hi += ((rh - 85.0) / 10.0) * ((87.0 - t) / 5.0);
    }
    hi as f32
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
    fn apparent_temperature_follows_nws_tables() {
        // NWS heat index chart: 90 °F at 60 % RH reads 100 °F.
        assert!((apparent_temperature_f(90.0, 60.0, 5.0) - 100.0).abs() < 1.0);
        // NWS wind chill chart: 20 °F with 15 mph wind reads 6 °F.
        assert!((apparent_temperature_f(20.0, 50.0, 15.0) - 6.0).abs() < 1.0);
        // Mild air, or calm cold air: the air temperature.
        assert_eq!(apparent_temperature_f(65.0, 90.0, 20.0), 65.0);
        assert_eq!(apparent_temperature_f(30.0, 50.0, 2.0), 30.0);
        assert!(apparent_temperature_f(f32::NAN, 50.0, 5.0).is_nan());
    }

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
