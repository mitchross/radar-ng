# radar-nowcast

The radar-ng nowcast, in two layers:

- **Numerical core** (pure computation on `Field2` buffers): pyramidal Lucas-Kanade motion,
  semi-Lagrangian advection, FFT cascade, AR(2), CDF probability matching, S-PROG, verification
  scores. A from-scratch replacement for the pysteps pieces the Python nowcast used.
- **Activity glue** (`nowcast_run`): input grid selection, grid reads, preprocessing, motion and
  forecast, point grids and tile pyramids, the atomic manifest swap, the status file and the
  post-commit skill scoring. A plain async function with no Temporal types; the worker wraps it.

## Status

| Area | State |
|---|---|
| Unit tests (`src/`) | 62 pass |
| Golden tests vs pysteps 1.21.5 (`tests/golden.rs`) | 16 pass |
| End-to-end `nowcast_run` (`tests/nowcast_run.rs`) | 8 pass |
| Skill log and scoring (`tests/score.rs`) | 3 pass |
| Perf test (`tests/perf.rs`, `--ignored`) | 1, runs; see Performance |
| Compiler warnings (`cargo test --release`) | none |
| Clippy | not run in this round |

Run:

```
cd rust
cargo test -p radar-nowcast                      # all suites, debug (nowcast_run ~40 s)
cargo test -p radar-nowcast --release            # all suites, release (nowcast_run ~4 s)
cargo test -p radar-nowcast --release --test perf -- --ignored --nocapture
```

## Public API

Re-exported from `lib.rs`:

- `nowcast_run(cfg: Arc<NowcastConfig>, heartbeat: Arc<dyn Fn() + Send + Sync>) -> anyhow::Result<NowcastOutput>`:
  the activity body. Returns `NowcastOutput { result: NowcastResult, skill_summary: Option<Summary>,
  prometheus: Vec<String> }`. `NowcastResult` comes from `radar-types` (`payloads::nowcast`). The
  heartbeat is called every `heartbeat_every` (30 s by default) while CPU work runs on `spawn_blocking`.
- `NowcastConfig` (`config`): paths, grid config, horizon and step, input frames (clamped to at least 3),
  input step and gap limits, `allow_persistence_fallback`, motion stride, point-grid cap, retention,
  render workers, palettes, PNG compression, skill config, skill match tolerance (150 s), heartbeat
  interval. `from_env()` for the worker, `with_paths(Paths)` for tests.
- `bench_nowcast(frames_dir: &Path, out: &Path) -> anyhow::Result<BenchReport>` (`bench`): times motion
  and forecast separately on a directory of grid dumps and writes `nowcast-bench.json`.
  `BenchReport { frames, width, height, motion_stride, leads, threads, motion_s, forecast_s, total_s,
  no_rain, checksum }`.
- `Field2<T>` (row-major `width x height`), `NowcastError`.
- `Velocity { x, y }`: pixels per input step, `x` = column, `y` = row.
- `MotionParams::radar_ng()`, `estimate_motion(stack, stride, &params) -> Velocity` (full resolution,
  stride-2 coarse LK, upsampled with scipy `zoom(order=1)` geometry and multiplied by the stride),
  `dense_lucaskanade(frames, &params)` (full-resolution LK on the given stack).
- `SprogParams::radar_ng()`, `forecast(frames, &velocity, &lead_steps, &params) -> Forecast`
  where `Forecast { fields: Vec<Field2<f32>>, no_rain: bool }`. Uses the last three frames.

Modules, public for the glue and for tests: `advection`, `ar`, `bench`, `cascade`, `config`, `error`,
`field`, `inputs` (grid loading, fill normalisation, input listing), `motion`, `probmatch`, `render`
(palette rendering, point grids, `FrameSpec`), `run` (the glue), `score` (observation scoring),
`select` (spec 4.2 input selection and cadence), `skill`, `sprog`, `status` (`status.json`).

## Algorithm fidelity

Reference: pysteps 1.21.5 with the radar-ng settings. "Exact" means matched to the fixture
to float precision. "Approximate" means an independent re-implementation, not bit-identical.

- **field**: NaN-aware min/max (`np.nanmin` semantics, `np.maximum` NaN propagation), strict `< threshold`
  masking that keeps NaN. Exact.
- **advection** (`advection.rs`): `map_coordinates(order=1)` with constant and nearest modes; a NaN
  corner with zero weight still poisons the sample, as scipy does; corners past the last index are
  skipped. Midpoint velocity, displacement accumulated across lead segments, pysteps lead binning
  `floor(lead)`, empty bins advance the displacement by `t + 1 - t_prev`. Exact (Appendix A.3
  interpolation rules checked in `appendix_a3_interpolation_rules`).
