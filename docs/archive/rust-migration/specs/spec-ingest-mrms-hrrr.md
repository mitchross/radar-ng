# Behavioral spec: IngestMrmsWorkflow and IngestHrrrWorkflow (Python to Rust port)

Date: 2026-10-13. Source: `/mnt/user-data/uploads/radar-ng` (read-only snapshot).
Where this spec and the code disagree, the code wins and the disagreement is listed in section 15.

## 0. Provenance and verification status

Read fully:
- `backend/ingest_mrms/activities.py`, `backend/ingest_mrms/test_activities.py`
- `backend/ingest_hrrr/activities.py`, `backend/ingest_hrrr/test_activities.py`
- `temporal/workflows/ingest_mrms.py`, `temporal/workflows/ingest_hrrr.py`, `temporal/workflows/test_ingest_hrrr.py`

Read for the functions these call:
- `backend/shared/tiler.py` (render pipeline, class model, tile geometry, pyramid identity and publication, footprint pooling)
- `backend/shared/manifest.py`, `backend/shared/state.py`, `backend/shared/grid_dump.py`, `backend/shared/storms.py`, `backend/shared/activity_heartbeat.py`, `backend/shared/palettes.py`
- `backend/shared/palettes/{classic,muted,vivid}.json`, `backend/shared/color_tables.json`
- `temporal/schedules/seed.py` (schedules), `temporal/worker.py` (role registration), `temporal/task_queues.py`
- `ARCHITECTURE.md`, `docs/tuning.md`, `docs/configuration.md`

Verified by execution (python3 -I against the read-only source; numpy 2.5.3, scipy 1.18.1, Pillow 12.3.0; the pinned numpy 1.26.4 was not available):
- MRMS grid block factors per zoom: z4 (5,8), z5 (2,4), z6 (1,2), z7 (1,1). So z4-z6 are footprint-max pooled, z7 is native.
- A fully transparent frame through the legacy renderer yields zero published palettes (`outcome=EMPTY`, `rendered_palettes=[]`).
- No palette JSON contains a `dewpoint` or `humidity` key.

NOT verified:
- The Python test suites. pytest, httpx, pyproj, temporalio and pygrib are not installed, and PyPI is unreachable. Test invariants in section 14 are read from the test source, not run.
- Live NOAA data. `noaa-mrms-pds.s3.amazonaws.com` and `noaa-hrrr-bdp-pds.s3.amazonaws.com` are blocked by the sandbox egress policy (CONNECT 403; the proxy status lists both hosts). The `.idx` line format, GRIB2 templates, grid sizes and the GRIB2 discipline/category/number codes in section 5.4 come from the code plus domain knowledge. They must be confirmed against one real HRRR `wrfsfcf01` file with its `.idx`, and one real MRMS `.grib2.gz`, before the Rust decoder is trusted.

Vendored Rust Temporal SDK available: `/home/claude/rust-port/vendor-src/temporalio-{client,common,macros,sdk,workflow}-1.1.0`.

---

## 1. Conventions

- **Valid time string**: Python `datetime.isoformat()` of a UTC-aware datetime, e.g. `2026-07-15T13:00:00+00:00`. Seconds only, no fractional part (valid times are whole hours or whole MRMS minutes). Used verbatim as directory names (they contain colons), manifest timestamps and grid keys. Every string the Rust code emits must match byte for byte.
- **Run id**: `YYYYMMDD_HH` (UTC init hour), e.g. `20260715_12`.
- **Layer**: namespace string used in manifest, tile and grid paths.
  - MRMS: `radar` (base reflectivity QC), `radar-composite`.
  - HRRR: `radar-hrrr`, `temperature`, `dewpoint`, `humidity`, `wind`, `cape`, `precip-type`, `precip-accum`, `cloud`.
- **Palette**: a named color set (`classic`, `muted`, `vivid`), loaded from `backend/shared/palettes/<name>.json`.
- **Pyramid**: a directory of XYZ PNG tiles `{z}/{x}/{y}.png`, 256 x 256 px, for every zoom in the layer's zoom list.
- **Pending**: an upstream object that has not been published yet (HTTP 404). Not an error.
- **fhr**: forecast hour, integer 1..horizon. Valid time = run init time + fhr hours.
- **Horizon**: number of forecast hours for a run (section 11.2).
- JSON is UTF-8. All JSON in the state files is ASCII in practice.

---

## 2. Temporal surface

### 2.1 Workflows

| Workflow type name (exact) | Python class | Input | Result |
|---|---|---|---|
| `IngestMrmsWorkflow` | `IngestMrmsWorkflow.run(args: IngestMrmsArgs \| None)` | JSON object or `null` (null means defaults) | `IngestMrmsResult` |
| `IngestHrrrWorkflow` | `IngestHrrrWorkflow.run()` | none | `IngestHrrrResult` |

### 2.2 Schedules (temporal/schedules/seed.py)

Workflow IDs are `sched-<schedule_id>` plus Temporal's per-fire timestamp suffix. Every schedule uses `ScheduleOverlapPolicy.SKIP`.

| Schedule ID | Workflow | Interval | Execution timeout (`max_runtime`) | Catchup window | Jitter | Input |
|---|---|---|---|---|---|---|
| `ingest-mrms-base` | IngestMrmsWorkflow | 1 min | 6 min | 5 min | 20 s | `[{"mrms_prefix": "CONUS/MergedBaseReflectivityQC_00.50", "layer_name": "radar"}]` |
| `ingest-mrms-composite` | IngestMrmsWorkflow | 1 min | 6 min | 5 min | 20 s | `[{"mrms_prefix": "CONUS/MergedReflectivityComposite_00.50", "layer_name": "radar-composite"}]` |
| `ingest-hrrr` | IngestHrrrWorkflow | 15 min | 45 min | 1 h (default) | none | none |

Task queue (`_spec_for`):
- If env `USE_ISOLATED_TASK_QUEUES == "1"`: `radar-ng-mrms` for MRMS schedules, `radar-ng-hrrr` for HRRR.
- Otherwise: `radar-ng` (legacy queue, all activities on one worker).

The nowcast kick targets the schedule ID given by `NOWCAST_SCHEDULE_ID` (default `nowcast`).

### 2.3 Worker role registration (temporal/worker.py)

- Role `mrms` -> queue `radar-ng-mrms`: workflow `IngestMrmsWorkflow`; activities `mrms_list_unprocessed_keys`, `mrms_process_frame`, `mrms_mark_processed`, `mrms_cleanup`.
- Role `hrrr` -> queue `radar-ng-hrrr`: workflow `IngestHrrrWorkflow`; activities `hrrr_find_latest_run`, `hrrr_horizon_for_run`, `hrrr_process_forecast_hour`, `hrrr_mark_processed`, `hrrr_publish_run`, `hrrr_cleanup`.
- Legacy/`all`: all ten activities and both workflows on `radar-ng`.
- Worker defaults: `TEMPORAL_MAX_CONCURRENT_ACTIVITIES` 4 (code default; compose lab 2, prod set per role), poll concurrency 2.

### 2.4 Activity registry

Activity names are the exact strings in `@activity.defn(name=...)`. Timeouts and retry policies are the ones the workflows pass at call time.

Retry policies:
- `_DEFAULT_RETRY`: initial interval 1 s, backoff 2.0, max interval 100 s, max attempts 5.
- `_LIST_RETRY` (MRMS list): initial 1 s, backoff 2.0, max interval 10 s, max attempts 3.
- `_FRAME_RETRY` (MRMS frame): initial 2 s, backoff 2.0, max interval 30 s, max attempts 2.
- `_FORECAST_RETRY` (HRRR hour): initial 2 s, backoff 2.0, max interval 120 s, max attempts 2.

| Activity name | Positional args on the wire | Input JSON | Output JSON | Caller | start_to_close | schedule_to_close | heartbeat_timeout | Retry |
|---|---|---|---|---|---|---|---|---|
| `mrms_list_unprocessed_keys` | 1 | `IngestMrmsArgs` object | `ListKeysResult` | MRMS wf | 30 s | - | - | `_LIST_RETRY` |
| `mrms_process_frame` | 1 | `ProcessFrameInput` object | `ProcessFrameResult` | MRMS wf (max 2 in flight) | 4 min | 5 min | 60 s | `_FRAME_RETRY` |
| `mrms_mark_processed` | 1 | `MarkProcessedInput` object | null | MRMS wf | 30 s | - | - | `_DEFAULT_RETRY` |
| `mrms_cleanup` | 1 | `CleanupInput` object | `CleanupResult` | MRMS wf | 5 min | - | - | `_DEFAULT_RETRY` |
| `hrrr_find_latest_run` | 0 | none | `FindRunResult` | HRRR wf | 120 s | - | - | `_DEFAULT_RETRY` |
| `hrrr_horizon_for_run` | 1 (`run_id` string) | `"YYYYMMDD_HH"` | integer | HRRR wf | 10 s | - | - | `_DEFAULT_RETRY` |
| `hrrr_process_forecast_hour` | 2 (`run_id` string, `fhr` integer) | `"YYYYMMDD_HH"`, `fhr` | `ForecastHourResult` | HRRR wf (max 4 in flight) | 8 min | 12 min | 90 s | `_FORECAST_RETRY` |
| `hrrr_publish_run` | 2 (`run_id` string, `results` list) | `"YYYYMMDD_HH"`, `[ForecastHourResult]` | list of strings | HRRR wf | 2 min | - | - | `_DEFAULT_RETRY` |
| `hrrr_mark_processed` | 1 (`run_id` string) | `"YYYYMMDD_HH"` | null | HRRR wf | 30 s | - | - | `_DEFAULT_RETRY` |
| `hrrr_cleanup` | 1 (`retention_hours` integer) | `12` | `HrrrCleanupResult` | HRRR wf | 5 min | - | - | `_DEFAULT_RETRY` |

Not an activity: `kick_nowcast` is a Temporal client call made from inside `mrms_process_frame` (section 12.1).

Heartbeat details (informational only; nothing resumes from them):
- MRMS frame: `{"phase":"download","key":...}`, then `{"phase":"render","timestamp":...,"palettes":[...]}`, then `{"phase":"grid","timestamp":...}`.
- HRRR hour: `{"phase":"download","fhr":...}` (plus `run_id` in the threaded beats), then `{"phase":"render","fhr":...}`.

Heartbeat cadence (`run_sync_with_heartbeat`): a background task in the activity's event loop calls `activity.heartbeat(details)` every N seconds while the blocking function runs in a thread. Cadence: MRMS download 20 s, MRMS render 30 s, HRRR download 30 s, HRRR render 30 s.

### 2.5 Payload schemas

Python dataclasses are serialized by the Temporal default JSON converter: a JSON object with the field names as written (snake_case). Defaults are shown; an absent field takes its default on decode. Unknown fields should be ignored.

`IngestMrmsArgs` (input to both MRMS list and the workflow):
- `mrms_prefix`: string, default env `MRMS_PREFIX` or `CONUS/MergedBaseReflectivityQC_00.50`.
- `layer_name`: string, default env `LAYER_NAME` or `radar`.

`ListKeysResult`:
- `keys`: array of string, default `[]`.
- `backlog_total`: integer, default 0.

`ProcessFrameInput`: `key` string (required), `layer_name` string (required).

`ProcessFrameResult`:
- `key`: string (required)
- `timestamp`: string, default `""`
- `rendered`: boolean, default false
- `palettes`: array of string, default `[]`
- `duration_s`: number, default 0.0

`MarkProcessedInput`: `key` string, `layer_name` string (both required).

`CleanupInput`: `layer_name` string, `retention_hours` integer (both required).

`CleanupResult` (MRMS) and `HrrrCleanupResult` (HRRR), same shape: `tile_dirs_removed` integer, `grid_files_removed` integer (both required).

`IngestMrmsResult` (workflow result):
- `layer`: string
- `backlog_total`: integer
- `rendered_count`: integer
- `cleanup`: `CleanupResult` or null (default null)

`FindRunResult`: `run_id` string or null (required), `already_processed` boolean (required).

`ForecastHourResult`:
- `fhr`: integer (required)
- `rendered_layers`: array of string, default `[]`
- `duration_s`: number, default 0.0
- `valid_timestamp`: string, default `""`

`IngestHrrrResult` (workflow result):
- `run_id`: string or null
- `skipped_already_processed`: boolean
- `forecast_hours_processed`: integer
- `cleanup`: `HrrrCleanupResult` or null (default null)
- `layers_per_hour`: array of array of string (default `[]`), ordered by fhr
- `published_layers`: array of string (default `[]`)
- `complete`: boolean (default false)

---

## 3. Configuration (every knob read by the in-scope code)

Read at import time unless noted. Parsing: integers with `int()`, floats with `float()`, booleans as exact string `"1"`.

| Env var | Default | Module | Meaning |
|---|---|---|---|
| `TILE_DIR` | `/data/tiles` | mrms, hrrr | Tile root |
| `STATE_DIR` | `/data/state` | mrms, hrrr, manifest, storms | Manifest, ProcessedSet, storms.json |
| `MRMS_TMP_ROOT` | `/tmp/mrms_work` | mrms | Per-attempt scratch root |
| `HRRR_TMP_ROOT` | `/tmp/hrrr_work` | hrrr | Per-attempt scratch root |
| `MRMS_PREFIX` | `CONUS/MergedBaseReflectivityQC_00.50` | mrms | Default prefix when args absent |
| `LAYER_NAME` | `radar` | mrms | Default layer when args absent |
| `BACKLOG_PER_CYCLE` | `3` | mrms | Max new keys processed per workflow run (newest first) |
| `NOWCAST_SCIENCE_GRID_MAX_CELLS` | `7000000` | mrms | Cell cap for the nowcast input grid |
| `NOWCAST_KICK_ENABLED` | `1` | mrms | `"1"` enables the kick |
| `NOWCAST_SCHEDULE_ID` | `nowcast` | mrms | Schedule the kick triggers |
| `NOWCAST_KICK_TIMEOUT_S` | `5` | mrms | RPC budget for the kick |
| `FORECAST_HOURS` | `18` | hrrr | Horizon for non-extended runs |
| `EXTENDED_FORECAST_HOURS` | `48` | hrrr | Horizon for 00/06/12/18z runs |
| `HRRR_ENABLED_LAYERS` | `radar-hrrr` | hrrr | Comma list; `all` enables every layer. `radar-hrrr` is always on. |
| `HRRR_GRID_DUMP_STEP_DEG` | `0.03` | hrrr | Regular grid step for HRRR grid dumps |
| `TILE_RENDERER` | `legacy` | tiler (per call) | `legacy` or `indexed` |
| `TILE_RENDERER_CANARY_ROLE` | unset | tiler (per call) | Must name one of `mrms`, `nowcast`, `hrrr`, `aux` when `TILE_RENDERER=indexed` |
| `TILE_GEOMETRY_CACHE_MB` | `256` (clamped 0..4096) | tiler | Process cache (performance only) |
| `TILE_GEOMETRY_CACHE_ENTRIES` | `4096` (clamped 1..1000000) | tiler | Process cache (performance only) |
| `TILE_PNG_COMPRESS_LEVEL` | `6` (clamped 0..9) | tiler | Indexed PNG only; legacy hard-codes 1 |
| `GRID_DIR` | `/data/grids` | grid_dump | Grid root |
| `GRID_MAX_AGE_S` | `43200` | grid_dump | Age-based grid sweep |
| `GRID_MAX_CELLS` | `810000` (= 900*900) | grid_dump | Default stride cap |
| `GRID_ORPHAN_GRACE_S` | `300` | grid_dump | Grace before unreferenced `.bin` is removed |
| `NOWCAST_INPUT_FRAMES` | `4` (min 3) | grid_dump | Keep-N for `radar-nowcast-input` |
| `NOWCAST_INPUT_STEP_MIN` | `5` | grid_dump | Feeds keep-N |
| `NOWCAST_GRID_INPUT_LAYER` | `radar-nowcast-input` | grid_dump | Layer name with keep-N |
| `STORM_THRESHOLD_DBZ` | `40` | storms | Cell threshold |
| `STORM_MIN_PIXELS` | `5` | storms | Minimum component size |
| `STORM_MAX_CELLS` | `500` | storms | Cells kept per frame |
| `STORM_MAX_TRACK_SPEED_KMH` | `160` | storms | Association and motion ceiling |
| `STORM_MATCH_SLACK_KM` | `8` | storms | Association gate floor |
| `STORM_TRACK_HISTORY_MIN` | `20` | storms | Track history window |
| `STORM_MIN_TRACK_SPAN_MIN` | `5` | storms | Minimum span for a motion fit |
| `STORM_PREFETCH_PADDING_KM` | `12` | storms | Padding on component bboxes |
| `PALETTES` | `classic` | palettes (per call) | Comma list of palette names |

