#!/usr/bin/env python3
"""Generate golden fixtures for radar-nowcast from the reference pysteps 1.21.5 code.

Run inside a venv that has numpy, scipy, opencv-python-headless, jsmin, jsonschema,
matplotlib and pysteps==1.21.5 installed (see README.md, "Regenerating fixtures"):

    python3 -I testdata/gen_golden.py

Outputs (all little-endian, row-major):
  synth128_stack.f32      (4, 128, 128) raw frames, -9999 = missing (production convention)
  synth128_uv_pysteps.f32 (2, 128, 128) motion from the radar-ng stride-2 wrapper
  synth128_fc_leads.f64   (12, 128, 128) S-PROG with leads 1..12, pysteps velocity, NaN kept
  synth128_fc_frac.f64    (3, 128, 128)  S-PROG with leads 2.5, 5.0, 7.5
  cascade128_field.f64    (128, 128) smooth test field
  cascade128_levels.f64   (6, 128, 128) normalised Gaussian cascade levels of that field
  cascade128_stats.json   level means and stds (population)
  bandpass128_w.f64       (6, 128, 65) pysteps weights_2d for shape (128, 128)
  lk200_stack.f32         (4, 200, 300) translating blobs for the Appendix A.6 check
  lk200_medians.json      pysteps median vector on echo cells, through the wrapper
  a4_fc.f64               (4, 32, 64) Appendix A.4 S-PROG output (leads 1, 2, 2.5, 3)
  ar_pairs.json           adjust_lag2 + Yule-Walker outcomes for random (g1, g2)
  probmatch.json          percentile masks and CDF-matching cases (binary arrays inside)
  manifest.json           shapes and dtypes of every file
"""
import json
import os
import sys
import warnings

import numpy as np

warnings.filterwarnings("ignore")

from scipy import ndimage  # noqa: E402
from pysteps import motion  # noqa: E402
from pysteps.cascade.bandpass_filters import filter_gaussian  # noqa: E402
from pysteps.cascade.decomposition import decomposition_fft, recompose_fft  # noqa: E402
from pysteps.nowcasts import sprog  # noqa: E402
from pysteps.nowcasts import utils as nutils  # noqa: E402
from pysteps.postprocessing import probmatching  # noqa: E402
from pysteps.timeseries import autoregression as ar  # noqa: E402
from pysteps.timeseries import correlation  # noqa: E402

OUT = os.path.dirname(os.path.abspath(__file__))
STRIDE = 2
MANIFEST = {}


def save(name, arr, dtype):
    a = np.ascontiguousarray(arr, dtype=dtype)
    a.tofile(os.path.join(OUT, name))
    MANIFEST[name] = {"shape": list(a.shape), "dtype": np.dtype(dtype).str}


def save_json(name, obj):
    with open(os.path.join(OUT, name), "w") as fh:
        json.dump(obj, fh, indent=1, sort_keys=True)
    MANIFEST[name] = {"json": True}


def blob(h, w, cy, cx, amp, sigma):
    y, x = np.mgrid[0:h, 0:w].astype(np.float64)
    return amp * np.exp(-((x - cx) ** 2 + (y - cy) ** 2) / (2.0 * sigma**2))


def synth128():
    """Three Gaussian blobs translating with different velocities; -9999 in a corner."""
    h = w = 128
    frames = np.zeros((4, h, w), dtype=np.float64)
    for t in range(4):
        f = blob(h, w, 40 + 1.0 * t, 40 + 2.0 * t, 40.0, 9.0)
        f += blob(h, w, 90 - 0.5 * t, 95 - 1.5 * t, 30.0, 7.0)
        f += blob(h, w, 70 + 0.3 * t, 20 + 1.2 * t, 18.0, 5.0)
        frames[t] = f - 5.0
    frames[:, 0:4, 0:4] = -9999.0
    return frames.astype(np.float32)


def lk_wrapper(stack_f32):
    """Byte-for-byte the radar-ng _estimate_motion wrapper (activities.py)."""
    stack = np.where(stack_f32 < -100, np.nan, stack_f32).astype(np.float32)
    k = STRIDE
    oflow = motion.get_method("LK")
    coarse = oflow(np.ascontiguousarray(stack[:, ::k, ::k]))
    h, w = stack.shape[1:]
    out = np.empty((2, h, w), dtype=np.float32)
    for c in range(2):
        up = ndimage.zoom(np.nan_to_num(coarse[c]), k, order=1)[:h, :w]
        out[c] = np.pad(up, ((0, h - up.shape[0]), (0, w - up.shape[1])), mode="edge") * k
    return out, stack