- **cascade** (`cascade.rs`): Gaussian bandpass with `q = (L/2)^(1/n)`, per-frequency normalisation
  over levels, DC to level 0; FFT via `realfft` rows and `rustfft` columns in numpy `rfft2`/`irfft2`
  convention (DC and Nyquist imaginary parts zeroed before `c2r`, `1/(hw)` scale). Weights match
  pysteps to 1e-12; levels match to 1e-4 (test tolerance). Exact up to f64 round-off.
- **ar** (`ar.rs`): `np.corrcoef` with clipping, lag-2 adjustment per pysteps, Yule-Walker AR(2) by a
  2x2 LU with partial pivoting, perturbation `sqrt(max(0, 1 - g1 phi1 - g2 phi2))`, stationarity by
  the quadratic-root test. Parameters match `ar_pairs.json` (203 cases, 201 with a pysteps phi) within
  1e-6, including the failure cases (singular and non-stationary).
- **probmatch** (`probmatch.rs`): threshold from `np.percentile` linear with the `_lerp` formula;
  CDF matching assigns the top-k sorted target values to wet cells in ascending initial order.
  Exact in the matched values; ties in the initial field may land on a different tied cell (see
  deviations). Matches the `probmatch.json` cases up to tied cells.
- **motion** (`motion/`): an own re-implementation of the OpenCV path used by pysteps:
  - Shi-Tomasi `goodFeaturesToTrack`: 5x5 block min-eigenvalue, 3x3 Sobel, REFLECT_101 borders,
    local maximum, greedy minimum-distance selection. Approximate.
  - Morphological opening with a 3x3 ellipse (a cross) on the echo mask; erosion ignores out-of-image
    neighbours, dilation counts them as 0. Exact to the pysteps step.
  - `calcOpticalFlowPyrLK`: 5-tap pyrDown, Scharr 1/32 gradients, 50x50 window at the half-pixel
    offsets, 10 Newton iterations, min-eigenvalue and area test at level 0, FLT_EPS singularity.
    Approximate (not OpenCV bit-exact).
  - Sparse post-processing: kNN Mahalanobis outlier removal (k = 30, 3 sigma, ddof = 1, singular
    gives distance 0), declustering into 20 px bins with medians, IDW (power 0.5, k = 20, offset 0.5)
    onto the grid. Exact in the rules. The IDW neighbours are exact: each 16 x 16 pixel block takes
    the points within `d_k(centre) + 2 * half-diagonal` of its centre, and each pixel selects its k
    nearest from that set. A unit test compares every pixel with a brute-force IDW (1e-4).
  - Stride-2 wrapper: `[::2, ::2]` subsample, NaN to 0, align-corners upsample, crop, times 2.
- **sprog** (`sprog.rs`): S-PROG with AR(2) per level, CDF probability matching against the
  precipitation climatology, the dry-window rule, and the forecast assembled from the summed levels.
- **skill** (`skill.rs`): contingency table (unknown = NaN or `<= -100`, fill is dry), POD / FAR / CSI /
  bias, Python `round(x, 3)` via `format!("{:.3}")`, point-grid stride rule, nearest resampling
  (`rint`), summary with headline lead nearest 30 min, minimum 6 runs, `no_rain_observed` reason.
  Verified by unit tests only. pysteps has no equivalent, so the spec's definitions are the reference.

## Activity glue (`run.rs`, `select.rs`, `inputs.rs`, `render.rs`, `score.rs`, `status.rs`)

Follows `docs/rust-migration/specs/spec-nowcast.md` sections 2 to 5, 8 to 11 and 13, and the former Python nowcast activity.

1. **Setup**: the processed-anchor file (`nowcast.json`, last 100 anchors) short-circuits an anchor
   already committed. `select_input_grids` (spec 4.2) picks the input grids on the 5-minute marks, with
   the earlier gap at most `0.6 * step`, tolerance `0.45 * step`, falling back to the newest N. Too few
   grids writes `warming_up`. A cadence violation (gap above `max(6, 1.5 * 5) = 7.5` min) is `degraded`
   and nothing runs.
2. **Inputs**: grid dumps are read with radar-core `grid_read`. Values below -100 become NaN, and fill is
   normalised to -9999. The grid geometry must match across frames.
3. **Motion and forecast**: `estimate_motion` (stride 2) and `forecast` (S-PROG). A failure is `degraded`
   with reason `pysteps_failed`. The persistence fallback (`NOWCAST_ALLOW_PERSISTENCE_FALLBACK`) is
   implemented and off by default. Leads are binned by `floor(lead)`, as in pysteps.