Notes:
- `docs/configuration.md` lists `PALETTES=classic,muted,vivid` and `BACKLOG_PER_CYCLE=2`. The code defaults are `classic` and `3`. See section 15.
- `docs/configuration.md` documents `STORM_MIN_TRACK_RADIUS_KM`, which the code never reads. The code uses `STORM_MATCH_SLACK_KM`.
- `manifest.py` reads `STATE_DIR` independently at import. Keep it equal to the activity module's value (the same env var, so it matches in production).

---

## 4. Upstream data sources

### 4.1 MRMS (NOAA MRMS on S3)

- Base URL: `https://noaa-mrms-pds.s3.amazonaws.com` (anonymous, no signing).
- Prefixes (from schedules): `CONUS/MergedBaseReflectivityQC_00.50` -> layer `radar`; `CONUS/MergedReflectivityComposite_00.50` -> layer `radar-composite`.
- Daily partition: `{prefix}/{YYYYMMDD}/`, where the date is the UTC date at listing time.
- Object key shape (inferred from the tests and the timestamp parser): `{prefix}/{YYYYMMDD}/MRMS_{product}_{YYYYMMDD-HHMMSS}.grib2.gz`.

Listing (`_list_keys_sync`):
- Request: `GET {base}?prefix={prefix}/{YYYYMMDD}&list-type=2&max-keys=1000`, with `&continuation-token={token}` on later pages. The prefix and token are interpolated into the query without URL encoding (see D6).
- Timeout 30 s per page. Non-2xx is an error (`raise_for_status`). httpx does not follow redirects by default; the Rust client must match (section 17).
- Response is XML with namespace `http://s3.amazonaws.com/doc/2006-03-01/`. Read `Contents/Key` (keep only keys ending in `.grib2.gz`), `IsTruncated` (string `"true"` means more) and `NextContinuationToken`.
- Loop until not truncated or no token.
- `_list_recent_files_sync`: list today's UTC date prefix. If that yields zero `.grib2.gz` keys, list yesterday's UTC date prefix. Return the keys sorted ascending.

Key timestamp (`_extract_timestamp`):
- Take the basename, strip the `.grib2.gz` suffix, split on `_`, take the last token, and parse it with `%Y%m%d-%H%M%S` as UTC.
- Output: the valid time string (section 1).
- Example: `.../MRMS_MergedBaseReflectivityQC_00.50_20260930-000000.grib2.gz` -> `2026-09-30T00:00:00+00:00`.
- A key that does not parse raises (activity failure). This happens after the download in the code (D21).

Object download (`_download_and_decode_sync`):
- `GET {base}/{key}`, timeout 60 s, `raise_for_status`. A 404 is an error here, not "pending". MRMS has no pending semantics.

### 4.2 HRRR (NOAA HRRR on S3)

- Base URL: `https://noaa-hrrr-bdp-pds.s3.amazonaws.com`.
- File key: `hrrr.{YYYYMMDD}/conus/hrrr.t{HH}z.wrfsfcf{FF:02d}.grib2`. Index: same key with `.idx` appended.

Latest-run discovery (`_find_latest_run_sync`):
- For `hours_ago` in 0..11: `t = now - hours_ago`, truncated to the hour. Build the f01 key for that run. `HEAD` with timeout 10 s.
- Status 200 wins: return `YYYYMMDD_HH`. Any `httpx.HTTPError` is skipped. Any other status (including 301/302/403/404) is skipped (redirects are not followed).
- Return None if no candidate succeeds.

Index format (`_get_idx_sync`):
- Text, one record per line. Split on `:`. Require at least 5 fields.
- `num` = int(field 0), `offset` = int(field 1), `var_name` = field 3, `level` = field 4, `fcst_time` = field 5 or `""`. The raw line is kept for matching.
- A non-integer num or offset raises `ValueError`, which is not an `httpx.HTTPError`, so it propagates as an activity failure.
- Timeout 15 s.

Range selection (`_pick_ranges`):
- A record matches if any matcher tuple has every term contained as a substring of the **raw line**.
- For each matching record `i` in file order: start = `offset_i`; end = `offset_{i+1} - 1` (the next line in the index, matched or not); the last record has an open end.
- Ranges are not merged.

Matchers (`IDX_MATCHERS`, all terms must be substrings of the raw line):

| Key | Terms |
|---|---|
| refc | `REFC`, `entire atmosphere` |
| t2m | `TMP`, `2 m above ground` |
| dpt2m | `DPT`, `2 m above ground` |
| cape | `CAPE`, `surface` |
| u10 | `UGRD`, `10 m above ground` |
| v10 | `VGRD`, `10 m above ground` |
| crain | `CRAIN`, `surface` |
| csnow | `CSNOW`, `surface` |
| cfrzr | `CFRZR`, `surface` |
| cicep | `CICEP`, `surface` |
| rh2m | `RH`, `2 m above ground` |
| apcp | `APCP`, `surface` |
| tcdc | `TCDC`, `entire atmosphere` |

Only the variables needed by enabled layers are matched (section 12.2, `_needed_vars`). With the default `HRRR_ENABLED_LAYERS`, only `refc` is needed, so one byte range is fetched per hour.

Download (`_download_subset_sync`):
1. Fetch the `.idx` (section above).
2. On any `httpx.HTTPError` from step 1 (this includes 404, 5xx, transport errors and parse-time HTTP status errors): fall back to the full file. `GET` the base URL with timeout 300 s.
   - 404: log `hour_pending` (fields `run_id`, `fhr`), return None. No retry.
   - Other non-2xx: raise.
   - 2xx: write the whole body to `{tmp}/hrrr_f{fhr:02d}.grib2`, return that path. The full file is about 130 MB (D4).
3. Otherwise match ranges. If none match, return None silently (no log).
4. Otherwise, for each range in order: `GET` base URL with header `Range: bytes={start}-{end}` (or `bytes={start}-` when open), timeout 120 s, `raise_for_status`, append body to `{tmp}/hrrr_f{fhr:02d}_subset.grib2`. The status 206 is not checked (D5).
5. Return the subset path.

The result is a concatenation of whole GRIB2 messages. Decode it as a GRIB2 stream.

---

## 5. GRIB2 decoding

### 5.1 Library in use

- Python: `pygrib` (>= 2.1), which uses eccodes (`libeccodes-dev` in the base image). HRRR also uses `pyproj` (>= 3.6).
- Rust options: section 17.

### 5.2 MRMS message

- Gunzip the whole `.grib2.gz` (multi-member gzip is valid input).
- Use message number 1 (pygrib numbering is 1-based: `grbs[1]`). The file is expected to hold one message. No shortName check is made.
- Grid axes (`_grid_axes`):
  - If `gridType == "regular_ll"`: `lat = linspace(latitudeOfFirstGridPointInDegrees, latitudeOfLastGridPointInDegrees, Nj)`; `lon = linspace(longitudeOfFirstGridPointInDegrees, longitudeOfLastGridPointInDegrees, Ni)`. Do not build 2D lat/lon meshes (the code comment cites about 400 MB of float64 and a 2 s deadlock detector).
  - Otherwise: `lats, lons = latlons()`; `lat = lats[:,0]`, `lon = lons[0,:]`.
- Expected MRMS geometry (from the test header and docstrings): `Nj = 3500`, `Ni = 7000`, lat 54.995 -> 20.005001 (descending), lon 230.005 -> 299.994998 (step 0.01 degree). Decoded float32 grid: 7000 x 3500 = 24.5 M cells, about 98 MB.
- Values: `grb.values`. If the result has `.filled`, call `.filled(NaN)`. Then cast to float32. Missing = NaN.
- Longitudes: `lon > 180` becomes `lon - 360`. Result is -129.995 .. -60.005 ascending.
- Latitude orientation: if `lat[0] > lat[-1]` (the MRMS case), flip the grid vertically and reverse `lat`. Longitudes are untouched. Post-flip: row 0 is the southernmost row, lat ascending, lon ascending.
- Downstream code uses the post-flip arrays.

### 5.3 HRRR messages

Selector semantics (`_extract_variable`):
- Scan messages in file order. The first message where every selector key matches is used.
- A key named `name` matches if the lowercased selector value is a substring of the lowercased `grb.name`.
- Any other key matches if `getattr(grb, key, None) == value` exactly.
- If nothing matches, return None. Any exception inside extraction is logged as `extract_failed` (warning) and returns None.

`VAR_SELECTORS`:

| Key | Selector |
|---|---|
| refc | `shortName=refc`, `typeOfLevel=atmosphere` |
| t2m | `name="2 metre temperature"`, `typeOfLevel=heightAboveGround`, `level=2` |
| dpt2m | `name="2 metre dewpoint temperature"`, `typeOfLevel=heightAboveGround`, `level=2` |
| cape | `name="Convective available potential energy"`, `typeOfLevel=surface` |
| u10 | `name="10 metre U wind component"`, `typeOfLevel=heightAboveGround`, `level=10` |
| v10 | `name="10 metre V wind component"`, `typeOfLevel=heightAboveGround`, `level=10` |
| crain | `name="Categorical rain"`, `typeOfLevel=surface` |
| csnow | `name="Categorical snow"`, `typeOfLevel=surface` |
| cfrzr | `name="Categorical freezing rain"`, `typeOfLevel=surface` |
| cicep | `name="Categorical ice pellets"`, `typeOfLevel=surface` |
| rh2m | `name="2 metre relative humidity"`, `typeOfLevel=heightAboveGround`, `level=2` |
| apcp | `name="Total Precipitation"`, `typeOfLevel=surface` |
| tcdc | `name="Total Cloud Cover"`, `typeOfLevel=atmosphere` |

Per match (`_extract_variable`):
- `data = grb.values` (masked -> NaN via `.filled`), cast float32.
- `lats, lons = grb.latlons()`; normalize lons (`> 180` -> `- 360`). Lats and lons are 2D (h x w).
- Native projection (`_extract_native_projection`):
  - `projparams = grb.projparams`; if missing, no native projection.
  - CRS = `CRS.from_user_input(projparams)`, falling back to `Proj(projparams).crs`.
  - Transform `(lon, lat)` from EPSG:4326 to CRS (always_xy) to get `xs, ys` (2D).
  - `x0 = nanmedian(xs[:,0])`, `x1 = nanmedian(xs[:,-1])`, `y0 = nanmedian(ys[0,:])`, `y1 = nanmedian(ys[-1,:])`.
  - Require all finite, `|x1-x0| >= 1e-6`, `|y1-y0| >= 1e-6`.
  - `source_x = linspace(x0, x1, w)`, `source_y = linspace(y0, y1, h)`. Store CRS as WKT.
  - Any failure: log `projection_extract_failed` (warning), native = None. Then the grid falls back to 1-D axes (`lats[:,0]`, `lons[0,:]`), which is not expected for HRRR.
- Grid-relative flag: `int(grb["resolutionAndComponentFlags"]) & 8` (bit 3 set means u/v are along grid axes). Exception -> false.

Expected HRRR geometry: Lambert conformal conic, sphere radius 6371229 m, lat_1 = lat_2 = lat_0 = 38.5, lon_0 = -97.5 (the test fixture uses these parameters), 3 km spacing, CONUS grid roughly 1799 x 1059 (verify on real data).

### 5.4 GRIB2 codes behind the selectors (domain knowledge; verify)

Discipline.category.parameter (WMO/NCEP GRIB2 tables), with the level that the selector implies:

| Key | shortName | Code | Level |
|---|---|---|---|
| refc | REFC | 0.16.196 | entire atmosphere |
| t2m | TMP | 0.0.0 | 2 m above ground |
| dpt2m | DPT | 0.0.6 | 2 m above ground |
| cape | CAPE | 0.7.6 | surface |
| u10 | UGRD | 0.2.2 | 10 m above ground |
| v10 | VGRD | 0.2.3 | 10 m above ground |
| crain | CRAIN | 0.1.192 | surface |
| csnow | CSNOW | 0.1.193 | surface |
| cfrzr | CFRZR | 0.1.194 | surface |
| cicep | CICEP | 0.1.195 | surface |
| rh2m | RH | 0.1.1 | 2 m above ground |
| apcp | APCP | 0.1.8 | surface (accumulation) |
| tcdc | TCDC | 0.6.1 | entire atmosphere |

A Rust decoder can match on these codes plus level type and value, but only after confirming on real files that they select the same messages as the Python selectors. The Python selectors (name substrings) are the reference behavior.

### 5.5 Unit conversions and derived fields

Arithmetic must be done in float32 and in the same operation order, or the results will differ in the last bit.

| Output | Source | Transform (float32, in this order) | Unit label | Notes |
|---|---|---|---|---|
| radar-hrrr | REFC | none | dBZ | |
| temperature | TMP (K) | `(k - 273.15) * 9 / 5 + 32` | degF | |
| dewpoint | DPT (K) | same as temperature | degF | Never rendered (D1) |
| humidity | RH (%) | none | % | Never rendered (D1) |
| wind (speed) | UGRD, VGRD (m/s) | `u_mph = u * 2.237`; `v_mph = v * 2.237`; `speed = sqrt(u_mph**2 + v_mph**2)` | mph | |
| wind_u, wind_v | as above | `u_mph`, `v_mph`, then rotated (below) | mph | Grid dump only |
| cape | CAPE | none | J/kg | |
| precip-accum | APCP (kg/m2 = mm) | `d / 25.4` | in | |
| cloud | TCDC | none | % | |
| precip-type | crain, csnow, cfrzr, cicep | category grid, below | category code | |

