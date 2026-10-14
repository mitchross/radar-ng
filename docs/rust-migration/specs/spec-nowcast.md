# Nowcast porting spec: radar-ng `nowcast` (Python) to Rust

Status: complete for implementation handoff. Written from the Python source in
`/mnt/user-data/uploads/radar-ng` and checked against the pysteps checkout at
`/home/claude/pysteps/pysteps` (pysteps 1.21.5, commit f4ebd1d). Golden values
in Appendix A were produced by running that pysteps code.

Read this spec without the Python. Where a behaviour is a defect that the
Python exhibits, the spec says so and gives a parity default; section 1.5 lists
the decisions the Rust owner must make.

## 0. Conventions and tags

Every behavioural statement carries one tag:

- **[SRC]**: read directly from the radar-ng Python (behaviour certain).
- **[PS]**: read from pysteps source, or reproduced by running it (certain).
- **[OCV]**: OpenCV internals (`goodFeaturesToTrack`, `calcOpticalFlowPyrLK`).
  Not in the local tree. Described from OpenCV's documented behaviour and
  common implementation. Treat as tolerance-level, not bit-exact.
- **[EST]**: estimate or derived number, not measured.
- **[DECIDE]**: a choice the Rust owner must make (section 1.5).

Units used throughout: "cell" or "px" is a cell of the science grid (about
0.02 deg, section 3.1) unless stated. Time steps in pysteps are *input steps*
(the measured cadence of the inputs, about 5 min), not minutes.

---

## 1. Summary

### 1.1 Pipeline

1. MRMS base-reflectivity frame lands. The ingest activity writes a science grid
   (`radar-nowcast-input`), prunes that layer, then kicks the `nowcast` Temporal
   Schedule. [SRC]
2. The `nowcast` schedule also fires every 2 min as a backstop. Each fire runs
   `NowcastWorkflow` -> activity `nowcast_run`. [SRC]
3. `nowcast_run` selects up to 4 input grids about 5 min apart, validates them,
   and skips the run if the newest grid was already forecast. [SRC]
4. Motion: dense pyramidal Lucas-Kanade on all selected frames, computed at
   stride 2 and upsampled back to full resolution. [SRC, PS]
5. Forecast: S-PROG on the newest 3 frames, in raw dBZ, threshold 5 dBZ,
   6 cascade levels, AR(2), CDF probability matching, 12 leads of 5 min. [SRC, PS]
6. Postprocess: values below 5 dBZ become the fill -9999. [SRC]
7. Per lead: write one point grid (for the API) and render tile pyramids for
   every configured palette. [SRC]
8. Commit: write the observed anchor grid, finalize its generation, atomically
   replace the layer manifest, prune old point-grid runs, record the anchor as
   processed. [SRC]
9. Verification: score every retained run that predicted the new observation,
   append to a rolling 24 h log. This happens after commit and never delays a
   frame. [SRC]

### 1.2 Things that differ from what you might assume

1. **No dBZ to rain-rate conversion in the worker.** S-PROG runs on raw dBZ with
   `precip_thr = 5.0` dBZ. The Marshall-Palmer Z-R (`R = (Z/200)^(1/1.6)`) is only
   in the API point endpoint (section 11). [SRC]
2. **S-PROG sees 3 frames; LK sees all of them.** `_run_nowcast` passes
   `stack[-3:]` to S-PROG and the full stack to LK. With 4 selected frames, the
   oldest is used only for motion. [SRC]
3. **Motion is computed at stride 2.** The velocity is in full-resolution pixels
   per input step (LK vectors times 2). The upsampling uses align-corners
   linear interpolation (section 6.1), which scipy's `zoom` does by default. [SRC, PS]
4. **Leads are fractional input steps.** Lead `i` (minutes `5(i+1)`) is at
   `5(i+1) / input_interval_min` input steps. Fractional leads are linearly
   blended between integer AR steps and then advected. [SRC, PS]
5. **The default tile renderer is the legacy RGBA path**, not the indexed path.
   `TILE_RENDERER` defaults to `legacy` and the nowcast role is not the canary.
   Both are specified (section 8.6). [SRC]
6. **One attempt, not two.** `RetryPolicy(maximum_attempts=1)` in the workflow.
   The activity docstring says "2 attempts". Code wins. [SRC]
7. **Three palettes in production.** `PALETTES=classic,muted,vivid` in the
   ConfigMap, so 36 pyramids per run (12 leads x 3 palettes). [SRC]

### 1.3 Documentation drift (code wins)

| Item | Code | Doc says |
|---|---|---|
| Heartbeat timeout | 120 s (`workflows/nowcast.py`) | 300 s (`ARCHITECTURE.md:80`) |
| Start-to-close | 10 min (`workflows/nowcast.py`) | 15 min (`ARCHITECTURE.md:80`) |
| Attempts | 1 | 2 (`activities.py` docstring) |
| Retention of point-grid runs | 36 (`NOWCAST_POINT_GRID_RETENTION_RUNS`) | "keep two full runs" (comment in `activities.py`, stale) |

### 1.4 Defects the parity build reproduces

Each defect has an ID so the Rust tests can refer to it.

- **D1. Percentile index overflow.** `compute_percentile_mask` indexes
  `precip_s[i+1]` after choosing index `i` nearest the wet fraction. When
  `war < 1.5/N` the chosen index is `N-1` and the lookup raises `IndexError`.
  `N = H*W` (about 6.1 M). In `_run_nowcast` this is caught as `Exception`, so
  the run is `degraded / pysteps_failed` and nothing is published. It fires
  when the newest frame has at most one cell at or above 5 dBZ, while some older
  frame in the 3-frame window still has a cell above 5 dBZ. [PS, verified by
  running: N=6 raises for war = 0.2 and 0.0, passes at 0.25]
- **D2. Dry window fails the commit.** If no cell in the 3-frame window is
  strictly above 5 dBZ, S-PROG returns a constant field equal to the input
  minimum for every lead (`check_norain` shortcut). After the `< 5 -> -9999`
  conversion every lead has zero visible tiles. `rendered_palettes` only counts
  palettes with `tile_count > 0`, so each lead is dropped, `len(rendered) != 12`,
  and `_commit` raises `RuntimeError("nowcast incomplete ...")`. The activity
  fails. `ProcessedSet` is not updated and the status file still says `ok`.
  [SRC, PS verified shape (4,H,W) constant output; SRC for the rest]
- **D3. Multiple scorings per valid time.** The matching window is +-150 s, but
  observations arrive every ~2 min. A forecast valid time is matched by 2 or 3
  observations (for example valid 16:05 with observations 16:04 and 16:06), and
  each match appends an entry with `valid = observed_ts`. Counts are per
  (anchor, observation), not per forecast valid time. [SRC]
- **D4. Status says `ok` before commit.** `_run_nowcast` writes
  `nowcast-status.json = ok` before the lead loop and commit. A later commit
  failure (D2 or an incomplete render) leaves it `ok`. [SRC]
- **D5. Skill log has no lock.** `append_entries` is read-modify-write without
  a lock. Safe only because the nowcast schedule never overlaps itself. [SRC]
- **D6. API fill substitution.** `_sample_grid_point` replaces a row pair that
  contains a fill with `valid[0]` / `valid[-1]` (the first/last valid corner),
  not a renormalised weighted mean. Section 11.2. [SRC]

### 1.5 Decisions required [DECIDE]

| ID | Decision | Parity default | Recommendation |
|---|---|---|---|
| DEC-1 | D1: IndexError path | Degraded `pysteps_failed`, no publish | Keep parity, make the status explicit |
| DEC-2 | D2: dry window | Raise in commit | Publish an empty run or skip with status `warming_up`/`no_rain`; do not fail |
| DEC-3 | D3: double scoring | Keep | Keep for series continuity; document |
| DEC-4 | Renderer | `legacy` | Port both; keep `legacy` until canary passes |
| DEC-5 | LK implementation | Pure Rust, tolerance-tested | Pure Rust; keep `opencv` crate as fallback |
| DEC-6 | S-PROG float precision | f64 in spectral and statistics, f32 storage | Same |
| DEC-7 | Persistence fallback | Off (`NOWCAST_ALLOW_PERSISTENCE_FALLBACK=0`) | Off |
| DEC-8 | Scoring after commit has no heartbeat | As is | Move scoring after the activity returns, or heartbeat inside it |

---

## 2. Temporal integration

### 2.1 Definitions [SRC]

- Workflow name: `NowcastWorkflow`. File `temporal/workflows/nowcast.py`.
- Activity name: `nowcast_run`. No arguments. File `backend/nowcast/activities.py`.
- Task queue: `radar-ng-nowcast` (`temporal/task_queues.py`). With isolated
  queues off, the legacy `radar-ng` queue is used.
- Activity options:
  - `start_to_close_timeout` = 10 min
  - `schedule_to_close_timeout` = 12 min
  - `heartbeat_timeout` = 120 s
  - `retry_policy` = `maximum_attempts=1`
- Heartbeats: `run_sync_with_heartbeat(..., heartbeat_every=30)` for the
  pysteps phase and for the render phase. Details dict:
  `{"phase": "pysteps"|"render", "input_frames": n, "leadtimes": 12}`.
  Note: `_setup` (reading about 100 MB) runs before the first heartbeat, and
  scoring after commit has no heartbeat (DEC-8).

### 2.2 Schedules and kick [SRC]

- Schedule id `nowcast`, interval 2 min, `max_runtime` 12 min,
  `catchup_window` 5 min, `jitter` 20 s, `overlap` SKIP (timer fires).
- Kick (producer, `backend/ingest_mrms/activities.py::kick_nowcast`): after a
  base-reflectivity frame (`layer_name == "radar"`) has rendered every palette
  and written the science grid, it calls
  `ScheduleHandle("nowcast").trigger(overlap=BUFFER_ONE, rpc_timeout=5 s)`.
  BUFFER_ONE means an idle nowcast starts now, and a busy one queues exactly one
  follow-up. Errors are logged and swallowed.