4. **Render**: each lead is rendered with radar-tiler `render_frame_palettes` (renderer identity
   `indexed-rs1`) at zooms 4, 5 and 6, with the 5 dBZ dry threshold. A lead is accepted only when its
   rendered palette set equals the expected set. Accepted leads also get point grids
   (`runs/<anchor>/<timestamp>`).
5. **Commit**: observed grid, `finalize_grid_generation`, atomic `replace_layer` with `ReplaceOptions`
   (palettes, frames, layer metadata), `prune_grid_generations` keeping whole runs, processed-anchor add.
   An incomplete render fails the commit (`nowcast incomplete: rendered n/N leadtimes`) before the
   manifest is touched. The status file is written only after the commit succeeds.
6. **Post-commit**: previous runs are scored against the observation (150 s match tolerance, persistence
   baseline from `runs/<anchor>/observed`, each observation matched once), appended to the skill log, and
   summarised as Prometheus text. Scoring failures are logged, not fatal.

Deviations from the Python reference are listed under Deviations, items 11 to 20.

## Golden tests (`tests/golden.rs`)

Fixtures live in `testdata/` and are written by `testdata/gen_golden.py`.

| Test | Reference | Result |
|---|---|---|
| `ar_params_match_pysteps_within_1e6` | `ar_pairs.json` | pass, 1e-6 |
| `appendix_a2_lag2_and_yw_values` | spec A.2 | pass |
| `bandpass_weights_match_pysteps_exactly` | `bandpass128_w.f64` | pass, 1e-12 |
| `cascade_levels_match_pysteps_within_1e4` | `cascade128_levels.f64` | pass, 1e-4 |
| `cdf_match_equals_pysteps_up_to_ties` | `probmatch.json` cdf | pass (ties only) |
| `percentile_mask_matches_pysteps` | `probmatch.json` percentile | pass; war = 0.2 and 0.0 are the D1 cases |
| `motion_matches_pysteps_on_synthetic_128` | `synth128_uv_pysteps.f32` | pass: mean u 1.034 vs 1.009 (+2.5 %), mean v 0.596 vs 0.602 (-0.9 %); RMSE u 0.026, v 0.006 px/step |
| `stride_two_translation_recovers_appendix_a6_velocity` | `lk200_stack.f32`, true (2, 1) | pass: median (2.0034, 0.9990) vs pysteps (2.0033, 0.9990) |
| `sprog_matches_pysteps_given_the_same_motion` | `synth128_fc_leads.f64`, pysteps velocity | pass: correlation 1.0000 at every lead 1 to 12 |
| `sprog_fractional_leads_match_pysteps` | `synth128_fc_frac.f64` | pass, correlation > 0.95 |
| `sprog_end_to_end_with_our_motion_is_reported` | same, our LK velocity | correlation 0.9988 to 0.9999 over leads 1 to 12 |
| `appendix_a4_sprog_end_to_end` | spec A.4 table | pass: NaN counts 100/131/223/223, sums, sampled pixels |
| `dry_window_matches_appendix_a5` | spec A.5 | pass: constant minimum, `no_rain = true` |
| `sprog_recovers_translation_of_a_blob` | synthetic | pass: lead-1 correlation 0.93 against shifted truth |
| `exact_translation_is_reported_as_singular_ar` | synthetic | pass: `ArFit` error, no panic |

Motion is judged against the 5 % mean and the per-pixel RMSE bounds. The RMSE bounds were set
to 0.25 px/step, which is loose compared with the measured 0.026 px on u. This is a bound for
regression, not a claim of OpenCV parity.

End-to-end and scoring tests (not golden), in `tests/nowcast_run.rs` and `tests/score.rs`:

| Test | Checks |
|---|---|
| `full_run_commits_manifest_tiles_points_and_status` | manifest layer replaced, tiles on disk, point grids, status written after commit |
| `second_run_scores_the_previous_forecast_into_the_skill_log` | skill log updated by the next run |
| `already_processed_anchor_is_not_rerun` | processed-anchor early exit |
| `dry_window_commits_an_empty_run_reported_as_no_rain` | D2: dry window ran, no render, empty commit, `ok` / `no_rain` |
| `too_few_grids_is_warming_up` | `warming_up` status, nothing committed |
| `cadence_violation_is_degraded_without_running` | `degraded`, nothing committed |
| `retention_removes_old_whole_runs` | retention keeps whole runs |
| `bench_entry_point_reports_timings_and_checksum` | `bench_nowcast` JSON |
| `score.rs` (3 tests) | tolerance matching, persistence baseline null when missing, log written only when something matched |