Precip-type category grid (`_process_forecast_hour_sync`):
- `cat` = int32 zeros, shape of the grid.
- Assignments in this order, each overwriting earlier ones: `cat[crain > 0] = 1`; `cat[csnow > 0] = 2`; `cat[cfrzr > 0] = 3`; `cat[cicep > 0] = 4`. NaN compares false.
- Effective precedence: ice pellets > freezing rain > snow > rain.
- Categories: 1 `rain`, 2 `snow`, 3 `freezing_rain`, 4 `ice_pellets`. Grid geometry comes from the last non-None categorical extraction.

Earth-relative wind rotation (`_latlon_resampler`, `_safe_wind_dump`), applied only when the grid-relative flag is set:
- `theta` = angle of the grid x-axis measured from east, in radians. Computed at each target lat/lon: forward-project the point and the point shifted 1000 m east in projected x, back-project both to lat/lon, then `theta = atan2(east_lat - lat, (east_lon - lon) * cos(lat))`.
- After resampling `u` and `v` to the regular grid: `u' = u*cos(theta) - v*sin(theta)`, `v' = u*sin(theta) + v*cos(theta)`.
- If the flag is clear, u and v are resampled but not rotated.

### 5.6 Missing values

- MRMS and HRRR decode masked values to NaN. Decoded arrays keep NaN; no fill substitution.
- Classification maps NaN to class 0 (transparent) (section 6.2).
- Grid dumps convert NaN to -9999.0 at write time (section 7).
- HRRR bilinear resampling for grid dumps uses `map_coordinates(order=1, mode="constant", cval=NaN)`. NaN neighbors propagate NaN.

---

## 6. Rendering

### 6.1 Palette tables

- Files: `backend/shared/palettes/<name>.json`. Each file is a JSON object keyed by color key.
- Continuous key shape: `{"ranges": [{"min": x, "max": y, "rgba": [r,g,b,a]}, ...], "no_data_below": v, "unit": "..."}`. Range order matters: a later range wins where two ranges cover the same bin.
- Categorical key shape (`precip_type`): `{"categories": {"rain": [r,g,b,a], ...}}`.
- Color keys in the shipped files: `reflectivity`, `temperature`, `wind_speed`, `cape`, `precip_type`, `precip_accum`, `cloud_cover`, plus `pm25` and `ozone` (used by air quality, not in scope). No `dewpoint` or `humidity` key exists in any palette file.
- `classic.json` `reflectivity`: `no_data_below` 5; 13 ranges from 5 to 75 dBZ. The first ranges are `[5,10) -> [110,185,225,115]`, `[10,15) -> [90,200,195,155]`, `[15,20) -> [110,220,120,185]`. Copy the JSON files verbatim into the Rust tree; do not retype the tables.
- Fallback: `load_palette("classic")` falls back to `backend/shared/color_tables.json` only if `classic.json` is missing (never in the shipped tree). That file's reflectivity has `no_data_below` 15, which differs from `classic.json` (5).

Palette loading per activity:
- MRMS (`_load_palette_tables`): for each name in `PALETTES` (insertion order preserved), `load_palette(name)["reflectivity"]`. A missing file (`FileNotFoundError`) or a missing key (`KeyError`) logs `palette_missing` and skips the palette. If nothing loads, use `classic`'s reflectivity table.
- HRRR (`_load_palette_tables`): for each name, the whole JSON dict. Only `FileNotFoundError` is caught. If nothing loads, use `classic`.

Layer to color key (`LAYER_COLOR_KEYS`):

| Layer | Color key |
|---|---|
| radar-hrrr | reflectivity |
| temperature | temperature |
| dewpoint | dewpoint |
| humidity | humidity |
| wind | wind_speed |
| cape | cape |
| precip-type | precip_type |
| precip-accum | precip_accum |
| cloud | cloud_cover |

A layer renders for a palette only if that palette has the color key. With the shipped palettes, `dewpoint` and `humidity` never render (D1).

### 6.2 Class model (`ClassModel`)

Given the continuous tables of a set of palettes `P`:

1. `edges` = sorted, de-duplicated union over all palettes of every range `min`, every range `max`, and each palette's `no_data_below` (default -999 if absent). Float64.
2. Raw index `r(v) = count of edges <= v` (searchsorted, side right). Range `[0, E]` with `E = len(edges)`. NaN and +inf give `E`. -inf gives 0.
3. Raw LUT per palette, index `i` in `1..E-1` covers bin `[edges[i-1], edges[i])`, with `lo = edges[i-1]`, `hi = edges[i]`:
   - If `lo < no_data_below(p)`: transparent (all zeros).
   - Else the colour is the rgba of the **last** range (list order) with `range.min <= lo` and `hi <= range.max`. If none, transparent.
   - Raw indices `0` and `E` are transparent.
4. Collapse: a raw class is visible if any palette has alpha > 0 there. Visible raw classes are renumbered `1..K` in ascending raw order. Class 0 is transparent in every palette. If `K > 255`, raise.
5. Per-palette LUT: `(K+1) x 4` uint8.
6. Value lookup: `class = remap[r(v)]`; colour = `LUT_p[class]`.

Bin rule: a value exactly on an edge belongs to the bin that starts at that edge.

Categorical model (`build_categorical_class_model`): LUT indexed by integer category value. Values `< 0` or above the max category are class 0. Categories are looked up by name from `category_map` (`{1:"rain",2:"snow",3:"freezing_rain",4:"ice_pellets"}`) into each palette's `categories` table. Same collapse step.

`apply_color_table(values, table)` = single-palette model, then LUT lookup per cell. This is the colorize step of the legacy renderer.

### 6.3 Renderer selection

`tile_renderer_for_role(role)` (read per call, not at import):
- `role` must be one of `mrms`, `nowcast`, `hrrr`, `aux`. MRMS uses `mrms`; HRRR uses `hrrr`.
- `TILE_RENDERER` (default `legacy`): values other than `legacy` or `indexed` raise.
- `legacy` returns `legacy` for every role.
- `indexed` requires `TILE_RENDERER_CANARY_ROLE` to be one of the role names; only that role gets `indexed`, every other role gets `legacy`. Missing or invalid canary raises.

Production default: `legacy` everywhere. The MVP for the Rust port is the legacy path. The indexed path is canary-only (section 6.5).

### 6.4 Legacy RGBA renderer (default)

Entry: `render_frame_palettes(values, lats, lons, tables, output_dirs, zooms, renderer="legacy", ...)`.

Continuous case (`category_map` absent):
1. For each palette: `rgba = apply_color_table(values, table)`. This colorizes each source cell to its class colour. No value interpolation happens here.
2. For each native zoom (MRMS: 4..7 minus pooled zooms, section 6.6; HRRR: 4, 5, 6):
   - Tile range: `tx` from `tile_x(lat_max, lon_min, z)` to `tile_x(lat_min, lon_max, z)` inclusive; `ty` from `tile_y(lat_max, lon_min, z)` to `tile_y(lat_min, lon_max, z)` inclusive. Tile index formulas (section 6.7).
   - Per tile, pixel centres `(i + 0.5) / 256`:
     - `lon_tile = (tx + px) / n * 360 - 180`, `lat_tile = degrees(atan(sinh(pi * (1 - 2*(ty + py)/n))))` with `n = 2^z`.
   - Regular source (1-D lat/lon): `col_frac = _axis_to_fractional_indices(lon_tile, lon)`, `row_frac = _axis_to_fractional_indices(lat_tile, lat)`. If `col_frac.max() < 0` or `col_frac.min() > w-1`, skip the tile (same for rows).
   - Projected source (HRRR): forward-project the tile mesh from EPSG:4326 to the source CRS (always_xy), then map `x` through `source_x` and `y` through `source_y` with `_axis_to_fractional_indices`. Non-finite results become -1.
   - Crop: `row_bounds = [floor(min), ceil(max)]` clipped to the grid (`_finite_index_bounds`); same for columns. If no valid bounds, skip. If the cropped alpha is all zero, skip.
   - Resample each of the four RGBA bands separately with `map_coordinates(band, [rows, cols], order=1, mode="constant", cval=0)`. Output dtype uint8.
   - If the tile's alpha max is zero, skip.
   - Write PNG RGBA with compress level 1 (no optimize) to `{out}/{z}/{tx}/{ty}.png`. Count it.
3. Return the count.

Consequences:
- Colour blends across class boundaries and alpha feathers at class edges, because the interpolation runs on colours, not values.
- Categorical layers (precip-type) also blend (bilinear on category colours).

Categorical case (`category_map` present, legacy): colorize with `apply_categorical_color_table`, then the same bilinear pass.

Output directory per palette: `{TILE_DIR}/{layer}/{palette}/{path}` where `path` is a valid-time string (MRMS) or `runs/{run_id}/{valid}` (HRRR).

Atomic publication wraps the render (section 6.8). The render writes into a staging directory.

### 6.5 Indexed renderer (canary only; port after legacy)

Recorded here so that identity handling is complete.

Continuous (`render_continuous_tiles`):
- Bilinearly sample physical values per tile (`PyramidGeometry.sample_bilinear`): four neighbors, weights from fractions; neighbors that are non-finite (or equal to `nodata_value` when set) are excluded; the output is `numerator / valid_weight` where `valid_weight >= min_valid_weight - 1e-6`, else NaN. Coordinates outside `[0, h-1] x [0, w-1]` are invalid.
- Classify the sampled tile (section 6.2). Write a mode-"P" PNG with PLTE and tRNS from the LUT (class 0 transparent). Other palettes reuse the IDAT and only rewrite PLTE/tRNS (`png_palette.replace_palette`).
- PNG compress level from `TILE_PNG_COMPRESS_LEVEL` (default 6).

Categorical (`render_indexed_tiles`):
- Classify the whole grid first, then nearest-neighbour sampling: `nearest(coord) = floor(coord + 0.5)`, valid on `[0, n-1]`, invalid to -1, which samples as class 0.

Tile existence: region-transparency probe on the covering bounds (skip if all zero), then skip if the sampled tile is all zero.

### 6.6 MRMS overview pooling (`overview="max"`)

Only the MRMS calls pass `overview="max"`. HRRR does not pool.

For zoom `z`:
- `px_deg = 360 / (256 * 2^z)`; `dlat = |lat[1]-lat[0]|`; `dlon = |lon[1]-lon[0]|`; `min_cos = cos(radians(min(89, max(|lat_min|, |lat_max|))))`.
- Block factors `(rows, cols) = (max(1, int(px_deg * min_cos / dlat)), max(1, int(px_deg / dlon)))`.
- Pooled if `(rows, cols) != (1, 1)`.

Verified for the MRMS grid (probe):

| Zoom | Block (rows, cols) | Mode |
|---|---|---|
| 4 | (5, 8) | footprint max (pooled) |
| 5 | (2, 4) | footprint max (pooled) |
| 6 | (1, 2) | footprint max (pooled) |
| 7 | (1, 1) | native legacy render |

Footprint-max algorithm (`render_footprint_max_tiles`, legacy: `indexed=False`):
1. `values = float32(data)`. No nodata substitution (MRMS passes `nodata_value=None`).
2. Class model over the palettes being written (section 6.2).
3. For each pooled zoom, for each tile column `tx`:
   - Pixel-edge longitudes: `(tx + k/256)/n*360 - 180` for `k = 0..256` (257 edges).
   - Source column ranges via `_footprint_edges`: fractional index of each edge; `lo = min(f[k], f[k+1])`, `hi = max(...)`; `start = clip(floor(lo + 0.5), 0, n_src)`, `stop = clip(floor(hi + 0.5), 0, n_src)`. Thin rule: if `stop <= start` and `hi >= -0.5` and `lo <= n_src - 0.5`, then `stop = min(start+1, n_src)`, and if `start >= n_src`, `start = n_src - 1`.
   - Reduce: for each row, nan-aware maximum over `values[row, start:stop]` (empty range -> NaN). Result has shape `(h, 256)`, restricted to the covering column span `[c0, c1)`.
4. For each tile row `ty` in the tile range: pixel-edge latitudes via Mercator (`degrees(atan(sinh(pi*(1 - 2*(ty+k/256)/n))))`), row ranges via `_footprint_edges` on `lats`, nan-aware maximum over rows, giving a `(256, 256)` tile of values.
5. Classify the tile (class model), skip if all zero, otherwise write RGBA via the LUT (legacy) with compress level 1.

Both the native zoom (z7) and pooled zooms write into the same palette directories and count toward the same tile count per palette.

### 6.7 Tile geometry (shared)

- `n = 2^z`.
- `tile_x(lat, lon, z) = clamp(int((lon + 180) / 360 * n), 0, n-1)`.
- `tile_y(lat, lon, z) = clamp(int((1 - ln(tan(phi) + 1/cos(phi)) / pi) / 2 * n), 0, n-1)`, where `phi = radians(lat)`. Python `int()` truncates toward zero; for the in-range values here this is floor.
- `tile_bounds(x, y, z)`: west `x/n*360 - 180`, east `(x+1)/n*360 - 180`, north `degrees(atan(sinh(pi*(1 - 2y/n))))`, south with `y+1`.
- `_axis_to_fractional_indices(values, axis)`: axis must be strictly monotonic with at least two points. For a uniform axis (step deviation within `rtol=1e-7`, `atol=|step|*1e-10`), use `(v - axis[0]) / (axis[-1] - axis[0]) * (n-1)` exactly in that operation order (the code says this must stay byte-identical). Otherwise interpolate linearly between the bracketing samples.
- PNG: 256 x 256 tile size; RGBA or mode P; `optimize=False`.

### 6.8 Immutable publication (`_render_and_publish_atomic`)

Terminology: a *final* directory is one palette's pyramid path. A *bound* final has a valid v2 completion marker. An *unbound* final exists but has no v2 marker (includes a v1 marker only).

Algorithm, in order:
1. Canonicalize final paths (resolve, not nested, distinct). Derive lock stripes: `stripe = big_endian_u16(sha256(str(resolved_final))[:2]) mod 64`. Lock file: `{lock_root}/.render-publish-locks/{stripe:02x}.lock`, `lock_root` = `{TILE_DIR}/{layer}`. Take exclusive `flock` on the unique stripe files in sorted order, release in reverse.
2. Inspect each final:
   - Missing -> unresolved.
   - Present and bound (valid v2 marker, schema 2, tile count and path digest and content digest all match) -> bound.
   - Present but unbound (no v2 marker) -> unresolved, needs adoption.
   - Present with a v2 marker that fails validation -> raise (`RuntimeError`).