- Order inside `mrms_process_frame` after a complete frame:
  render all palettes -> write `radar-nowcast-input` (max cells 7,000,000,
  NaN -> -9999) -> `prune_grid_layer("radar-nowcast-input")` (keep 12) ->
  `kick_nowcast` -> observation manifest frame. An incomplete frame returns
  before any of these, so nowcast never anchors to an unpublishable frame.

### 2.3 Environment variables (defaults)

| Variable | Default | Notes |
|---|---|---|
| `GRID_DIR` | `/data/grids` | |
| `TILE_DIR` | `/data/tiles` | |
| `STATE_DIR` | `/data/state` | |
| `NOWCAST_HORIZON_MIN` | 60 | leads = 60 / 5 = 12 |
| `NOWCAST_STEP_MIN` | 5 | output cadence |
| `NOWCAST_INPUT_FRAMES` | 4 | clamped to `max(3, x)` |
| `NOWCAST_INPUT_STEP_MIN` | 5.0 | target spacing of inputs |
| `NOWCAST_GRID_INPUT_LAYER` | `radar-nowcast-input` | falls back to `radar` only if the dir is absent and the name is not `radar` |
| `NOWCAST_MAX_INPUT_GAP_MIN` | 6 | cadence limit = `max(6, 1.5*5) = 7.5` min |
| `NOWCAST_ALLOW_PERSISTENCE_FALLBACK` | `0` | DEC-7 |
| `NOWCAST_MOTION_STRIDE` | 2 | LK stride |
| `NOWCAST_POINT_GRID_MAX_CELLS` | 900000 | point grid cap |
| `NOWCAST_POINT_GRID_RETENTION_RUNS` | 36 | min 2 |
| `NOWCAST_RENDER_WORKERS` | 4 | threads over leads |
| `PALETTES` | `classic` (ConfigMap sets `classic,muted,vivid`) | comma list |
| `TILE_RENDERER` | `legacy` | `indexed` needs `TILE_RENDERER_CANARY_ROLE` |
| `TILE_RENDERER_CANARY_ROLE` | unset | one of `mrms,nowcast,hrrr,aux` |
| `TILE_PNG_COMPRESS_LEVEL` | 6 | indexed path |
| `TILE_GEOMETRY_CACHE_MB` | 256 | cache only |
| `NOWCAST_SKILL_WINDOW_HOURS` | 24 | |
| `NOWCAST_SKILL_HEADLINE_LEAD_MIN` | 30 | |
| `NOWCAST_SKILL_MIN_RUNS` | 6 | |
| `NOWCAST_SKILL_MATCH_TOLERANCE_S` | 150 | |
| `NOWCAST_KICK_ENABLED` | 1 | producer side |
| `NOWCAST_SCHEDULE_ID` | `nowcast` | producer side |
| `NOWCAST_KICK_TIMEOUT_S` | 5 | producer side |
| `NOWCAST_SCIENCE_GRID_MAX_CELLS` | 7000000 | producer side |

### 2.4 Output payload `NowcastResult` [SRC]

Temporal serialises the dataclass as a JSON object with these keys:

```json
{
  "ran": true,
  "anchor_ts": "2026-07-15T16:00:00+00:00",
  "leadtimes": 12,
  "palettes": ["classic", "muted", "vivid"],
  "duration_s": 512.37,
  "skill_scored": 9
}
```

- `ran=false` cases (`warming_up`, already processed, invalid cadence, pysteps
  unavailable or failed): `leadtimes=0`, `palettes=[]`, `duration_s=0.0`,
  `skill_scored=0`, `anchor_ts` set to the newest grid's timestamp when known,
  otherwise `null`.
- `palettes` is the sorted list of palettes that produced at least one tile.
- `duration_s` is rounded to 2 decimals.

---

## 3. Inputs on disk

### 3.1 Science grid derived from MRMS [SRC]

- Raw product: `CONUS/MergedBaseReflectivityQC_00.50`, GRIB2, values in dBZ,
  masked cells -> NaN. Raw grid 3500 rows x 7000 columns, 0.01 deg.
- Decode (`_download_and_decode_sync`): `data.filled(np.nan)`, float32; longitudes
  converted to -180..180; axes read from header (`regular_ll`).
- Orientation (`mrms_process_frame`): latitudes ascending, then the array is
  flipped to south-first. `write_grid` then stores north-first (descending
  latitudes). This is the stored convention.
- Science grid: `write_grid(..., max_cells=7_000_000)` with default fill NaN.
  Stride rule: start at 1; while `(h//s)*(w//s) > max_cells`, `s *= 2`. For
  3500x7000 this gives s=2, so the stored grid is **1750 rows x 3500 columns**
  (6,125,000 cells), 0.02 deg spacing. [SRC, EST for the exact shape]
- Stored data: little-endian float32, row-major, NaN replaced by -9999.0.
  Meta fill = -9999.0. [SRC]

### 3.2 Grid files (format for all layers) [SRC]

Layout for a layer `L` and a key `K` (`K = "<ts>"` for the input layer, or
`K = "runs/<anchor>/<ts>"` for nowcast point grids):

```
$GRID_DIR/L/K.meta.json              commit pointer, replaced atomically
$GRID_DIR/L/K.<32-hex gen>.bin       immutable data, written and fsynced first
$GRID_DIR/L/.grid.lock               flock target for layer metadata and retention
```

`meta.json` (keys sorted, separators `,` and `:`, trailing newline):

```json
{"data_file":"2026-07-15T16:00:00+00:00.<gen>.bin","fill":-9999.0,
 "height":1750,"lat_max":54.99,"lat_min":20.01,"lon_max":-60.01,
 "lon_min":-129.99,"stride":2,"unit":"dBZ","width":3500}
```

Commit protocol: create a unique `gen` (uuid4 hex), write `.bin` with fsync,
fsync the directory, write the meta into a temp file in the same dir with fsync,
`os.replace` onto `K.meta.json`, fsync the directory. All of this is inside an
exclusive `flock` on `L/.grid.lock`.

Rust must use the same lock file name and the same flock semantics. Python
workers may still be running during a rollout.

### 3.3 Reader contract for grid files [SRC]

`_load_grid(meta_path)`:

- reads `height`, `width`, `fill` (default -9999.0), `data_file` (default
  `<name>.bin`), `lat_max`, `lat_min`, `lon_min`, `lon_max`;
- reads `(h, w)` float32 little-endian; values with `|v - fill| < 1e-3` become -9999.0;
- rebuilds axes: `lats = linspace(lat_max, lat_min, h)` (descending),
  `lons = linspace(lon_min, lon_max, w)` (ascending);
- on `OSError`, `KeyError` or `ValueError`, logs and returns None (the grid is skipped).

---

## 4. Input selection and validation

### 4.1 Listing [SRC]

`_list_recent_grids()` lists `*.meta.json` in `GRID_DIR/NOWCAST_GRID_INPUT_LAYER`
sorted by filename (ISO-8601 with `+00:00`, so lexicographic = chronological),
then calls `select_input_grids(metas)` with defaults `step_min = 5.0`,
`frames = 4`.

### 4.2 `select_input_grids` (exact) [SRC, tested]

```
timed = [(t(m), m) for m in metas if t(m) parses]    # t = fromisoformat(name minus .meta.json)
if timed is empty: return metas[-frames:]
chosen = [timed[-1]]                                 # newest
for k in 1 .. frames-1:
    target = timed[-1].t - k*step                    # newest - 5k min
    earlier = [c in timed if chosen[-1].t - c.t >= 0.6*step*60]   # >= 180 s older than last chosen
    if earlier is empty: break
    best = first minimum over earlier of |c.t - target|            # Python min(): first wins ties
    if |best.t - target| > 0.45*step*60: break                      # > 135 s away: give up
    chosen.append(best)
if len(chosen) < frames: return metas[-frames:]     # newest consecutive fallback
return reversed(chosen)                              # oldest first
```

Two subtleties: `earlier` is measured from the most recently chosen grid, not
from the newest; and any failure to find a mark falls back to the newest
consecutive grids (it never stretches the step).

Tests pin this: 5-min marks from `[0, 1.9, 4.1, 6.0, 8.2, 10.1, 12.0, 14.2, 16.1]`
select `[1.9, 6.0, 12.0, 16.1]` (minutes after 22:00). Consecutive
`[0,2,4,6,8]` return the last four. A missing mark returns the last four.

### 4.3 Validation, status outcomes, and early exits [SRC]

Steps in `_setup` (thread), in order. Each failure returns `ran=false`.

1. `mkdir` `TILE_DIR`, `STATE_DIR`. List inputs (4.2).
2. `len(metas) < 3` -> status `warming_up`, reason `insufficient_inputs`, detail `count=n,required=3`.
3. `latest` = newest stem. If `latest` is in `ProcessedSet(nowcast.json)` -> return `ran=false, anchor=latest` (no reads).
4. Load each selected grid (3.3). Parse each stem's time. Keep successes.
   `<3` valid -> `warming_up`, reason `insufficient_valid_inputs`.
5. Keep only grids with the newest grid's shape. `<3` -> `warming_up`, reason `inconsistent_input_shapes`.
6. Re-check `ProcessedSet` with the newest stem.
7. `intervals` = minutes between consecutive kept grids (2 or more).
   Fail if `intervals` is empty, `min <= 0`, or `max > max(6, 1.5*5) = 7.5` ->
   status `degraded`, reason `invalid_input_cadence`, detail `intervals_minutes=[...]`.
8. `input_interval_min = median(intervals)`. Return the inputs.

Typical cadence: 2-min MRMS, selection about 5 min, so intervals are about
4 to 6 min and the median is near 5.

### 4.4 Lead steps [SRC]

