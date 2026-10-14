//! S-PROG nowcast (pysteps `nowcasts.sprog.forecast` with radar-ng's parameters).
//!
//! radar-ng calls it with `precip_thr = 5 dBZ`, `norain_thr = 0`, 6 cascade levels,
//! AR(2), semi-Lagrangian advection, FFT Gaussian cascade, CDF probability matching,
//! and the three newest frames. The motion field is supplied by the caller.
//!
//! Pipeline (spec 7):
//! 1. Domain mask and precipitation minimum over the three frames, no-rain shortcut (D2).
//! 2. Lagrangian pre-advection of the two older frames, then NaN fill with each frame's minimum.
//! 3. Gaussian cascade of each frame, normalised levels, lag-1 and lag-2 correlations per
//!    level, lag-2 adjustment and Yule-Walker AR(2) fit.
//! 4. Per integer step: AR(2) iteration per level, recomposition, percentile mask at the
//!    wet-area ratio, CDF matching to the newest frame, domain NaN.
//! 5. Leads are blended between integer steps and advected by the displacement that
//!    accumulates over the lead segments, as pysteps' `nowcast_main_loop` does.

use crate::advection::{Displacement, advect_segment, advect_unit_steps, warp};
use crate::ar::{self, Ar2State, ArError};
use crate::cascade::{Fft2, gaussian_bandpass, normalise};
use crate::error::NowcastError;
use crate::field::{Field2, nan_min_f64};
use crate::motion::Velocity;
use crate::probmatch;

/// S-PROG parameters that radar-ng varies or pins (spec 7, "Fixed parameters from the call").
#[derive(Clone, Debug, PartialEq)]
pub struct SprogParams {
    /// Rain threshold in dBZ (`precip_thr`), required by pysteps. radar-ng uses 5.
    pub precip_thr: f64,
    /// `norain_thr`: the no-rain shortcut fires when the rain fraction is at most this.
    pub norain_thr: f64,
    /// Number of Gaussian cascade levels (`n_cascade_levels`).
    pub n_cascade_levels: usize,
    /// Gaussian bandpass width in log-wavenumber units (`gauss_scale`).
    pub gauss_scale: f64,
}

impl SprogParams {
    /// The radar-ng configuration: 5 dBZ threshold, 6 levels, `gauss_scale = 0.5`.
    pub fn radar_ng() -> Self {
        Self {
            precip_thr: 5.0,
            norain_thr: 0.0,
            n_cascade_levels: 6,
            gauss_scale: 0.5,
        }
    }
}

impl Default for SprogParams {
    fn default() -> Self {
        Self::radar_ng()
    }
}

/// Forecast fields plus whether the no-rain shortcut produced them.
#[derive(Clone, Debug, PartialEq)]
pub struct Forecast {
    /// One field per requested lead, in lead order, `f32`, NaN where the domain is missing.
    pub fields: Vec<Field2<f32>>,
    /// True when the window had no cell above the threshold (spec 7.1, D2). Every field is
    /// then the constant precipitation minimum. The caller should report the run as dry,
    /// not as an error.
    pub no_rain: bool,
}

/// Per-level state of the AR(2) model and the recomposition statistics.
struct LevelModel {
    state: Ar2State,
    phi1: f64,
    phi2: f64,
    mu: f64,
    sigma: f64,
}