3. If every palette is bound:
   - All identities equal the expected identity -> `EXISTING_VALID` for each.
   - Else, if every bound identity is compatible with the expected one (section 6.8.2) and all bound render specs match -> `EXISTING_COMPATIBLE`.
   - Else -> `PyramidIdentityConflict`.
4. If some palettes are bound: each bound identity must equal the expected identity, else conflict. Those palettes become `EXISTING_VALID`.
5. Stage every unresolved palette: render into `{final.parent}/.{final.name}.tmp-{uuid4hex}` (`render` callback writes into the staging map).
6. For each staged palette, `count` must equal the number of `.png` files found, else raise. `count == 0` -> the palette is `EMPTY` (no publish). If a final exists with count 0, raise.
7. For each non-empty staged palette:
   - `fsync` every tile file, then write the v2 marker (section 6.8.1), then fsync the directory, then re-inspect with the expected identity.
8. Decide every action before mutating anything:
   - count 0 -> `empty`.
   - final missing -> `create`.
   - final unbound -> `adopt` only if the final's PNG path set and content digests equal the staging tree's; else conflict.
   - final bound -> `existing` if its identity equals expected; else conflict.
9. Execute:
   - `empty`: remove staging, outcome `EMPTY`, tile count 0.
   - `adopt`: fsync the final's tiles, write marker into the final, inspect, fsync the parent chain, remove staging, outcome `ADOPTED`.
   - `existing`: remove staging, outcome `EXISTING_VALID`.
   - `create`: `rename(staging, final)`. On `EEXIST`/`ENOTEMPTY` (race), inspect the winner: unbound and matching -> adopt (`ADOPTED`); bound -> `EXISTING_VALID`; otherwise conflict. On success: fsync the parent chain, outcome `CREATED`.
10. On any exception: remove every staging directory and re-raise.
11. Return outcomes per palette. `rendered_palettes` = palettes with tile count > 0.

#### 6.8.1 Completion marker (`.render-complete-v2.json`)

Written as a single line of compact sorted-key JSON plus a trailing newline. Written to a temp file (`.{marker}.tmp-{uuid}`), fsynced, then hard-linked into place (create-only). If the link fails with exists, the existing bytes must equal the new bytes, else `PyramidIdentityConflict`. Document keys:
- `schema_version`: 2
- `tile_count`: integer, number of `.png` files
- `tile_path_digest`: sha256 hex of `"\0".join(sorted(relative_paths))` (UTF-8). Relative paths use `/`, e.g. `4/3/5.png`.
- `tile_content_digest`: sha256 over, for each relative path in sorted order: 4-byte big-endian length of the path bytes, the path bytes, 8-byte big-endian file size, the file bytes.
- `identity`: object with exactly these 10 string fields (sorted): `renderer`, `algorithm`, `source_id`, `source_digest`, `grid_spec_digest`, `palette_name`, `palette_digest`, `tile_spec_digest`, `semantic_digest`, `policy_digest`. Any missing or extra field, or any empty value, is invalid.

#### 6.8.2 Identity values

Algorithm names:
- `rgba-bilinear-v1`: legacy (both continuous and categorical).
- `physical-bilinear-classify-v1`: indexed continuous.
- `category-nearest-v1`: indexed categorical.

Cross-renderer compatibility (`is_renderer_compatible_with`): two identities are compatible only if their (renderer, algorithm) pairs are `{legacy/rgba-bilinear-v1, indexed/physical-bilinear-classify-v1}` or `{legacy/rgba-bilinear-v1, indexed/category-nearest-v1}`, the renderers differ, and these fields match: `source_id`, `source_digest`, `grid_spec_digest`, `palette_name`, `palette_digest`, `tile_spec_digest`, `semantic_digest`. (`policy_digest` is excluded on purpose.)

Field values for the legacy continuous render (MRMS and HRRR):
- `renderer` = `legacy`; `algorithm` = `rgba-bilinear-v1`.
- `source_id`: MRMS `mrms:{layer}:{valid}`; HRRR `hrrr:{layer}:runs/{run}/{valid}`.
- `source_digest`: sha256 over a JSON header `{"dtype": "<f4", "shape": [h, w]}` (compact, sorted keys), then the raw C-order bytes of the float32 source grid. Non-contiguous arrays are hashed row-chunk by row-chunk; the bytes are the same.
- `grid_spec_digest` (regular source): stable digest of `{kind: "regular", height, width, lat_bounds: [min, max], lon_bounds: [min, max], coordinate_identity: [fingerprint(lats), fingerprint(lons)]}`. Projected source: `kind: "projected"` and `coordinate_identity: [crs_string, fingerprint(source_x), fingerprint(source_y)]`. A fingerprint is `[shape, dtype_str, blake2b-16 of the raw bytes]`.
- `palette_digest`: stable digest of the palette's table.
- `tile_spec_digest`: stable digest of `{"tile_size": 256, "zoom_levels": [...]}`.
- `semantic_digest`: stable digest of `{"kind": "continuous", "nodata_value": null, "min_valid_weight": 1.0}`, with `"overview": "max"` added for MRMS.
- `policy_digest`: stable digest of `{"sampling": "bilinear-rgba", "category_map": null, "png_compress_level": 1}`, with `"overview": "max"` added for MRMS.
- Categorical legacy: `semantic` is `{"kind": "categorical", "category_map": {...}}`; `policy` is `{"sampling": "bilinear-rgba", "category_map": {...}, "png_compress_level": 1}`.

Stable digest: `sha256(json.dumps(stable_value(v), separators=(",", ":"), sort_keys=True))`. `stable_value`: arrays become `{array_digest, dtype, shape}`; numpy scalars become Python scalars; bytes become `{"bytes": hex}`; mappings get sorted string keys; tuples and lists become lists; non-finite floats become `{"float": repr}`; anything else `str()`.

Interoperability note: the Rust renderer will produce the same pixels but not necessarily the same PNG bytes (encoder filter choice, zlib build). The marker's content digest is file-byte based, so a Rust-written pyramid is self-consistent. A Rust worker cannot adopt a Python pyramid by byte comparison (section 17.4).

---

## 7. Grid dumps (`backend/shared/grid_dump.py`)

Purpose: point-inspect and nowcast input. Each dump is a float32 raster plus a JSON pointer.

### 7.1 Layout

- Data: `{GRID_DIR}/{layer}/{key}.{gen}.bin`, `gen` = uuid4 hex (32 chars), immutable once written.
- Commit pointer: `{GRID_DIR}/{layer}/{key}.meta.json`.
- Lock: `{GRID_DIR}/{layer}/.grid.lock` (exclusive flock). Held for commit and for retention.
- `key` = valid time string (no run scoping in this pipeline; the `grid_key` parameter is unused here).

### 7.2 `write_grid(layer, timestamp, data, lats, lons, unit, fill=NaN, max_cells=None)`

Procedure:
1. Reject (return None silently) if `data` is not 2-D, `lats`/`lons` not 1-D, or shapes disagree.
2. If `lats[0] < lats[-1]` (ascending): flip `data` vertically and reverse `lats`. Result: rows north to south (descending lat).
3. If `max(lons) > 180`: `lons -= 360` where `> 180`, then sort columns by lon ascending (`data = data[:, argsort(lons)]`).
4. Stride downsample: `cell_limit = max(1, max_cells or GRID_MAX_CELLS)`. `stride = 1`; while `(h // stride) * (w // stride) > cell_limit`: `stride *= 2`. (`h`, `w` are the original dimensions.) If stride > 1: `data[::s, ::s]`, `lats[::s]`, `lons[::s]`.
5. `arr = contiguous float32`. NaN -> -9999.0 (when `fill` is NaN, which is the default). `fill_val = -9999.0`.
6. Metadata (sorted-key compact JSON): `height`, `width`, `lat_min`, `lat_max`, `lon_min`, `lon_max` (floats of the post-stride arrays), `unit`, `fill`, `stride`, `data_file` (bin file name).
7. Under the layer lock: create the layer directory durably; write the bin (C-order, little-endian float32 as written by the host; x86 in practice), flush, fsync; fsync the directory; write meta to a temp file in the same directory, flush, fsync, `os.replace` to `{key}.meta.json`; fsync the directory.
8. Return the bin path.

Effective strides in this pipeline (derived from the formula):
- MRMS `radar` and `radar-composite` at the default cap 810000: stride 8 for the 7000 x 3500 grid (875 x 437 cells).
- MRMS `radar-nowcast-input` at cap 7000000: stride 2 (3500 x 1750 cells, 24.5 MB float32).
- HRRR regular dumps: estimated stride 2 over CONUS at 0.03 degree (verify on a real grid).

### 7.3 Retention

- `GRID_KEEP_LAST` = `{ "radar-nowcast-input": (NOWCAST_INPUT_FRAMES - 1) * max(1, ceil(NOWCAST_INPUT_STEP_MIN / 1.8)) + 3 }`. With defaults (4, 5): `(4-1)*3 + 3 = 12`.
- `prune_grid_layer(layer)` (called after each nowcast-input write): keep the newest `keep` meta files by name (ISO names sort lexically in time order), plus the data files they reference. Remove other meta files. Remove `.bin` files not referenced by a kept meta only if older than `GRID_ORPHAN_GRACE_S` (300 s by mtime). Skip the lock file and `.tmp` files.
- `cleanup_old_grids()` (called from every cleanup activity): for every layer directory, apply `GRID_KEEP_LAST` if present, then remove any entry (except the lock) with mtime older than `GRID_MAX_AGE_S` (43200 s). Errors on a layer directory (for example `lost+found`) skip that directory.

### 7.4 Grids written by this pipeline

| Pipeline | Layer | Unit | When | Dims or notes |
|---|---|---|---|---|
| MRMS base | `radar` | dBZ | after all palettes render, every frame | default cap, stride 8 |
| MRMS base | `radar-nowcast-input` | dBZ | after `radar` grid, not guarded (section 12.1) | cap 7000000, keep 12 |
| MRMS composite | `radar-composite` | dBZ | after all palettes render | default cap |
| HRRR | `radar-hrrr` | dBZ | per hour, if any radar palette rendered | regular 0.03 deg |
| HRRR | `temperature` | degF | per hour, if any palette rendered | regular 0.03 deg |
| HRRR | `cape` | J/kg | per hour, if any palette rendered | regular 0.03 deg |
| HRRR | `wind` | mph | per hour, if the wind layer rendered | speed, regular 0.03 deg |
| HRRR | `wind_u`, `wind_v` | mph | with `wind` | earth-relative, rotated, regular 0.03 deg |
| HRRR | `precip-accum` | in | per hour, if rendered | regular 0.03 deg |
| HRRR | `cloud` | % | per hour, if rendered | regular 0.03 deg |

HRRR grid dumps are not written for `dewpoint`, `humidity`, `precip-type` (no dump call exists for them).

HRRR dumps are keyed by valid time and overwritten by pointer swap when a newer run covers the same valid time. They are mutable, not run-scoped (D18). MRMS grid keys are also valid times.

### 7.5 HRRR dump resampling (`_dump_fields`, `_latlon_resampler`)

- Projected source (always for HRRR): target axes
  - `lats_t = arange(nanmin(lat2d), nanmax(lat2d), step)`
  - `lons_t = arange(nanmin(lon2d), nanmax(lon2d), step)` (end excluded)
  - `step = HRRR_GRID_DUMP_STEP_DEG` (0.03).
- Forward-project the target mesh to source x, y; map through `source_x`, `source_y` with `_axis_to_fractional_indices` to fractional `(rows, cols)`.
- `theta` as in section 5.5.
- Resample each field with `scipy.ndimage.map_coordinates(float32, [rows, cols], order=1, mode="constant", cval=NaN)`, output float32.
- `write_grid(layer, valid, resampled, lats_t, lons_t, unit=unit)`.
- Resampler cache: single entry, keyed by (CRS string, step, source lengths, first and last source_x and source_y). Cache only; no output effect.
- All dump failures are logged (`grid_dump_failed`) and do not fail the hour.

---

## 8. Storm cells (`backend/shared/storms.py`, `write_storms_json`)

Only the MRMS base layer (`radar`) writes `storms.json`, using the full-resolution decoded grid (post-flip, lats ascending).

### 8.1 Writer

- Path `{STATE_DIR}/storms.json`; lock `{STATE_DIR}/storms.lock` (exclusive flock across read, compare, write).
- Read the previous file (missing or invalid -> None).
- If both the current and previous timestamps parse and `current <= previous`: return (monotonic guard; backfilled frames never move the file backwards). Parse failure of either timestamp: write anyway.
- `payload = detect_storms(data, lats, lons, timestamp, previous)`.
- Write compact JSON with a trailing newline to a temp file in `STATE_DIR` and `os.replace`. No fsync.

### 8.2 Detection

- `mask = data >= STORM_THRESHOLD_DBZ` (NaN is false).
- `labels, n = ndimage.label(mask)` with default structure (4-connectivity in 2-D).
- If `n == 0`: return `{type: "FeatureCollection", features: [], generated_at, timestamp, prediction_minutes: [0,5,10]}`. This is the only output without `threshold_dbz`, `cell_count` and `tracking_vectors` (D16).
- Per component `i` (1-based label):
  - `size` = pixel count; `peak` = max of `data` in the component; `(row, col)` = mean row and column of its pixels (`center_of_mass` on the mask); bounding slices from `find_objects`.
  - Skip if `size < STORM_MIN_PIXELS` or no slice.
  - Coordinates: `lat = lats[0] + (lats[-1] - lats[0]) * (row / max(1, h-1))`; `lon = lons_n[0] + (lons_n[-1] - lons_n[0]) * (col / max(1, w-1))`, where `lons_n` is lons normalized to -180..180.
  - Area: `size * (|lat[1]-lat[0]| * 110.574) * (|lon[1]-lon[0]| * 111.320 * cos(radians(lat)))` km2.
  - `bbox` (component): `[west, south, east, north]` from `lats[row_start]`, `lats[row_stop - 1]` (min/max), `lons_n[col_start]`, `lons_n[col_stop - 1]` (min/max), padded by `STORM_PREFETCH_PADDING_KM` (12): lat pad `12 / 110.574`; lon pad `12 / (111.320 * cos(radians(center_lat)))` with `center_lat` = mid of the south and north values, the cosine floored at 1e-6 scale. Bbox values are not rounded.
- Sort features by `peak` descending (stable). Keep the first `STORM_MAX_CELLS` (500).
- Output feature: `{type: "Feature", geometry: {type: "Point", coordinates: [lon, lat]}, properties: {peak_dbz, area_km2, pixel_count, bbox}}`, then tracking adds more properties.

### 8.3 Tracking (`_attach_tracking`)