```
n_lead = HORIZON_MIN // STEP_MIN                     # 12
lead_steps[i] = ((i+1) * STEP_MIN) / input_interval_min   for i in 0..n_lead-1
valid_i = latest + (i+1)*STEP_MIN minutes             # wall-clock 5-min timeline
```

Example: `input_interval_min = 2.0` gives `[2.5, 5.0, ..., 30.0]`, the
`test_activities` fixture. For `5.0` it gives `[1, 2, ..., 12]`. For `4.0` it
gives `[1.25, 2.5, ...]`.

---

## 5. Preprocessing [SRC]

- `stack = np.stack(frames).astype(float32)` with shape `(T, H, W)`, `T` = 3 or 4.
- `stack = where(stack < -100, NaN, stack)`. Values at or below -100 are
  missing. This covers -9999 fill and MRMS -999 and -99 remnants. Values in
  `(-100, 5)` are kept as real dBZ.
- LK receives the whole `(T, H, W)` stack.
- S-PROG receives `stack[-3:]`.
- There is no dB transform, no clipping, and no Z-R conversion.

---

## 6. Motion estimation

### 6.1 Stride and upsampling [SRC, PS]

With `k = MOTION_STRIDE = 2`:

```
coarse_in  = stack[:, ::k, ::k]                # shape (T, h_c, w_c), h_c = ceil(H/k)
U_c        = LK_dense(coarse_in)               # shape (2, h_c, w_c), coarse px per input step
U_nan0     = nan_to_num(U_c[c])                # NaN -> 0
U_up[c]    = zoom(U_nan0, k, order=1)[:H, :W]  # scipy zoom, grid_mode=False (align-corners)
U_full[c]  = pad_edge(U_up[c] to (H, W)) * k   # 2 x H x W, full-res px per input step
```

scipy `zoom` with `grid_mode=False` maps output index `o` to input coordinate
`o * (n_in - 1) / (n_out - 1)`, then bilinear. Verified:
`zoom([0,10], 2, order=1) = [0, 3.333, 6.667, 10]`. For the production shape
(H even) this is `src(o) = o * (h_c - 1) / (H - 1)`, not `o/2`. Implement this
mapping exactly. [PS, verified]

Output: `(2, H, W)` float32. `U[0]` is x (column) velocity, `U[1]` is y (row)
velocity, in full-res cells per input step.

### 6.2 Dense LK, per consecutive pair [PS]

`dense_lucaskanade(input_images, ...)` loops over pairs `(n, n+1)`,
`n = 0..T-2`, and pools their sparse vectors. For each pair:

1. **Mask and fill.** Invalid (NaN) cells are masked. Fill value = min of the
   valid values. [PS]
2. **Morphological opening** (`size_opening = 3`). Let `thr = min(valid)`.
   Binary image `b = (img > thr)` (filled cells are 0). Structuring element:
   `MORPH_ELLIPSE` 3x3. `opened = open(b)` with OpenCV's default border
   handling (erosion does not erode from outside). `removed = (b - opened) > 0`.
   Set `img[removed] = nanmin(img)`. [PS for the logic, OCV for border/ellipse]
3. **Feature detection** (`shitomasi`): input is the opened prev image.
   - Invalid mask dilated with a 5x5 square (`buffer_mask = 5`, one iteration),
     and the dilated cells are excluded from features.
   - Scale to uint8: `u8 = trunc((img - min) / (max - min) * 255)`, where min/max
     are over valid cells. If `max - min <= 1e-8` use `img - min` (no scaling).
   - `cv2.goodFeaturesToTrack(u8, mask, maxCorners=1000, qualityLevel=0.01,
     minDistance=10, blockSize=5, useHarrisDetector=False, k=0.04)`. [OCV] Shi-Tomasi
     min-eigenvalue response over a 5x5 block (Sobel 3x3 gradients), threshold
     `0.01 * max response`, local maxima, sorted by response, greedy
     minimum-distance 10, cap 1000. The output points are `(x=col, y=row)` in
     the coarse grid. If none, skip the pair.
4. **Pyramidal LK** (`track_features`): both images are scaled to uint8 by their
   own valid min/max (same rule as step 3). Then
   `cv2.calcOpticalFlowPyrLK(prev, next, p0, winSize=(50,50), maxLevel=3,
   criteria=(COUNT|EPS, 10, 0), flags=0, minEigThreshold=1e-4)`. [OCV]
   Keep points with status 1. `xy = p0`, `uv = p1 - p0`. If none, skip the pair.
   OpenCV details (levels = 0..3, pyrDown, Scharr derivatives, Newton
   iterations, the eigenvalue criterion normalised by window area) are [OCV].
   Bit-exact parity is not required (DEC-5).

### 6.3 Sparse aggregation [PS]

Pool all `(xy, uv)` across pairs. If no vectors, return zeros `(2, h_c, w_c)`.

1. **Outlier removal** (`detect_outliers(uv, thr=3, coord=xy, k=30)`).
   For each sample `i`, take its `k=min(N, 31)` nearest neighbours by Euclidean
   distance in `xy` (including itself, excluding index 0 which is itself) ->
   effectively 30 neighbours. Let `m = mean(neighbours uv)` and
   `C = cov(neighbours uv, ddof=1)`. Outlier iff
   `sqrt((uv_i - m)^T C^{-1} (uv_i - m)) > 3`. If `C` is singular, the distance is 0
   (not an outlier). If `N < 2`, no outliers. Use a KD-tree.
2. **Declustering** (`decluster(xy, uv, scale=20, min_samples=1)`). Bin
   `b = floor(xy / 20)` (both axes, coarse px, so 40x40 full-res px). For each
   unique bin, output the component-wise median of `xy` and of `uv`. Order by
   bin (lexicographic), but order does not change the IDW result.
3. **IDW interpolation to every coarse cell** (`idwinterp2d`, `power=0.5`,
   `k=20`, `dist_offset=0.5`). For each grid cell `g`: take the
   `k' = min(20, n_points)` nearest declustered vectors (distance `d_j`, coarse px).
   Weight `w_j = 1 / (d_j + 0.5)^0.5` (`mean_res = 1` because the grid spacing is
   1 px). Normalise and take `uv(g) = sum_j w_j uv_j / sum_j w_j`. There is no
   radius limit, so the whole coarse grid is filled. [PS]
4. Output `U_c` of shape `(2, h_c, w_c)`, coarse px per input step.

The decorator `prepare_interpolator` validates and copies inputs. It raises on
non-finite values, which the pipeline does not produce. [PS]

### 6.4 Invariants and validation for the Rust LK [tests]

- Synthetic translation through the production wrapper (stride 2, zoom, times 2)
  recovers the true velocity. Measured: true `(2.0, 1.0)` gives median `(2.004, 0.999)`
  on echo cells; true `(-1.5, 0.5)` gives `(-1.499, 0.500)`. Require agreement
  within 0.02 px on synthetic translations.
- Output is finite everywhere and has shape `(2, H, W)`.
- Replay acceptance on archived runs (DEC-5): median per-cell |du|, |dv| below 0.05 px
  against Python, 95th percentile below 0.2 px, and skill (CSI at 30 min, rain)
  within 0.01 of the Python run [EST: the tolerances are proposals].

---

## 7. S-PROG (pysteps `nowcasts.sprog.forecast`) [PS unless noted]

Inputs: `P_raw` of shape `(3, H, W)` float32, NaN for missing; `V` of shape
`(2, H, W)` float32, finite; `timesteps` ascending list of floats. Fixed
parameters from the call: `precip_thr = 5.0`, `norain_thr = 0.0`,
`n_cascade_levels = 6`, `ar_order = 2`, `extrap = semilagrangian`,
`decomp = fft`, `bandpass = gaussian` (`gauss_scale = 0.5`, `d = 1.0`,
`normalize = True`, `include_mean = True`), `probmatching = cdf`, `domain = spatial`,
`conditional = False`, `fft = numpy` (float64 complex in production, numpy<2).

### 7.1 Domain mask and no-rain check

- `domain_mask = OR_t ~isfinite(P_raw[t])`, computed before any advection.
- `precip_min = nanmin(P_raw)` over all 3 frames, before advection.
- No-rain: `rain = count(P_raw > precip_thr)` over all 3 frames (strict `>`,
  NaN is false). If `rain / (3*H*W) <= norain_thr`, i.e. `rain == 0`, return
  for every timestep a constant field equal to `precip_min`. This is D2.
  The shape is `(len(timesteps), H, W)`. [PS, verified (4, 16, 24)]

### 7.2 Lagrangian pre-advection of the older frames

For `i = 0 .. ar_order-1` (frames 0 and 1):

```
P[i] <- semilagrangian_advect(P[i], V, n_steps = ar_order - i, outval = nanmin(P[i]))[-1]
```

Frame 0 is advected 2 steps, frame 1 is advected 1 step, and frame 2 (the
newest) is left as is. `outval` is the frame's own min over finite values, used
for samples outside the domain (not NaN). The semi-Lagrangian scheme is in 7.8.

### 7.3 NaN fill

For each `t`: `P[t][~isfinite] = nanmin(P[t])`, after the pre-advection. The
newest frame is filled with its own min. [PS]

### 7.4 Gaussian bandpass (exact constants) [PS]

Let `H, W` be the shape, `L = max(H, W)` (3500 in production), `n = 6`,
`s = 0.5`.

```
q        = (L/2)^(1/n)                              # production: 3.4714134
r_k      = 0.5 * (q^(k-1) + q^k),  k = 1..n         # central wavenumbers, index units
log_q(x) = ln(x)/ln(q),  with log_q(0) := 0
w_k(r)   = exp( -(log_q(r) - log_q(r_k))^2 / (2 s^2) )
```

2-D frequency grid, `rfft2` layout `(H, W/2+1)`: `kx_j = j` for `j = 0..W/2`,
`ky_i` = signed FFT index (`i` for `i <= H/2`, `i - H` above, the numpy
`fftfreq` convention for even `H`). Radius `r(i,j) = sqrt(kx_j^2 + ky_i^2)`.