def sprog_forecast(stack, uv, leads):
    """Same call as _run_nowcast (stack[-3:], precip_thr=5, 6 levels)."""
    nowcaster = sprog.forecast
    return nowcaster(
        stack[-3:, :, :],
        uv,
        leads,
        n_cascade_levels=6,
        precip_thr=5.0,
    )


def main():
    print("pysteps", motion.__name__)
    raw = synth128()
    save("synth128_stack.f32", raw, np.float32)
    uv, stack = lk_wrapper(raw)
    save("synth128_uv_pysteps.f32", uv, np.float32)
    leads = [float(i) for i in range(1, 13)]
    fc = sprog_forecast(stack, uv.astype(np.float64), leads)
    save("synth128_fc_leads.f64", fc, np.float64)
    fc_frac = sprog_forecast(stack, uv.astype(np.float64), [2.5, 5.0, 7.5])
    save("synth128_fc_frac.f64", fc_frac, np.float64)

    # Cascade on a smooth field, with the normalised levels as pysteps returns them.
    h = w = 128
    field = blob(h, w, 50, 60, 35.0, 12.0) + blob(h, w, 90, 30, 20.0, 6.0) - 4.0
    field = field.astype(np.float64)
    bp = filter_gaussian((h, w), 6, gauss_scale=0.5, d=1.0, normalize=True, include_mean=True)
    dec = decomposition_fft(
        field, bp, normalize=True, compute_stats=True, compact_output=False, output_domain="spatial"
    )
    levels = np.stack(dec["cascade_levels"])
    save("cascade128_field.f64", field, np.float64)
    save("cascade128_levels.f64", levels, np.float64)
    save_json(
        "cascade128_stats.json",
        {"means": [float(v) for v in dec["means"]], "stds": [float(v) for v in dec["stds"]]},
    )
    save("bandpass128_w.f64", bp["weights_2d"], np.float64)

    # Appendix A.6 translating blobs, 200 x 300, values minus 5, 4 frames.
    h2, w2 = 200, 300
    lk = np.zeros((4, h2, w2), dtype=np.float64)
    true_uv = (2.0, 1.0)
    for t in range(4):
        f = (
            blob(h2, w2, 90 + true_uv[1] * t, 80 + true_uv[0] * t, 35.0, 14.0)
            + blob(h2, w2, 120 + true_uv[1] * t, 190 + true_uv[0] * t, 28.0, 10.0)
            + blob(h2, w2, 40 + true_uv[1] * t, 140 + true_uv[0] * t, 20.0, 8.0)
        )
        lk[t] = f - 5.0
    lk = lk.astype(np.float32)
    save("lk200_stack.f32", lk, np.float32)
    uv200, stack200 = lk_wrapper(lk)
    echo = stack200[0] > 0.0
    save_json(
        "lk200_medians.json",
        {
            "true_uv": list(true_uv),
            "median_u": float(np.median(uv200[0][echo])),
            "median_v": float(np.median(uv200[1][echo])),
            "coarse_shape": [int(np.ceil(h2 / STRIDE)), int(np.ceil(w2 / STRIDE))],
        },
    )

    # Appendix A.4 S-PROG end to end.
    t_idx = np.arange(3)
    a4 = np.zeros((3, 32, 64), dtype=np.float32)
    yy, xx = np.mgrid[0:32, 0:64].astype(np.float64)
    for t in t_idx:
        cx, cy = 20 + t, 16 + 0.5 * t
        a4[t] = (
            30 * np.exp(-((xx - cx) ** 2 + (yy - cy) ** 2) / (2 * 6**2))
            + 12 * np.exp(-((xx - 45) ** 2 + (yy - 6) ** 2) / (2 * 4**2))
            - 10
        )
    a4[0, 0, 0:5] = np.nan
    vel = np.zeros((2, 32, 64), dtype=np.float32)
    vel[0] = 1.0
    vel[1] = 0.5
    a4_out = sprog.forecast(a4, vel, [1.0, 2.0, 2.5, 3.0], n_cascade_levels=6, precip_thr=5.0)
    save("a4_fc.f64", a4_out, np.float64)
    save("a4_input.f32", a4, np.float32)

    # AR(2): adjust_lag2 + Yule-Walker, random stationary-ish pairs, and error outcomes.
    rng = np.random.default_rng(12345)
    cases = []
    for _ in range(200):
        g1 = float(rng.uniform(-0.99, 0.99))
        g2 = float(rng.uniform(-1.0, 1.0))
        adj = float(ar.adjust_lag2_corrcoef2(np.float64(g1), np.float64(g2)))
        rec = {"g1": g1, "g2": g2, "adj": adj}
        try:
            phi = ar.estimate_ar_params_yw(np.array([g1, adj]))
            rec["phi"] = [float(v) for v in phi]
        except Exception as exc:  # noqa: BLE001
            rec["error"] = type(exc).__name__
        cases.append(rec)
    for g1, g2 in [(0.0, 0.5), (1.0, 0.99), (0.99, 0.2)]:
        adj = float(ar.adjust_lag2_corrcoef2(np.float64(g1), np.float64(g2)))
        rec = {"g1": g1, "g2": g2, "adj": adj}
        try:
            phi = ar.estimate_ar_params_yw(np.array([g1, adj]))
            rec["phi"] = [float(v) for v in phi]
        except Exception as exc:  # noqa: BLE001
            rec["error"] = type(exc).__name__
        cases.append(rec)
    save_json("ar_pairs.json", cases)

    # Autocorrelation on a pair of fields (checks correlation.temporal_autocorrelation).
    z = np.stack([field, field * 0.9 + 0.1 * np.roll(field, 3, axis=1), field * 0.7 + 0.2])
    gam = correlation.temporal_autocorrelation(z)
    save_json("corr_check.json", {"gamma": [float(g) for g in gam]})

    # Probability matching and percentile mask cases.
    pm_cases = []
    n = 60 * 60
    for frac in [0.0, 0.05, 0.4, 1.0]:
        init = rng.standard_normal(n) * 5.0
        init = np.round(init, 4)  # ties are unlikely; keep distinct values
        k = int(round(frac * n))
        order = np.argsort(init)
        dry = np.zeros(n, dtype=bool)
        dry[order[: n - k]] = True
        init[dry] = init.min() - 3.0
        target = rng.gamma(0.7, 4.0, size=n)
        target[rng.choice(n, size=n // 20, replace=False)] = np.nan
        out = probmatching.nonparam_match_empirical_cdf(init.reshape(60, 60), target.reshape(60, 60))
        pm_cases.append(
            {
                "frac": frac,
                "initial": [float(v) for v in init],
                "target": [None if np.isnan(v) else float(v) for v in target],
                "out": [float(v) for v in out.ravel()],
            }
        )
    mask_cases = []
    field_mask = rng.gamma(0.5, 6.0, size=n)
    for war in [0.5, 0.34, 0.25, 0.2, 0.05, 0.0]:
        try:
            m = nutils.compute_percentile_mask(field_mask.reshape(60, 60), war)
            mask_cases.append({"war": war, "mask": [int(v) for v in m.ravel()]})
        except Exception as exc:  # noqa: BLE001
            mask_cases.append({"war": war, "error": type(exc).__name__})
    small = np.array([[1, 5, 3], [2, 4, 3]], dtype=float)
    small_cases = []
    for war in [0.5, 0.34, 0.25, 0.2, 0.0]:
        try:
            m = nutils.compute_percentile_mask(small, war)
            small_cases.append({"war": war, "mask": [int(v) for v in m.ravel()]})
        except Exception as exc:  # noqa: BLE001
            small_cases.append({"war": war, "error": type(exc).__name__})
    save_json(
        "probmatch.json",
        {"cdf": pm_cases, "percentile_random": mask_cases, "percentile_small": small_cases,
         "percentile_random_shape": [60, 60]},
    )

    save_json("manifest.json", MANIFEST)
    print("wrote", len(MANIFEST), "fixture files to", OUT)
    return 0


if __name__ == "__main__":
    sys.exit(main())