Inputs: current features, previous payload's features and timestamp, current timestamp.
- `elapsed_min` = current minus previous timestamp (minutes); 0 if either missing.
- `now_s` = current epoch seconds.
- `previous_ids` from previous features' `properties.cell_id`; `next_id = max(previous_ids, default 0) + 1`.
- Association runs only if there are current and previous features and `0 < elapsed_min <= 15`:
  - Predicted positions: each previous feature advanced by its previous motion `(east_kmh, north_kmh)` over `elapsed_min` (`_advance`: lon += east*h/(111.32*cos(lat)), lat += north*h/110.574, with h = elapsed/60, cos floored at 1e-6 scale).
  - Cost matrix: haversine km from each current centroid to each predicted position. Missing predictions cost 1e9.
  - Hungarian assignment (`linear_sum_assignment`).
  - Gate: `base_gate = max(STORM_MATCH_SLACK_KM (8), STORM_MAX_TRACK_SPEED_KMH (160) * elapsed_min / 60 * 0.5)`. Accept a pair if `dist <= max(base_gate, 0.5 * sqrt(area_km2))`.
- For each current feature in order:
  - Matched: `cell_id` = previous cell_id; `history` = previous `track_history` entries that are 3-element lists with `now_s - t <= STORM_TRACK_HISTORY_MIN * 60`.
  - Unmatched: `cell_id = next_id`, `next_id += 1`; `history = []`.
  - Append `[round(now_s, 1), round(lon, 5), round(lat, 5)]`; store as `track_history`.
  - Motion: if `len(history) >= 2` and `span_min = (last_t - first_t)/60 >= STORM_MIN_TRACK_SPAN_MIN` (5): least-squares fit (`_fit_motion`) over samples, with positions converted to km relative to the last sample (`x = (lon - lon_last) * 111.32 * cos(lat_last)`, `y = (lat - lat_last) * 110.574`) and time in hours relative to the mean. Returns `(east_kmh, north_kmh, rms_km)`. `confidence = min(1, span_min/15) / (1 + rms_km/3)`.
  - Else `east = north = 0`, `confidence = 0`.
  - `speed = hypot(east, north)`. If `speed > 160`: zero the vector, `speed = 0`, `confidence = 0`.
  - `bearing = (degrees(atan2(east, north)) + 360) % 360` if speed else 0 (clockwise from north).
  - Properties: `cell_id`; `track_history`; `tracking_vector = {east_kmh, north_kmh, speed_kmh, bearing_deg}` rounded to 3, 3, 3, 2 decimals; `tracking_confidence` (2 decimals); `predicted_bboxes`: for lead in `[0, 5, 10]`: `{lead_minutes, bbox}` where bbox = component bbox translated by `east*h/(111.32*cos(lat))` and `north*h/110.574` with `h = lead/60`, rounded to 6 decimals.

Top-level fields added to the non-empty payload: `threshold_dbz` (float), `cell_count` (= features length), `prediction_minutes` `[0, 5, 10]`, `tracking_vectors` = `[{cell_id, east_kmh, north_kmh, speed_kmh, bearing_deg}, ...]` in feature order. `generated_at` = epoch seconds (float, `time.time()` at detection).

Storm writes are best-effort in the frame activity (section 12.1): an exception is logged `storm_detect_failed` and does not fail the frame.

---

## 9. State files: manifest and ProcessedSet

### 9.1 `manifest.json` (`backend/shared/manifest.py`)

Path `{STATE_DIR}/manifest.json`. Lock `{STATE_DIR}/manifest.lock` (exclusive flock held across read-modify-write). The API serves this file directly.

Top level:
- `schema_version`: 2
- `layers`: object keyed by layer name
- `tile_url_template`: `/tiles/{layer}/{palette}/{path}/{z}/{x}/{y}.png`
- `tile_url_template_legacy`: `/tiles/{layer}/{timestamp}/{z}/{x}/{y}.png`
- `updated_at`: `datetime.now(UTC).isoformat()` (microseconds included when non-zero)

Write: sorted-key compact JSON plus `\n`, temp file in the same directory, `os.replace`. No fsync (D11).

Read (`read_manifest_file`):
- Missing file -> empty manifest (`layers: {}`, current timestamps).
- Any other read or parse error raises.
- Non-object or non-dict `layers` -> empty manifest.
- Sets defaults for `tile_url_template`, `tile_url_template_legacy`, `updated_at`, `schema_version` (1 if absent).
- Legacy layers without a `frames` list get frames built from `timestamps` with `palettes` from the layer (default `["classic"]`).

Layer object:
- `frames`: list of frame objects, unique by `timestamp`, sorted by `timestamp` (string sort, last supplied frame wins for a duplicate).
- `timestamps`: the same timestamps, in the same order.
- `latest`: last frame's timestamp (absent when there are no frames).
- `palettes`: sorted intersection of every frame's `palettes` (empty list when no frames).
- Metadata keys merged in from `layer_metadata` (MRMS: `title`, `kind`; HRRR: `title`, `kind`, `run_id`, `complete`).

Frame object:
- Always: `timestamp` (valid time string), `path` (defaults to `timestamp`), `palettes` (sorted unique strings).
- Plus whatever the writer supplied in `frame`: MRMS: `source`, `kind`, `issued_at`, `lead_minutes`, `spatial_resolution_km`, `max_zoom`. HRRR: `source`, `kind`, `issued_at`, `run_id`, `lead_minutes`, `spatial_resolution_km`, `max_zoom`.
- Palette rule: a non-empty `palettes` argument overrides the frame's own `palettes` for every frame it writes.

Operations:
- `update_manifest_file(layer, ts, palettes, action="add", frame, layer_metadata)`:
  - `add`: create the layer if absent (`timestamps: [], frames: [], palettes: []`), drop any frame with the same timestamp, append the new frame, merge metadata, reindex.
  - `remove`: drop the frame with that timestamp; reindex; if no frames remain, delete the layer.
  - Any other action raises.
- `replace_layer_manifest(layer, timestamps, palettes, frames, layer_metadata)` (HRRR): if `timestamps` is non-empty, the layer's frames are replaced entirely by `sorted(set(timestamps))` (each frame built from `frames` where supplied); metadata merged; reindex. If `timestamps` is empty, the layer is deleted.
- Both: `schema_version` set to 2, `updated_at` set to now, written atomically under the lock.

### 9.2 ProcessedSet (`backend/shared/state.py`)

Files:
- MRMS: `{STATE_DIR}/ingest-mrms-{layer}.json` (for example `ingest-mrms-radar.json`), `max_entries = 2000`. Keys are full S3 object keys.
- HRRR: `{STATE_DIR}/ingest-hrrr.json`, `max_entries = 200`. Keys are run IDs.

Format: `{"items": [..]}` compact JSON, oldest first.

Behavior:
- Load: missing file -> empty. Invalid JSON or OSError -> silently empty (D23). A bare list is also accepted as the item list.
- `add(key)`: no-op if present. Else append, and if the length exceeds `max_entries`, drop the oldest entries. Then flush: temp file in the same directory, `os.replace`. No fsync, no lock. Concurrency relies on SKIP plus one writer per file.
- `contains(key)`: membership in the loaded set. The set is loaded fresh on each activity call; nothing is cached across activities.

---

## 10. Disk layout

```
{TILE_DIR}/                                  (default /data/tiles)
  radar/                                     MRMS base
    classic|muted|vivid/
      2026-09-30T00:00:00+00:00/             immutable pyramid
        {z}/{x}/{y}.png                      z in {4,5,6,7}
        .render-complete-v2.json             completion marker
      .render-publish-locks/00.lock .. 3f.lock
      .2026-09-30T00:00:00+00:00.tmp-<hex>/  staging (transient)
  radar-composite/                           MRMS composite, same shape
  radar-hrrr/                                HRRR reflectivity
    classic|muted|vivid/
      runs/20260715_12/2026-07-15T13:00:00+00:00/{z}/{x}/{y}.png   z in {4,5,6}
  temperature|cape|wind|precip-accum|cloud|precip-type|dewpoint|humidity/   (HRRR, when enabled)
    <palette>/runs/<run>/<valid>/...
  .render-publish-locks/ (under each {TILE_DIR}/{layer})

{GRID_DIR}/                                  (default /data/grids)
  radar/<valid>.meta.json, radar/<valid>.<gen32>.bin, radar/.grid.lock
  radar-composite/...
  radar-nowcast-input/...                    keep newest 12
  radar-hrrr/, temperature/, cape/, wind/ (+ wind_u, wind_v), precip-accum/, cloud/

{STATE_DIR}/                                 (default /data/state)
  manifest.json, manifest.lock
  ingest-mrms-radar.json, ingest-mrms-radar-composite.json
  ingest-hrrr.json
  storms.json, storms.lock

{MRMS_TMP_ROOT}/ (/tmp/mrms_work)            per-attempt scratch, removed in finally
  mrms-<wf>-<run>-<act>-attempt<N>-<layer>-<hint>/
{HRRR_TMP_ROOT}/ (/tmp/hrrr_work)
  hrrr-<wf>-<run>-<act>-attempt<N>-<run_id>-f<FF>/
```

Scratch directory names: each part is sanitized by replacing every character that is not alphanumeric, `-` or `_` with `_`, stripping leading and trailing `_`, and substituting `unknown` for an empty result. Parts are joined with `-`. The MRMS hint is the key basename without `.grib2.gz`; `.` becomes `_`. Example from the tests: workflow `sched-ingest-hrrr-2026-05-06T02:00:00Z` (colons become `_`), run `run-one`, activity `3`, attempt 1, parts `20260506_02`, `f01`, gives `hrrr-sched_ingest_hrrr-run_one-3-attempt1-20260506_02-f01`.

---

## 11. Workflow logic

Both workflows are deterministic: no clock, randomness or I/O in workflow code. All I/O is in activities.

### 11.1 IngestMrmsWorkflow.run(args)

Constants: `RETENTION_HOURS = 4`, `FRAME_CONCURRENCY = 2`.

1. `args = args or IngestMrmsArgs()`.
2. `listing = execute mrms_list_unprocessed_keys(args)` with `_LIST_RETRY`, start_to_close 30 s. Failure after retries fails the workflow (no cleanup).
3. If `listing.keys` is empty:
   - log `no new <layer> frames; backlog=<n>`;
   - `cleanup = execute mrms_cleanup(layer, 4)`;
   - return `IngestMrmsResult(layer, backlog_total, rendered_count=0, cleanup)`.
4. Otherwise, a semaphore of 2 and a concurrent fan-out over all keys (`asyncio.gather`, results in input order). Each key:
   - acquire the semaphore;
   - `execute mrms_process_frame(ProcessFrameInput(key, layer))` with `_FRAME_RETRY`, start_to_close 4 min, schedule_to_close 5 min, heartbeat_timeout 60 s;
   - on `ActivityError` only: log `frame failed after retries: <key>` and return `ProcessFrameResult(key, rendered=False)`. Any other exception propagates and cancels the gather.
5. Sequential loop over results in listing order:
   - if `rendered`: `execute mrms_mark_processed(MarkProcessedInput(key, layer))` with `_DEFAULT_RETRY`, start_to_close 30 s; increment `rendered`;
   - else log `frame skipped: <key>`.
   - A failure of the mark activity (after retries) is not caught: the workflow fails and no cleanup runs. Frames already rendered stay published and are re-rendered idempotently next time (section 12.1).
6. `cleanup = execute mrms_cleanup(layer, 4)` with `_DEFAULT_RETRY`, start_to_close 5 min.
7. Return `IngestMrmsResult(layer, backlog_total = listing.backlog_total, rendered_count = rendered, cleanup)`.

Per-item isolation: only `ActivityError` is isolated. A rendered=false frame stays unmarked, so a later run can retry it while it is still among the newest `BACKLOG_PER_CYCLE` unprocessed keys.

### 11.2 IngestHrrrWorkflow.run()

Constants: `RETENTION_HOURS = 12`, `FORECAST_CONCURRENCY = 4`, `HOUR_START_TO_CLOSE = 8 min`, `HOUR_SCHEDULE_TO_CLOSE = 12 min`, `HOUR_HEARTBEAT_TIMEOUT = 90 s`.

1. `find = execute hrrr_find_latest_run()` with `_DEFAULT_RETRY`, start_to_close 120 s.
2. If `find.run_id` is None:
   - log `no HRRR run available`;
   - cleanup; return `IngestHrrrResult(run_id=None, skipped=False, hours=0, cleanup)`.
3. If `find.already_processed`:
   - log;
   - cleanup; return `IngestHrrrResult(run_id, skipped=True, hours=0, cleanup)`.
4. `horizon = execute hrrr_horizon_for_run(run_id)`, start_to_close 10 s.
5. Semaphore of 4. For `fhr` in `1..horizon`, concurrently (gather, order preserved):
   - `execute hrrr_process_forecast_hour(run_id, fhr)` with `_FORECAST_RETRY`, start_to_close 8 min, schedule_to_close 12 min, heartbeat_timeout 90 s;
   - on `ActivityError` only: log `HRRR f%02d failed after retries for run <run> — continuing without it` and return `ForecastHourResult(fhr=fhr)` (no layers).
6. `results` sorted by `fhr`. `layers_per_hour = [r.rendered_layers for r in results]`.
7. `prefix = len(publishable_prefix(results))`; `complete = (prefix == horizon)`.
8. If `prefix == 0`:
   - log that f01 is pending (not publishing);
   - cleanup; return `IngestHrrrResult(run_id, skipped=False, hours=0, cleanup, layers_per_hour)`.
9. `published_layers = execute hrrr_publish_run(run_id, results)` with `_DEFAULT_RETRY`, start_to_close 2 min. Note: all `results` are passed; the activity recomputes the prefix. Failure after retries is not caught (fails the workflow).
10. If `"radar-hrrr"` is not in `published_layers`:
    - log error `failed coherent publication; not marking processed`;
    - cleanup; return `IngestHrrrResult(run_id, skipped=False, hours=0, cleanup, layers_per_hour)`.
11. If `complete`: `execute hrrr_mark_processed(run_id)` with `_DEFAULT_RETRY`, start_to_close 30 s. Else log `published partially: <prefix>/<horizon> hours; will extend on the next fire`.
12. `cleanup = execute hrrr_cleanup(12)` with `_DEFAULT_RETRY`, start_to_close 5 min.
13. Return `IngestHrrrResult(run_id, skipped=False, hours=prefix, cleanup, layers_per_hour, published_layers, complete)`.

Note the mismatch in step 8 and 10 returns: `forecast_hours_processed` is 0 there, but `prefix` in step 13 (D25).

### 11.3 Publication coherence rules (HRRR)