```
W_k(i,j) = w_k(r) / sum_{k'} w_{k'}(r)          # normalised over levels at each frequency
W_0(0,0) = 1;  W_k(0,0) = 0 for k >= 1          # DC belongs to the lowest level
```

`sum_k W_k(i,j) = 1` everywhere. Production `r_k` for `L = 3500`:
`[2.235707, 7.761062, 26.941856, 93.526322, 324.668533, 1127.058711]`.
For `L = 32` (test shape): `[1.293701, 2.053622, 3.259921, 5.174802, 8.214486, 13.039684]`.

### 7.5 Decomposition and normalisation [PS]

For each frame `t` (after 7.2-7.3):

```
F      = rfft2(P[t])                       # no mean subtraction
field_k = irfft2(F * W_k)                  # real H x W, k = 0..5
mu_k    = mean(field_k)
sigma_k = std(field_k)                     # population std (ddof 0)
Z_k     = (field_k - mu_k) / sigma_k       # normalised level
```

Keep `(mu_k, sigma_k)` for the newest frame (`t = 2`); they are used for
recomposition in 7.7.

### 7.6 Temporal autocorrelation and AR(2) parameters [PS]

Per level `k`, with `x = Z_k` over `t = 0,1,2` (oldest to newest), using all
cells (no mask):

```
gamma1(k) = pearson(x[2], x[1])
gamma2(k) = pearson(x[2], x[0])
```

Lag-2 adjustment (AR order 2, applied per level):

```
gamma2' = max( gamma2,  2*gamma1^2 - 1,  (3*gamma1^2 - 2 + 2*(1 - gamma1^2)^1.5) / gamma1^2 )
```

If `gamma1 == 0`, the last term is `0/0 = NaN` in numpy float64, and `np.maximum`
propagates NaN. This is numpy semantics and the Rust code must reproduce it
(or document the difference). Pure Python floats raise instead. [PS, verified:
`adjust_lag2_corrcoef2(np.float64(0), 0.5) = nan`]

Yule-Walker AR(2). Solve
`[[1, g1], [g1, 1]] [phi1, phi2]^T = [g1, g2']^T`:

```
phi1 = g1 * (1 - g2') / (1 - g1^2)
phi2 = (g2' - g1^2) / (1 - g1^2)
phi_pert = sqrt(max(0, 1 - g1*phi1 - g2'*phi2))     # innovation coefficient, unused (no noise)
```

Stationarity: roots of `z^2 - phi1*z - phi2 = 0` must have `|z| < 1`, or pysteps
raises `RuntimeError` (caught as `pysteps_failed`). [PS]