/// Run S-PROG.
///
/// - `frames`: at least three `h x w` frames in time order (oldest first). Only the last
///   three are used. NaN marks missing cells. Values `< -100` must already be NaN.
/// - `velocity`: motion field in pixels per input step, same shape as the frames.
/// - `lead_steps`: lead times in input steps, positive and ascending (for example
///   `[1, 2, ..., 12]` at 5-minute input cadence).
pub fn forecast(
    frames: &[Field2<f32>],
    velocity: &Velocity,
    lead_steps: &[f64],
    params: &SprogParams,
) -> Result<Forecast, NowcastError> {
    if frames.len() < 3 {
        return Err(NowcastError::InvalidInput(format!(
            "S-PROG needs 3 frames, got {}",
            frames.len()
        )));
    }
    let raw = &frames[frames.len() - 3..];
    let (w, h) = (raw[0].width(), raw[0].height());
    if raw.iter().any(|f| f.width() != w || f.height() != h) {
        return Err(NowcastError::InvalidInput("frames differ in shape".into()));
    }
    if velocity.width() != w || velocity.height() != h {
        return Err(NowcastError::InvalidInput(format!(
            "velocity {}x{} does not match frames {}x{}",
            velocity.width(),
            velocity.height(),
            w,
            h
        )));
    }
    if lead_steps.is_empty() {
        return Err(NowcastError::InvalidInput("no lead times".into()));
    }
    if lead_steps.iter().any(|&t| !t.is_finite() || t <= 0.0) {
        return Err(NowcastError::InvalidInput(
            "lead times must be finite and positive".into(),
        ));
    }
    if lead_steps.windows(2).any(|p| p[1] < p[0]) {
        return Err(NowcastError::InvalidInput(
            "lead times must be ascending".into(),
        ));
    }
    if params.n_cascade_levels < 3 {
        return Err(NowcastError::InvalidInput(
            "n_cascade_levels must be at least 3".into(),
        ));
    }
    let n = w * h;

    // Step 1: domain mask, minimum, no-rain check (7.1).
    let domain_mask: Vec<bool> = (0..n)
        .map(|i| raw.iter().any(|f| !f.as_slice()[i].is_finite()))
        .collect();
    let mut p: Vec<Vec<f64>> = raw.iter().map(|f| f.to_f64().into_vec()).collect();
    let precip_min = p
        .iter()
        .filter_map(|f| nan_min_f64(f))
        .fold(None, |acc: Option<f64>, v| {
            Some(acc.map_or(v, |b| b.min(v)))
        })
        .ok_or_else(|| NowcastError::InvalidInput("all frames are NaN".into()))?;
    let rain_cells: usize = p
        .iter()
        .map(|f| f.iter().filter(|&&v| v > params.precip_thr).count())
        .sum();
    if (rain_cells as f64) / ((3 * n) as f64) <= params.norain_thr {
        let fields = lead_steps
            .iter()
            .map(|_| Field2::filled(w, h, precip_min as f32))
            .collect();
        return Ok(Forecast {
            fields,
            no_rain: true,
        });
    }

    // Step 2: Lagrangian pre-advection of frames 0 and 1 (7.2), then NaN fill (7.3).
    let vx = velocity.x.as_slice();
    let vy = velocity.y.as_slice();
    for (i, steps) in [(0usize, 2usize), (1, 1)] {
        let outval = nan_min_f64(&p[i])
            .ok_or_else(|| NowcastError::InvalidInput("frame is all NaN".into()))?;
        p[i] = advect_unit_steps(&p[i], vx, vy, w, h, steps, outval);
    }
    for frame in p.iter_mut() {
        let m = nan_min_f64(frame)
            .ok_or_else(|| NowcastError::InvalidInput("frame is all NaN after advection".into()))?;
        for v in frame.iter_mut() {
            if !v.is_finite() {
                *v = m;
            }
        }
    }
    let precip_0 = p[2].clone();
    let war = precip_0.iter().filter(|&&v| v >= params.precip_thr).count() as f64 / n as f64;

    // Step 3: cascade, normalised levels, correlations, AR(2) per level (7.4 to 7.6).
    let levels = params.n_cascade_levels;
    let fft = Fft2::new(h, w);
    let bp = gaussian_bandpass(h, w, levels, params.gauss_scale)?;
    let spectra: Vec<_> = p.iter().map(|f| fft.rfft2(f)).collect();
    let mut models: Vec<LevelModel> = Vec::with_capacity(levels);
    for k in 0..levels {
        let mut zs: Vec<Vec<f64>> = Vec::with_capacity(3);
        let mut newest_stats = (0.0f64, 0.0f64);
        for (t, spec) in spectra.iter().enumerate() {
            let field = fft.irfft2_weighted(spec, &bp.weights[k]);
            let (z, mu, sigma) = normalise(&field);
            if t == 2 {
                newest_stats = (mu, sigma);
            }
            zs.push(z);
        }
        let g1 = ar::pearson(&zs[2], &zs[1]);
        let g2 = ar::pearson(&zs[2], &zs[0]);
        let g2_adj = ar::adjust_lag2(g1, g2);
        let phi = ar::yule_walker_ar2(g1, g2_adj).map_err(|e| match e {
            ArError::NonStationary => NowcastError::NonStationary { level: k },
            other => NowcastError::ArFit {
                level: k,
                source: other,
            },
        })?;
        let z2 = zs.pop().expect("three frames");
        let z1 = zs.pop().expect("three frames");
        models.push(LevelModel {
            state: Ar2State::new(z1, z2),
            phi1: phi[0],
            phi2: phi[1],
            mu: newest_stats.0,
            sigma: newest_stats.1,
        });
    }
    drop(spectra);

    // Step 4 and 5: integer AR steps, lead blending and advection (7.7 to 7.8).
    let max_lead = *lead_steps.last().expect("non-empty");
    let max_bin = max_lead.floor() as usize;
    // Bin each lead by floor(lead), as pysteps' binned_timesteps does.
    let mut bins: Vec<Vec<usize>> = vec![Vec::new(); max_bin + 1];
    for (idx, &t) in lead_steps.iter().enumerate() {
        bins[t.floor() as usize].push(idx);
    }
    let mut outputs: Vec<Vec<f64>> = vec![Vec::new(); lead_steps.len()];
    let mut disp = Displacement::zeros(w, h);
    let mut t_prev = 0.0f64;
    let mut prev = precip_0.clone(); // F_0: the observed (filled) newest frame.
    for t in 0..=max_bin {
        let has_fractional = bins[t].iter().any(|&i| lead_steps[i].fract() > 0.0);
        let need_next = t < max_bin || has_fractional;
        let next = if need_next {
            Some(ar_step(
                &mut models,
                war,
                precip_min,
                &precip_0,
                &domain_mask,
            )?)
        } else {
            None
        };
        if t == 0 || !bins[t].is_empty() {
            for &idx in &bins[t] {
                let lead = lead_steps[idx];
                let frac = lead - lead.floor();
                let ip: Vec<f64> = if frac > 0.0 {
                    let nxt = next.as_ref().expect("fractional lead needs the next step");
                    prev.iter()
                        .zip(nxt)
                        .map(|(a, b)| (1.0 - frac) * a + frac * b)
                        .collect()
                } else {
                    prev.clone()
                };
                let dt = lead - t_prev;
                advect_segment(&mut disp, vx, vy, dt);
                outputs[idx] = warp(&ip, &disp, f64::NAN);
                t_prev = lead;
            }
        } else {
            let dt = (t + 1) as f64 - t_prev;
            advect_segment(&mut disp, vx, vy, dt);
            t_prev = (t + 1) as f64;
        }
        if let Some(nxt) = next {
            prev = nxt;
        }
    }

    let fields = outputs
        .into_iter()
        .map(|o| Field2::new(w, h, o).expect("shape").to_f32())
        .collect();
    Ok(Forecast {
        fields,
        no_rain: false,
    })
}