- R1 (prefix): only the consecutive run `f01, f02, ...` in which every hour has `radar-hrrr` is ever advertised. The first gap ends the prefix; later rendered hours are not advertised.
- R2 (layer intersection): a layer is advertised only if every hour in the prefix has that layer rendered. Advertised layers are `sorted(intersection)`.
- R3 (complete): `complete = len(prefix) == horizon`. Partial runs are advertised with `complete: false` in layer metadata. The run is marked processed only when complete, so the next 15-minute fire re-polls it.
- R4 (incomplete run precedence): while a partially published run is younger than 3 h and still incomplete, it is resumed in preference to a newer run (section 12.2, `hrrr_find_latest_run`).
- R5 (extended tail): an older extended run (hour in {0, 6, 12, 18}) whose frames are valid after this run's last valid time, and whose init is strictly earlier and at most 12 h before this run's init, keeps its frames in this layer's manifest after this run's frames (`_extended_tail`).
- R6 (per-layer replacement): each advertised layer is replaced in the manifest with one `replace_layer_manifest` call, in sorted layer order. The manifest write is atomic per layer, not across layers.
- R7 (immutable tiles): tile paths include the run, so a newer run never overwrites an older run's tiles. Older run directories stay on disk while any frame references them (section 12.2, cleanup).
- R8 (grids): HRRR grid dumps are not run-scoped and are overwritten by pointer swap (section 7.4).

---

## 12. Activity logic

### 12.1 MRMS activities

**`mrms_list_unprocessed_keys(args)`**
1. `keys = _list_recent_files_sync(args.mrms_prefix)` (today's UTC prefix; yesterday's only if today is empty; sorted ascending).
2. `state = ProcessedSet(MRMS state file for args.layer_name, max_entries=2000)`.
3. `new_keys = [k for k in keys if k not in state]` (ascending order kept).
4. `ordered = list(reversed(new_keys))[:BACKLOG_PER_CYCLE]` (newest first).
5. Return `ListKeysResult(keys=ordered, backlog_total=len(new_keys))`. No writes.

**`mrms_process_frame(inp)`** (inp: `key`, `layer_name`)
1. `palette_tables = _load_palette_tables()` (thread).
2. `timestamp_hint` = key basename without `.grib2.gz`. `tmp_dir = _current_activity_tmp_dir("mrms", layer, timestamp_hint)` (created).
3. Heartbeat `{"phase":"download","key":key}`.
4. Download and decode (thread; heartbeat every 20 s with `{"phase":"download","key":key}`):
   - `GET {base}/{key}`, timeout 60 s, `raise_for_status`;
   - write `mrms.grib2.gz`, gunzip to `mrms.grib2`;
   - pygrib open, message 1, `values`, `lats`, `lons` (section 5.2), lon normalized;
   - delete both temp files in `finally`.
5. `flip = lats[0] > lats[-1]`. If flip: `lats = lats[::-1]`, `grid = flipud(data)`. Else `grid = data`.
6. `timestamp = _extract_timestamp(key)` (fails here if the key does not parse).
7. Heartbeat `{"phase":"render",...}`. Render (thread; heartbeat every 30 s):
   - `render_frame_palettes(grid, lats, lons, palette_tables_reflectivity, output_dirs = {TILE_DIR}/{layer}/{palette}/{timestamp}, zooms=[4,5,6,7], renderer=tile_renderer_for_role("mrms"), overview="max", source_id="mrms:{layer}:{timestamp}", publication_lock_root={TILE_DIR}/{layer}, nodata_value=None, min_valid_weight=1.0)`.
   - Output: `rendered_palettes` (palettes with tile count > 0).
   - `finally`: remove `tmp_dir`.
8. If `set(rendered) != set(all palettes)`:
   - log ERROR `frame_incomplete` (layer, timestamp, expected, rendered);
   - return `ProcessFrameResult(key, timestamp, rendered=False, palettes=sorted(rendered), duration_s)`.
   - No grids, storms, nowcast kick or manifest update. Tiles of palettes that did render stay (immutable winners).
   - This is the path for an all-transparent frame (D2).
9. `_grids()` (thread):
   - `write_grid(layer, timestamp, grid, lats, lons, unit="dBZ")`; failure logs `grid_dump_failed`;
   - if `layer == "radar"`: `write_storms_json(STATE_DIR, grid, lats, lons, timestamp)`; failure logs `storm_detect_failed`.
10. If `layer == "radar"`:
    - `write_grid("radar-nowcast-input", timestamp, grid, lats, lons, "dBZ", max_cells=7000000)`. **Not guarded**: an exception fails the activity (D12). Retry converges because tiles are already immutable.
    - `prune_grid_layer("radar-nowcast-input")` (keep 12).
    - `kick_nowcast(timestamp)` (section 12.1.1). Never raises.
11. `update_manifest_file(layer, timestamp, palettes=rendered, action="add", frame={path: timestamp, source: "mrms", kind: "observation", issued_at: timestamp, lead_minutes: 0, spatial_resolution_km: 1.0, max_zoom: 7}, layer_metadata={title: "MRMS observed reflectivity", kind: "observation"})`. The frame is advertised only here.
12. Log `frame_done`. Return `ProcessFrameResult(key, timestamp, rendered=True, palettes=rendered, duration_s=round(elapsed, 2))`.
- Exception handling: the tmp dir is removed in `finally` on success, failure and cancellation alike.
- Ordering invariant: tiles -> grids and storms -> nowcast kick -> manifest.

**12.1.1 `kick_nowcast(timestamp)`**
- If `NOWCAST_KICK_ENABLED != "1"`: return False without contacting Temporal.
- Otherwise: get a Temporal client; `get_schedule_handle(NOWCAST_SCHEDULE_ID).trigger(overlap=BUFFER_ONE, rpc_timeout=NOWCAST_KICK_TIMEOUT_S seconds)`.
- Any exception: log `nowcast_kick_failed` (timestamp, schedule_id, err), return False.
- Success: log `nowcast_kicked`, return True.

**`mrms_mark_processed(inp)`**: `ProcessedSet(MRMS state file for layer).add(key)`.

**`mrms_cleanup(inp)`** (layer, retention_hours)
1. `cutoff = now - retention_hours * 3600`.
2. If `{TILE_DIR}/{layer}` exists: candidates are the timestamp directories under each palette directory (a directory whose name starts with a digit is a legacy timestamp directory and is a candidate itself).
3. For each candidate whose name parses with `datetime.fromisoformat` and whose timestamp is `< cutoff`: `update_manifest_file(layer, name, action="remove")` first, then remove the directory. Count removals.
4. `grids_removed = cleanup_old_grids()` (all grid layers).
5. Return `CleanupResult(tile_dirs_removed, grid_files_removed)`.

### 12.2 HRRR activities

**`hrrr_find_latest_run()`**
1. `latest = _find_latest_run_sync()` (section 4.2). If None: return `FindRunResult(None, False)`. No resume check (D8).
2. `resume = _resume_incomplete_run(read_manifest(STATE_DIR), now)`. Returns the `radar-hrrr` layer's `run_id` when all of these hold: the layer exists; `run_id` is present; `complete` is false (absent means complete); `run_id` parses as `YYYYMMDD_HH`; `now - init <= 3 h`.
3. If `resume` is not None and `resume != latest`: log `resuming_incomplete_run`; `run = resume`. Otherwise `run = latest`.
4. `already_processed = run in ProcessedSet(ingest-hrrr.json, 200)`.
5. Return `FindRunResult(run, already_processed)`.

**`hrrr_horizon_for_run(run_id)`**: `EXTENDED_FORECAST_HOURS` (48) if the run hour is in `{0, 6, 12, 18}`, else `FORECAST_HOURS` (18).

**`hrrr_process_forecast_hour(run_id, fhr)`**
1. `palette_tables` = full dicts. `needed = _needed_vars()`: sorted unique union of `LAYER_VARS` over enabled layers (`radar-hrrr` is always enabled). `valid = run_init + fhr h` as a valid time string.
2. Resume check: `existing = _existing_rendered_layers(TILE_DIR, "runs/{run}/{valid}", palette_tables)`. For each layer with a color key present in some palette, it counts as existing only if every palette's pyramid at that path is complete and its identity passes `frame_pyramid_identity_is_compatible` (same renderer contract, same zooms, same palette digest, etc.). The check ignores `HRRR_ENABLED_LAYERS`.
3. If `required ⊆ existing`, where `required = {enabled layers with a color key in some palette}`: log `hour_resumed`; return `ForecastHourResult(fhr, rendered_layers=existing, duration_s, valid_timestamp=valid)`. No download, no temp dir.
4. `tmp_dir = _current_activity_tmp_dir("hrrr", run_id, "f%02d" % fhr)`. Heartbeat `{"phase":"download","fhr":fhr}`.
5. Download subset (thread; heartbeat every 30 s; section 4.2). Returns a path or None.
6. If None or the file does not exist: remove `tmp_dir`; return `ForecastHourResult(fhr, [], duration, valid)` (pending).
7. Render (thread; heartbeat every 30 s): `_process_forecast_hour_sync(grib, run_id, fhr, palette_tables, TILE_DIR)`.
   - On exception: remove `tmp_dir`, re-raise (this is the retry path).
   - On success: unlink the GRIB, remove `tmp_dir`.
8. Log `hour_done`. Return `ForecastHourResult(fhr, rendered, duration, valid)`.

Gap (D3): if the download (step 5) raises a non-404 error, `tmp_dir` is not removed on that path.

**`_process_forecast_hour_sync(grib, run_id, fhr, palettes, tile_base)`** returns the rendered layer list (in this order). `ts` = valid time; `path` = `runs/{run_id}/{ts}`.
1. `refc = _extract_variable(refc)`. If None: `RuntimeError("required HRRR variable refc is missing from downloaded GRIB")`.
2. `radar-hrrr`: `_render_per_palette(... "reflectivity")`. If rendered: append `radar-hrrr`; `_safe_grid_dump("radar-hrrr", ts, refc, "dBZ")`.
3. `temperature` (if enabled): `t2m`, convert to degF. Render with `temperature`. If rendered: append; dump `degF`.
4. `dewpoint` (if enabled AND some palette has the `dewpoint` key): `dpt2m` to degF. Render. Append if rendered. No dump. (Never runs with shipped palettes, D1.)
5. `humidity` (if enabled AND some palette has the `humidity` key): `rh2m`. Render. Append if rendered. No dump. (Never runs with shipped palettes, D1.)
6. `cape` (if enabled): render with `cape`. If rendered: append; dump `J/kg`.
7. `wind` (if enabled): `u10` and `v10` must both extract (else skip silently). `u_mph = u*2.237`, `v_mph = v*2.237`, `speed = sqrt(u_mph**2 + v_mph**2)`. Render `speed` with `wind_speed`, using the `u` grid's geometry. If rendered: append; dump `wind` speed (mph); `_safe_wind_dump(u_mph, v_mph)` writes `wind_u`, `wind_v` (earth-relative, rotated when grid-relative).
8. `precip-accum` (if enabled): `apcp / 25.4`. Render with `precip_accum`. If rendered: append; dump `in`.
9. `cloud` (if enabled): `tcdc`. Render with `cloud_cover`. If rendered: append; dump `%`.
10. `precip-type` (if enabled): extract `crain`, `csnow`, `cfrzr`, `cicep` (each optional); build the category grid (section 5.5). If any found: render categorical with `precip_type` and `PRECIP_TYPE_CATEGORIES`. If rendered: append. No dump.

A secondary variable that fails to extract does not fail the hour; only `refc` is required.

`_render_per_palette(tile_base, layer, ts, data, grid, palettes, color_key, categorical=False, categories_map=None)`:
- `color_tables = {name: tables[color_key] for palettes where the key is truthy}`. If empty, or categorical without a categories map: return [].
- `result = render_frame_palettes(data, lats, lons, color_tables, out_dirs={tile_base/layer/palette/path}, zooms=[4,5,6], category_map (if categorical), nodata_value=None, min_valid_weight=1.0, renderer=tile_renderer_for_role("hrrr"), source_id="hrrr:{layer}:{path}", publication_lock_root={tile_base}/{layer}, source_crs, source_x, source_y, lats, lons with the source's y flip applied)`.
- If `set(result.rendered_palettes) != set(color_tables)`: return []. This includes the case where every palette is empty (D2).
- Else return the rendered list.

Projected-grid orientation before rendering (`_write_palette_tiles`): if `source_y[0] > source_y[-1]`, flip the data vertically, flip `lats` and `lons` vertically, and reverse `source_y`. `source_x` is unchanged. Regular grids: flip when `lats[0] > lats[-1]`.

`_safe_grid_dump` and `_safe_wind_dump` (section 7.5) log failures and never raise.

**`publishable_prefix(results)`**: starting at `fhr = 1`, collect each result while `fhr` is present and `"radar-hrrr"` is in its `rendered_layers`; stop at the first miss. Results are keyed by `fhr`; order of input does not matter.

**`hrrr_publish_run(run_id, results)`**: load palette tables (full dicts), then `_publish_hrrr_run_sync(run_id, results, tables)`. Return the published layer list.

`_publish_hrrr_run_sync(run_id, results, palettes, state_dir=None, horizon=None)`:
1. `ordered = publishable_prefix(results)`. If empty: return [].
2. `horizon = horizon or _horizon_for_run(run_id)`; `complete = len(ordered) >= horizon`.
3. `complete_layers = intersection of rendered_layers over ordered`.
4. `run_issued_at = _run_init_time(run_id)` as valid time string.
5. For each `layer` in `sorted(complete_layers)`:
   - skip if no color key (never happens) or no palette has the key; `palettes_for_layer = sorted(names with the key)`;
   - `own_frames` = for each prefix result: `{timestamp: valid, path: "runs/{run}/{valid}", source: "hrrr", kind: "model_guidance", issued_at: run_issued_at, run_id, lead_minutes: fhr*60, spatial_resolution_km: 3.0, max_zoom: 6}`;
   - `frames = own_frames + _extended_tail(existing layer dict, run_id, own_frames[-1].timestamp)`;
   - `replace_layer_manifest(layer, [f.timestamp for f in frames], palettes=palettes_for_layer, frames=frames, layer_metadata={title: "HRRR simulated reflectivity" if layer == "radar-hrrr" else f"HRRR {layer}", kind: "model_guidance", run_id, complete})`;
   - append layer to `published`.
6. Return `published`.

`_extended_tail(layer_dict, run_id, last_valid)`: for each existing frame with a `run_id` (other than this run) whose `run_id` init hour is in `{0,6,12,18}`, init strictly earlier than this run, `run_init - frame_init <= 12 h`, and whose `timestamp` is strictly after `last_valid`: keep it. Frames that fail to parse are skipped.

`_referenced_runs(layer_dict)`: `{layer.run_id}` plus each frame's `run_id`.

**`hrrr_mark_processed(run_id)`**: `ProcessedSet(ingest-hrrr.json, 200).add(run_id)`.

**`hrrr_cleanup(retention_hours)`** (12 from the workflow)
1. `manifest = read_manifest(STATE_DIR)`; `cutoff = now - retention_hours * 3600`.
2. For each layer in `HRRR_TILE_LAYERS` (the nine layers): `live = _referenced_runs(manifest.layers[layer])`.
3. For each palette directory under `{TILE_DIR}/{layer}`:
   - if its name starts with a digit: treat it as a legacy timestamp directory (queue it);
   - else, for each `runs/<run>` directory: skip if `<run>` is live; parse `<run>` as `%Y%m%d_%H` UTC (skip on failure); if `init < cutoff`: remove it. Count.
   - then queue each other child directory of the palette directory (except `runs`) as a legacy candidate.