Verified cases (g1, g2 in, g2' out -> phi1, phi2, phi_pert):

- (0.95, 0.85) -> 0.851400 -> (1.447895, -0.524100, 0.265930)
- (0.80, 0.50) -> 0.550000 -> (1.000000, -0.250000, 0.580948)
- (0.60, 0.30) -> 0.300000 -> (0.656250, -0.093750, 0.796477)

Keep the last `ar_order = 2` frames per level as AR state:
`state_k = [Z_k[1], Z_k[2]]` (oldest first).

### 7.7 Initial state and per-step update [PS]

Initial scalars:

- `precip_0 = P[2]` (filled, unadvected newest frame). This is the CDF target.
- `war = count(P[2] >= 5) / (H*W)`. NaN-filled cells count as dry, so `war` is
  diluted by missing area. The 5 dBZ comparison is `>=` here, but the no-rain
  check (7.1) uses `>`.
- `precip_min` (7.1).

Each integer step `n = 1, 2, ..., K` (computed in 7.8):

**(a) AR iteration** per level (deterministic, no innovation):

```
state_k <- [state_k[1], phi1 * state_k[1] + phi2 * state_k[0]]
```

**(b) Recomposition** (in the normalised domain, then denormalised with the
newest frame's stats):

```
R = sum_k ( state_k[newest] * sigma_k + mu_k )       # H x W, float64
```

**(c) Percentile mask** (`compute_percentile_mask(R, war)`). With
`s = sort(R)` ascending and `N = H*W`:

```
x_j = (N - j) / N,  j = 0..N-1
i   = first argmin_j |x_j - war|
if s[i] == s[i+1]:            # executed even when i = N-1  -> D1
    i = last index with s == s[i]
thr  = s[i]
mask = R >= thr
R[~mask] = precip_min
```

Because `x_{N-1} = 1/N`, `i = N-1` exactly when `war < 1.5/N`. Then `s[i+1]`
is out of range and pysteps raises (D1). For `N = 6` this is `war < 0.25`
(verified: 0.2 and 0.0 raise, 0.25 passes). Take `thr` from a selection
(`select_nth_unstable`) rather than a full sort, but keep the same index
arithmetic and the tie rule.

**(d) CDF matching** to `precip_0` (pysteps `nonparam_match_empirical_cdf`,
Python implementation `_match_empirical_cdf` is equivalent). Let:

```
z0 = min(R);   zt = nanmin(precip_0)
T  = precip_0 with NaN -> zt  (flattened)
k  = count(R > z0)                                  # wet cells in R
if count(T > zt) > k:
    p = percentile(T, 100*(1 - k/N))                # numpy linear interpolation
    T[T < p] = zt
values = top-k of T, sorted ascending               # v_1 <= ... <= v_k
out    = full(N, zt)
out[wet cells] = values, assigned by ascending rank of R among wet cells
```

Dry cells (`R == z0`) get `zt`. Ties within wet cells are unspecified (numpy
argsort), which is harmless because tied values are equal. The output multiset
of wet values equals the top-k of the newest frame, so the sum of values is
fixed by `k` alone. [PS, verified: the Appendix A sums are identical at leads 2.5 and 3.0]

**(e) Domain NaN.** `out[domain_mask] = NaN`.

The result for this step is `F_n = out` (H x W, float64).

### 7.8 Timestep interpolation and semi-Lagrangian advection [PS]

Let `t_j` be the requested leads (ascending, `t_j > 0`), `K = ceil(t_max) + 1`
integer AR steps `F_1 .. F_K` are computed (7.7). `F_0` is the Lagrangian
newest frame and is never output.

For each lead `t` with `k = floor(t)` and `f = t - k`:

```
ip(t) = F_k                        if f == 0
ip(t) = (1-f)*F_k + f*F_{k+1}      if f > 0       # NaN in either operand -> NaN
```

Advection is a cumulative backward displacement `D(x)` (2 components per
cell, `D = 0` at start), updated from the previous lead's time:

```
for each lead j with dt = t_j - t_{j-1}  (t_0 = 0):
    v   = V sampled at (x + D)            # bilinear; coordinates clamped to [0, n-1]
    v   = v * dt
    v   = V sampled at (x + D - v/2) * dt # midpoint predictor
    D   = D - v
    E_j(x) = ip(t_j) sampled at (x + D(x)) # bilinear; outside [0, n-1] -> NaN
```

Rules:

- Samples are bilinear. Coordinates are in `(row, col)`, while `V[0]` is x and `V[1]` is y.
- NaN propagation: if any of the (up to) 4 corner values is NaN, the result is NaN,
  even when that corner has zero weight. Verified with `map_coordinates`:
  a zero-weight NaN corner yields NaN. [PS, verified]
- Out of range (for the warp): NaN (`cval = NaN`). The velocity uses `mode =
  nearest`, i.e. clamped coordinates.
- Integer leads (`f == 0`) use `F_k` without blending.
- `D` is cumulative across leads (per member, per cell). It is not reset per lead.

### 7.9 Output convention [SRC]

`forecast` is `(12, H, W)` float64. `_run_nowcast` converts to float32, sets NaN to -9999,
and returns method `pysteps-sprog`. Then `status = ok` is written (D4).

### 7.10 Degenerate paths

- No-rain (7.1, D2): constant `precip_min` for every lead. After 8.1 every lead
  is empty, so the run fails at commit (D2).
- `war < 1.5/N` (7.7c, D1): `IndexError`, caught as `pysteps_failed` (degraded, no publish).
- Stationarity failure (7.6): `RuntimeError`, same handling.
- `gamma1 == 0` (7.6): NaN phi, which makes the field NaN and then all -9999 for that
  level's contribution.

---

## 8. Postprocessing and rendering

### 8.1 Value conventions [SRC]

- Forecast NaN -> -9999.0.
- Per lead: `frame = where(forecast < 5, -9999.0, forecast)` (dry is any value below 5 dBZ).
- Values at or above 5 dBZ are unchanged (float32).

### 8.2 Point grids [SRC]

Per lead: `write_grid("nowcast", ts, frame, lats, lons, "dBZ", -9999.0,
max_cells=900000, grid_key="runs/<anchor>/<ts>")` (section 3.2 format).

- Stride from 1750 x 3500: `h//s * w//s` goes 6.1 M, 1.5 M, then 382 k at
  `s = 4`. The slice `data[::4, ::4]` gives **438 x 875** (ceil of 1750/4 and 3500/4).
  Each point grid is 1,533,000 bytes. [SRC for the rule, EST for the exact shape]
  Note: the stride decision uses floor division, the slice uses ceil. The
  shapes stay consistent because the stored lat/lon arrays use the same slice.
- Stored `lat`/`lon` are the sampled coordinates. Meta `lat_max` is the first
  sampled latitude and `lat_min` the last sampled one (not the full science extent).
  Uniform spacing is kept, so bilinear lookup from meta is exact.
- Observed anchor: `write_grid("nowcast", latest_iso, grids[-1], ...,
  fill=-9999.0, grid_key="runs/<anchor>/observed")` with the raw science
  grid (no 5 dBZ threshold), so dry cells keep their real dBZ values (not -9999)
  and only no-coverage cells are fill.

### 8.3 Palettes and class model [SRC]

Palette file `backend/shared/palettes/<name>.json`, key `reflectivity`:
`{"unit": ..., "ranges": [{"min", "max", "rgba": [r,g,b,a]}, ...] (13 entries,
min 5 to max 75 in 5-dBZ bins), "no_data_below": 5}`. All three palettes
share edges. Values at or above 75 are transparent.

`build_class_model(tables)`:

- `edges` = sorted union of every `min`, `max` and `no_data_below` (strictly increasing).
- Raw index for a value `v`: `searchsorted(edges, v, side='right')`, in
  `0 .. len(edges)`. NaN and values at or above the last edge map to the last index
  (transparent).
- For each palette and each interval `[edges[i-1], edges[i])` with `i` in `1..len(edges)-1`:
  if `edges[i-1] < no_data_below`, transparent. Otherwise the colour is the **last**
  range (in list order) whose `[min, max]` contains the interval.
- Classes that are transparent in every palette are collapsed to class 0. Visible
  classes are numbered `1..`, with at most 255 (uint8).

`classify(v)` returns a uint8 class index with NaN or inf as class 0.

### 8.4 Tile geometry (Web Mercator XYZ, 256 px) [SRC]

- Tile index: `x = floor((lon+180)/360 * 2^z)`; `y = floor((1 - ln(tan(lat) + sec(lat))/pi)/2 * 2^z)`,
  both clamped to `[0, 2^z - 1]`.
- Pixel centres: `lon_p = (tx + (i+0.5)/256)/n*360 - 180`;
  `lat_p = degrees(atan(sinh(pi*(1 - 2*(ty + (j+0.5)/256)/n))))`.
- Source fractional index on a regular axis: `frac = (v - axis[0]) / (axis[-1] - axis[0]) * (len-1)`
  (works for descending latitudes, which are north-first). The index is in pixel units.
- Tile `(z, tx, ty)` is produced only if its bounding box intersects the grid.

### 8.5 Overview pooling for coarse zooms [SRC]

`block(z) = (max(1, int(px_deg * cos(min(89, max(|lat_min|,|lat_max|))) / dlat)),
             max(1, int(px_deg / dlon)))` with `px_deg = 360 / (256 * 2^z)`.

For the production grid (`dlat = dlon = 0.02`, `lat` up to 55 deg):

- z4: `block = (2, 4)` -> pooled. z5: `(1, 2)` -> pooled. z6: `(1, 1)` -> native.

Pooled zooms use **footprint max** (`render_footprint_max_tiles`): each tile
pixel's footprint is the set of source cells whose edges fall in the pixel (via
`_footprint_edges`, rounding `floor(x + 0.5)`, one cell minimum). Take the
nan-aware max over the footprint of the dBZ values (-9999 is treated as NaN),
then classify. A footprint with no valid cell is NaN (transparent). The max is
separable (columns then rows).

### 8.6 Native zoom rendering [SRC]

Two algorithms. Choose by `TILE_RENDERER` (8.6.2 vs 8.6.1).

**8.6.1 Legacy (default, `rgba-bilinear-v1`).**

1. Colourise first: `rgba = apply_color_table(frame)` (nearest class per cell,
   transparent for -9999 and dry).
2. For each tile: `rows, cols` = fractional indices of pixel centres. If the
   bounding box of the source region is all transparent, skip.
3. Per channel (R, G, B, A): `map_coordinates(rgba[..., c], [rows, cols], order=1,
   mode='constant', cval=0)`. Bilinear **RGBA** interpolation, so colours blend
   and alpha is soft at edges.
4. If the tile's alpha is all zero, skip. Otherwise save RGBA PNG, compress level 1.

**8.6.2 Indexed (`TILE_RENDERER=indexed`, `TILE_RENDERER_CANARY_ROLE=nowcast`).**

1. Sample the physical value once per tile pixel by bilinear interpolation of
   dBZ with `nodata = -9999` and `min_valid_weight = 1.0`:
   a pixel is valid only if **every corner with nonzero weight** is finite and
   not -9999. Otherwise it is NaN. So rain edges erode by one source cell.
2. Classify the sampled values (8.3). Palette indices are never interpolated.
3. Encode one class-index PNG (mode P, PLTE and tRNS per palette, compress
   level 6). Other palettes are derived by replacing PLTE and tRNS only.
4. Skip the tile if all classes are 0.

### 8.7 Publication [SRC]

- Per `(palette, lead)`: render into a staging directory, then write the
  completion marker `.render-complete-v2.json` (schema 2, `tile_count`,
  `tile_path_digest`, `tile_content_digest`, `identity`) in the staging root. Publish with
  `os.rename(staging, final)` under a publication lock (`TILE_DIR/nowcast`).
  If `final` already has a valid marker with the same identity, reuse it. An empty
  pyramid is never published.
- Tile path (pyramid root): `TILE_DIR/nowcast/<palette>/runs/<anchor>/<valid>/`
  with `{z}/{x}/{y}.png` below it.
- Only the newest run is referenced by the manifest. Old tile dirs linger until
  cleanup.

### 8.8 Lead acceptance and the D2 failure [SRC]

- A lead is accepted only if `set(rendered_palettes) == expected_palettes`, where
  `rendered_palettes` = palettes with `tile_count > 0`, and `expected_palettes` =
  palettes with a `reflectivity` table. Otherwise its tile dirs are deleted and the
  lead is dropped.
- `_commit` requires all 12 leads: `RuntimeError("nowcast incomplete: rendered n/12 leadtimes")`.
  So a single dropped lead fails the run (D2 in its empty form).

---

## 9. Output layout, manifest, and state

### 9.1 Directory tree [SRC]

```
$GRID_DIR/
  radar-nowcast-input/
    <ts>.meta.json  <ts>.<gen>.bin  .grid.lock         # newest 12 kept
  nowcast/
    .grid.lock
    runs/<anchor>/
      <valid_i>.meta.json  <valid_i>.<gen>.bin          # 12 leads (438x875 f32)
      observed.meta.json   observed.<gen>.bin           # anchor observation
      .grid-generation-complete-v1.json                 # written at finalize
$TILE_DIR/nowcast/<palette>/runs/<anchor>/<valid_i>/<z>/<x>/<y>.png
$TILE_DIR/nowcast/<palette>/runs/<anchor>/<valid_i>/.render-complete-v2.json
$TILE_DIR/nowcast/.publication locks
$STATE_DIR/
  manifest.json  manifest.lock                          # shared with the other layers
  nowcast.json                                          # ProcessedSet of anchors
  nowcast-status.json
  nowcast-skill.json
```

Generation marker (`.grid-generation-complete-v1.json`): keys `schema_version: 1`,
`generation: "<anchor>"`, `grid_count` (number of meta files), `grid_metadata`
(sorted meta filenames). Written by `finalize_grid_generation` after all 13
metas exist.

### 9.2 Manifest (`manifest.json`, schema 2) [SRC]

Top level: `schema_version: 2`, `layers`, `tile_url_template:
"/tiles/{layer}/{palette}/{path}/{z}/{x}/{y}.png"`, `tile_url_template_legacy`,
`updated_at` (isoformat with `+00:00`). Files are written with sorted keys,
separators `,` and `:`, and a trailing newline.

`layers.nowcast` (from `replace_layer_manifest`):

```json
{
  "timestamps": ["2026-07-15T16:05:00+00:00", "...", "2026-07-15T17:00:00+00:00"],
  "frames": [{
      "timestamp": "2026-07-15T16:05:00+00:00",
      "path": "runs/2026-07-15T16:00:00+00:00/2026-07-15T16:05:00+00:00",
      "palettes": ["classic","muted","vivid"],
      "grid_key": "runs/2026-07-15T16:00:00+00:00/2026-07-15T16:05:00+00:00",
      "source": "mrms-nowcast", "kind": "nowcast",
      "issued_at": "2026-07-15T16:00:00+00:00",
      "lead_minutes": 5, "input_interval_minutes": 5.0,
      "method": "pysteps-sprog", "spatial_resolution_km": 2.22, "max_zoom": 6}],
  "latest": "2026-07-15T17:00:00+00:00",
  "palettes": ["classic","muted","vivid"],
  "title": "MRMS motion nowcast", "kind": "nowcast",
  "horizon_minutes": 60, "step_minutes": 5, "method": "pysteps-sprog",
  "run_id": "2026-07-15T16:00:00+00:00"
}
```

Replace semantics (`replace_layer_manifest`):

- Under `flock(manifest.lock)`, read the manifest (missing -> empty), then set
  `layers.nowcast` to the new run **entirely**. Frames are sorted by timestamp and
  deduplicated. Each frame gets the run-level palette set.
- `timestamps` empty -> the layer is removed.
- `latest` = last timestamp; `palettes` = intersection over frames.
- Write atomically (temp file in the same dir, then `os.replace`).
- Old frames from previous runs are dropped, so the app never mixes vintages.

`spatial_resolution_km = round(|lat_max - lat_min| / (H_sci - 1) * 111, 2)` = **2.22**
for the science grid (0.02 deg). [SRC, test asserts 2.22]

### 9.3 Status file `nowcast-status.json` [SRC]

```json
{"detail":"count=2,required=3","reason":"insufficient_inputs","status":"warming_up","updated_at":"2026-10-13T22:39:01.123456+00:00"}
```

- Written with sorted keys, atomic. `status` in `{ok, warming_up, degraded}`.
- Reasons: `insufficient_inputs`, `insufficient_valid_inputs`, `inconsistent_input_shapes`
  (warming_up); `invalid_input_cadence`, `pysteps_unavailable`, `pysteps_failed`,
  `persistence` (degraded).
- `ok` is written before commit (D4).

### 9.4 Processed set `nowcast.json` [SRC]

`ProcessedSet(path, max_entries=100)`: JSON `{"items": [anchor, ...]}`, oldest
dropped beyond 100, atomic write. An anchor is added only in `_commit`, after the
manifest swap, so a crash before that leaves the old manifest readable and the
anchor is retried by the next fire.

### 9.5 Commit order and crash semantics [SRC]

Order inside `_commit` (thread):

1. Require 12 rendered leads (8.8).
2. Store the observed anchor grid (failure is logged, not fatal).
3. `finalize_grid_generation("nowcast", anchor)` (marker).
4. `replace_layer_manifest(...)` (publication point).
5. `prune_grid_generations("nowcast", keep=36, active_generation=anchor)`.
6. `ProcessedSet.add(anchor)`.

Then the skill scoring (9.5/10) runs. Its errors are swallowed.

Crash analysis: a crash before 4 leaves the old manifest and no marker, so the
run is retried. A crash between 4 and 6 leaves the new manifest and no
`ProcessedSet` entry, so the next fire may rerun the same anchor
(harmless: same output, replaced atomically).

### 9.6 Retention [SRC]

- Point-grid runs: `prune_grid_generations("nowcast", keep=36,
  active_generation=anchor)`. Remove complete runs (those with a marker) beyond the
  newest 36 by name, but always keep the active run. Incomplete run directories
  (no marker) are never touched. Removal is per whole run, not per frame.
- Input layer: `prune_grid_layer("radar-nowcast-input")` keeps the newest
  `GRID_KEEP_LAST = (4-1)*ceil(5/1.8) + 3 = 12` metas and the bins they reference.
  Unreferenced `.bin` files are removed only after a 300 s grace period.
- Age sweep: `cleanup_old_grids` also removes files older than 12 h.

---

## 10. Verification (skill) [SRC]

### 10.1 Trigger and inputs

After commit, for the new observation `observed = grids[-1]` (science grid at
`latest_iso`), call `score_observation(observed, latest_iso, lats, lons,
grid_dir, state_dir, point_grid_max_cells=900000)`. Errors are logged, not fatal.

### 10.2 Matching rule (exact)

1. `obs_dt = parse(latest_iso)`. Downsample `observed` with the point-grid stride
   rule (same as 8.2; 1750x3500 -> 438x875).
2. `obs_meta` = bounds of the full science axes (used only if shapes differ).
3. For each run directory `runs/<anchor>` (sorted by name):
   - Skip if `anchor_dt >= obs_dt` (strict).
   - For each `*.meta.json` (sorted) except `observed`:
     - `valid_dt` from the stem. Skip unless `|valid_dt - obs_dt| <= 150 s`.
     - Load the forecast point grid. Load failure -> stop this run.
     - If shapes differ, `resample_nearest` onto the observed grid.
     - Entry: `anchor`, `valid = latest_iso` (observed time), `lead_minutes = round((valid_dt - anchor_dt)/60)`,
       `scored_at = now (isoformat)`.
     - For `rain` (20 dBZ) and `heavy` (35 dBZ): `contingency(forecast, observed, threshold)`.
       `cells = known count` (same for both thresholds).
     - `persistence`: contingency of the run's `observed` point grid (anchor observation)
       against the new observation, per threshold (no `cells` key). Missing -> null.
     - Append, then `break` (at most one match per run per observation).
4. If any entries, `append_entries` to the log (10.4).

D3 is a consequence of step 3: a forecast valid time can match 2 or 3 observations.

### 10.3 Contingency and scores

```
known     = isfinite(obs) & (obs > -100)            # unknown: fill and outside coverage
obs_event = known & (obs >= T)
fcst_event= known & isfinite(fcst) & (fcst >= T)    # fill -9999 is never an event
hits   = count(fcst_event & obs_event)
misses = count(obs_event & ~fcst_event)
false_alarms = count(fcst_event & ~obs_event)
cells  = count(known)
```

Score formulas (`scores_from_counts`), each rounded with Python `round(x, 3)`
(half-to-even on the exact binary value; `round(0.0625, 3) = 0.062`):

```
POD  = hits / (hits + misses)                 # null if denominator 0
FAR  = false_alarms / (hits + false_alarms)
CSI  = hits / (hits + misses + false_alarms)
BIAS = (hits + false_alarms) / (hits + misses)
```

Thresholds: `rain = 20.0`, `heavy = 35.0`.

### 10.4 Log file `nowcast-skill.json` [SRC]

```json
{"entries":[{"anchor":"2026-09-30T12:00:00+00:00","cells":438,
             "lead_minutes":30,"persistence":{"heavy":{"false_alarms":0,"hits":0,"misses":0},
             "rain":{"false_alarms":4,"hits":4,"misses":4}},
             "scored_at":"2026-09-30T12:30:00.123456+00:00",
             "thresholds":{"heavy":{"false_alarms":0,"hits":0,"misses":0},
             "rain":{"false_alarms":1,"hits":2,"misses":3}},
             "valid":"2026-09-30T12:30:00+00:00"}],
 "schema_version":1,"thresholds_dbz":{"heavy":35.0,"rain":20.0},
 "updated_at":"2026-09-30T12:30:00.123456+00:00","window_hours":24}
```

- `append_entries`: load (missing or unreadable -> empty), add new entries, drop
  entries whose `scored_at` is more than `window_hours` before `now`, sort by
  `(scored_at, lead_minutes)`, write atomically (sorted keys, separators `,` `:`).
- Unreadable JSON, non-dict, or non-list `entries` -> treated as empty.

### 10.5 Summary (`summarize`) [SRC]

1. Keep entries with `scored_at` within the window of `now`.
2. Group by `lead_minutes`. For each lead: `runs = number of unique anchors`, and sum
   `hits`, `misses`, `false_alarms` per threshold, and the same for `persistence`.
3. Per lead and threshold: scores from counts, plus
   `persistence_csi = scores(persistence).csi` (null if persistence hits+misses == 0).
4. Output keys: `available` (false until a headline), `window_hours`, `thresholds_dbz`,
   `runs_scored` (unique anchors overall), `updated_at`, `by_lead`, `headline`, and
   `reason` only when not available.
5. Headline: the lead row closest to 30 min (ties to the smaller lead).
   - If `runs < 6`: `reason = warming_up`.
   - If `rain.pod` is null: `reason = no_rain_observed`.
   - Else `available = true`, `headline = {lead_minutes, window_hours, runs, pod, far, csi, persistence_csi}` (rain threshold).
6. No leads: `reason = no_scored_runs`.

### 10.6 Prometheus [SRC]

```
# TYPE radar_ng_nowcast_skill_runs gauge
radar_ng_nowcast_skill_runs <runs_scored>
# TYPE radar_ng_nowcast_pod gauge
radar_ng_nowcast_pod{lead_minutes="15",threshold="rain"} 0.833
...                       (same pattern for far and csi; both thresholds; skip nulls)
```

### 10.7 JSON Schema (draft 2020-12, summary)

```json
{
  "$schema": "https://json-schema.org/draft/2020-12/schema",
  "type": "object",
  "required": ["available","window_hours","thresholds_dbz","runs_scored","updated_at","by_lead","headline"],
  "properties": {
    "available": {"type":"boolean"},
    "window_hours": {"type":"integer"},
    "thresholds_dbz": {"type":"object","required":["rain","heavy"]},
    "runs_scored": {"type":"integer","minimum":0},
    "updated_at": {"type":["string","null"]},
    "reason": {"enum":["no_scored_runs","warming_up","no_rain_observed"]},
    "by_lead": {"type":"array","items":{"type":"object","required":["lead_minutes","runs","rain","heavy"],
       "properties":{"lead_minutes":{"type":"integer"},"runs":{"type":"integer"},
       "rain":{"$ref":"#/$defs/scores"},"heavy":{"$ref":"#/$defs/scores"}}}},
    "headline": {"oneOf":[{"type":"null"},{"type":"object","required":["lead_minutes","window_hours","runs","pod","far","csi","persistence_csi"]}]}
  },
  "$defs": {
    "scores": {"type":"object","required":["hits","misses","false_alarms","pod","far","csi","bias"],
      "properties":{"hits":{"type":"integer"},"misses":{"type":"integer"},"false_alarms":{"type":"integer"},
        "pod":{"type":["number","null"]},"far":{"type":["number","null"]},"csi":{"type":["number","null"]},
        "bias":{"type":["number","null"]},"persistence_csi":{"type":["number","null"]}}}
  }
}
```

---

## 11. Reader-side API contract (compatibility surface)

The app depends on these shapes. Keep them byte-compatible.

### 11.1 `GET /api/nowcast/{lat}/{lon}` [SRC]

- Range check: `-90 <= lat <= 90`, `-180 <= lon <= 180`, else HTTP 422.
- Without a nowcast layer or frames:
  `{"status":"unavailable","reason":<status.reason or "no_frames">,"detail":<status.detail>,"points":[]}`.
- Grid point: `round(lat, 3)`, `round(lon, 3)`. For each frame in the layer: sample
  the point grid at `grid_key` (or the legacy timestamp-only path). Points:
  `{"timestamp", "lead_minutes", "dbz", "precipitation_mm_h"}`.
- `precipitation_mm_h = 0 if dbz < 5 else min(300, max(0, (10^(dbz/10)/200)^(1/1.6)))`.
- No points: `{"status":"unavailable","reason":"out_of_bounds" | "grids_warming_up","issued_at","points":[]}`.
- Success: `{"status":"ok"|"degraded" (degraded if any frame missing),"source":"mrms-nowcast",
  "method","issued_at","horizon_minutes","step_minutes","spatial_resolution_km",
  "latitude","longitude","points":[...],"skill": <headline or null>}`.
- Cache: keyed by `(run_id, lat3, lon3)`, bounded LRU, 30 s `Cache-Control` max-age.

### 11.2 Bilinear sampler `_sample_grid_point` [SRC]

- Bounds from meta: `lat_min <= lat <= lat_max` and `lon_min <= lon <= lon_max`, else `out_of_bounds`.
- `fx = (lon - lon_min)/(lon_max - lon_min)*(w-1)`, `fy = (lat_max - lat)/(lat_max - lat_min)*(h-1)`.
  `x0 = clamp(floor(fx), 0, w-2)`, `y0 = clamp(floor(fy), 0, h-2)`, `dx = fx - x0`, `dy = fy - y0`.
- Read four corners. A value is fill if NaN or `|v - fill| < 1e-6`.
- Quirk (D6): `v0 = v00*(1-dx)+v10*dx` if neither of v00, v10 is fill, otherwise `valid[0]`
  (first valid corner in order v00, v10, v01, v11). `v1` analogously with `valid[-1]`.
  `value = v0*(1-dy) + v1*dy`. No valid corner -> `value = null, reason = no_data`.
- Parity: reproduce this. A fix is a product decision.

### 11.3 `GET /api/nowcast/skill` [SRC]

Returns the summary of 10.5/10.7. Cached for the manifest TTL.

---

## 12. Performance budget

- Python baseline (workflow comment): about 9 min per run. Timeouts 10 min
  start-to-close, 12 min schedule-to-close, heartbeat 120 s. [SRC]
- Target: canary exit criterion is "nowcast under 90 s" (`docs/reliability-and-scale-plan.md:462`). [SRC]
- Reads per run: 4 input grids x 24.5 MB = about 98 MB. [EST: 1750x3500x4 bytes]
- Writes per run: 13 point grids x 1.53 MB = about 20 MB, plus observed and metadata. [EST]
- Retention footprint: point grids for 36 runs = about 720 MB. Input layer: 12 x 24.5 MB = about 294 MB. [EST]
- Tile output: bounding-box tiles per pyramid at z4, z5, z6: 12 + 35 + 126 = 173 (upper bound
  over the CONUS box). 36 pyramids (3 palettes x 12 leads) gives at most about 6,230 PNG
  candidates per run, fewer after transparent skip. [EST, computed with the tiler's tile math]
- Memory: S-PROG in float64 keeps 3 frames x 6 levels x 49 MB for decomposition and
  2 frames x 6 levels x 49 MB for AR state, so about 1 GB plus LK and tile buffers. Budget
  about 2 GB peak in Rust with f64 spectral work and f32 storage. [EST]
- Concurrency: `TEMPORAL_MAX_CONCURRENT_ACTIVITIES` for the nowcast pool is 1
  (`docs/tuning.md`). The 4 render workers are threads over leads. [SRC]
- Rust estimate for the same work: under 15 s single host with rayon over rows,
  levels and leads [EST]. This is the basis for the 90 s target.

---

## 13. Invariants checklist (from tests and derived)

Each item is a test the Rust port should carry, with the reference fixture in
brackets.

Selection and cadence:
1. Five-minute marks chosen from the 9-element fixture (4.2). [test_activities]
2. Short history returns the newest 4 consecutive. [test_activities]
3. A missing mark returns the newest 4, never stretches. [test_activities]
4. Cadence violation -> `degraded/invalid_input_cadence`, `ran=false`. [4.3]
5. Already-forecast anchor returns before any grid is read. [test_activities]

Motion:
6. LK runs on the full stack; the stride-2 wrapper returns `(2, H, W)` with x2 scaling
   (1.5 -> 3.0, -0.5 -> -1.0 on a constant field). [test_activities]
7. Synthetic translation recovered within 0.02 px (6.4). [lk_check, Appendix A]

S-PROG:
8. Fractional timesteps reach S-PROG unchanged (`[2.5, 5.0, 7.5]`). [test_activities]
9. Output is `(len(timesteps), H, W)`; method `pysteps-sprog`. [test_activities]
10. CDF matching equals the reference nonparam matching for wet fractions 0, 0.05, 0.4, 1.0
    with NaN in the target. [test_fast_cdf_match_equals_pysteps_reference]
11. Percentile mask: with `war=0.5` on `[[1,5,3],[2,4,3]]` -> `[0,1,1,0,1,1]`; threshold rule (7.7c). [Appendix A]
12. Lag-2 adjustment and YW match Appendix A values (7.6).
13. Non-finite velocity -> error (pysteps `ValueError`); precip_thr required.

Pipeline and grids:
14. Run-scoped keys: 12 leads plus `runs/<anchor>/observed`, observed last. [test_activities]
15. Event order: finalize -> manifest publish -> prune (keep 36, active anchor). [test_activities]
16. Manifest frames in lead order; `lead_minutes = 5..60`. [test_activities]
17. Pysteps unavailable -> `unavailable`, `ran=false`, no state change. [test_activities]
18. Tile path distinct per anchor, ends with `/<valid>`. [test_activities]
19. Retention removes whole runs (25 = 12 metas + 12 bins + marker in the fixture),
    keeps the active run and the newest. [test_activities]
20. Point-grid write stride and shapes: 8x8 with max 20 -> 4x4; max 3 -> 1x1. [test_skill]

Verification:
21. Contingency: unknown cells excluded; fill is dry. [test_skill: hits 2, misses 2, false_alarms 1, cells 6]
22. Downsample rule matches the writer. [test_skill]
23. Resample nearest on coarser grids: `out[0,0]=0`, `out[1,1]=15` for the 4x4 -> 2x2 fixture. [test_skill]
24. Score matches valid times within 150 s, skips runs starting after the observation,
    logs persistence; 35 min frame not matched. [test_skill]
25. Window drops old entries; atomic write; directory contains only the log file. [shared test]
26. Summary: `warming_up` below 6 runs; headline at 30 min; POD/FAR/CSI/persistence_csi as
    in the fixture (pod 80/96, far 24/104, persistence_csi 48/144 round 3). [shared test]
27. Prometheus lines and label format. [shared test]
28. API: warming grids body exactly `{"status","reason":"grids_warming_up","issued_at","points":[]}`. [test_nowcast_point]
29. API: 2.22 km spatial resolution; multi-run manifest keeps old run readable until swap. [test_nowcast_point]

---

## 14. Rust design notes

### 14.1 Module layout

```
nowcast/
  config.rs      env parsing with the defaults in 2.3
  grids.rs       meta/bin read, atomic commit, flock on .grid.lock, prune
  select.rs      select_input_grids, cadence, lead steps
  motion/        lk.rs (pyramid LK), shitomasi.rs, sparse.rs (outlier, decluster, idw), upsample.rs
  sprog/         gauss.rs (weights), decomp.rs (rfft2/irfft2), ar.rs (acf, lag2, yw),
                 pmatch.rs (percentile mask, CDF match), advect.rs (semi-Lagrangian + bilinear), run.rs
  render/        classes.rs, geometry.rs, legacy.rs, indexed.rs, footprint.rs, publish.rs
  manifest.rs    manifest read/replace, sorted-key JSON, atomic write
  status.rs      nowcast-status.json
  processed.rs   ProcessedSet (max 100)
  skill.rs       scoring, log, summary, prometheus
  api_types.rs   serde types for the reader contract (section 11)
  activity.rs    temporalio-sdk wiring: nowcast_run, heartbeats, result type
```

### 14.2 Crates (versions not pinned here; check at implementation)

- **temporalio-sdk 1.1.0**: already vendored at `/home/claude/rust-port/vendor-src/temporalio-sdk-1.1.0`
  (with client, common, macros, workflow). Verify the heartbeat and timeout API names
  against it. [inferred from the directory, not read]
- **ndarray**: 2D and 3D arrays, views, slicing that matches numpy `[::k, ::k]`.
- **realfft** (over **rustfft**): real-to-complex FFT. Implement `rfft2`/`irfft2` as
  1-D real FFT along the row, then complex FFT along the column, with layout `(H, W/2+1)`.
  Frequency signs follow the FFT convention (7.4).
- **rayon**: data-parallel loops over levels, leads, rows and tiles. The Python
  equivalent is `ThreadPoolExecutor(4)` over leads.
- **png**: indexed (PLTE/tRNS) and RGBA encoders with level control. Byte-identical
  output is not required, only decoded pixels.
- **serde / serde_json**: BTreeMap for sorted keys. `serde_json` is in the local
  registry cache. Float formatting is shortest round-trip, which matches Python's `repr`.
- **chrono**: `DateTime<Utc>` with `to_rfc3339_opts(SecondsFormat::AutoSi, false)`
  to match `isoformat()` (microseconds only when non-zero, `+00:00` not `Z`).
  `chrono` is in the local registry cache.
- **rustix** (flock): same lock file names as Python (`.grid.lock`, `manifest.lock`).
  `rustix` is in the local registry cache.
- **kd-tree** (for IDW and outlier kNN): `kiddo` or a small in-house KD tree.
  Use k-nearest with ties unspecified.
- **approx** (optional) for golden tests with tolerances.

### 14.3 Numeric policy

- Stored frames, point grids and tiles: f32 (matches on-disk format).
- S-PROG spectral and statistical work: f64 (numpy float64 in production). Store AR
  state as f32 if memory needs it (DEC-6), and then re-measure the Appendix A values.
- Percentiles: numpy `linear` interpolation, index `100*(1 - k/N)` as `p = q*(N-1)` with
  linear interpolation between sorted order statistics.
- `round(x, 3)` in the skill JSON: implement half-to-even on the exact binary value.
- NaN semantics: `max`, comparisons and `np.maximum` propagate NaN as numpy does. Use
  explicit helpers, not `f64::max` (which ignores NaN).
- Bilinear sampling (7.8, 8.6): compute all corners, poison on any NaN corner,
  clamp per mode.

### 14.4 Concurrency and time

- One activity at a time per worker (matches the Python pool). Inside the activity,
  rayon for CPU work. Keep heartbeats every 30 s from an async task, not from rayon threads.
- Scoring should run after the activity returns (DEC-8), or the activity must send
  heartbeats during it. Keep under the 120 s heartbeat budget.

### 14.5 Simplification path (if exact parity costs too much)

Ordered by value over risk. Each step has its own gate.

1. **Exact, no simplification:** selection, cadence, grid I/O and lock protocol,
   point-grid stride, manifest, status, ProcessedSet, skill math and log, API
   types. These are deterministic. Gate: Section 13 items 1-5, 14-29 pass against
   fixtures.
2. **Exact math, cheaper implementation:** S-PROG with selection (`select_nth`) in
   place of full sorts, FFT once per frame, shared weights. Gate: Appendix A values
   within 1e-6 relative (f64), item 11.
3. **Approximate motion:** own pyramidal LK and Shi-Tomasi (6.2), matching pysteps'
   structure but not OpenCV's exact numerics. Gate: 6.4 synthetic test and replay
   acceptance. Fallback if the gate fails: the `opencv` crate (FFI, exact).
4. **Renderer:** port `legacy` first (default). Port `indexed` behind the same flag.
   Gate: decoded RGBA equals Python on the golden tiles for legacy; decoded
   class indices equal for indexed.
5. **Defects:** keep D1 and D2 as parity (DEC-1, DEC-2) until the product owner
   decides. Changing them alters what the app shows during dry or dissipating periods.

### 14.6 Risks

- **Motion sensitivity.** Small LK differences shift every lead. Mitigation: replay
  acceptance (6.4) with CSI tolerance, not pixel equality.
- **Align-corners upsampling.** Using `o/2` instead of `o*(h_c-1)/(H-1)` shifts the
  velocity by up to half a coarse cell at the edges. Test with 6.1.
- **Edge erosion vs blending.** The legacy (soft alpha) and indexed (eroded) renderers
  look different at rain edges. The app's expectations are set by legacy.
- **Dry and dissipating windows.** D1 and D2 are silent for the user: no publication,
  or a failed activity while the status says `ok`. Surface them in the status file if
  parity is kept.
- **Double scoring (D3)** inflates absolute counts. Ratios are roughly unchanged,
  but the sample weights differ. Any change must be versioned with the log schema.
- **Heartbeat gap.** `_setup` (about 100 MB read) and post-commit scoring have no
  heartbeats. Mitigation in DEC-8.
- **Single-writer assumption (D5).** Keep the schedule overlap rules so only one run
  writes the skill log at a time.
- **Docs drift** (1.3) can mislead operators. Fix the docs when the port lands.
- **Upgrade ordering.** Deploy the API reader before any writer that emits
  `grid_key`, and roll back the writer before the reader (`docs/reliability-and-scale-plan.md`).
  The Rust writer must keep the same `grid_key` layout.

---

## Appendix A. Golden values

Generated with the local pysteps 1.21.5 checkout, run in an isolated Python
(`python3 -I`) with an in-process `jsmin` shim and stubs for unbuilt Cython
extensions. The stubs are not used by LK or S-PROG. The golden numbers themselves
come from pysteps code and are not changed by the stubs.

### A.1 Gaussian bandpass centre wavenumbers

- `L = 3500`, `n = 6`: `r = [2.235707, 7.761062, 26.941856, 93.526322, 324.668533, 1127.058711]`,
  `q = 3.4714134`.
- `L = 32`, `n = 6` (shape 16x32): `r = [1.293701, 2.053622, 3.259921, 5.174802, 8.214486, 13.039684]`.
  At `(i,j) = (0,0)`: `W = [1,0,0,0,0,0]`. At `(0,1)`: `[0.985639, 0.014357, 0.000004, 0, 0, 0]`.

### A.2 Lag-2 adjustment, Yule-Walker, percentile, AR iteration

- `(g1,g2) = (0.95,0.85)`: `g2' = 0.851400`, `phi = (1.447895, -0.524100, 0.265930)`.
- `(0.80,0.50)`: `g2' = 0.550000`, `phi = (1.000000, -0.250000, 0.580948)`.
- `(0.60,0.30)`: `g2' = 0.300000`, `phi = (0.656250, -0.093750, 0.796477)`.
- `adjust_lag2_corrcoef2(np.float64(0), 0.5) = nan`.
- `compute_percentile_mask([[1,5,3],[2,4,3]], war)`:
  `war=0.5 -> [0,1,1,0,1,1]`; `war=0.34 -> [0,1,0,0,1,0]`; `war=0.25 -> [0,1,0,0,1,0]`;
  `war=0.2` and `war=0.0` raise `IndexError`. Ties: `[[1,2,2],[2,3,4]]`, `war=0.5 -> [0,1,1,1,1,1]`.
- `iterate_ar_model([1, 2], phi=(1.2, -0.3, 0.1)) = [2.0, 2.1]`.

### A.3 Interpolation and NaN rules

- `zoom([0,10], 2, order=1) = [0, 3.333, 6.667, 10]`; `zoom([0,10,20], 2, order=1) = [0,4,8,12,16,20]`.
- `map_coordinates` order 1 with a NaN corner of zero weight -> NaN. Outside `[0, n-1]` -> cval.

### A.4 S-PROG end-to-end (32 x 64, deterministic)

Input: `cx = 20 + t`, `cy = 16 + 0.5t`, `frame(t)[y,x] = 30*exp(-((x-cx)^2+(y-cy)^2)/(2*6^2)) + 12*exp(-((x-45)^2+(y-6)^2)/(2*4^2)) - 10`,
`t = 0, 1, 2` (oldest first), float32. `frame0[0, 0:5] = NaN`.
Velocity: `V[0] = 1.0` (x), `V[1] = 0.5` (y), constant. Leads `[1.0, 2.0, 2.5, 3.0]`.
`precip_thr = 5`, `norain = 0`, `n_cascade = 6`, `ar_order = 2`.

Intermediates (pre-adjustment gamma as printed by pysteps; adjusted lag-2; YW):

| Level | g1 (pre) | g2 (pre) | g2' | phi1 | phi2 | phi_pert |
|---|---|---|---|---|---|---|
| 1 | 0.999491528 | 0.998291908 | 0.998291908 | 1.679205835 | -0.680060098 | 0.023377055 |
| 2 | 0.996147683 | 0.986406518 | 0.986406518 | 1.760920809 | -0.767730667 | 0.056190293 |
| 3 | 0.968162005 | 0.891808812 | 0.899766285 | 1.548657232 | -0.599584805 | 0.200337472 |
| 4 | 0.605173366 | 0.444502966 | 0.444502966 | 0.530436212 | 0.123497098 | 0.789999561 |
| 5 | -0.322730043 | -0.235544520 | 0.079527937 | -0.331601876 | -0.027489951 | 0.946133358 |
| 6 | -0.477963744 | -0.351531011 | 0.178493342 | -0.508910715 | -0.064747529 | 0.876536441 |

`war = 0.07275390625`, `precip_min = -10.0`.

Outputs (`px[y,x]`):

| Lead | NaN count | Sum of finite | px[16,20] | px[8,30] | px[0,40] | px[31,63] |
|---|---|---|---|---|---|---|
| 1.0 | 100 | -16206.395170212 | 15.576966763 | -10 | NaN | -10 |
| 2.0 | 131 | -15896.395170212 | 12.723956108 | -10 | NaN | -10 |
| 2.5 | 223 | -14976.395170212 | 11.030153394 | -10 | NaN | -10 |
| 3.0 | 223 | -14976.395170212 | 9.381134987 | -10 | NaN | -10 |

Note the identical sums at 2.5 and 3.0 (7.7d invariant: fixed multiset of values).

### A.5 Dry window

`stack` of three constant frames `-10, -9, -8` (no cell above 5), `V = 0`, leads `[2.5, 5, 7.5, 10]`:
output shape `(4, 16, 24)`, all values `-10` (`precip_min`). D2 path.

### A.6 LK synthetic translation (through the production wrapper)

200 x 300 field, three Gaussian blobs at `(80,90,a=35,s=14)`, `(190,120,28,10)`,
`(140,40,20,8)`, values minus 5, 4 frames translating by `(u,v)` per step:

| True (u,v) | Median on echo cells (full res, after stride-2 wrapper) |
|---|---|
| (2.0, 1.0) | (2.004, 0.999) |
| (-1.5, 0.5) | (-1.499, 0.500) |

Coarse shape `(2, 100, 150)` before upsampling.

---

## Appendix B. Reference file index

- `backend/nowcast/activities.py`: orchestration, selection, LK wrapper, S-PROG call, render, commit, scoring trigger.
- `backend/nowcast/skill.py`: scoring, contingency, point-grid stride rule, resampling.
- `backend/shared/nowcast_skill.py`: rolling log, scores, summary, prometheus.
- `backend/shared/grid_dump.py`: grid write, finalize, prune (writer side; no reader).
- `backend/shared/tiler.py`: class model, geometry, legacy and indexed renderers, footprint max, publication.
- `backend/shared/manifest.py`: manifest read and replace.
- `backend/shared/state.py`: ProcessedSet.
- `backend/shared/activity_heartbeat.py`: heartbeat wrapper.
- `backend/ingest_mrms/activities.py`: MRMS science grid, kick, retention.
- `backend/api/api/server.py`: reader (`_sample_grid_point`), `nowcast_point`, skill endpoint.
- `temporal/workflows/nowcast.py`, `temporal/schedules/seed.py`: workflow, schedule.
- pysteps (local): `pysteps/nowcasts/sprog.py`, `pysteps/nowcasts/utils.py`,
  `pysteps/motion/lucaskanade.py`, `pysteps/tracking/lucaskanade.py`,
  `pysteps/feature/shitomasi.py`, `pysteps/utils/cleansing.py`, `pysteps/utils/interpolate.py`,
  `pysteps/utils/images.py`, `pysteps/extrapolation/semilagrangian.py`,
  `pysteps/cascade/bandpass_filters.py`, `pysteps/cascade/decomposition.py`,
  `pysteps/timeseries/autoregression.py`, `pysteps/timeseries/correlation.py`,
  `pysteps/postprocessing/probmatching.py`, `pysteps/utils/check_norain.py`.