/// Advance every level one step and produce the next forecast field (spec 7.7 a to e).
fn ar_step(
    models: &mut [LevelModel],
    war: f64,
    precip_min: f64,
    precip_0: &[f64],
    domain_mask: &[bool],
) -> Result<Vec<f64>, NowcastError> {
    for m in models.iter_mut() {
        m.state.step(m.phi1, m.phi2);
    }
    let n = precip_0.len();
    // (b) recomposition in the normalised domain, then denormalised with the newest stats.
    let mut r = vec![0.0f64; n];
    for m in models.iter() {
        let newest = m.state.newest();
        for (dst, &z) in r.iter_mut().zip(newest) {
            *dst += z * m.sigma + m.mu;
        }
    }
    // (c) percentile mask at the wet-area ratio; dry cells become the minimum.
    let thr = probmatch::percentile_threshold(&r, war)?;
    probmatch::apply_threshold(&mut r, thr, precip_min);
    // (d) CDF matching to the newest observed frame.
    let mut matched = probmatch::cdf_match(&r, precip_0)?;
    // (e) domain.
    for (v, &masked) in matched.iter_mut().zip(domain_mask) {
        if masked {
            *v = f64::NAN;
        }
    }
    Ok(matched)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::motion::Velocity;

    #[test]
    fn dry_window_returns_constant_minimum_and_flags_no_rain() {
        let mut frames = Vec::new();
        for t in 0..3 {
            let v: Vec<f32> = (0..16 * 24)
                .map(|i| -10.0 + t as f32 + (i % 3) as f32 * 0.0)
                .collect();
            frames.push(Field2::new(24, 16, v).unwrap());
        }
        let vel = Velocity::zeros(24, 16);
        let out = forecast(
            &frames,
            &vel,
            &[2.5, 5.0, 7.5, 10.0],
            &SprogParams::radar_ng(),
        )
        .unwrap();
        assert!(out.no_rain);
        assert_eq!(out.fields.len(), 4);
        for f in &out.fields {
            assert_eq!((f.width(), f.height()), (24, 16));
            assert!(f.as_slice().iter().all(|&v| v == -10.0));
        }
    }

    #[test]
    fn rejects_bad_inputs() {
        let f = Field2::filled(8, 8, 10.0f32);
        let vel = Velocity::zeros(8, 8);
        let p = SprogParams::radar_ng();
        assert!(forecast(&[f.clone(), f.clone()], &vel, &[1.0], &p).is_err());
        assert!(forecast(&[f.clone(), f.clone(), f.clone()], &vel, &[], &p).is_err());
        assert!(forecast(&[f.clone(), f.clone(), f.clone()], &vel, &[2.0, 1.0], &p).is_err());
        assert!(forecast(&[f.clone(), f.clone(), f.clone()], &vel, &[0.0], &p).is_err());
        let wrong = Velocity::zeros(4, 8);
        assert!(forecast(&[f.clone(), f.clone(), f], &wrong, &[1.0], &p).is_err());
    }
}