4. For each queued legacy candidate that parses as a valid time and is `< cutoff`: `update_manifest_file(layer, name, action="remove")` first, then remove. Count.
5. `grids_removed = cleanup_old_grids()`.
6. Return `HrrrCleanupResult(tile_dirs_removed, grid_files_removed)`.

---

## 13. Log event contract

Logs are one JSON line per event (`backend/shared/logger.py`). Event names and the listed fields are kept stable for dashboards and the debug harness. Level is the Python level.

| Event (`msg`) | Level | Where | Fields |
|---|---|---|---|
| `palette_missing` | warning | MRMS and HRRR `_load_palette_tables` | `palette` |
| `nowcast_kicked` | info | `kick_nowcast` | `timestamp`, `schedule_id` |
| `nowcast_kick_failed` | warning | `kick_nowcast` | `timestamp`, `schedule_id`, `err` |
| `grid_dump_failed` | warning | MRMS `_grids`; HRRR `_safe_grid_dump`, `_safe_wind_dump` | `err` (HRRR also `layer`) |
| `storm_detect_failed` | warning | MRMS `_grids` | `err` |
| `frame_incomplete` | error | MRMS frame | `layer`, `timestamp`, `expected_palettes`, `rendered_palettes` |
| `frame_done` | info | MRMS frame | `layer`, `timestamp`, `duration_s` |
| `resuming_incomplete_run` | info | HRRR find | `run_id`, `latest` |
| `projection_extract_failed` | warning | HRRR native projection | `err` |
| `extract_failed` | warning | HRRR `_extract_variable` | `err`, `match` |
| `hour_pending` | info | HRRR download (full-file 404) | `run_id`, `fhr` |
| `hour_resumed` | info | HRRR hour resume | `run_id`, `fhr`, `rendered` |
| `hour_done` | info | HRRR hour | `run_id`, `fhr`, `rendered`, `duration_s` |

Workflow `logger.info/warning/error` messages (Temporal workflow logger):
- `no new %s frames; backlog=%d`
- `frame failed after retries: %s` (warning); `frame skipped: %s` (warning)
- `no HRRR run available`; `HRRR run already processed: %s`; `HRRR run %s horizon=%dh`
- `HRRR f%02d failed after retries for run %s — continuing without it` (warning)
- `HRRR run %s has no publishable hours yet (f01 pending); not publishing`
- `HRRR run %s failed coherent publication; not marking processed` (error)
- `HRRR run %s published partially: %d/%d hours; will extend on the next fire`

Heartbeat detail keys are in section 2.4.

---

## 14. Test invariants (read from source; not executed)

Each item is an invariant the Python tests assert. The Rust port needs an equivalent test for each. The tests use stubs for `pygrib`, `httpx.MockTransport`, and monkeypatched activity functions; equivalents in Rust should use a decoded fixture or an injected trait.

### 14.1 `backend/ingest_mrms/test_activities.py`

- **M1** `_grid_axes` on a `regular_ll` header returns `lat` of length `Nj` (3500) and `lon` of length `Ni` (7000); `lat[0] == 54.995`, `lat[-1] == 20.005001`; `diff(lat) == -0.01` and `diff(lon) == +0.01` (atol 1e-6). `latlons()` must NOT be called for regular grids.
- **M2** `_grid_axes` on a non-regular grid falls back to `latlons()` and returns `lat = lats[:,0] == [30, 29]`, `lon = lons[0,:] == [250, 251]`.
- **M3** `kick_nowcast` with enabled kick triggers schedule `"nowcast"` exactly once, with kwargs exactly `{overlap: BUFFER_ONE, rpc_timeout: timedelta(NOWCAST_KICK_TIMEOUT_S)}`, and returns True.
- **M4** `kick_nowcast` where connecting raises returns False and does not raise.
- **M5** `kick_nowcast` with kick disabled returns False and never connects (the factory must not be called).
- **M6** A frame for layer `radar` (key `...20260930-000000...`) kicks once with timestamp `"2026-09-30T00:00:00+00:00"`.
- **M7** A frame for layer `radar-composite` never kicks.
- **M8** `mrms_process_frame` with one palette rendering returns `rendered == True` (flow: download stub, render stub, grid stubs, manifest stub).

### 14.2 `backend/ingest_hrrr/test_activities.py`

- **H1** The `refc` selector (`shortName=refc`, `typeOfLevel=atmosphere`) matches the first message; `grid.data` equals the input values exactly `[[10,20],[30,40]]` (no scaling); lats and lons come from `latlons()` of that message.
- **H2** `_process_forecast_hour_sync` with no `refc` raises `RuntimeError` whose message contains `required HRRR variable refc`.
- **H3** `_activity_tmp_dir` yields distinct paths for different attempts of the same activity, distinct paths for different activity IDs, and every path has `TMP_ROOT` as parent.
- **H4** `_activity_tmp_dir` sanitizes workflow IDs: `sched/ingest:hrrr` -> `sched_ingest_hrrr`; `run+one` -> `run_one`; the full name is exactly `hrrr-sched_ingest_hrrr-run_one-3-attempt1-20260506_02-f01`; no `/` in the name.
- **H5** `_publish_hrrr_run_sync` with hours `[2]` only (no f01) returns `[]` and does not create `manifest.json`.
- **H6** `_publish_hrrr_run_sync` with complete hours 1..2 returns `["radar-hrrr"]`; manifest layer has `complete: true`, `run_id: "20260715_12"`, frame paths `runs/20260715_12/2026-07-15T13:00:00+00:00` and `.../14:00:00+00:00`.
- **H7** Partial publication with hour 3 pending (`rendered_layers=[]`) and hour 4 rendered: advertised lead minutes are `[60, 120]`, `complete: false`, `run_id` set. After hour 3 is filled, lead minutes are `[60, 120, 180, 240]`, `complete: true`.
- **H8** `publishable_prefix`: `[]` for empty input; `[]` for `[fhr=2]`; for `[3(radar), 1(radar), 2(temperature only), 4(radar)]` returns `[1]`.
- **H9** `_horizon_for_run("20260715_12") == EXTENDED_FORECAST_HOURS` (48); `_horizon_for_run("20260715_13") == FORECAST_HOURS` (18).
- **H10** Index 404 and full file 404: `_download_subset_sync` returns None; `tmp_dir` is empty afterwards; exactly one `hour_pending` log with `run_id == "20260715_18"` and `fhr == 42`.
- **H11** Index 404 but full file 200: returns `tmp/hrrr_f07.grib2` whose bytes equal the response body.
- **H12** Index 503 and full file 503: raises `httpx.HTTPStatusError`.
- **H13** `_existing_rendered_layers` with two palettes: none complete -> `[]`; one complete -> `[]`; an empty final directory -> `[]`; a non-empty directory without marker -> `[]`; both complete -> `["radar-hrrr"]`.
- **H14** `_write_palette_tiles` (legacy, no source CRS) over a pre-marker pyramid that equals the fresh deterministic render: outcome `ADOPTED`; identity `renderer == "legacy"`; identity `source_id == "hrrr:radar-hrrr:runs/20260715_12/2026-07-15T13:00:00+00:00"`.
- **H15** `hrrr_process_forecast_hour("20260715_12", 1)` with all palette pyramids complete: no download (download stub raises if called), `rendered_layers == ["radar-hrrr"]`, `valid_timestamp == "2026-07-15T13:00:00+00:00"`, and the work root does not exist.
- **H16** Both palettes complete under `indexed` renderer identity: `_existing_rendered_layers` returns `["radar-hrrr"]` (foreign renderer accepted).
- **H17** One palette `legacy`, one palette `indexed`: `_existing_rendered_layers` returns `[]`.
- **H18** Pyramids whose identity no longer matches the current contract return `[]`, one case each: algorithm `future-render-v9`; zooms `[4, 5]`; palette payload changed (`ranges: [], no_data_below: -999`); semantic `nodata_value: -9999.0`.
- **H19** `hrrr_process_forecast_hour("20260715_12", 42)` with download returning None: `rendered_layers == []`, `fhr == 42`, work root empty.
- **H20** `_resume_incomplete_run`: young partial (`complete: false`, 30 min old) returns the run; `complete: true` returns None; missing `complete` key returns None (treated as complete); a partial older than 3 h + 1 min returns None; empty layers returns None; `run_id: "not-a-run"` returns None.
- **H21** Hourly 13z run after an extended 12z run: `runs[:18]` are all `20260716_13`; `runs[18:]` are 29 frames of `20260716_12`; the last timestamp is `2026-07-18T12:00:00+00:00`; layer `run_id == "20260716_13"`; `_referenced_runs == {"20260716_12", "20260716_13"}`. A later hourly 01z run (`20260717_01`, horizon 18) drops the 12z tail, leaving only `20260717_01` frames.
- **H22** `_extended_tail` ignores non-extended runs (a 13z frame is not returned) and keeps a 12z frame with valid time after `last_valid` when the current run is 14z.
- **H23** With `ENABLED_LAYERS = {radar-hrrr, wind}`: `_needed_vars() == ["refc", "u10", "v10"]`; `_required_layers` for a palette with `reflectivity`, `wind_speed`, `temperature` is `{"radar-hrrr", "wind"}`. With only `radar-hrrr` enabled, `_needed_vars() == ["refc"]`.
- **H24** `_safe_wind_dump` with `uv_grid_relative=True` on an LCC grid (1 unit along grid x everywhere): the rotated `(wind_u, wind_v)` direction matches the expected angle `radians(-sin(radians(38.5)) * (lon + 97.5))` within 0.5 degree at a probe point, and the speed is 1 +- 1e-3.
- **H25** `_safe_wind_dump` with `uv_grid_relative=False` and `(u, v) = (1, 0)`: resampled `wind_u` is 1 and `wind_v` is 0 wherever finite; at least one finite cell.

### 14.3 `temporal/workflows/test_ingest_hrrr.py`

- **W1** `FORECAST_CONCURRENCY == 4`; `HOUR_START_TO_CLOSE == 8 min`; `HOUR_SCHEDULE_TO_CLOSE == 12 min`; `HOUR_HEARTBEAT_TIMEOUT == 90 s`; `_FORECAST_RETRY.maximum_attempts == 2`; `HOUR_START_TO_CLOSE < HOUR_SCHEDULE_TO_CLOSE` (so a retry can run within the budget).
- **W2** `IngestHrrrResult(run_id, skipped_already_processed, forecast_hours_processed=3).complete is False` (default).

### 14.4 Gaps (not asserted by any test read here)

- No test for MRMS frame incomplete (zero-echo path) or for MRMS `mark_processed` ordering.
- No test for MRMS listing pagination or UTC day rollover.
- No test for `_pick_ranges`, idx parsing, or the Range request (206) path.
- No test for HRRR grid dumps under the default radar-only layer set, nor for the storm JSON.
- No test for `hrrr_cleanup` run-dir retention or for `mrms_cleanup`.
- No test for the workflow-level behavior (fan-out, per-item isolation, early returns); only two constant/default checks exist.
- Other test files exist and were NOT read: `backend/shared/test_tiler*.py`, `test_manifest.py`, `test_grid_dump_*.py`, `test_storms.py`, `test_state*`, `temporal/workflows/test_replay.py`. The Rust port needs equivalents for the shared modules too.

---

## 15. Discrepancies, latent defects, and behaviors to decide

Each item states the evidence. "Keep" means the Rust port reproduces the behavior exactly. "Decide" means the team must pick. "Fix" is a recommended change.