Unit tests also cover the commit ordering: an incomplete commit leaves no status, manifest or processed
file (D4).

## Performance (`tests/perf.rs`, ignored by default)

Input: 4 synthetic frames, 3500 x 1750 (the 875 x 1750 coarse grid is what LK sees with stride 2).
Release profile on 2 vCPUs shared with other agents' builds (load average 2 to 4). Wall times are noisy.
Each row is three interleaved rounds (original build, optimised build, original, optimised, ...), and the
best run is reported.

| | Original (3 runs) | Optimised (3 runs) | Best original | Best optimised |
|---|---|---|---|---|
| `estimate_motion`, stride 2 | 21.0, 37.0, 18.9 s | 6.8, 7.5, 3.8 s | 18.9 s | **3.8 s** |
| `forecast`, 12 leads | 21.3, 22.8, 16.1 s | 21.0, 20.1, 15.6 s | 16.1 s | 15.6 s (no measurable change) |
| Total | | | 35.0 s | **19.4 s** |

The spec estimate is 15 s total on the production host (section 12). Under this load the total is
19.4 s. The machine is shared, so this is not a clean single-tenant figure.

What changed:

- **IDW interpolation** (`motion/sparse.rs`). The old per-pixel bucket search re-sorted its best list on
  every insertion and walked growing rings. It took about 48 s of a 57 s run under load, and was the
  motion hot spot. Now one candidate set is built per 16 x 16 block (see Algorithm fidelity). The
  neighbours are the same as the brute force.
- **FFT transposes** (`cascade.rs`). The forward transpose and the inverse row gather read with a
  column stride. Both now use 16-column or 16-row blocks, so reads are contiguous. The weighted inverse
  (`irfft2_weighted`) no longer clones the spectrum once per level.
- **CDF matching** (`probmatch.rs`). The wet-cell and top-k sorts compare `u64` keys
  (order-preserving, NaN last, `-0.0 == 0.0`) instead of `f64` pairs. Same order, including the index
  tie-break.
- **Advection** (`advection.rs`). The two velocity components are sampled at the same point with shared
  corner arithmetic. Same arithmetic per component.

The first three changes keep the arithmetic unchanged, apart from the `sqrt` note in deviation 14. All
golden tests pass, and the crate's test suites pass in release.

Forecast breakdown, from an instrumented run made after the IDW change and before the FFT and sampler
changes (under load, approximate):

- cascade and AR fit: about 5.5 s (3 forward and 3 x 6 inverse 2-D FFTs on 6.1 M points, plus normalise)
- per lead: AR step about 0.6 s (recomposition about 0.1 s, percentile about 0.07 s, CDF matching about
  0.35 to 0.4 s, state update the rest), advection plus warp about 0.5 s (15 segments in total,
  including the pre-advection of frames 0 and 1)

The forecast is bound by full-domain f64 passes and by the number of FFTs, not by a single hot loop.
The FFT and sort changes did not show up in the wall-clock A/B. The remaining options, each of which
changes numerics and needs a golden re-check:

- f32 storage for the AR state and the displacement (halves memory traffic; the displacement is the
  sensitive one).
- Fusing the per-lead passes (state update, recomposition, percentile, CDF) into fewer sweeps.
- Obtaining the AR correlations from the spectra (Parseval) and skipping the inverse FFT for frame 0.