| ID | Where | Finding | Evidence | Recommendation |
|---|---|---|---|---|
| D1 | palettes, HRRR `dewpoint` and `humidity` | Both layers can never render. No palette has the color key, so the layer is skipped and no grid is dumped. | grep of all palette JSON: no match; gate in `_process_forecast_hour_sync` | Decide: remove the layers, or add palette keys. Default config does not enable them. |
| D2 | MRMS frame; HRRR `_render_per_palette` | A frame or hour with no visible echo is not published. MRMS logs ERROR `frame_incomplete` and never marks the frame processed. HRRR returns `[]` silently, which truncates the publish prefix at that hour. | Probe: legacy render of all-below-threshold grid returns `EMPTY` for every palette; HRRR `set(rendered) != set(color_tables)` check | Decide. Likely intent: empty frames are valid, not failures. Fix by treating all-empty as a successful, unpublished frame (MRMS) and as a rendered-empty hour (HRRR). |
| D3 | HRRR `hrrr_process_forecast_hour` | If the download raises (non-404, or any exception before the render try), `tmp_dir` is not removed. Partial subsets or a full-file fallback (about 130 MB) leak under `HRRR_TMP_ROOT`. | Code path between lines `grib_path = await run_sync_with_heartbeat(_download...)` and the render `try` | Fix: wrap the download in the same try/finally used for render. MRMS already does this. |
| D4 | HRRR `_download_subset_sync` | Any `httpx.HTTPError` from the index fetch triggers a full-file download (timeout 300 s): 5xx, timeouts, and transport errors included. Up to about 130 MB per hour, 48 hours per extended run. | Code | Decide. Recommend falling back only on 404 of the index. |
| D5 | HRRR range download | Range responses are not checked for 206. A server answering 200 to a Range request would write the whole file once per range. | Code | Fix: require 206 (or Content-Range match) and fail otherwise. |
| D6 | MRMS and HRRR listing | Prefix and continuation token are interpolated unencoded into the query. Continuation tokens can contain `+`, `/`, `=`. | Code (`_list_keys_sync`) | Fix: percent-encode in the Rust client. Current prefixes are safe. |
| D7 | MRMS listing at UTC rollover | Only today's prefix is listed once it has any keys. Yesterday's late keys that were never processed are not revisited. | Code (`_list_recent_files_sync`) | Decide. Low impact (cycles run every minute). Fix by listing both days near rollover. |
| D8 | HRRR `hrrr_find_latest_run` | If no f01 is found in the last 12 hours, an incomplete run is not resumed, even if it is younger than 3 h. | Ordering in `hrrr_find_latest_run` (return before resume check) | Decide. Likely fix: run the resume check independently. |
| D9 | HRRR resume rule | An incomplete run younger than 3 h blocks newer runs. A run whose later hours never appear delays the next run by up to 3 h. | Intended per comment; cost noted here | Keep, but document the latency cost. |
| D10 | Both workflows | Find, horizon, publish, mark, and the MRMS mark-processed activity are not isolated. Their failure fails the whole workflow and skips cleanup. | Code (no try around these calls) | Decide. Consider isolating cleanup so it always runs. |
| D11 | manifest, ProcessedSet | Manifest and ProcessedSet files are not fsynced. Grid writes are. A crash can lose a manifest update (rename is atomic, the data may not be durable). | Code | Fix: fsync temp file before rename, and the directory after. |
| D12 | MRMS `radar-nowcast-input` write | Not guarded. A failure fails the frame activity after tiles are published; retry converges. Other grid writes are guarded. | Code | Keep, or make the failure non-fatal and rely on the next frame. Decide. |
| D13 | Legacy renderer, continuous | Colour is interpolated, not value. Colour blends and alpha feathers at class boundaries. | Section 6.4 | Keep for parity (default). The indexed renderer interpolates values; visible edges differ. |
| D14 | Legacy renderer, categorical | Precip-type is bilinear on category colours, so rain/snow/ice boundaries blend. | Section 6.4 | Keep for parity; document. |
| D15 | MRMS pooling | Zooms 4-6 are footprint-max (nearest-style); zoom 7 is bilinear RGBA. Character changes at z7. | Section 6.6 probe | Keep. This is part of the rendered product. |
| D16 | storms.json empty case | Output lacks `threshold_dbz`, `cell_count`, `tracking_vectors` when there are no cells. Consumers see a different schema. | Code (`detect_storms`, `n == 0` branch) | Fix: emit the full schema with empty lists and zero count. |
| D17 | `_existing_rendered_layers` | Ignores `HRRR_ENABLED_LAYERS`. A previously rendered disabled layer appears in `rendered_layers` on resume. | Code | Decide. Harmless today (layer not published unless enabled). |
| D18 | HRRR grid dumps | Keyed by valid time, not run; a newer run overwrites an older one's grid for the same valid time. Inspector values can change after the first run. | Section 7.4 | Keep (newest wins), document. |
| D19 | Identity and Rust | A Rust renderer must not reuse Python's algorithm and renderer names unless its output is proven pixel-identical; otherwise a mixed fleet publishes different pixels under one identity. | Section 6.8.2 and 17.4 | Decide: use a distinct renderer name (for example `rust-legacy`), and add an explicit compatibility rule, or prove parity. |
| D20 | Pre-v2 and v1-marker pyramids | An unbound pyramid that is not byte-identical to a fresh render raises `PyramidIdentityConflict`. The frame then fails on every retry. | Section 6.8 step 8 | Decide: treat as a repair case (rewrite to a new run path) instead of failing forever. |
| D21 | MRMS key timestamp | `_extract_timestamp` runs after the full download and decode. A malformed key wastes a download, then fails. | Order in `mrms_process_frame` | Fix: parse the key before downloading. |
| D22 | Docs vs code | `docs/configuration.md`: `BACKLOG_PER_CYCLE` default 2 (code 3); `PALETTES` default `classic,muted,vivid` (code `classic`); `STORM_MIN_TRACK_RADIUS_KM` documented but not read (code uses `STORM_MATCH_SLACK_KM`, default 8). `docs/tuning.md` and `ARCHITECTURE.md` say HRRR hours get 20 min per attempt and 30 min total (code: 8 min and 12 min). | Compare against section 3 and 11.2 | Code is authoritative. Update docs or make the defaults match, after the team decides. |
| D23 | ProcessedSet load | Corrupt JSON silently resets the set to empty, so every key is re-processed (idempotent, but costly and silent). | `state.py` `_load` | Fix: log a warning, or quarantine the corrupt file. |
| D24 | Frame output order | `ProcessFrameResult.palettes` order follows `PALETTES` insertion order; the manifest uses sorted palette names. | Code | Keep. Only the manifest is external. |
| D25 | HRRR workflow result | `forecast_hours_processed` is the prefix length on the normal path but 0 on the early-return paths (steps 8 and 10). | Section 11.2 | Decide. Consumers should not rely on it. |
| D26 | Indexed vs legacy parity | If `TILE_RENDERER=indexed` is ever enabled in a deployment, the published pixels differ from legacy at class edges and the tile identity changes. | Section 6.5 | Ask ops whether any environment sets this. If none, defer the indexed renderer. |

Behaviors that look like bugs but are required for parity (do not "fix" without a decision):
- MRMS and HRRR pyramid paths use the valid time, with colons (`2026-09-30T00:00:00+00:00`). Changing the format breaks the API's tile URL template.
- Manifest frame timestamps are the same valid-time strings used for tile directories and grid keys.
- `publishable_prefix` requires `radar-hrrr` in each hour; a layer-only hour does not extend the prefix.

---

## 17. Rust design notes

### 17.1 Temporal

- The vendored SDK (`temporalio-sdk 1.1.0`, `temporalio-client 1.1.0`) exposes what this port needs: `execute_activity` with per-call timeouts (`ActivityOptions` with close timeouts and heartbeat timeout), schedule handles with `trigger` and `ScheduleOverlapPolicy::BufferOne`, and activity heartbeats. Confirm the retry-policy field names in `temporalio-common` before writing code (the grep for the Python field names found nothing).
- Two migration paths:
  - A. Rust workflows and activities in one Rust worker, same queue names and same activity and workflow type names, same JSON payloads.
  - B. Keep the Python workflows; run a Rust worker that registers only the ten activities on the same queue names. Activities are addressed by name, so this works if JSON payloads match exactly (section 2.5). This is the lower-risk cutover.
- Payload compatibility is the contract. Field names, defaults, and nullability in section 2.5 must match exactly, including `layers_per_hour` as a nested array.
- Workflow code in Rust must not read the wall clock. The Python workflows read none.
- Concurrency: replace `asyncio.Semaphore` plus `gather` with a bounded `buffer_unordered(2)` (MRMS) and `buffer_unordered(4)` (HRRR), keeping results in input order. Only `ActivityError` is isolated (section 11).
- Heartbeats: a spawned task that heartbeats every N seconds while the blocking work runs on the blocking pool (equivalent of `run_sync_with_heartbeat`). Heartbeat timeouts: 60 s (MRMS frame), 90 s (HRRR hour).

### 17.2 HTTP and S3

- `reqwest` with rustls. Set redirect policy to none (httpx default); HEAD 200 is the only success for the HRRR finder.
- Per-call timeouts exactly as in section 4 (10, 15, 30, 60, 120, 300 s).
- ListObjectsV2: `quick-xml` (or `serde-xml-rs`), matching elements by local name to tolerate the namespace. Fix D6 by percent-encoding `prefix` and `continuation-token`.
- Ranged GET: `Range` header; require 206 (fix D5).

### 17.3 Compression and GRIB2

- gzip: `flate2::read::MultiGzDecoder` (multi-member input).
- GRIB2: the Python decoder is pygrib over eccodes, so decoded bits are eccodes' bits. Options:
  - `eccodes` via FFI (`libeccodes` already in the base image): highest parity, same as pygrib. Recommended first.
  - Pure-Rust `grib` crate: check support for grid templates 3.0 (lat/lon) and 3.30 (Lambert), and the data templates used by MRMS and HRRR (determine them from a real file with `grib_ls -p packingType` or pygrib `packingType`). Only adopt if the decoded float32 arrays are bit-identical to pygrib on the same files.
  - Do not start with an unverified pure-Rust decoder.
- Validation fixtures (generate on a machine that can reach NOAA S3 and has pygrib): one HRRR `wrfsfcf01` subset and one MRMS file. Save decoded arrays as `.npy` (float32, plus lat/lon) and assert bit equality.

### 17.4 Projection, resampling, PNG, hashing

- Projection: `proj` crate (PROJ, same library family as pyproj) for the HRRR LCC CRS, or a closed-form spherical LCC with R = 6371229. Either way, check forward and inverse against pyproj outputs at a sample of points to about 1e-9 relative.
- Resampling: implement `map_coordinates(order=1, mode=constant, cval)` for float32 and uint8 inputs. Verify against Python on synthetic inputs. For uint8 RGBA bands, scipy's rounding rule must be matched exactly; test against Python output, do not assume it.
- Index helpers: `_axis_to_fractional_indices` (uniform-axis branch uses the exact operation order in section 6.7), `_footprint_edges` and `_reduce_ranges` (section 6.6), `_lat_lon_to_tile`, `_tile_bounds`.
- Float32 arithmetic: follow the operation order in section 5.5 (`(k - 273.15) * 9 / 5 + 32`, `ms * 2.237`, `d / 25.4`). Use `f32` throughout the decode and render paths.
- PNG: `png` crate for RGBA (legacy) and indexed (mode P with PLTE/tRNS). Compression: flate2 level 1 for legacy, level from `TILE_PNG_COMPRESS_LEVEL` for indexed. Pillow's adaptive filter choice differs from the `png` crate, so bytes differ while pixels match. This is why adoption by byte comparison cannot cross languages (D19, D20).
- Hashing: `sha2` (sha256 for digests and the publish lock stripe), `blake2` (blake2b with 16-byte output for array fingerprints), `uuid` v4 for staging names and grid generations.
- Atomic publish: `std::fs::rename` for directories; `std::fs::hard_link` for the create-only marker (mirrors `os.link`); `File::sync_all` for files; open the directory and `sync_all` for directory fsync.
- File locks: `rustix::fs::flock` or `fs2`. Lock files and paths exactly as in section 6.8 and 7.1.
- JSON: `serde_json` with sorted keys (a `BTreeMap` or a custom serializer), compact separators, and a trailing newline where Python writes one. Float formatting: `serde_json` (ryu) matches Python `repr` for normal values; check exponent formatting (Python `1e-05` vs ryu `1e-5`) for any float field a reader compares as text. The storm `generated_at` is only read as a number.
- Timestamps: valid-time strings are `%Y-%m-%dT%H:%M:%S%:z` with UTC. Do not use the default RFC 3339 formatter that may emit `Z`. `manifest.updated_at` uses microseconds (Python `isoformat`); valid times never have fractional seconds.

### 17.5 Memory and CPU

- MRMS decoded grid: 7000 x 3500 float32 = about 98 MB. Two frames in flight (FRAME_CONCURRENCY 2) can use about 200 MB plus the class-model LUTs and tile buffers. Budget about 300 MB per frame including the flipped copy and the storm pass.
- MRMS nowcast-input grid after stride 2: 24.5 MB float32 per write.
- HRRR grid: about 1799 x 1059 float32 = about 7.6 MB per field, plus 2-D float64 lat and lon (about 15 MB each). Per hour, several fields are alive at once.
- Use rayon inside tile rendering only. Pooling and I/O stay on the blocking pool with the same concurrency limits as the workflow.

### 17.6 Test plan

- Golden fixtures from Python (on a machine with pygrib, pyproj, scipy, Pillow 12.3.0, numpy 1.26.4): class-model LUTs for each shipped palette; a synthetic grid's legacy pyramid (decoded pixel equality per tile, not byte equality); the pooled MRMS block factors for the test header; `_footprint_edges` outputs for a few edges; marker JSON and digests for one synthetic pyramid.
- Decoding fixtures: one MRMS and one HRRR file (section 17.3).
- Manifest and ProcessedSet: round-trip of a fixture JSON, including the frame ordering, palette intersection, and extended-tail carry in section 14.2 H21.
- Grid dump: meta JSON equality; bin bytes equal for a fixed input (the generation id differs, so compare the bin by content and the meta with `data_file` ignored).
- Storms: `storms.json` equality except `generated_at`, over a synthetic multi-frame sequence that exercises matching, new ids, and the speed cap.
- Every test in section 14 has a Rust equivalent.

### 17.7 What to port first (suggested order)

1. Payload types and the activity registry (section 2), with JSON round-trip tests.
2. Manifest and ProcessedSet (section 9), since every activity touches them.
3. MRMS list, key parsing, and ProcessedSet logic (no GRIB needed).
4. MRMS decode (section 5.2) behind the chosen GRIB2 library, plus the legacy render (section 6.4) and footprint pooling (6.6), then publication (6.8).
5. HRRR find, idx parsing, and range selection (section 4.2), with a fixture index.
6. HRRR decode and the refc-only path (default config), then publish and resume.
7. Grid dumps, storms, cleanup.
8. Optional layers and the indexed renderer, only after the team decides on D1, D26.

---

## 18. Open questions for the team

1. Migration path A or B (section 17.1)?
2. Distinct renderer name for the Rust legacy renderer, and the compatibility rule (D19), or demand pixel parity with Python before cutover?
3. Is `TILE_RENDERER=indexed` set in any environment (D26)? If not, defer the indexed renderer.
4. Keep or remove `dewpoint` and `humidity` (D1)?
5. Treat an all-transparent frame or hour as a success without publication, and what should the MRMS log level be (D2)?
6. Keep the full-file fallback on any index error (D4), or restrict it to 404?
7. Which palettes run in production? `docs/configuration.md` says `classic,muted,vivid`; the code default is `classic`.
8. Confirm the GRIB2 codes in section 5.4 and the selector-to-message mapping on real NOAA files. This is the largest unverified item in the spec.
9. Isolate cleanup from failed steps (D10)?
10. Which of the docs-vs-code values in D22 is the intended default?

---

## Appendix A. Worked examples

**MRMS base frame.** Key (inferred format):
`CONUS/MergedBaseReflectivityQC_00.50/20260930/MRMS_MergedBaseReflectivityQC_00.50_20260930-000000.grib2.gz`

- Valid time: `2026-09-30T00:00:00+00:00`
- Tiles: `{TILE_DIR}/radar/classic/2026-09-30T00:00:00+00:00/4/3/5.png` (and for `muted`, `vivid`)
- Staging (transient): `{TILE_DIR}/radar/classic/.2026-09-30T00:00:00+00:00.tmp-<hex>/`
- Grid: `{GRID_DIR}/radar/2026-09-30T00:00:00+00:00.meta.json` plus `.<gen>.bin`
- Nowcast input: `{GRID_DIR}/radar-nowcast-input/2026-09-30T00:00:00+00:00.*`
- Manifest frame: `{"timestamp": "2026-09-30T00:00:00+00:00", "path": "2026-09-30T00:00:00+00:00", "palettes": [...], "source": "mrms", "kind": "observation", "issued_at": ..., "lead_minutes": 0, "spatial_resolution_km": 1.0, "max_zoom": 7}`
- Storms: `{STATE_DIR}/storms.json` with `timestamp` equal to the valid time.

**HRRR forecast hour.** Run `20260715_12`, fhr 1, layer `radar-hrrr`, palette `classic`:
- Valid time: `2026-07-15T13:00:00+00:00`
- Source key: `hrrr.20260715/conus/hrrr.t12z.wrfsfcf01.grib2` (index `.idx`)
- Tiles: `{TILE_DIR}/radar-hrrr/classic/runs/20260715_12/2026-07-15T13:00:00+00:00/{z}/{x}/{y}.png`, z in {4, 5, 6}
- Grid (regular 0.03 deg): `{GRID_DIR}/radar-hrrr/2026-07-15T13:00:00+00:00.meta.json`
- Manifest frame: `{"timestamp": "2026-07-15T13:00:00+00:00", "path": "runs/20260715_12/2026-07-15T13:00:00+00:00", "run_id": "20260715_12", "lead_minutes": 60, "spatial_resolution_km": 3.0, "max_zoom": 6, "kind": "model_guidance", ...}`
- Layer metadata: `{"title": "HRRR simulated reflectivity", "kind": "model_guidance", "run_id": "20260715_12", "complete": <bool>}`

---