Motion now costs about 1.3 s per pair after the IDW fix. Most of that is pyramidal LK: 1000 points,
50 x 50 window, 10 Newton iterations and `epsilon = 0`, which is about 1.3e8 bilinear samples per pair.
That is the fidelity setting (OpenCV's `TermCriteria`), so it is left as is. A specialised window
sampler (the bilinear weights are the same across the window) is the next step there.

## Deviations from the spec text

1. **`adjust_lag2`** uses `2*g1*g2 - 1` as pysteps does, not the spec's `2*g1^2 - 1`. The golden
   values come from pysteps, so the code follows pysteps (see `ar.rs` module docs).
2. **NaN gamma** (for example `gamma1 = 0`, where pysteps yields NaN and `LinAlgError`) returns a typed
   `ArError` (`NonFinite` or `Singular`), wrapped as `NowcastError::ArFit { level }`. It does not
   produce NaN fields.
3. **Exact translation** makes the Yule-Walker system singular (gamma1 = 1). pysteps raises
   `LinAlgError` here too. The crate returns `ArFit`, not a forecast. Real radar data is not exactly
   rigid, but this path is reachable in principle.
4. **D1, low wet-area ratio**: pysteps raises `IndexError` in `percentile` when `war` is near 0 or
   small. The fix keeps the maximum cell(s) as the threshold (`thr = max`). Matches the spec's intent
   that the forecast does not fail. Covered by `percentile_mask_matches_pysteps`.
5. **D2, dry window**: a window with no cell above the rain threshold is reported as dry through
   `Forecast::no_rain = true`, and the glue commits it as an empty run (item 12 below).
6. **D4, status**: status is set only after a successful commit of outputs (item 13 below).
7. **CDF tie order**: tied initial values are assigned in index order (stable argsort). pysteps'
   `argsort` is unstable, so a tie can land on a different cell. The matched values are the same.
8. **kNN**: no KD-tree. The outlier kNN is brute force; the IDW neighbour search is a block candidate
   set (exact, see Algorithm fidelity). Both give the same neighbours as a brute-force search.
9. **OpenCV**: Shi-Tomasi and pyrLK are re-implementations following the OpenCV algorithms. They are
   not bit-exact. The `opencv` crate is the fallback if exact parity is required. Its absence is the
   reason the motion tests use measured bounds.
10. **Precision**: internal arithmetic is f64 (DEC-6); stored fields are f32, as in the spec.
11. **Renderer**: tiles use radar-tiler `indexed-rs1`, as specified for this port, not the Python
    default renderer. `NowcastResult` palettes are the rendered union over accepted leads.
12. **D2 in the glue**: a dry input window counts as ran (`ran = true`), renders nothing, reports
    `leadtimes = 0`, and replaces the layer with an empty frame set. The status is `ok` with reason
    `no_rain` and detail "no cell above 5 dBZ in the input window". `no_rain` is an extension of the
    spec 9.3 reasons.
13. **D4 in the glue**: `status.json` is written only after a successful commit, and only for method
    `pysteps-sprog`. Persistence commits keep the degraded status. A commit failure leaves the previous
    `status.json` in place, which may still read `ok` from an earlier run. A `commit_failed` status
    would deviate from "status only after commit", so it is not written. This is an open question for
    the team. A status write failure after a successful commit is logged, not fatal.
14. **IDW `powf(0.5)`**: the default exponent is evaluated as `sqrt`. Results may differ from `powf` by
    one ulp. Other exponents use `powf`.
15. **Persistence fallback**: implemented but off by default. `NOWCAST_ALLOW_PERSISTENCE_FALLBACK`
    accepts `1`, `true`, `yes` or `on`. The Python code accepted only `1`.
16. **Observed bounds** for scoring are taken from the full axes, as in Python `skill.py`. Point-grid
    loads for scoring skip the fill rewrite, and the loop stops at the first load failure.
17. **Heartbeat**: the callback is `Fn()`, so it carries no phase details. It fires every 30 s while the
    blocking work runs, through `radar_core::heartbeat`.
18. **Leads in parallel**: rendering uses a rayon pool of `render_workers` threads, one lead per task.
19. **Pysteps-unavailable path** (spec 13, item 17) does not apply: there is no pysteps in the Rust
    build. Spec 13 items 28 and 29 belong to radar-api.
20. **Selection**: the gap check in `select_input_grids` is `secs_between(candidate, last_chosen) < min_gap`.
    An earlier version compared the arguments the other way round and picked the later grids; the unit
    tests for spec 13 items 1 to 4 caught it.

## Not implemented (deferred)

- **Temporal wrapper**: `nowcast_run` is a plain function. The worker in `radar-worker` wraps it in an
  activity and supplies the heartbeat.
- **Spec 13 items 28 and 29** (API invariants): owned by radar-api.
- **Python vs Rust timing parity**: `bench_nowcast` provides the Rust side. The comparison with the
  Python timings is not in this crate.
- **OpenCV exact parity** (deviation 9).
- **Performance follow-ups** listed under Performance: f32 storage, fused passes, spectral correlations,
  and a window sampler for LK.

## Regenerating the fixtures

Prerequisites: Python 3.13 with pysteps 1.21.5 (the fixtures were made with numpy 2.5.4; pysteps also
needs `jsmin`, `jsonschema` and `matplotlib` to import):

```
python3 -m venv <venv>
<venv>/bin/pip install pysteps==1.21.5 numpy jsmin jsonschema matplotlib
cd rust/crates/radar-nowcast/testdata
<venv>/bin/python -I gen_golden.py
```

`-I` keeps the script from importing modules from the current directory. The generator writes
the fixture files and `manifest.json`. After regenerating, rerun `cargo test -p radar-nowcast`.
