# Porting spec: radar-ng "aux" ingest pipelines (Python -> Rust)

Status: draft v1, 2026-10-13. Written from the read-only source tree at
`/mnt/user-data/uploads/radar-ng`. Target readers: the Rust implementer, and the
reviewer who decides the items in section 14.

Scope: six Temporal workflows and their activities:

| Pipeline | Workflow | Python sources |
|---|---|---|
| NAQFC air quality (PM2.5, ozone) | `IngestAirQualityWorkflow` | `backend/ingest_airquality/activities.py`, `temporal/workflows/ingest_airquality.py` |
| Blitzortung lightning | `IngestLightningWorkflow` | `backend/ingest_lightning/activities.py`, `temporal/workflows/ingest_lightning.py` |
| NHC tropical cyclones | `IngestTropicalWorkflow` | `backend/ingest_tropical/activities.py`, `temporal/workflows/ingest_tropical.py` |
| Tile/grid retention | `TileCleanupWorkflow` | `backend/tile_cleanup/activities.py`, `cleanup.sh`, `temporal/workflows/tile_cleanup.py` |
| Open-Meteo model sync | `OpenMeteoSyncWorkflow` | `backend/open_meteo_sync/activities.py`, `temporal/open_meteo_worker.py`, `temporal/workflows/open_meteo_sync.py` |
| NWS active alerts | `PollAlertsWorkflow` | `temporal/workflows/poll_alerts.py`, `backend/api/api/storm_watch_activities.py` |

Shared modules these depend on are specified in section 12 only to the depth the
aux pipelines need: `backend/shared/{manifest,tiler,grid_dump,state,palettes,
png_palette,activity_heartbeat}.py`, `temporal/{worker,task_queues}.py`,
`temporal/schedules/seed.py`.

Out of scope: MRMS, HRRR, nowcast, the storm-watch workflow itself, push
delivery, API routes, the frontend, and the tiler's internal tests.

Conventions:

- MUST = the port must match the Python behavior. DECIDE = an owner must choose
  (section 14). Q-n = observed quirk; replicate it unless a DECIDE item says
  otherwise. Source references look like `[aqm:113]` (file-short-name:line).
- Where code and docs disagree, the code wins. Each disagreement is in section 13.
- Test vectors marked "verified" were produced by running the Python functions
  while writing this spec.

---

## 1. Inventory and schedules

### 1.1 Pipelines at a glance

| Pipeline | Schedule ID | Interval | `execution_timeout` (max_runtime) | Catch-up window | Output |
|---|---|---|---|---|---|
| AQM | `ingest-airquality` | 30 min | 75 min | 1 h (default) | PNG pyramids `air-quality`, `ozone`; manifest; grid dumps |
| Lightning | `ingest-lightning` | 60 min | 56 min | 1 h (default) | `STATE_DIR/lightning.json` |
| Tropical | `ingest-tropical` | 1 h | 5 min | 1 h (default) | `STATE_DIR/tropical.json` |
| Tile cleanup | `tile-cleanup` | 1 h | 15 min | 1 h (default) | deletes tiles/grids; manifest edits |
| Open-Meteo GFS | `open-meteo-sync-gfs` | 6 h | 60 min | 1 h (default) | Open-Meteo data volume (binary side effect) |
| Open-Meteo GFS 0.25 | `open-meteo-sync-gfs025` | 6 h | 60 min | 1 h (default) | same |
| Open-Meteo HRRR | `open-meteo-sync-hrrr` | 1 h | 55 min | 1 h (default) | same |
| NWS alerts | `poll-alerts` | 5 min | 4 min 30 s | 10 min | `STATE_DIR/alerts_seen.json`, `STATE_DIR/alerts_active_snapshot.json`; push signals |

No schedule has jitter (jitter exists only on the MRMS/nowcast fast schedules).
Every schedule uses `ScheduleOverlapPolicy.SKIP`.

### 1.2 Schedule action rules [seed:291-315]

- Action: `ScheduleActionStartWorkflow(<WorkflowType>, *workflow_input,
  id="sched-<schedule_id>", task_queue=<q>, execution_timeout=max_runtime)`.
- Task queue `q`: if env `USE_ISOLATED_TASK_QUEUES == "1"`, `q` is the
  ScheduleDef's queue; otherwise `q = "radar-ng"` (legacy queue).
  ScheduleDef queues: `ingest-airquality`, `ingest-lightning`, `ingest-tropical`,
  `tile-cleanup`, `open-meteo-sync-*` all default to `"radar-ng-aux"`;
  `poll-alerts` is `"radar-ng-alerts"`.
- The Open-Meteo *activity* always runs on `"radar-ng-open-meteo"` regardless
  of the schedule's queue [open_meteo_sync:43].
- Schedule policy: `SchedulePolicy(overlap=SKIP, catchup_window=...)`.
- Seeding: `create_schedule`; on `ScheduleAlreadyRunningError`, update only if
  the semantic definition differs (protobuf comparison, live pause state kept).
  Never delete a schedule. Each RPC has a 15 s timeout. Retryable RPC codes:
  CANCELLED, UNAVAILABLE, DEADLINE_EXCEEDED, RESOURCE_EXHAUSTED, ABORTED,
  INTERNAL, UNKNOWN, NOT_FOUND. Up to 10 passes with backoff
  `min(20, 1 * 2**(attempt-1))` seconds. Permanent errors are aggregated.
- Which process seeds: `SEED_SCHEDULES == "1"`, or unset and `WORKER_ROLE` in
  {legacy, all}; and `SKIP_SCHEDULE_SEED != "1"`.

### 1.3 Exact schedule inputs

`workflow_input` is a JSON array of arguments. Only the open-meteo schedules pass
arguments:

- `open-meteo-sync-gfs`: `[{"model": "ncep_gfs013", "variables": GFS013_VARIABLES, "past_days": 2}]`
- `open-meteo-sync-gfs025`: `[{"model": "ncep_gfs025", "variables": GFS025_SURFACE_VARIABLES, "past_days": 2}]`
- `open-meteo-sync-hrrr`: `[{"model": "ncep_hrrr_conus", "variables": HRRR_VARIABLES, "past_days": 1}]`

Variable lists are in section 9.6 (copied from `seed.py`, verified).

All other schedules have `[]` arguments.

### 1.4 Workers and role mapping [worker:95-208]

Role `aux` (queue `radar-ng-aux`) registers workflows `IngestLightningWorkflow`,
`IngestTropicalWorkflow`, `IngestAirQualityWorkflow`, `TileCleanupWorkflow`,
`OpenMeteoSyncWorkflow`, and the activities `lightning_consume_stream`,
`tropical_fetch_and_publish`, `aqm_find_latest_run`, `aqm_render_chunk`,
`aqm_publish_run`, `aqm_mark_processed`, `aqm_cleanup`, `tile_cleanup_sweep`.

Role `alerts` (queue `radar-ng-alerts`) registers `PollAlertsWorkflow` and the
activities `fetch_nws_active_alerts`, `mark_alerts_seen`,
`signal_matching_storm_watches` (plus push/watch activities out of scope).

Open-Meteo worker (`temporal/open_meteo_worker.py`): queue `radar-ng-open-meteo`,
registers only `open_meteo_sync`. Its image is `FROM ghcr.io/open-meteo/open-meteo`
with the binary at `/app/openmeteo-api`.

Worker options: max concurrent activities env `TEMPORAL_MAX_CONCURRENT_ACTIVITIES`
(code default 4), activity task polls env `TEMPORAL_MAX_CONCURRENT_ACTIVITY_TASK_POLLS`
(code default 2), graceful shutdown `TEMPORAL_GRACEFUL_SHUTDOWN_S` (default 25 s).
On SIGTERM, stop polling and drain. Activity cancellation is delivered after the
drain window.

---

## 2. Temporal payload conventions

- Payload encoding is the Temporal Python default JSON converter. Each dataclass
  is a JSON object whose keys are the exact Python field names (snake_case). All
  fields are emitted on encode. `None` is `null`. Lists are arrays. Python
  `float` and `int` are JSON numbers. The port must not emit NaN or Infinity
  tokens (no payload in this spec should contain them).
- On decode, a missing key takes the dataclass default. The only defaulted field
  in scope is `OpenMeteoSyncArgs.past_days = 1`. Unknown keys should be ignored.
  Rust: use `#[serde(default)]` for defaulted fields and `Option<T>` for
  nullable ones.
- Activities with no arguments and no return value return `null`
  (`mark_alerts_seen`, `aqm_mark_processed`).
- Activity and workflow *type names* are the strings in section 3.9. Activity
  names are the `@activity.defn(name=...)` strings, not Python function names.
  Workflow names are the `@workflow.defn(name=...)` strings.

---

## 3. Workflow logic and activity options

### 3.1 Retry policies

| Name (Python) | initial | backoff | max interval | max attempts | Used by |
|---|---|---|---|---|---|
| `_DEFAULT_RETRY` (AQM) | 1 s | 2.0 | 100 s | 5 | find, publish, mark, cleanup |
| `_CHUNK_RETRY` (AQM) | 5 s | 2.0 | 60 s | 2 | render chunk |
| lightning `_RETRY` | 5 s | 2.0 | 120 s | 3 | lightning_consume_stream |
| tropical `_RETRY` | 2 s | 2.0 | 100 s | 5 | tropical_fetch_and_publish |
| tile cleanup `_RETRY` | 2 s | 2.0 | 60 s | 3 | tile_cleanup_sweep |
| open-meteo `_RETRY` | 10 s | 2.0 | 5 min | 3 | open_meteo_sync |
| poll-alerts `_RETRY` | 2 s | 2.0 | 15 s | 3 | fetch, signal, mark |

Every retry policy has the default non-retryable error list (none set).

### 3.2 Activity timeouts and heartbeats

| Activity | Call site | start_to_close | schedule_to_close | heartbeat_timeout | Retry |
|---|---|---|---|---|---|
| `aqm_find_latest_run` | AQM | 120 s | - | - | DEFAULT |
| `aqm_render_chunk` | AQM, one per (layer, start_msg) | 20 min | 25 min | 120 s | CHUNK |
| `aqm_publish_run` | AQM | 2 min | - | - | DEFAULT |
| `aqm_mark_processed` | AQM | 30 s | - | - | DEFAULT |
| `aqm_cleanup` | AQM (arg 36) | 5 min | - | - | DEFAULT |
| `lightning_consume_stream` | Lightning (arg 3000) | 3060 s (= 3000+60) | 55 min | 90 s | lightning |
| `tropical_fetch_and_publish` | Tropical | 60 s | - | - | tropical |
| `tile_cleanup_sweep` | Tile cleanup | 5 min | - | - | tile cleanup |
| `open_meteo_sync` | Open-Meteo, queue `radar-ng-open-meteo` | 35 min | 50 min | 90 s | open-meteo |
| `fetch_nws_active_alerts` | poll | 60 s | - | - | poll |
| `signal_matching_storm_watches` | poll, one per new alert | 60 s | - | - | poll |
| `mark_alerts_seen` | poll | 30 s | - | - | poll |

Heartbeat behavior is per activity (sections 5.16, 6.11, 9.3).

### 3.3 IngestAirQualityWorkflow (no input) [wf-aqm]

Result: `IngestAirQualityResult` (section 4).

```
find = aqm_find_latest_run()
if find.run_id is None:
    cleanup = aqm_cleanup(36)
    return {run_id: null, skipped_already_processed: false, cleanup}
if find.already_processed:
    cleanup = aqm_cleanup(36)
    return {run_id: find.run_id, skipped_already_processed: true, cleanup}

sem = Semaphore(2)   # CHUNK_CONCURRENCY
for layer in ["air-quality", "ozone"]:            # AQM_LAYERS insertion order
  for start in [0, 24, 48]:                       # range(0, 72, 24)
    concurrently (bounded by sem):
      try:  r = aqm_render_chunk(run_id, layer, start)
      except ActivityError:
            r = {layer, start_msg: start, rendered_timestamps: []}
results = gather(all 6, in the order above)
frames_rendered = sum(len(r.rendered_timestamps) for r in results)
published_layers = aqm_publish_run(run_id, results)
if "air-quality" in published_layers:
    aqm_mark_processed(run_id)
else:
    log error "not marking processed"
cleanup = aqm_cleanup(36)
return {run_id, skipped_already_processed: false, published_layers,
        frames_rendered, cleanup}
```

- A failed chunk does not fail the workflow. It contributes zero timestamps.
  Only a successful `air-quality` publish marks the run processed. An ozone-only
  publish is retried on the next tick.
- `frames_rendered` counts both layers.
- `published_layers` order follows `AQM_LAYERS` insertion order.
- Cleanup always runs last, including on the skip paths.

### 3.4 IngestLightningWorkflow (no input) [wf-lightning]

```
return lightning_consume_stream(3000)   # RUN_DURATION_S = 50*60
```

Single activity. Result `LightningRunResult`.

### 3.5 IngestTropicalWorkflow (no input) [wf-tropical]

```
return tropical_fetch_and_publish()
```

### 3.6 TileCleanupWorkflow (no input) [wf-tilecleanup]

```
return tile_cleanup_sweep()
```

### 3.7 OpenMeteoSyncWorkflow (input: OpenMeteoSyncArgs) [wf-openmeteo]

```
return open_meteo_sync(args) on queue "radar-ng-open-meteo"
```

The workflow runs on the aux worker (the workflow class is registered only
there). Only the activity uses the open-meteo queue.

### 3.8 PollAlertsWorkflow (no input) [wf-poll]

```
fetched = fetch_nws_active_alerts()          # diff only; does NOT commit seen
normalized = [(a.alert_id, a.geometry or {}) for a in fetched.new_alerts]
handled = [id for (id, geo) in normalized if geo is empty]   # zoneless: handled w/o signal
to_signal = [(id, geo) for (id, geo) in normalized if geo non-empty]

results = gather over to_signal (unbounded fan-out; worker slots bound it):
    try:  matched = signal_matching_storm_watches({alert_id, geometry}).matched
    except ActivityError: return None         # not handled; retried next poll
    return (alert_id, matched)
for r in results:
    if r is None: failed += 1
    else:         handled.append(r.id); signaled += r.matched

mark_alerts_seen({handled_ids: handled})
return {total: fetched.alert_count, new: len(fetched.new_alert_ids),
        signaled_watches: signaled, failed_alerts: failed,
        new_ids: fetched.new_alert_ids}
```

Note: `new_ids` is the full list of new alert ids, including ones that later
failed. `new` is its length.

### 3.9 Activity and workflow name registry

| Kind | Name string | Module |
|---|---|---|
| workflow | `IngestAirQualityWorkflow` | `temporal/workflows/ingest_airquality.py` |
| workflow | `IngestLightningWorkflow` | `temporal/workflows/ingest_lightning.py` |
| workflow | `IngestTropicalWorkflow` | `temporal/workflows/ingest_tropical.py` |
| workflow | `TileCleanupWorkflow` | `temporal/workflows/tile_cleanup.py` |
| workflow | `OpenMeteoSyncWorkflow` | `temporal/workflows/open_meteo_sync.py` |
| workflow | `PollAlertsWorkflow` | `temporal/workflows/poll_alerts.py` |
| activity | `aqm_find_latest_run` | `backend/ingest_airquality/activities.py` |
| activity | `aqm_render_chunk` | same |
| activity | `aqm_publish_run` | same |
| activity | `aqm_mark_processed` | same |
| activity | `aqm_cleanup` | same |
| activity | `lightning_consume_stream` | `backend/ingest_lightning/activities.py` |
| activity | `tropical_fetch_and_publish` | `backend/ingest_tropical/activities.py` |
| activity | `tile_cleanup_sweep` | `backend/tile_cleanup/activities.py` |
| activity | `open_meteo_sync` | `backend/open_meteo_sync/activities.py` |
| activity | `fetch_nws_active_alerts` | `backend/api/api/storm_watch_activities.py` |
| activity | `signal_matching_storm_watches` | same |
| activity | `mark_alerts_seen` | same |

Other registry facts: the Python `Client` the alerts activity uses is a
process-wide singleton from `backend/api/api/temporal_client.py` (not in
scope). Query and signal names used against WatchStormWorkflow are in 10.4.

---

## 4. Payload schemas (exact JSON)

All types below are Python dataclasses. Keys are the field names.

### 4.1 AQM

```
AqmFindRunResult   { "run_id": string|null, "already_processed": bool }
AqmChunkResult     { "layer": string, "start_msg": int, "rendered_timestamps": [string] }
AqmCleanupResult   { "tile_dirs_removed": int }
IngestAirQualityResult {
    "run_id": string|null,
    "skipped_already_processed": bool,
    "published_layers": [string],        # default []
    "frames_rendered": int,              # default 0
    "cleanup": AqmCleanupResult|null     # default null
}
```

Inputs:
- `aqm_find_latest_run()`: no args.
- `aqm_render_chunk(run_id: string, layer: string, start_msg: int)`: positional.
- `aqm_publish_run(run_id: string, results: [AqmChunkResult])`: returns `[string]`
  (published layer names).
- `aqm_mark_processed(run_id: string)`: returns null.
- `aqm_cleanup(retention_hours: int)`: returns `AqmCleanupResult`.

### 4.2 Lightning

```
lightning_consume_stream(duration_s: int) -> LightningRunResult
LightningRunResult {
  "duration_s": float,        # rounded to 1 decimal place
  "msgs": int,                # raw websocket messages (text or bytes) seen
  "parsed": int,              # frames that became a JSON object
  "in_bbox": int,             # strikes inside BBOX with time != 0
  "final_buffer": int,        # strikes in buffer at exit
  "seeded": int,              # default 0; strikes loaded from the previous file
  "connect_failures": int     # default 0; sessions that delivered no message
}
```

Input is a single JSON integer `3000` (the workflow passes a bare int, not an
object).

### 4.3 Tropical

```
tropical_fetch_and_publish() -> TropicalResult
TropicalResult { "storm_count": int, "feature_count": int }
```

### 4.4 Tile cleanup

```
tile_cleanup_sweep() -> TileCleanupResult
TileCleanupResult {
  "layers_swept": int,          # always len(LAYER_RETENTION_MIN) == 12 (Q-1)
  "tile_dirs_removed": int,
  "grid_files_removed": int
}
```

### 4.5 Open-Meteo

```
OpenMeteoSyncArgs { "model": string, "variables": string, "past_days": int = 1 }
OpenMeteoSyncResult {
  "model": string,
  "succeeded": bool,          # always true on success
  "duration_s": float,        # rounded to 1 decimal place
  "return_code": int,         # default 0
  "stderr_tail": string       # default ""; never set on success
}
```

### 4.6 NWS alerts

```
FetchAlertsResult { "alert_count": int, "new_alert_ids": [string],
                    "new_alerts": [AlertForSignal] }
AlertForSignal    { "alert_id": string, "geometry": object }   # geometry may be {}
SignalWatchesInput  { "alert_id": string, "geometry": object }
SignalWatchesResult { "matched": int }
MarkAlertsSeenInput { "handled_ids": [string] }
```

### 4.7 PollAlertsWorkflow

```
PollAlertsResult { "total": int, "new": int, "signaled_watches": int,
                   "failed_alerts": int, "new_ids": [string] }
```

---

## 5. NAQFC air quality (AQM / AQMv7 CONUS)

### 5.1 Product facts [aqm:1-17, 46-69]

- One AQMv7 CONUS cycle is issued at 06z or 12z. Each pollutant is a single GRIB2
  file holding 72 hourly-average messages, covering valid times run+0h through
  run+71h. The timestamp of each frame is the message's `validDate`.
- Products:
  - `ave_1hr_pm25_bc`: layer `air-quality`, color key `pm25`, unit string
    `"µg/m³"` (bytes: U+00B5 `µ`, U+0067 `g`, U+002F `/`, U+006D `m`, U+00B3 `³`).
    Title for the manifest: `"AQM PM2.5 (US AQI)"`.
  - `ave_1hr_o3_bc`: layer `ozone`, color key `ozone`, unit `"ppb"`.
    Title: `"AQM ozone"`.
- Grid: NCEP grid 227, roughly 5 km Lambert-conformal over CONUS (per the docstring).
  The code asserts the grid is curvilinear and handles projected coordinates
  generically. Do not hard-code the dimensions. Read them from each GRIB2
  message. The code comment cites "~1.5M points", so expect roughly 1.4-1.5M cells.
- The code never checks the message's parameter, level, or type. It selects
  messages purely by index.
- Bias-corrected (`_bc`) guidance is the product AirNow forecasters use.

### 5.2 Constants

| Name | Value | Note |
|---|---|---|
| `AQM_BASE` | `https://noaa-nws-naqfc-pds.s3.amazonaws.com/AQMv7/CS` | AWS mirror, tried first |
| `AQM_NOMADS_BASE` | `https://nomads.ncep.noaa.gov/pub/data/nccf/com/aqm/prod` | NCEP, tried second |
| `TILE_DIR` | env, default `/data/tiles` | |
| `STATE_DIR` | env, default `/data/state` | |
| `AQM_TMP_ROOT` | env, default `/tmp/aqm_work` | per-attempt scratch |
| `ZOOM_LEVELS` | `[4, 5, 6]` | max zoom = 6 |
| `FORECAST_MESSAGES` | 72 | |
| `CHUNK_MESSAGES` | 24 | chunk starts 0, 24, 48 |
| `AQM_CYCLES` | `(12, 6)` | try 12z before 06z on the same day |
| `RETENTION_HOURS` | 36 | passed to `aqm_cleanup` by the workflow |
| processed-set max | 100 entries | `STATE_DIR/ingest-airquality.json` |
| spatial_resolution_km | 5.0 | written into frame metadata |

The NOMADS comment says the AWS mirror stopped receiving AQM after
2026-09-22 12z while NOMADS kept publishing. That is why AWS is tried first and
NOMADS second, for every run.

### 5.3 URL construction [aqm:104-110]

Given `run_id = "YYYYMMDD_CC"` (`CC` in {06, 12}), `product` = `ave_1hr_pm25_bc`
or `ave_1hr_o3_bc`:

```
urls[0] = AQM_BASE + "/{YYYYMMDD}/{CC}/aqm.t{CC}z.{product}.{YYYYMMDD}.227.grib2"
urls[1] = AQM_NOMADS_BASE + "/aqm.{YYYYMMDD}/{CC}/aqm.t{CC}z.{product}.227.grib2"
```

Verified against `backend/ingest_airquality/test_activities.py`:
`_grib_urls("20260716_12", "ave_1hr_pm25_bc")` returns
`https://noaa-nws-naqfc-pds.s3.amazonaws.com/AQMv7/CS/20260716/12/aqm.t12z.ave_1hr_pm25_bc.20260716.227.grib2`
followed by
`https://nomads.ncep.noaa.gov/pub/data/nccf/com/aqm/prod/aqm.20260716/12/aqm.t12z.ave_1hr_pm25_bc.227.grib2`.

### 5.4 Latest-run discovery (`aqm_find_latest_run`) [aqm:113-130, 364-372]

```
now = utcnow()
for days_ago in 0, 1, 2:                       # 3 days, newest first
  date = (now - days_ago days) formatted YYYYMMDD
  for cycle in 12, 6:                          # 12z before 06z
    run_dt = date at cycle:00:00 UTC
    if run_dt > now: continue                  # not yet issued
    run_id = "{date}_{cycle:02d}"
    for url in _grib_urls(run_id, "ave_1hr_pm25_bc"):   # AWS then NOMADS
      try:
        if HEAD(url, timeout=10s).status == 200: return run_id
      except HTTP transport error: continue    # network error -> next URL
return None
```

- The probe is a HEAD on the pm2.5 file only.
- Only status code 200 counts. Any other status, including 3xx, is a miss. The
  client does not follow redirects (httpx default). Q-2: a port using reqwest
  must set `redirect::Policy::none()` to match.
- Worst case is 3 days x 2 cycles x 2 URLs = 12 HEADs.
- The activity then checks the processed set. The result is
  `{run_id, already_processed}`. A run is "already processed" when its id is in
  `STATE_DIR/ingest-airquality.json`.

### 5.5 Chunk download (`aqm_render_chunk` download phase) [aqm:389-402]

```
out = tmp_dir / "{product}.grib2"
with client:                                   # httpx default: no redirects
  for url in urls (AWS, NOMADS):
    stream GET url, timeout=300 s
      if status == 404 and url is not last: continue     # ONLY 404 falls through
      raise_for_status()                       # any non-2xx (incl. 3xx) raises
      write body in 1 MiB chunks to out
      return out
raise RuntimeError("no AQM source for {run_id}")
```

- Fallback to NOMADS happens only on HTTP 404 from AWS. A 500, 403, or other
  error from AWS fails the attempt (Q-3, DECIDE D-5).
- Each chunk downloads the full GRIB2 file (50-90 MB). A run therefore downloads
  the same file six times (two layers x three chunks). The port may cache one
  download per (run, layer) across its three chunks if it preserves outputs
  (DECIDE D-6).
- The temp directory is removed in a `finally` block after the chunk, success or
  failure.

### 5.6 GRIB decode and grid build [aqm:138-211, 339-347]

Per message `i` in `[start_msg, end_msg)` (`end_msg = min(start+24, 72)`):

1. Select message `i + 1` (pygrib is 1-based).
2. `valid = validDate` with tzinfo UTC. Frame timestamp `ts = valid.isoformat()`
   (form `YYYY-MM-DDTHH:MM:SS+00:00`).
3. Values: `data = grb.values`. If it is a masked array, `filled(NaN)`. Cast to
   float32. Missing values are NaN.
4. Geometry: reuse the geometry from the previous message in the same chunk
   (`geometry` variable). For the first message compute it:
   - `lats, lons = grb.latlons()`. Normalize lons: values > 180 become `v - 360`.
   - Native projection extraction (`_extract_native_projection`):
     - `projparams = grb.projparams`. If empty, no projection.
     - `crs = PROJ CRS from projparams` (fallback `Proj(projparams).crs`).
     - `xs, ys = transform(EPSG:4326 -> crs, always_xy=True)(normalized lons, lats)`.
     - `x0 = nanmedian(xs[:,0])`, `x1 = nanmedian(xs[:,-1])`,
       `y0 = nanmedian(ys[0,:])`, `y1 = nanmedian(ys[-1,:])`.
     - If any is non-finite, or `|x1-x0| < 1e-6`, or `|y1-y0| < 1e-6`: no projection.
     - Else `source_x = linspace(x0, x1, w)`, `source_y = linspace(y0, y1, h)`
       (float64), `source_crs = crs.to_wkt()`.
   - If projection succeeded: grid = `(data, lats f64 2D, lons f64 2D, source_crs, source_x, source_y)`.
   - Else: grid = `(data, lats[:,0] 1D, lons[0,:] 1D)` (no projection).
   - Note: the linspace step makes projected axes regular. Port this exactly. Do
     not use the true per-pixel coordinates.
5. Render (section 5.9) and grid dump (section 5.13) for this frame.

### 5.7 Orientation before rendering [aqm:219-233]

- No projection, 1D lats: if `lats[0] > lats[-1]` (north first), flip `data`
  vertically and reverse `lats`. Lons are unchanged.
- Projected: if `source_y[0] > source_y[-1]`, flip `data` vertically and reverse
  rows of `lats`, `lons`, and `source_y`. `source_x` is unchanged.
- After orientation, the source is in the order the renderer expects (ascending
  `source_y`, or ascending `lats`).

### 5.8 Palettes and AQI color tables

Palette set: env `PALETTES` (comma-separated, trimmed, empty entries dropped;
code default `classic`; production `classic,muted,vivid`). For each name,
`load_palette(name)` reads `backend/shared/palettes/<name>.json`. A missing file
logs a warning and is skipped. If no palette loads, `classic` is loaded as a
fallback. The legacy `color_tables.json` is used as `classic` only when
`classic.json` is missing, and it has no `pm25` or `ozone` keys, so AQM would
then publish nothing (Q-4).

Palettes that carry AQ keys: all three shipped palettes (`classic`, `muted`,
`vivid`) contain `pm25` and `ozone`. Their bin edges are identical. Only the
RGBA values differ. A single shared class model therefore covers all of them.

Each table is `{"_comment"?, "unit", "ranges": [{min, max, rgba}], "no_data_below"}`.
The port must embed the three JSON files (for example with `include_str!`) and
keep every key. The palette digest (section 12.2) hashes the table as loaded,
including `_comment`.

Per-layer render selection:
- `color_key` is `pm25` for `air-quality`, `ozone` for `ozone`.
- For a given layer, a palette participates only if `color_key in table`
  (key presence, not truthiness) [aqm:253-266, 467-471].

#### 5.8.1 Verbatim classic AQ tables (source: `backend/shared/palettes/classic.json`)

`pm25` (`unit` = `µg/m³`, `no_data_below` = 0):

| min | max | rgba |
|---|---|---|
| 0 | 4.5 | [0, 228, 0, 70] |
| 4.5 | 9.1 | [70, 225, 40, 95] |
| 9.1 | 20 | [255, 255, 0, 125] |
| 20 | 35.5 | [255, 215, 0, 150] |
| 35.5 | 45 | [255, 160, 0, 170] |
| 45 | 55.5 | [255, 126, 0, 185] |
| 55.5 | 90 | [255, 55, 0, 200] |
| 90 | 125.5 | [225, 0, 0, 215] |
| 125.5 | 175 | [165, 45, 160, 225] |
| 175 | 225.5 | [143, 63, 151, 235] |
| 225.5 | 500 | [126, 0, 35, 245] |
| 500 | 20000 | [80, 0, 22, 250] |

Its `_comment`: "US AQI category hues on the 2024 EPA PM2.5 breakpoints (ug/m3): Good
<=9, Moderate <=35.4, USG <=55.4, Unhealthy <=125.4, Very Unhealthy <=225.4,
Hazardous above. Full-field wash like IQAir/Windy — Good stays visible as a light
green."

`ozone` (`unit` = `ppb`, `no_data_below` = 0):

| min | max | rgba |
|---|---|---|
| 0 | 30 | [0, 228, 0, 60] |
| 30 | 55 | [110, 230, 20, 90] |
| 55 | 63 | [255, 255, 0, 130] |
| 63 | 71 | [255, 215, 0, 155] |
| 71 | 78 | [255, 160, 0, 175] |
| 78 | 86 | [255, 126, 0, 190] |
| 86 | 95 | [255, 55, 0, 205] |
| 95 | 106 | [225, 0, 0, 215] |
| 106 | 150 | [165, 45, 160, 230] |
| 150 | 200 | [143, 63, 151, 240] |
| 200 | 600 | [126, 0, 35, 250] |

Its `_comment`: "US AQI category hues on the 8-hour ozone breakpoints (ppb), applied to
hourly concentration for display."

`muted` and `vivid` use the same edges. Their RGBA values differ (for example
`muted` pm25 row 0 is `[90, 200, 110, 55]`, `vivid` is `[0, 240, 60, 85]`). Use
the JSON files, not these tables, for those palettes.

These are display color bins only. No AQI index is computed anywhere in the
pipeline. "AQI" appears in names and comments only.

### 5.9 Per-frame render, legacy path (the production default) [aqm:253-275, tiler:1809-2056]

`TILE_RENDERER` defaults to `legacy`. `tile_renderer_for_role("aux")` returns
`indexed` only when `TILE_RENDERER=indexed` and `TILE_RENDERER_CANARY_ROLE=aux`.
Otherwise `legacy`. Assume legacy unless configured otherwise.

For each frame (one timestamp, all palettes of one layer):

```
color_tables = {palette: table[color_key] for palette, table in palette_tables if color_key in table}
if none: return []                          # no timestamp recorded
out_dirs = {palette: TILE_DIR/{layer}/{palette}/runs/{run_id}/{ts}}   # ts = isoformat
outcomes = render_frame_palettes(...)       # section 12.2 publication protocol
rendered = [p for p in color_tables if outcome[p].tile_count > 0]
if set(rendered) != set(color_tables): return []   # all-or-nothing, no timestamp
return rendered
```

Legacy rendering for each palette (`_render_legacy`):

1. `rgba = apply_color_table(data, table)`: classify each float32 value into a bin,
   then map the bin to its RGBA (section 12.2.1). NaN and out-of-range values
   give alpha 0.
2. `render_tiles(rgba, lats, lons, staging_dir, [4,5,6], tile_size=256, ...)`:
   for each zoom z, the tile range is from `_lat_lon_to_tile(lat_max, lon_min, z)`
   to `_lat_lon_to_tile(lat_min, lon_max, z)` (inclusive). For each tile:
   - Compute pixel-center lon/lat (section 12.2.3).
   - Map to fractional source indices (projected: pyproj transform to source CRS,
     then fractional index on `source_x` / `source_y`; regular: fractional index
     on 1D axes).
   - Compute finite row and column bounds (`_finite_index_bounds`). If none, skip.
   - If the source sub-rectangle has alpha max 0, skip.
   - Bilinear-sample each RGBA channel independently (scipy `map_coordinates`
     order=1, mode constant, cval 0). Output uint8.
   - If the tile's alpha max is 0, skip.
   - Write `staging/{z}/{tx}/{ty}.png` as RGBA PNG, compress_level=1, optimize off.

Counts: the pyramid for one palette is all tiles across zooms 4-6 that survived
the skips.

### 5.10 Indexed path (canary only) [tiler:1016-1142]

Used only when `TILE_RENDERER_CANARY_ROLE=aux` and `TILE_RENDERER=indexed`.
Summary:
- One class model per frame (shared edges across palettes).
- `render_continuous_tiles`: per tile, bilinear physical-value sampling with
  `nodata_value=None` and `min_valid_weight=1.0`. Any invalid contributor
  (NaN/inf) with non-zero weight makes the output NaN. Then classify to a class
  index (0 = transparent).
- Skip tile if the classified tile has no non-zero index.
- Encode one mode-"P" PNG, then produce the other palettes by replacing PLTE and
  tRNS chunks. PNG `compress_level` = `TILE_PNG_COMPRESS_LEVEL` (default 6).
- Renderer name `indexed`, algorithm `physical-bilinear-classify-v1`.

The indexed path changes output bytes and identity. Mixing legacy and indexed
for the same run fails (section 12.2.6). Do not enable it on the Rust port
without a cutover decision (DECIDE D-1).

### 5.11 Chunk result [aqm:327-358, 441-441]

`aqm_render_chunk` returns `{layer, start_msg, rendered_timestamps}`.
`rendered_timestamps` lists only frames that were recorded (section 5.9), in
message order. The grid dump runs only for recorded frames.

Log line on success: `chunk_done` with run_id, layer, start_msg, rendered count,
and duration. This is informational.

### 5.12 Publish (`aqm_publish_run`) [aqm:444-512]

```
run_dt = parse(run_id "YYYYMMDD_CC") as UTC
run_issued_at = run_dt.isoformat()           # "YYYY-MM-DDT{CC}:00:00+00:00"
published = []
for layer, cfg in AQM_LAYERS (air-quality, ozone):
  timestamps = sorted union of rendered_timestamps for results where layer matches
  if len(timestamps) != 72: continue
  palettes = sorted names where cfg.color_key in palette table
  if no palettes: continue
  frames = [{timestamp: ts,
             path: "runs/{run_id}/{ts}",
             source: "aqm",
             kind: "model_guidance",
             issued_at: run_issued_at,
             run_id: run_id,
             lead_minutes: int((ts - run_dt).total_seconds() // 60),
             spatial_resolution_km: 5.0,
             max_zoom: 6}  for ts in timestamps]
  replace_layer_manifest(layer, timestamps, palettes=palettes,
                         frames=frames,
                         layer_metadata={title, kind: "model_guidance",
                                         run_id, complete: true})
  published.append(layer)
return published
```

- Title: `air-quality` -> `"AQM PM2.5 (US AQI)"`, `ozone` -> `"AQM ozone"`.
- `replace_layer_manifest` replaces the layer's entire frame list atomically
  (section 12.1). The previous run is no longer advertised.
- Partial layers are never written. A layer must have exactly 72 distinct
  timestamps.

### 5.13 Grid dumps (per recorded frame) [aqm:286-291, grid_dump:112-222]

- Layer keys `air-quality` and `ozone` (same as the manifest layer names).
- The dump is written with `write_grid(layer, ts, data, lats_1d, lons_1d, unit)`:
  - `lats_1d`: for a 2D grid, column `w//2` of `lats` (length h). For a 1D grid,
    the 1D array itself. Same for `lons_1d` using row `h//2`.
  - Lat order: if `lats[0] < lats[-1]`, flip data rows and reverse lats (result:
    north first).
  - Lon order: normalized and sorted only if `max(lons) > 180`. For AQM, lons
    were already normalized to [-180, 180], so the stored column order is
    whatever order the source provided. Q-5: verify against a real file (DECIDE D-7).
  - Stride: while `(h//s) * (w//s) > 810000` (MAX_CELLS = 900*900), double `s`.
    Slice `data[::s, ::s]`, `lats[::s]`, `lons[::s]`.
  - NaN values become `-9999.0` (float32). `fill` written as `-9999.0`.
  - Files (section 11.4): `GRID_DIR/{layer}/{ts}.{generation}.bin` and
    `GRID_DIR/{layer}/{ts}.meta.json`. Data is little-endian (native) float32,
    row-major, `height x width`.
- Errors are logged and swallowed. A failed grid dump does not fail the frame.

### 5.14 Processed state [state:12-79, aqm:371, 515-521]

- File: `STATE_DIR/ingest-airquality.json`. Format `{"items": ["20260716_12", ...]}`.
  Load also accepts a bare JSON array.
- Max 100 entries. Adding past the cap drops the oldest. Writes are atomic
  (tempfile in the same dir, then rename).
- `aqm_mark_processed` adds the run id. It is called only after the air-quality
  layer published.

### 5.15 Cleanup (`aqm_cleanup`) [aqm:524-556]

```
manifest = read_manifest_file(STATE_DIR)
cutoff = now - retention_hours * 3600
removed = 0
for layer in [air-quality, ozone]:
  current = manifest.layers[layer].run_id (or none)
  for palette_dir in sorted(TILE_DIR/layer):
    runs_root = palette_dir/runs
    if not dir: continue
    for run_dir in runs_root:
      if not dir or run_dir.name == current: continue
      try run_dt = strptime(run_dir.name, "%Y%m%d_%H") as UTC   except ValueError: continue
      if run_dt.timestamp() < cutoff: rmtree(run_dir, ignore_errors); removed += 1
return {tile_dirs_removed: removed}
```

- Age is measured from the run's issue time, not from publish time.
- Only whole run directories are removed. The manifest is not edited, since
  the removed runs are not current.
- Staging `.tmp` directories inside a run dir are removed only when the whole
  run dir ages out (Q-6).

### 5.16 Heartbeats and temp dirs [aqm:380-441, activity_heartbeat:16-38]

- Per-attempt temp dir: `AQM_TMP_ROOT / "{prefix}-{wf_id}-{run_id}-{act_id}-attempt{N}-{run}-{layer}-m{start:02d}"`
  with each component sanitized to `[A-Za-z0-9_-]` (other chars become `_`,
  then trim `_`). Created before the download, removed in `finally`.
- Heartbeat at the start of download (`{phase: download, layer, start_msg}`).
- While the download thread runs, a background task heartbeats every 30 s with
  the same details (`run_sync_with_heartbeat`).
- Then heartbeat `{phase: render, layer, start_msg}`, then during the render
  thread every 30 s with `{phase: render, run_id, layer, start_msg}`.
- heartbeat_timeout 120 s. A heartbeat detail is sent every 30 s, so a chunk
  stalls only if the blocking work stops returning control for 120 s.

### 5.17 Output tree (AQM)

```
TILE_DIR/
  air-quality/
    classic/runs/{run_id}/{ts}/{z}/{x}/{y}.png        (z in 4..6)
    muted/runs/...      (if in PALETTES)
    vivid/runs/...
    .render-publish-locks/{00..3f}.lock
  ozone/
    classic/runs/{run_id}/{ts}/...
    ...
GRID_DIR/
  air-quality/{ts}.meta.json, {ts}.{gen}.bin
  ozone/{ts}.meta.json, {ts}.{gen}.bin
STATE_DIR/
  manifest.json (+ manifest.lock)
  ingest-airquality.json
```

Tile path shape for a frame: `runs/20260716_12/2026-07-16T12:00:00+00:00/4/3/5.png`.
Manifest `path` equals `runs/{run_id}/{ts}`. The client URL is
`/tiles/{layer}/{palette}/{path}/{z}/{x}/{y}.png` (manifest template).

---

## 6. Blitzortung lightning [lightning]

### 6.1 Endpoints [lightning:43-68]

Primary (verified 2026-09-29 as accepting wss on 443):

```
wss://ws1.blitzortung.org:443/
wss://ws2.blitzortung.org:443/
wss://ws7.blitzortung.org:443/
wss://ws8.blitzortung.org:443/
```

Legacy (plain ws; `ws1`..`ws8` x ports 8087, 8088, 8089, 8090, in that nested
order: host outer, port inner) — 32 URLs, e.g. `ws://ws1.blitzortung.org:8087/`
through `ws://ws8.blitzortung.org:8090/`. Every legacy port refused connections
when verified; they are a last resort.

Override: env `LIGHTNING_WS_ENDPOINTS` (comma-separated, each trimmed, empties
dropped). When set, it *replaces the primary list only*. Legacy remains.

### 6.2 Configuration knobs

| Env | Default | Meaning |
|---|---|---|
| `STATE_DIR` | `/data/state` | output dir; output file `lightning.json` |
| `LIGHTNING_RETENTION_MIN` | 15 | rolling window, minutes |
| `LIGHTNING_FLUSH_S` | 2.0 | min seconds between flushes driven by strikes |
| `LIGHTNING_MAX_STRIKES` | 5000 | buffer cap |
| `LIGHTNING_WS_ENDPOINTS` | unset | override primaries |
| `LIGHTNING_LAT_MIN` | 15 | BBOX south |
| `LIGHTNING_LAT_MAX` | 55 | BBOX north |
| `LIGHTNING_LON_MIN` | -130 | BBOX west |
| `LIGHTNING_LON_MAX` | -50 | BBOX east |

Constants (not env): `RECONNECT_AFTER_CLOSE_S = 1.0`, `FAILURE_BACKOFF_MAX_S = 30.0`,
`DEMOTE_PRIMARY_S = 60.0`, `DEMOTE_LEGACY_S = 900.0`, heartbeat/flush tick 30 s,
websocket `ping_interval=30`, `ping_timeout=30`, `open_timeout=10`.
`RUN_DURATION_S = 3000`.

### 6.3 EndpointPicker (exact) [lightning:92-146]

State: `primary[]`, `legacy[]`, `cursor` (starts 0), `demoted_until: map url -> t`,
`consecutive_failures` (starts 0).

```
healthy(urls) = [u for u in urls if demoted_until.get(u, 0) <= now]

next():
  candidates = healthy(primary)
  if empty: candidates = healthy(legacy)
  if empty:  demoted_until.clear(); candidates = primary       # never stall
  url = candidates[cursor % len(candidates)]
  cursor += 1
  return url

succeeded(url):                       # session delivered >= 1 message
  consecutive_failures = 0
  demoted_until.pop(url)
  return RECONNECT_AFTER_CLOSE_S      # 1.0

failed(url):                          # session delivered 0 messages
  consecutive_failures += 1
  demote = DEMOTE_PRIMARY_S if url in primary else DEMOTE_LEGACY_S
  demoted_until[url] = now + demote
  base = min(30.0, 2 ** min(consecutive_failures, 8))
  return base * (0.5 + 0.5 * rand())   # rand() in [0,1)
```

Verified delay table (with rand = 0.5; the upper bound for rand -> 1 is `base`):

| consecutive_failures n | base = min(30, 2^min(n,8)) | delay at rand=0.5 | delay max |
|---|---|---|---|
| 1 | 2 | 1.5 | 2 |
| 2 | 4 | 3.0 | 4 |
| 3 | 8 | 6.0 | 8 |
| 4 | 16 | 12.0 | 16 |
| 5..9 | 30 | 22.5 | 30 |

`cursor` is global. It is not reset when the candidate list changes. Replicate
that modulo behavior, since the index it produces determines which endpoint is
picked.

### 6.4 Activity loop [lightning:256-414]

```
deadline = monotonic() + duration_s
strikes = deque(maxlen=MAX_STRIKES)
stats = {msgs, parsed, in_bbox, buffer}                 # + seeded later
seed = _load_existing_strikes()                         # section 6.10
strikes.extend(seed); prune(); stats.seeded = len(strikes)
write_geojson(strikes)                                  # publish immediately
start background beat task (below)
picker = EndpointPicker(primary, legacy)
while monotonic() < deadline:
  url = picker.next()
  session_started = monotonic(); session_msgs = 0
  try:
    connect ws(url, ping_interval=30, ping_timeout=30, open_timeout=10)
    send TEXT  {"a": 111}          # exact bytes: {"a": 111}
    async for msg in ws:
      if monotonic() >= deadline: break
      stats.msgs += 1; session_msgs += 1
      decode message (6.5); if no object: continue
      stats.parsed += 1
      lat = float(data.get("lat", 0)); lon = float(data.get("lon", 0))
      t_ns = int(data.get("time", 0))
      if not in_bbox(lat, lon) or t_ns == 0: continue
      stats.in_bbox += 1
      strikes.append({t: t_ns/1e9, lat, lon, pol: int(data.get("pol",0)), mds: int(data.get("mds",0))})
      prune()
      now = time(); if now - last_flush >= FLUSH_EVERY_S: flush(); last_flush = now
  except Exception as exc:
    if session_msgs: delay = picker.succeeded(url); log ws_closed
    else:            delay = picker.failed(url); connect_failures += 1; log ws_disconnect
    flush()
  else:                                                 # clean close (or deadline break)
    delay = picker.succeeded(url) if session_msgs else picker.failed(url)
    if not session_msgs: connect_failures += 1
    log ws_closed
  remaining = deadline - monotonic()
  if remaining <= 0: break
  sleep(min(delay, remaining))
cancel beat; flush()
return LightningRunResult(...)
```

Q-7: the `try` covers the entire message loop. Any exception raised inside the loop
body (for example `data.get` on a JSON array, `float(None)`, `int("x")`) is caught
by the same `except`. The session then ends, counts as `succeeded` if it had
already delivered a message, and the port reconnects. The bad frame is not skipped.
DECIDE D-4.

The `break` on deadline exits the `async for` normally, so it takes the `else`
path.

### 6.5 Frame decoding [lightning:319-339]

For each websocket message:

1. If binary: decode as latin-1 (each byte becomes code point 0..255). Decoding
   never fails for latin-1. Q-8: the LZW decoder below never fires on binary
   frames, since all code points are < 256.
2. If neither text nor binary: skip (count `msgs` already incremented).
3. Try, in order:
   a. `json.loads(msg)` (raw text).
   b. `json.loads(_decode_payload(msg))`.
   If both fail: skip the frame.
4. The parsed value may be any JSON type. Only an object works. Anything else
   triggers Q-7.

Per frame the code reads: `lat` (float, default 0), `lon` (float, default 0),
`time` (int, nanoseconds since epoch; default 0, and 0 means skip), `pol` (int,
default 0), `mds` (int, default 0). The raw-frame keys are the Blitzortung feed's
own; the port must keep these exact names.

### 6.6 Blitzortung LZW-variant decode (`_decode_payload`) [lightning:190-212]

Verbatim algorithm. Input `raw` (a string of code points).

```
def decode(raw):
    if not raw: return raw
    try:
        dict_codes = {}                    # code (>=256) -> string
        curr = raw[0]
        result = [curr]
        code = 256
        for i in 1 .. len(raw)-1:
            ch = ord(raw[i])
            if ch < 256:
                entry = raw[i]                             # literal char
            elif ch in dict_codes:
                entry = dict_codes[ch]                     # known code
            else:
                entry = curr + curr[0]                     # KwKwK case
            result.append(entry)
            dict_codes[code] = curr + entry[0]             # new dictionary entry
            code += 1
            curr = entry
        return "".join(result)
    except Exception:
        return raw
```

Notes for the port:
- Operate on Unicode code points, not bytes or UTF-8 bytes.
- `entry[0]` and `curr[0]` are the first code point of the string.
- Dictionary keys are code points starting at 256, one per emitted symbol after
  the first.
- It never raises for well-formed input. The `except` returns the input unchanged.

Verified vectors (produced by running this exact function):

| raw | decoded |
|---|---|
| `""` | `""` |
| `"abc"` | `"abc"` |
| `"aĀ"` | `"aaa"` |
| `"abĀ"` | `"abab"` |
| `"abĀā"` | `"ababba"` |
| `"{\"a\":1}"` | `"{\"a\":1}"` (plain JSON passes through) |

### 6.7 Strike filter and record [lightning:341-353]

- Keep strike iff `BBOX_LAT_MIN <= lat <= BBOX_LAT_MAX` and
  `BBOX_LON_MIN <= lon <= BBOX_LON_MAX` (inclusive) and `t_ns != 0`.
- Stored record: `{t: t_ns / 1e9 (float), lat, lon, pol, mds}`.
- `t` is seconds as float, the unit used everywhere after this point.

### 6.8 Buffer and prune

- `strikes` is a deque with `maxlen = MAX_STRIKES`. Appending past the cap
  silently drops the oldest.
- `prune()` pops from the left while `strikes[0].t < now - RETENTION_MIN*60`.
- Prune runs after each append and in every 30 s beat.

### 6.9 Output file `STATE_DIR/lightning.json` [lightning:219-253]

Written by `_write_geojson(strikes)` (strikes snapshot via `list(strikes)`).
Format: `json.dumps` with default separators (`", "`, `": "`), not compact.

```json
{
  "type": "FeatureCollection",
  "features": [
    {
      "type": "Feature",
      "geometry": {"type": "Point", "coordinates": [lon, lat]},
      "properties": {
        "time": 1789999999.123456789,   // float seconds
        "age_s": 12,                    // int(now - time), truncated toward zero
        "polarity": 1,                  // int, default 0
        "mds": 4                        // int, default 0
      }
    }
  ],
  "generated_at": 1789999999.5,         // float, time.time() at write
  "retention_min": 15                   // int, RETENTION_MIN
}
```

Atomic write: `mkstemp(prefix=".lightning.json.", suffix=".tmp", dir=STATE_DIR)`,
write, then `os.replace` to `lightning.json`, then unlink the temp if it still
exists. Creates `STATE_DIR` if missing. The temp file leaves nothing behind
(verified by the test).

Flush cadence: after each in-bbox strike, if `now - last_flush >= 2.0`; every
30 s from the beat task; once at start; once after the loop. `last_flush`
starts at `0.0`, so the first strike flushes immediately.

### 6.10 Seeding [lightning:149-187]

`_load_existing_strikes(path=OUT_PATH, retention_min=RETENTION_MIN, now=time())`:

- Read `lightning.json`. Missing file, bad JSON, or non-object top level: `[]`.
- For each feature (skip any that fail): `props = feature.properties`,
  `lon, lat = feature.geometry.coordinates[:2]`, `t = float(props.time)`.
  Skip if `t < now - retention*60`. Keep `{t, lat, lon, pol: int(props.polarity or 0),
  mds: int(props.mds or 0)}`.
  Catches: KeyError, TypeError, ValueError, IndexError.
- Sort by `t` ascending, keep the last `MAX_STRIKES`.

Verified by tests: a feature 20 min old is dropped, a 60 s and a 30 s old feature
are kept, and malformed features are ignored.

### 6.11 Heartbeats and task shape [lightning:278-306]

- A background task sleeps 30 s, then: `prune()`, `stats.buffer = len(strikes)`,
  flush (errors logged, not raised), `activity.heartbeat(stats)`.
- The beat task is cancelled and awaited in `finally`.
- Rationale in the code: the stream loop can block for long stretches. Inline
  heartbeats would starve.
- The blocking JSON write is off the event loop (`to_thread`) with a snapshot.

### 6.12 Result

`duration_s = round(elapsed, 1)` (Python `round`, half-to-even). `msgs`, `parsed`,
`in_bbox`, `final_buffer`, `seeded`, `connect_failures` as above.

### 6.13 Behavioral summary

- A run lasts about 50 min (3000 s). The schedule fires hourly with SKIP. A
  still-running run makes the next tick a no-op.
- Strikes older than 15 min disappear from the file at the next flush.
- A new run seeds from the previous file, so the layer does not blank at rollover.
- A server close after data reconnects after 1 s. A session that delivered
  nothing backs off, and its endpoint is demoted for 60 s (primary) or 900 s
  (legacy).

---

## 7. NHC tropical cyclones [tropical]

### 7.1 Sources

- Feed: env `TROPICAL_FEED_URL`, default `https://www.nhc.noaa.gov/CurrentStorms.json`.
  `GET` with 20 s timeout, `raise_for_status`, JSON body.
- GIS: env `TROPICAL_GIS_URL`, default
  `https://mapservices.weather.noaa.gov/tropical/rest/services/tropical/NHC_tropical_weather/MapServer`.
  The feed links only KMZ/zip files, so the GIS MapServer supplies cone, track
  and points as GeoJSON.

### 7.2 GIS fetch [tropical:125-162]

Best effort. Any failure of the index returns `{}` (feed positions still publish).

```
bins = [s.binNumber for s in feed.activeStorms if s.binNumber]   # may repeat
index = GET {GIS_URL}?f=json                    # timeout 15 s, raise_for_status
ids = {layer.name: layer.id for layer in index.layers}           # from JSON
jobs = {(b, kind): ids["{b} {suffix}"]
        for b in bins for kind, suffix in
          [("cone", "Forecast Cone"), ("track", "Forecast Track"), ("points", "Forecast Points")]
        if "{b} {suffix}" in ids}                               # dedup via dict
parallel (8 threads) for each job:
    GET {GIS_URL}/{layer_id}/query?where=1%3D1&outFields=*&f=geojson   # timeout 15 s
    on HTTP error or ValueError: result None (logged)
out[b][kind] = fc  only if fc is truthy (non-empty dict)
```

Query parameters, exactly: `where=1=1`, `outFields=*`, `f=geojson`.

Failures are per-layer. A failed layer contributes nothing and does not fail the run.

### 7.3 GeoJSON build [tropical:59-122]

Inputs: the feed and `gis[bin][kind]` (FeatureCollection dicts).
`storms = feed.activeStorms or feed.storms or []`.

Per storm, with:

| Local | Source (first non-empty, Python `or` semantics) |
|---|---|
| `sid` | `id` or `stormId` or `""` |
| `name` | `name` or `storm_name` or `"Unknown"` |
| `classification` | `classification` or `""` |
| `basin` | `binNumber` or `basin` or `""` |
| `lat` | `latitudeNumeric` or `lat` (raw value) |
| `lon` | `longitudeNumeric` or `lon` (raw value) |
| `wind_kt` | `_num(intensity or windSpeed)` |
| `pressure` | `_num(pressure or minPressure)` |
| `base` | `{"storm_id": sid, "name": name}` |

`_num(v)`: `float(v)` or `None` on TypeError/ValueError. NHC sends numbers as
strings, so `"100"` becomes `100.0`.

Features are appended in this order for each storm:

1. **position** (only if `lat is not None and lon is not None`):
   - `geometry`: `{"type": "Point", "coordinates": [float(lon), float(lat)]}`
   - `properties`: `kind="position"`, `storm_id`, `name`, `classification`, `basin`,
     `wind_kt` (float or null), `wind_mph` (`round(wind_kt*1.15078)` or null),
     `category` (Saffir-Simpson or null), `pressure_mb` (float or null),
     `movement_dir_deg` (`_num(movementDir)`), `movement_mph` (`_num(movementSpeed)`),
     `updated_at` (raw `lastUpdate`), `advisory_url` (`publicAdvisory.url` or null).
2. **cone**: for each feature in `gis[basin].cone` with a truthy `geometry`:
   `{type: Feature, geometry: f.geometry, properties: {kind: "cone", storm_id, name}}`.
3. **track**: same with `kind: "track"` from `gis[basin].track`.
4. **forecast_point**: for each feature in `gis[basin].points` with truthy geometry.
   `props = f.properties or {}`, `kt = _num(props.maxwind)`.
   `properties`: `kind: "forecast_point"`, `storm_id`, `name`,
   `tau_h = _num(props.tau)`, `label = props.datelbl or props.fldatelbl`,
   `storm_type = props.dvlbl`, `wind_kt = kt`, `wind_mph = round(kt*1.15078)` or null,
   `category = _saffir_simpson(kt)`.

`basin` is the lookup key. Storms whose bin has no GIS layers only produce
the position feature. Storms with empty `basin` look up `gis[""]` (none), so
they also produce position only.

Saffir-Simpson, verified:

```
_saffir_simpson(kt):
  if kt is None or kt < 64: return None
  return 1 + count(kt >= t for t in [83, 96, 113, 137])
```

Verified: `[None, 63, 64, 83, 96, 113, 137] -> [None, None, 1, 2, 3, 4, 5]`.

Top level:
```json
{"type": "FeatureCollection", "features": [...],
 "generated_at": 1789999999.0, "storm_count": 1}
```
`storm_count = len(storms)` (not features). `generated_at = time.time()`.

### 7.4 Numeric rules

- `KT_TO_MPH = 1.15078`. `wind_mph = round(kt * 1.15078)` uses Python `round`:
  **half to even** (verified: `round(0.5) == 0`, `round(1.5) == 2`, `round(2.5) == 2`).
  Rust: `f64::round_ties_even`. Using `f64::round` would be wrong for exact halves.
- `round(100*1.15078) == 115`, `round(75*1.15078) == 86` (verified).
- Values come out as Python floats. In JSON, integers vs floats matter for
  readers only through numeric equality. Emit `wind_kt` as a float (`100.0`).

### 7.5 Output and write [tropical:165-182]

- File: `STATE_DIR/tropical.json`. `json.dumps` default separators.
- Write: `tmp = OUT_PATH.with_suffix(".tmp")` (so `tropical.tmp`), `write_text`,
  `tmp.replace(OUT_PATH)`. Q-9: the temp name is fixed, not unique (see DECIDE D-10).
  No fsync.
- Result: `TropicalResult(storm_count, feature_count)`.

### 7.6 Quirks

- Q-10: `latitudeNumeric or lat` treats a numeric `0` as missing. A storm at
  exactly `lat == 0` or `lon == 0` loses its position if the fallback key is
  absent. Same for `intensity or windSpeed` and `pressure or minPressure`. DECIDE D-11.
- KMZ-only feeds (`forecastTrack.kmzFile`) produce no fake tracks. Only real GIS
  geometry produces cone or track features (verified by test).

---

## 8. Tile and grid retention [tile_cleanup, cleanup.sh]

### 8.1 Canonical rules: `tile_cleanup_sweep` (Python) [tile_cleanup:25-139]

The Python activity replaces the legacy shell script. Treat it as canonical. The
shell script is specified in 8.8 so the differences are explicit.

Retention per layer (minutes), verbatim from `LAYER_RETENTION_MIN`:

| Layer | Minutes | Hours |
|---|---|---|
| `radar` | 240 | 4 |
| `radar-composite` | 240 | 4 |
| `nowcast` | 60 | 1 |
| `radar-hrrr` | 720 | 12 |
| `temperature` | 720 | 12 |
| `dewpoint` | 720 | 12 |
| `humidity` | 720 | 12 |
| `wind` | 720 | 12 |
| `cape` | 720 | 12 |
| `precip-type` | 720 | 12 |
| `precip-accum` | 720 | 12 |
| `cloud` | 720 | 12 |

Layers `air-quality` and `ozone` are not in this table. AQM is cleaned by
`aqm_cleanup` (5.15). `tile_cleanup` never touches them.

Activity result: `layers_swept = len(LAYER_RETENTION_MIN)` (always 12), plus
`tile_dirs_removed` (sum over layers) and `grid_files_removed` (from
`cleanup_old_grids()`).

### 8.2 Per-layer algorithm `_sweep_layer(layer, retention_min)` [tile_cleanup:55-120]

```
base = TILE_DIR/layer; if not exists: return 0
cutoff = now - retention_min*60
manifest = read_manifest_file(STATE_DIR)           # raises on corrupt file
current_run = manifest.layers[layer].run_id or None
candidates = []        # timestamp-named dirs (legacy and palette layouts)
versioned = []         # run dirs under <palette>/runs/
for entry in sorted(base children):
  if not dir: continue
  if entry.name starts with a digit:                     # legacy /{layer}/{ts}/
      candidates.append(entry)
  else:
      runs_root = entry/runs
      if runs_root is dir:
          versioned += [child dirs of runs_root]
          candidates += [child dirs of entry, excluding "runs"]
      else:
          candidates += [child dirs of entry]            # palette dir without runs
removed = 0
for run_dir in versioned:                                 # 8.3
  if run_dir.name == current_run: continue
  try issued = datetime.fromisoformat(run_dir.name) except ValueError: continue
  if issued.timestamp() < cutoff: rmtree(run_dir); removed += 1
for ts_dir in candidates:                                 # 8.4 and 8.5
  if ".tmp-" in name or name endswith ".tmp":             # staging leftover
      if mtime(ts_dir) < now - 3600: rmtree; removed += 1
      continue
  try dt = datetime.fromisoformat(ts_dir.name) except ValueError: continue
  if dt.timestamp() < cutoff:
      update_manifest(layer, ts_dir.name, action="remove")   # de-list FIRST
      rmtree(ts_dir); removed += 1
return removed
```

Ordering matters: the manifest entry is removed before the directory is deleted,
so the API never advertises a frame whose tiles are gone.

`datetime.fromisoformat` accepts ISO 8601 with offset, e.g.
`2026-07-16T12:00:00+00:00`. A naive value would be interpreted in local time by
`.timestamp()`. The container runs in UTC, so treat naive values as UTC.

### 8.3 Versioned runs

- Applies to `<layer>/<palette>/runs/<run_name>/`. Run names are ISO timestamps
  for the forecast layers (for example `2000-01-01T00:00:00+00:00`).
- A run named as the manifest's `layers[layer].run_id` is never removed.
- Removal is `rmtree` without manifest edits (the run is not current, so nothing
  advertised it).

### 8.4 Timestamp directories

- A timestamp directory is named by its valid time (ISO). Removal is by name, not mtime.
- The manifest entry is removed with `update_manifest_file(layer, name, "remove")`
  before `rmtree`. If the frame list becomes empty, the layer key is removed from
  `layers`.

### 8.5 Staging leftovers

- A directory name containing `.tmp-` or ending in `.tmp` is an orphaned
  atomic-render staging dir (created by `_render_and_publish_atomic`).
- It is removed only when its own mtime is older than 3600 s. Name-embedded
  time is not used because for forecast layers it can be in the future.

### 8.6 Grid retention `cleanup_old_grids()` [grid_dump:421-457]

Runs after the tile sweep. `GRID_DIR` (env, default `/data/grids`).

```
cutoff = now - GRID_MAX_AGE_S                      # env, default 43200 (12 h)
for layer_dir in GRID_DIR children (dirs only):
  with layer lock (.grid.lock, flock):
    keep = GRID_KEEP_LAST.get(layer_dir.name)
    if keep is not None: removed += _prune_grid_layer_unlocked(layer_dir, keep, now)
    for path in layer_dir children (except .grid.lock):
        if mtime(path) < cutoff: unlink; removed += 1   # errors ignored
  on OSError for the layer (e.g. lost+found): skip the layer
```

`GRID_KEEP_LAST` has one entry: layer named by env
`NOWCAST_GRID_INPUT_LAYER` (default `radar-nowcast-input`), keep =
`(max(3, NOWCAST_INPUT_FRAMES) - 1) * max(1, ceil(NOWCAST_INPUT_STEP_MIN / 1.8)) + 3`.
Defaults give 12 (verified). AQM layers are not in `GRID_KEEP_LAST`, so only the
age sweep applies to them.

`_prune_grid_layer_unlocked(layer_dir, keep, now)`:

1. `metas` = `*.meta.json` files sorted by name; `kept_metas` = last `keep`.
2. `keep_names` = kept meta names plus each kept meta's `data_file` (or
   `{ts}.bin` if the value is missing or not a bare name).
3. `committed_data` = `data_file` of every meta.
4. For each file: skip `.grid.lock`; skip names in `keep_names`; skip `.*.tmp`;
   only consider `*.bin` and meta names. For a `.bin` not referenced by any
   committed meta, skip if its mtime is within `GRID_ORPHAN_GRACE_S` (env,
   default 300 s). Otherwise unlink.

Q-11: the age sweep afterwards does not check `keep_names`. A kept file older
than `GRID_MAX_AGE_S` is deleted anyway. Consequence: if a keep-N layer stops
updating for 12 h, its whole history is deleted. Harmless for AQM (not keep-N).
DECIDE D-12 for nowcast owners.

### 8.7 Result

`TileCleanupResult(layers_swept=12, tile_dirs_removed, grid_files_removed)`.
Counting: each removed run dir, each removed timestamp dir, and each removed
grid file counts 1 (a `.bin` and `.meta.json` count separately).

### 8.8 Legacy `cleanup.sh` (superseded; specified for comparison) [cleanup.sh]

- `TILE_DIR` env default `/data/tiles`. Logs `Tile cleanup running at <UTC date>`.
- `cleanup_layer(layer, minutes)`: if `TILE_DIR/layer` exists:
  - `find base -mindepth 1 -maxdepth 1 -type d -name "[0-9]*" -mmin +M -exec rm -rf {} +`
  - `find base -mindepth 2 -maxdepth 2 -type d -name "[0-9]*" -mmin +M -exec rm -rf {} +`
- Calls: `radar` 240; `nowcast` 60; for `radar-hrrr temperature dewpoint humidity wind cape precip-type`: 720.
- `-mmin` uses the **directory's own mtime**, not the name.

### 8.9 Differences between `cleanup.sh` and `tile_cleanup_sweep`

| Aspect | `cleanup.sh` | Python (canonical) |
|---|---|---|
| Age source | directory mtime (`-mmin`) | name (valid time) for timestamp dirs; mtime for `.tmp` only |
| Versioned `runs/<run>/` dirs | never removed (depth 3, not searched) | removed when older than retention and not current |
| Manifest update | none (can advertise deleted frames) | `remove` before rmtree |
| `radar-composite` | not swept | 240 min |
| `precip-accum`, `cloud` | not swept | 720 min |
| `air-quality`, `ozone` | not swept | not swept (aqm_cleanup handles them) |
| grid retention | none | `cleanup_old_grids()` |
| staging `.tmp-` dirs | `-name "[0-9]*"` excludes them | swept after 3600 s mtime |

Treat `cleanup.sh` as retired (DECIDE D-2). If the port must keep any of its
behaviors, it is the versioned-runs leak that is most likely to matter in
production (disk growth).

---

## 9. Open-Meteo sync [open_meteo_sync, open_meteo_worker]

### 9.1 Binary and invocation [open_meteo_sync:28-64]

- Binary: env `OPENMETEO_BIN`, default `/app/openmeteo-api`.
- Data dir: env `OPENMETEO_DATA_DIR`, default `/app/data`. It is used only to
  compute the working directory: `cwd = parent(OPENMETEO_DATA_DIR)` (default `/app`).
- Command (exact argv):
  `[OPENMETEO_BIN, "sync", model, variables, "--past-days", str(past_days)]`.
  `model` is `ncep_gfs013`, `ncep_gfs025`, or `ncep_hrrr_conus`. `variables` is
  a comma-joined list with no spaces (section 9.6).
- Environment: inherited from the worker process. No env overrides.
- stdout and stderr are piped and drained line by line.
  Each line is decoded UTF-8 with replacement, right-trimmed, appended to a
  deque of the last 50 lines, and logged at debug.

### 9.2 Heartbeats and waiting [open_meteo_sync:90-101]

Loop until the process exits: `wait_for(proc.wait(), timeout=15)`. Every
iteration, if at least 30 s passed since the last heartbeat, send
`{model, elapsed_s}`.

### 9.3 Result classification [open_meteo_sync:105-119]

```
rc = returncode or 0
critical = first line in (stdout_tail + stderr_tail) containing "[ CRITICAL ]" or "[ ERROR ]"
if critical:
    raise ApplicationError("open-meteo {model} sync reported an error (rc={rc}): {critical[-500:]}",
                           non_retryable=False)
if rc != 0:
    raise ApplicationError("open-meteo {model} sync exited rc={rc}: {stderr_tail_joined[-500:]}",
                           non_retryable=False)
return {model, succeeded: true, duration_s: round(elapsed, 1), return_code: rc, stderr_tail: ""}
```

- The binary logs errors on stdout and exits 0 (linkAt failure, 1.5.6). A
  zero exit code therefore does not mean success. Only the scan decides.
- Q-12: only the last 50 lines of each stream are scanned. A critical line
  earlier than that is missed.
- Q-13: `critical[-500:]` keeps the **last** 500 characters of the matched line.
- Error messages are part of the Temporal failure text. Keep the format.

Verified examples (from `backend/open_meteo_sync/test_activities.py`):
- `"[ CRITICAL ] Error during sync linkAt(error: -1)"` is flagged and returned verbatim.
- `"[ ERROR ] Could not download chunk"` is flagged.
- Lines like `"[ INFO ] ..."` are not flagged.

### 9.4 Cancellation (Q-14)

When the Temporal activity is cancelled or times out, the Python activity does
not terminate the child process. The `while` loop raises `CancelledError`,
leaving the subprocess running. The port must kill the child on cancellation
(for example `kill_on_drop(true)` plus an explicit kill). DECIDE D-9 confirms.

### 9.5 Results and retries

- `OpenMeteoSyncResult` as in 4.5. Any error is retried by the workflow policy
  (3 attempts, 10 s initial, 5 min max).
- A retry runs the binary again from scratch. There is no resume state.

### 9.6 Variable lists (verbatim from `temporal/schedules/seed.py`, verified)

GFS 0.13 (`ncep_gfs013`):
```
temperature_2m,relative_humidity_2m,precipitation,showers,snowfall_water_equivalent,frozen_precipitation_percent,cloud_cover,wind_u_component_10m,wind_v_component_10m,shortwave_radiation,uv_index
```

GFS 0.25 surface (`ncep_gfs025`):
```
wind_gusts_10m,pressure_msl,cape,lifted_index,visibility,categorical_freezing_rain
```

HRRR CONUS (`ncep_hrrr_conus`):
```
temperature_2m,relative_humidity_2m,precipitation,snowfall_water_equivalent,frozen_precipitation_percent,categorical_freezing_rain,cloud_cover,wind_u_component_10m,wind_v_component_10m,wind_gusts_10m,pressure_msl,cape,lifted_index,visibility,shortwave_radiation
```

Comment in the source: `ncep_gfs025` no longer has surface variables. Those moved
to `ncep_gfs013`. Syncing the wrong model left the 7-day forecast null on
2026-06-13.

Derived variables (weather_code, wind speed/direction, apparent temperature,
dew point, surface pressure) are not synced. They are computed at query time from
the raw inputs.

### 9.7 Worker shape

- The `radar-ng-open-meteo-worker` pod (`deploy/k8s/open-meteo-worker-deployment.yaml`)
  runs image `registry.vanillax.me/radar-ng-open-meteo-worker:v1.1.1` (as of this
  spec), uid 999, env `OPENMETEO_BIN=/app/openmeteo-api` and
  `OPENMETEO_DATA_DIR=/app/data`, a PVC at `/app/data`, resources 200m/512Mi
  request, 2 CPU/4 GiB limit. Rollout is Progressive with a 50% step.
- No Kubernetes Jobs are created. The binary runs in the worker pod.

---

## 10. NWS active alerts (`PollAlertsWorkflow`) [storm_watch_activities, poll_alerts]

### 10.1 Fetch [storm_watch_activities:266-308]

- URL: env `NWS_ALERTS_URL`, default `https://api.weather.gov/alerts/active?status=actual`.
- Header: `User-Agent: <NWS_USER_AGENT>`. Env `NWS_USER_AGENT`, code default
  `(radar-ng, <placeholder contact>)` (a placeholder; see section 13). The NWS
  requires a real contact. The production configmap sets a real address; the
  spec does not repeat it. No other headers are set (httpx default `Accept: */*`).
- Timeout 20 s. `raise_for_status`. Body is GeoJSON. Only `features` is read.
- `alert_count = len(features)` (including features without an id).
- Each feature (`_alert_from_feature`): `props = feature.properties or {}`,
  `alert_id = props.id or feature.id`. Skip if falsy. `geometry = feature.geometry`
  if it is a dict, else `{}`. Stored as `str(alert_id)`.
- `active` = set of all ids seen this poll.
- `new` = ids not in `seen` (`alerts_seen.json`). Order is feed order.
- After the diff, write `alerts_active_snapshot.json` (sorted list of all active
  ids). A write failure is logged and ignored.
- The fetch does **not** commit `alerts_seen.json`. That happens in
  `mark_alerts_seen` after signalling (at-least-once).

### 10.2 State files

| File | Format | Writer | Reader |
|---|---|---|---|
| `STATE_DIR/alerts_seen.json` | JSON array of strings | `_save_seen` | `_load_seen` (missing or bad = empty set) |
| `STATE_DIR/alerts_active_snapshot.json` | JSON array (sorted) | fetch | `mark_alerts_seen` (missing/bad = None) |

Writes use `mkstemp(prefix=".alerts-seen." or ".alerts-active.", suffix=".tmp")`
then `os.replace`. Default `json.dump` separators.

### 10.3 `mark_alerts_seen(handled_ids)` [storm_watch_activities:338-360]

```
seen = load_seen()
handled = set(handled_ids)
active = load_active_snapshot()        # None if missing/bad
if active is None:
    save_seen(active=seen | handled, expired=set())          # never forget
else:
    save_seen(active=(seen & active) | handled, expired=seen - active)
```

`save_seen(active, expired)`:
```
capped = sorted(active) + sorted(expired)[: max(0, 5000 - len(active))]
```
Every currently-active id survives. Expired ids fill the rest of the 5000 cap.
Sorting is lexical (Python string order).

Verified by tests:
- seen `[old-1]`, snapshot `[old-1, new-ok, new-failed]`, handled `[new-ok]` gives
  seen containing `new-ok` and `old-1`, and not `new-failed`.
- seen `[expired-1, active-1]`, snapshot `[active-1]`, handled `[]` gives
  `{active-1, expired-1}`.
- no snapshot, seen `[old-1]`, handled `[new-ok]` gives `{old-1, new-ok}`.

### 10.4 Watch fan-out `signal_matching_storm_watches({alert_id, geometry})` [storm_watch_activities:435-467]

```
if not geometry: return {matched: 0}                  # defensive; workflow never sends this
client = singleton Temporal client
matched = 0
for w in client.list_workflows("WorkflowType='WatchStormWorkflow' AND ExecutionStatus='Running'"):
    try:
        state = query(w.id, "getCurrentState")
        lat = state.lat; lng = state.lng                  # dict key or attribute
        if lat is None or lng is None: continue
        if not point_in_geojson(lng=float(lng), lat=float(lat), geometry): continue
        signal(w.id, "alertMatchSignal", alert_id)        # payload: the alert id string
        matched += 1
    except Exception: log warning; continue                # per-watch errors do not fail
return {matched}
```

- Errors from `list_workflows` or client creation fail the activity (retried by
  the workflow). Per-watch errors are logged and swallowed. Q-15: an alert can
  therefore be marked handled even though one watch's signal failed. It is
  at-least-once per activity, not per watch.
- Pagination: `list_workflows` is async-iterated (the SDK pages).

### 10.5 Point in polygon (verbatim algorithm) [storm_watch_activities:383-426]

```
point_in_ring(lng, lat, ring):
    if ring is not a list or len(ring) < 4: return False
    inside = False
    prev = coord(ring[-1]); if prev is None: return False
    for point in ring:
        curr = coord(point); if curr is None: return False
        (curr_lng, curr_lat) = curr; (prev_lng, prev_lat) = prev
        crosses = (curr_lat > lat) != (prev_lat > lat)
        if crosses:
            intersect_lng = (prev_lng - curr_lng) * (lat - curr_lat) / (prev_lat - curr_lat) + curr_lng
            if lng < intersect_lng: inside = not inside
        prev = curr
    return inside

point_in_polygon(lng, lat, polygon):
    if polygon not a non-empty list: return False
    if not point_in_ring(lng, lat, polygon[0]): return False
    return not any(point_in_ring(lng, lat, hole) for hole in polygon[1:])

point_in_geojson(lng, lat, geometry):
    if not geometry: return False
    Polygon      -> point_in_polygon(lng, lat, coordinates)
    MultiPolygon -> any(point_in_polygon(lng, lat, p) for p in coordinates)
    else         -> False

coord(point): list/tuple with len >= 2; float(point[0]), float(point[1]); else None.
```

Coordinates are `[lng, lat]`. Holes subtract. Malformed hole rings count as "not in hole".

### 10.6 Workflow behavior invariants

- At-least-once: an alert is committed only if its watch signal activity succeeded
  or it had no geometry. A failed alert stays unseen and is retried next poll.
- Expired alerts stay in `alerts_seen.json` (within the cap) so a flapping feed
  does not re-notify.
- Missing snapshot never forgets seen alerts.
- Schedule runs never overlap (SKIP), so the snapshot is always from the same run.

---

## 11. Disk outputs catalog

### 11.1 Summary

| Path (relative to env dir) | Writer | Atomic | Reader |
|---|---|---|---|
| `TILE_DIR/<layer>/<palette>/runs/<run>/<ts>/{z}/{x}/{y}.png` | AQM render | staged then renamed | API tile server |
| `TILE_DIR/<layer>/<palette>/runs/<run>/<ts>/.render-complete-v2.json` | AQM render | create-only link | publication validation |
| `TILE_DIR/<layer>/.render-publish-locks/XX.lock` | publication | flock | publication |
| `STATE_DIR/manifest.json` (+ `manifest.lock`) | AQM publish, tile cleanup | tempfile + rename, flock | API (serves directly) |
| `STATE_DIR/ingest-airquality.json` | AQM mark | tempfile + rename | AQM find |
| `STATE_DIR/lightning.json` | lightning | tempfile + rename | API |
| `STATE_DIR/tropical.json` | tropical | `tropical.tmp` + replace (no fsync) | API |
| `STATE_DIR/alerts_seen.json` | mark alerts | tempfile + rename | fetch |
| `STATE_DIR/alerts_active_snapshot.json` | fetch alerts | tempfile + rename | mark alerts |
| `GRID_DIR/<layer>/<ts>.meta.json`, `<ts>.<gen>.bin` | AQM grid dump | fsynced bin, then meta rename | API inspector |
| `GRID_DIR/<layer>/.grid.lock` | grid writers | flock | grid writers |
| Open-Meteo data dir `OPENMETEO_DATA_DIR` | binary | binary-owned | forecast API |

### 11.2 `lightning.json`: see section 6.9.

### 11.3 `tropical.json`

```json
{
  "type": "FeatureCollection",
  "features": [
    {"type": "Feature",
     "geometry": {"type": "Point", "coordinates": [lon, lat]},
     "properties": {"kind": "position", "storm_id": "ep172026", "name": "Polo",
                    "classification": "HU", "basin": "EP2", "wind_kt": 100.0,
                    "wind_mph": 115, "category": 3, "pressure_mb": 963.0,
                    "movement_dir_deg": 35.0, "movement_mph": 13.0,
                    "updated_at": "2026-09-29T03:00:00.000Z",
                    "advisory_url": "https://www.nhc.noaa.gov/text/MIATCPEP2.shtml"}},
    {"type": "Feature", "geometry": {...Polygon or MultiPolygon from GIS...},
     "properties": {"kind": "cone", "storm_id": "...", "name": "..."}},
    {"type": "Feature", "geometry": {...LineString...},
     "properties": {"kind": "track", "storm_id": "...", "name": "..."}},
    {"type": "Feature", "geometry": {"type": "Point", ...},
     "properties": {"kind": "forecast_point", "storm_id": "...", "name": "...",
                    "tau_h": 12.0, "label": "11:00 AM Tue", "storm_type": "H",
                    "wind_kt": 75.0, "wind_mph": 86, "category": 1}}
  ],
  "generated_at": 1789999999.0,
  "storm_count": 1
}
```

Position properties are the exact set in 7.3. Empty lists are valid output.

### 11.4 Grid metadata `GRID_DIR/<layer>/<ts>.meta.json`

Keys (sorted in the file, compact separators, `sort_keys=True`, trailing newline):

```
data_file   string   "<ts>.<32-hex generation>.bin"
fill        float    -9999.0 (or the given fill)
height      int      rows after stride
lat_max     float    max lat after stride
lat_min     float
lon_max     float
lon_min     float
stride      int      1, 2, 4, ... (>= 1)
unit        string   "µg/m³" or "ppb"
width       int      cols after stride
```

Binary: float32, row-major, `height * width` values, native (little-endian)
byte order. Row 0 is the northernmost row. Missing values are `-9999.0`.

Commit order: write `.bin` and fsync -> fsync dir -> write meta via
mkstemp in the same dir, fsync, `os.replace` onto `<ts>.meta.json`, fsync dir.
The meta file is the commit pointer. The `.bin` for the old generation becomes
an orphan. Pruning removes it later.

### 11.5 Tile pyramid completion marker `.render-complete-v2.json`

Written once per palette pyramid, after tiles are fsynced:

```json
{"identity":{"algorithm":"rgba-bilinear-v1","grid_spec_digest":"<sha256>",
  "palette_digest":"<sha256>","palette_name":"classic","policy_digest":"<sha256>",
  "renderer":"legacy","semantic_digest":"<sha256>","source_digest":"<sha256>",
  "source_id":"aqm:air-quality:runs/20260716_12/2026-07-16T12:00:00+00:00",
  "tile_spec_digest":"<sha256>"},
 "schema_version":2,"tile_content_digest":"<sha256>","tile_count":N,
 "tile_path_digest":"<sha256>"}
```

Keys sorted; separators `,` and `:`; trailing newline. Create-only: written to a
temp file, fsynced, then `os.link`ed to the marker. If the marker already exists
with different bytes, the publish fails with a conflict. Digest algorithms are in
Appendix B.

### 11.6 `manifest.json` (AQM-relevant keys)

Top level (JSON keys sorted in the file; compact separators; trailing newline):

```
layers                 object   { "<layer>": LayerEntry }
schema_version         int      2
tile_url_template      string   "/tiles/{layer}/{palette}/{path}/{z}/{x}/{y}.png"
tile_url_template_legacy string "/tiles/{layer}/{timestamp}/{z}/{x}/{y}.png"
updated_at             string   ISO 8601 UTC with microseconds
```

LayerEntry (AQM):

```
complete      bool      true
frames        [Frame]   sorted by timestamp
timestamps    [string]  sorted (derived from frames)
latest        string    last frame's timestamp
palettes      [string]  sorted intersection of frame palettes
title         string    "AQM PM2.5 (US AQI)" or "AQM ozone"
kind          string    "model_guidance"
run_id        string    e.g. "20260716_12"
```

Frame:

```
timestamp              string  "2026-07-16T12:00:00+00:00"
path                   string  "runs/20260716_12/2026-07-16T12:00:00+00:00"
palettes               [string] sorted
source                 string  "aqm"
kind                   string  "model_guidance"
issued_at              string  "2026-07-16T12:00:00+00:00"   (run time)
run_id                 string  "20260716_12"
lead_minutes           int     0 .. 4260 (71*60)
spatial_resolution_km  float   5.0
max_zoom               int     6
```

`replace_layer_manifest` behavior is in section 12.1. Readers must not assume
legacy `frames` absence: `read_manifest_file` synthesizes frames from `timestamps`
for old manifests.

### 11.7 Processed set `STATE_DIR/ingest-airquality.json`

`{"items": ["20260716_12", ...]}` with at most 100 entries. Oldest dropped first.

---

## 12. Shared-module contracts

### 12.1 Manifest [manifest:22-222]

- `read_manifest_file(state_dir)`: missing file = empty manifest (schema 2, empty
  layers, now). Any other read or JSON failure **raises** (callers rewrite the
  file, so an error must not wipe history). If the top level is not an object or
  has no `layers` object: empty manifest. Fills missing keys (`tile_url_template`,
  `tile_url_template_legacy`, `updated_at`, `schema_version=1` if absent). For each
  layer, if `frames` is not a list, synthesize `frames` from `timestamps` with
  `palettes` from the layer (or `["classic"]`).
- `update_manifest_file(layer, timestamp, action, ...)` under `fcntl.flock` on
  `manifest.lock`:
  - `add`: replace any frame with the same timestamp, append the normalized frame,
    update layer metadata, re-sync indexes.
  - `remove`: drop frames with that timestamp; re-sync; if no frames remain, drop
    the layer entry.
- `replace_layer_manifest(layer, timestamps, palettes, frames, layer_metadata)`:
  under the same lock. If `timestamps` non-empty: replace the entire `frames` list
  with `sorted(set(timestamps))`, each normalized, merging the matching `frames`
  entry by timestamp; apply `layer_metadata`; re-sync. If empty: drop the layer.
- `_normalize_frame(ts, palettes, frame)`: copy the frame dict; set `timestamp`;
  `path = str(frame.path or ts)`; `palettes = sorted(set(str(p) for p in
  (palettes or frame.palettes or ["classic"])))`.
- `_sync_layer_indexes(layer)`: frames keyed by timestamp (dup-safe), sorted by
  key; `timestamps` = sorted timestamps; `latest` = last timestamp (absent if no
  frames); `palettes` = sorted intersection of each frame's palettes (empty list
  if none). Removed `latest` if no frames.
- Atomic write: `mkstemp(prefix=".manifest.json.", suffix=".tmp", dir=STATE_DIR)`
  (mode 0600), `json.dump(sort_keys=True, separators=(",", ":"))` + `"\n"`, then
  `os.replace`. Q-16: the 0600 mode from mkstemp is inherited. The API must be
  able to read it (DECIDE D-13 covers the mode).
- Every update sets `schema_version = 2` and `updated_at = now (UTC, isoformat)`.

### 12.2 Tile renderer (the parts aux uses)

#### 12.2.1 Class model [tiler:38-193]

- `build_class_model(tables)`: edges = sorted union of every table's
  `ranges[].min`, `ranges[].max`, and `no_data_below` (default -999). `raw_n =
  len(edges)+1`. For each palette, `lut[i]` for `i in 1..len(edges)-1` takes
  the RGBA of the **last** range (list order) with `range.min <= edges[i-1]` and
  `edges[i] <= range.max`, and only if `edges[i-1] >= no_data_below`. Otherwise
  transparent `[0,0,0,0]`.
- `_collapse_transparent`: classes visible in at least one palette are renumbered
  `1..k` in ascending raw order; class 0 is the transparent class. More than 255
  visible classes is an error.
- `classify(data)`: `idx = searchsorted(edges, v, side="right")`, then `remap[idx]`.
  Values are compared in the data's precision: if `data` is float32 and the cast
  edges are strictly increasing, edges are cast to float32 first. NaN or inf gives
  the last raw index, which is transparent (class 0). Values below the first edge
  are class 0.
- Interval semantics: a raw index covers `[edges[i-1], edges[i])`. Ranges are
  min-inclusive and max-exclusive. Verified edge: a value exactly `4.5` lands in
  `[4.5, 9.1)`.
- AQ class models: all bins are continuous, `no_data_below` 0 for both pm25 and
  ozone, so anything below 0 is transparent, as is anything at or above the last
  edge (500 for pm25, 600 for ozone).

#### 12.2.2 Legacy colorize-first path

`apply_color_table(data, table)` = `model(table).luts[0][classify(data)]`
(single palette model; the RGBA result has alpha 0 for class 0).

#### 12.2.3 Tile geometry (both paths) [tiler:196-216, 794-878]

- Tile x/y from lon/lat at zoom z: `n = 2^z`, `x = floor((lon+180)/360*n)`,
  `y = floor((1 - ln(tan(lat)+sec(lat))/pi)/2*n)`, both clamped to `[0, n-1]`.
- Tile bounds: `west = x/n*360-180`, `east = (x+1)/n*360-180`,
  `north = deg(atan(sinh(pi*(1-2y/n))))`, `south = deg(atan(sinh(pi*(1-2(y+1)/n))))`.
- Pixel centers for a 256-pixel tile: `px = (i+0.5)/256`.
  `lon_tile = (tx + px)/n*360 - 180`, `lat_tile = deg(atan(sinh(pi*(1-2*(ty+px)/n))))`.
- Regular grids (1D lat/lon): fractional index by `_axis_to_fractional_indices`:
  - Strictly increasing or decreasing axis; if all steps are equal (`allclose`
    rtol 1e-7, atol |step|*1e-10): affine `(v - axis[0]) / (axis[-1]-axis[0]) * (n-1)`.
  - Otherwise piecewise linear through `searchsorted` on the ordered axis.
  - Values outside the axis extent give fractional indices outside `[0, n-1]`.
    They are not clipped here.
- Projected grids: `(x,y) = transform(EPSG:4326 -> source_crs)(lon_mesh, lat_mesh)`,
  then `col = frac(x, source_x)`, `row = frac(y, source_y)`. Non-finite becomes -1.
- Row and column bounds are `[floor(min), ceil(max)]` clamped, over finite values.
  If no finite overlap, the tile is skipped.
- Tile iteration: for each zoom, `tx` from `tile_of(lat_max, lon_min)` to
  `tile_of(lat_min, lon_max)`, and `ty` the same, all inclusive.

#### 12.2.4 Sampling

- Legacy continuous: bilinear per RGBA channel (scipy `map_coordinates`, order 1,
  mode constant, cval 0). Output uint8. **Verify** scipy's constant-mode edge
  behavior against golden tiles before trusting a Rust reimplementation
  (DECIDE D-6 covers byte fidelity).
- Indexed continuous (canary): bilinear physical values, excluding invalid
  contributors, emit only when `valid_weight >= min_valid_weight` (1.0 for aux).
  Then classify.

#### 12.2.5 PNG encoding

- Legacy: RGBA PNG via Pillow, `compress_level=1`, `optimize=False`.
- Indexed: mode "P" PNG, PLTE = 256-entry RGB table, tRNS = alpha bytes, IDAT
  from the class index tile, `compress_level = TILE_PNG_COMPRESS_LEVEL` (default 6,
  clamped to 0..9). Other palettes: replace PLTE/tRNS chunks and their CRCs.
- The base requirements pin `Pillow==12.3.0` and `numpy==1.26.4` because "tile
  bytes must stay identical across releases" (docs/reliability-and-scale-plan.md,
  Gate 2). The legacy renderer is byte-identical to the deployed worker
  (615/615 tiles).

#### 12.2.6 Publication protocol (`_render_and_publish_atomic`) [tiler:1809-2056]

Inputs: `output_dirs` (palette -> final dir), `identities` (palette -> identity),
`render(staging_dirs) -> counts`, `lock_root` (= `TILE_DIR/<layer>`).

1. Outputs must be distinct and non-nested.
2. Locks: `lock_root/.render-publish-locks/`, 64 stripes. Stripe =
   `big_endian_u16(sha256(str(final_resolved))[:2]) % 64`. File `"{stripe:02x}.lock"`.
   Take `flock(LOCK_EX)` on each distinct stripe, in sorted path order.
3. Classify each final: missing -> unresolved; valid v2 marker with matching
   content -> bound; marker absent or schema v1 -> unresolved (adoption path);
   present but inconsistent -> error.
4. All bound:
   - all identities equal -> `EXISTING_VALID` for each, no render.
   - single compatible foreign renderer (section 12.2.7) -> `EXISTING_COMPATIBLE`.
   - else -> `PyramidIdentityConflict`.
5. Some bound: each bound identity must equal the expected one, else conflict.
   Bound ones are `EXISTING_VALID`.
6. Render unresolved palettes into staging: `final.parent/.{final.name}.tmp-{uuid4}`.
7. For each staged palette: PNG file count must equal the render's reported count.
   Count 0: discard, outcome `EMPTY` (and error if a final already exists).
8. Count > 0: fsync tiles and dirs; write the v2 marker (`identity`); re-inspect.
9. Decide per palette before mutating:
   - count 0 -> `EMPTY`.
   - final missing -> `CREATED` by rename.
   - final exists unbound -> `ADOPTED` only if every relative PNG path and byte
     matches the staging tree; else conflict.
   - final exists bound with a different identity -> conflict.
   - final exists bound with the same identity -> `EXISTING_VALID`.
10. Rename staging to final. On `EEXIST`/`ENOTEMPTY`, re-inspect the winner and
    adopt or accept it.
11. On any exception, remove all staging dirs and re-raise.

The manifest is the visibility gate. The group of palette directories is not
renamed together. Each palette pyramid is atomic, and frame visibility is decided
later by the manifest.

#### 12.2.7 Cross-renderer compatibility

`legacy/rgba-bilinear-v1` and `indexed/physical-bilinear-classify-v1` (or
`indexed/category-nearest-v1`) are compatible only if they match on `source_id`,
`source_digest`, `grid_spec_digest`, `palette_name`, `palette_digest`,
`tile_spec_digest`, and `semantic_digest`. Anything else is a conflict.

### 12.3 Grid dump: write path [grid_dump:112-222]

Covered in 5.13. `GRID_MAX_CELLS` (env, default 810000) controls stride.

### 12.4 ProcessedSet [state:12-79]

Covered in 5.14. Load tolerates bad JSON (empty set). `add` flushes atomically.

### 12.5 Palette loader [palettes:14-41]

- `get_palette_names()`: env `PALETTES` split on `,`, trimmed, empty dropped,
  default `classic`.
- `load_palette(name)`: `backend/shared/palettes/<name>.json`. Fallback to
  `color_tables.json` only for `classic`.

### 12.6 Heartbeat helper [activity_heartbeat:16-38]

`run_sync_with_heartbeat(fn, *args, heartbeat_every, heartbeat_details)`: run `fn`
in a worker thread; a sibling task sleeps `heartbeat_every` and calls
`activity.heartbeat(details)` (details may be a callable) until the work is done.
The sibling is cancelled afterwards. Exceptions from `fn` propagate.

---

## 13. Environment variables and knobs (in-scope only)

Columns: env var; code default (verified in source); documented default
(docs/configuration.md or .env.example); production value (k8s configmap) if set.

| Variable | Code default | Doc default | Production / notes |
|---|---|---|---|
| `TILE_DIR` | `/data/tiles` | - | compose sets `/data/tiles` |
| `STATE_DIR` | `/data/state` | - | compose sets `/data/state` |
| `GRID_DIR` | `/data/grids` | - | |
| `AQM_TMP_ROOT` | `/tmp/aqm_work` | - | |
| `PALETTES` | `classic` | `classic,muted,vivid` (**mismatch**, D-8) | `classic,muted,vivid` |
| `TILE_RENDERER` | `legacy` | - | legacy (canary default) |
| `TILE_RENDERER_CANARY_ROLE` | unset | - | must be one of mrms,nowcast,hrrr,aux; required with indexed |
| `TILE_GEOMETRY_CACHE_MB` | 256 (0..4096) | - | |
| `TILE_GEOMETRY_CACHE_ENTRIES` | 4096 (1..1e6) | - | |
| `TILE_PNG_COMPRESS_LEVEL` | 6 (0..9) | - | |
| `GRID_MAX_AGE_S` | 43200 | - | |
| `GRID_MAX_CELLS` | 810000 (900*900) | - | |
| `GRID_ORPHAN_GRACE_S` | 300 (>=0) | - | |
| `NOWCAST_GRID_INPUT_LAYER` | `radar-nowcast-input` | - | affects keep-N only |
| `NOWCAST_INPUT_FRAMES` | 4 (>=3) | 4 | 4 |
| `NOWCAST_INPUT_STEP_MIN` | 5.0 | 5 | 5 |
| `LIGHTNING_RETENTION_MIN` | 15 | 15 | 15 |
| `LIGHTNING_FLUSH_S` | 2.0 | 2.0 | 2.0 |
| `LIGHTNING_MAX_STRIKES` | 5000 | - | |
| `LIGHTNING_WS_ENDPOINTS` | unset | unset | unset |
| `LIGHTNING_LAT_MIN` / `_LAT_MAX` | 15 / 55 | - | |
| `LIGHTNING_LON_MIN` / `_LON_MAX` | -130 / -50 | - | |
| `TROPICAL_FEED_URL` | `https://www.nhc.noaa.gov/CurrentStorms.json` | same | same |
| `TROPICAL_GIS_URL` | NHC MapServer URL (section 7.1) | - | |
| `NWS_ALERTS_URL` | `https://api.weather.gov/alerts/active?status=actual` | same | same |
| `NWS_USER_AGENT` | `(radar-ng, <placeholder contact>)` (placeholder) | `(radar-ng, you@example.com)` | real contact set (not reproduced here) |
| `OPENMETEO_BIN` | `/app/openmeteo-api` | - | same |
| `OPENMETEO_DATA_DIR` | `/app/data` | - | same |
| `WORKER_ROLE` | `legacy` | `legacy` | role per deployment |
| `TEMPORAL_TASK_QUEUE` | role default | - | override queue |
| `USE_ISOLATED_TASK_QUEUES` | `0` (schedule module) | `0` | role queues when `1` |
| `SEED_SCHEDULES` | unset (legacy/all seeds) | - | |
| `SKIP_SCHEDULE_SEED` | `0` | `0` | |
| `TEMPORAL_ADDRESS` | `localhost:7233` (worker code) | `temporal:7233` (compose) | cluster |
| `TEMPORAL_NAMESPACE` | `default` | `default` | |
| `TEMPORAL_MAX_CONCURRENT_ACTIVITIES` | 4 | 2 (**mismatch**, D-8) | 4 |
| `TEMPORAL_MAX_CONCURRENT_ACTIVITY_TASK_POLLS` | 2 | 1 (**mismatch**) | 2 |
| `TEMPORAL_GRACEFUL_SHUTDOWN_S` | 25 | - | |
| `TEMPORAL_DEPLOYMENT_NAME`, `TEMPORAL_WORKER_BUILD_ID` | unset (no versioning) | - | injected by controller |

Also read in the shared code (not aux-specific): `NOWCAST_*` other knobs, storm
watch knobs, `PUSH_DISABLED` — out of scope.

---

## 14. Discrepancies, quirks, and DECIDE items

### 14.1 Doc/code mismatches (Q-series)

| ID | Item | Code | Docs | Recommendation |
|---|---|---|---|---|
| D-8 | `PALETTES` default | `classic` (`palettes.py:27`) | `classic,muted,vivid` (configuration.md, .env.example) | Port the code default, and set production explicitly |
| D-8b | `TEMPORAL_MAX_CONCURRENT_ACTIVITIES` default | 4 | 2 | Code default |
| D-8c | `NWS_USER_AGENT` default | `(radar-ng, <placeholder contact>)` | `(radar-ng, you@example.com)` | Fail fast if unset in production; NWS needs a real contact |
| Q-1 | `layers_swept` | always 12 | "layers swept" | Keep for compat; or report actual count |
| Q-4 | palette fallback | to `color_tables.json` for classic only, lacks AQ keys | - | Fail if no palette loads |

### 14.2 Other quirks

| ID | Where | Behavior | Note |
|---|---|---|---|
| Q-2 | AQM find | HEAD must return 200 exactly; no redirects followed | reqwest needs `redirect::Policy::none()` |
| Q-3 | AQM download | only 404 falls back to NOMADS; other status fails the attempt | see D-5 |
| Q-5 | grid dump | lon order not normalized unless max > 180 | verify source order (D-7) |
| Q-6 | AQM cleanup | orphaned `.tmp` staging inside a live run dir waits for the run to age out | acceptable |
| Q-7 | lightning | a bad frame (valid non-object JSON, or wrong types) ends the session | see D-4 |
| Q-8 | lightning | LZW decode is only meaningful for text frames | |
| Q-9 | tropical | fixed temp name `tropical.tmp`; no fsync | see D-10 |
| Q-10 | tropical | `or`-fallback treats numeric 0 as missing | see D-11 |
| Q-11 | grid cleanup | age sweep ignores keep-N protection | see D-12 |
| Q-12 | open-meteo | only last 50 lines of each stream scanned | |
| Q-13 | open-meteo | error message keeps last 500 chars | keep format |
| Q-14 | open-meteo | child process not killed on cancel | see D-9 |
| Q-15 | alerts | per-watch failures swallowed; alert still marked handled | at-least-once per activity |
| Q-16 | manifest/state files | mkstemp creates mode 0600 | see D-13 |
| Q-17 | tile cleanup | `read_manifest_file(STATE_DIR)` and `update_manifest_file` may use different state dirs in tests | one state dir in the port |
| Q-18 | cleanup | `update_manifest_file` default state_dir from env, not the sweeper's | same as Q-17 |
| Q-19 | AQM publish | frame palettes are key-presence based (`color_key in table`) | keep |
| Q-20 | AQM chunk | full GRIB redownloaded per chunk (x6 per run) | optimization candidate |

### 14.3 DECIDE items (owner must choose before implementation)

| ID | Question | Options | Recommendation |
|---|---|---|---|
| D-1 | Must the Rust port adopt or coexist with Python-written pyramids and markers in the same `TILE_DIR`? | (a) byte-identical tiles + identical digests (large effort; zlib-level match needed); (b) new `TILE_DIR` root or distinct path namespace; (c) cutover at a run boundary with exactly one schedule owner | (c) plus (b) for safety. Mixed renderers on one run fail closed. |
| D-2 | Retire `cleanup.sh` semantics? | retire / keep | Retire. Port the Python rules. |
| D-3 | PNG bytes: byte-identical to Python legacy output, or semantic equivalence? | byte-identical (requires zlib-compatible deflate at level 1) / semantic | Semantic, unless D-1(a) |
| D-4 | Malformed lightning frame: end session (today) or skip frame and count? | keep / skip | Skip and count; log once per session |
| D-5 | AQM download fallback: 404-only (today) or any non-2xx? | keep / fallback on any non-2xx / error | Fallback on any non-2xx, keep 404 semantics for logging |
| D-6 | Download once per (run, layer) and reuse across chunks? | per chunk (today) / cached | Cache per run, same outputs |
| D-7 | Lon order for grid dumps | keep / normalize ascending | Normalize ascending, and document the change |
| D-8 | Palette/concurrency defaults | code / docs | Code values, with explicit production values |
| D-9 | Kill open-meteo child on cancel | kill / leave | Kill (with `kill_on_drop`) |
| D-10 | Tropical temp name | fixed / unique | Unique (`tempfile` in STATE_DIR) |
| D-11 | Tropical zero-as-missing | keep / explicit None checks | Explicit None checks |
| D-12 | Grid age sweep vs keep-N | keep / protect keep-N | Protect keep-N (nowcast owners to confirm) |
| D-13 | Manifest and state file mode | 0600 (today) / 0644 | Confirm the API's uid; use 0644 if they differ |

---

## 15. Invariant checklist (from tests and code assertions)

### 15.1 AQM (`backend/ingest_airquality/test_activities.py`)

- [ ] `_grib_urls("20260716_12", "ave_1hr_pm25_bc")` returns exactly the two URLs in 5.3, AWS first.
- [ ] Discovery with AWS 404 and NOMADS 200 returns the run id, and the HEAD order is the URLs of that run in order (AWS then NOMADS).
- [ ] `_publish_run_sync` with only 50 of 72 frames for `air-quality` returns `[]` and writes no `manifest.json`.
- [ ] With two complete chunks for `air-quality` (0-35 and 36-71) and one incomplete `ozone` chunk, it returns `["air-quality"]`.
- [ ] Layer entry: `complete` is true, `run_id` is `20260716_12`, `frames` length is 72.
- [ ] `palettes` is `["classic","vivid"]` when `muted` lacks the key.
- [ ] First frame path is `runs/20260716_12/2026-07-16T12:00:00+00:00`, `lead_minutes` 0, `source` `"aqm"`.
- [ ] Last frame `lead_minutes` is `71*60 = 4260`.
- [ ] No `ozone` key in the manifest when ozone is incomplete.

### 15.2 Lightning (`backend/ingest_lightning/test_activities.py`)

- [ ] `_write_geojson` is atomic and leaves only `lightning.json` in the directory; accepts any iterable snapshot; coordinates `[lon, lat]`.
- [ ] Primary list is exactly the four `wss://wsN.blitzortung.org:443/` URLs; legacy list is all `ws://`; sets disjoint.
- [ ] Override `" wss://relay.local/ ,, ws://other.local:8087/ "` gives `["wss://relay.local/", "ws://other.local:8087/"]`, legacy unchanged.
- [ ] Picker round-robins healthy primaries with rand 0.5: `[a, b, a, b]`.
- [ ] After one failure, the next pick is the other primary; after both fail, the pick is legacy.
- [ ] `delay_2 > delay_1 > 0` and `delay_2 <= 30`. Wss returns after the 60 s demotion expires.
- [ ] With all four demoted, `next()` returns a primary (no stall).
- [ ] `succeeded(url)` returns 1.0 and resets consecutive failures; healed endpoint is eligible.
- [ ] 12 consecutive failures: max delay <= 30 and last >= first.
- [ ] Seed keeps strikes inside retention (now - 900 s, inclusive), drops malformed features, sorts ascending.
- [ ] Seed tolerates a missing file, invalid JSON, or a top-level array (all give `[]`).
- [ ] Write then seed round-trips (`{t, lat, lon, pol, mds}` preserved).

### 15.3 Tropical (`backend/ingest_tropical/test_activities.py`)

- [ ] 100 kt -> `wind_mph` 115, `category` 3, `pressure_mb` 963.0, `movement_mph` 13.0, `movement_dir_deg` 35.0.
- [ ] Without GIS, no cone or track features (KMZ-only links produce nothing).
- [ ] With GIS for bin `EP2`: one cone, one track; forecast point has `name` "Polo", `tau_h` 12.0, `wind_mph` 86 (75 kt), `category` 1, `storm_type` "H".
- [ ] GIS for a different bin is ignored.
- [ ] Saffir-Simpson: `[None, None, 1, 2, 3, 4, 5]` for `[None, 63, 64, 83, 96, 113, 137]`.

### 15.4 Tile cleanup (`backend/tile_cleanup/test_activities.py`)

- [ ] `_sweep_layer("nowcast", 60)` removes the old versioned run (`2000-01-01T00:00:00+00:00`) for both palettes (count 2).
- [ ] It keeps the run equal to the manifest's `run_id` (`2999-01-01T00:00:00+00:00`).

### 15.5 Open-Meteo (`backend/open_meteo_sync/test_activities.py`)

- [ ] `critical_error` returns the exact `[ CRITICAL ]` line for the linkAt case.
- [ ] `[ ERROR ]` lines are flagged.
- [ ] Clean output (only `[ INFO ]` lines) returns None.

### 15.6 Alerts (`backend/api/api/test_storm_watch_activities.py`)

- [ ] Point in polygon with a hole (inside outer, inside hole = out).
- [ ] MultiPolygon: inside one part is in; outside all parts is out.
- [ ] Alert id prefers `properties.id` over `feature.id`.
- [ ] `mark_alerts_seen` skips unhandled ids (they stay unseen).
- [ ] Expired ids are kept as cap filler.
- [ ] Missing snapshot never forgets seen ids.

### 15.7 Code-asserted invariants (no test)

- [ ] Manifest writes are create-or-replace; corrupt manifest raises (never silently empties).
- [ ] A frame is recorded only when every palette rendered (all-or-nothing).
- [ ] A layer is published only with exactly 72 timestamps.
- [ ] AQM run is marked processed only after the air-quality layer published.
- [ ] Manifest entry is removed before its tiles are deleted (tile cleanup).
- [ ] `.tmp` staging dirs older than 3600 s are removed.
- [ ] Grid `.bin` is fsynced before the meta pointer is replaced.
- [ ] Each alert id is committed only after its watch signal succeeded (or it had no geometry).
- [ ] Schedules are SKIP; a still-running workflow makes the next tick a no-op.

---

## 16. Rust design notes

### 16.1 Crates

Temporal (vendored in `/home/claude/rust-port/vendor-src`, version 1.1.0; the SDK
requires Rust 1.92 and edition 2024):

- `temporalio-sdk`: `#[workflow]`, `#[workflow_methods]`, `#[activity]`,
  `#[activity_definitions]`. Activity context gives `record_heartbeat(details)`,
  `cancelled()`, `is_cancelled()`, `heartbeat_timeout()`. Workflows call
  `execute_activity(...)` with per-call options (queue, timeouts, retry).
- `temporalio-client`: `Client::create_schedule`, `get_schedule_handle`,
  `list_workflows`, `get_workflow_handle` (query/signal). Use the same
  schedule IDs, `sched-<id>` workflow IDs, and execution timeouts as section 1.

General:

- `tokio` (runtime, `spawn_blocking` for CPU and file work, `time`, `signal` for
  SIGTERM).
- `reqwest` with `rustls-tls` and `stream`. Build clients with
  `redirect::Policy::none()` to match httpx (Q-2). Per-call timeouts.
- `tokio-tungstenite` (or `async-tungstenite`) for the Blitzortung socket.
  Set `max_message_size = 1 << 20` to match websockets 12 (Python default
  `max_size`). Do not depend on permessage-deflate. Verify with the live endpoint.
- `futures-util` for `StreamExt`.
- `serde`, `serde_json`. For the palette JSON, keep `serde_json::Value` or a
  struct with `#[serde(flatten)]` so `_comment` survives (Appendix B). Keep int
  vs float distinction (`20` vs `20.0`) when hashing.
- `chrono` (UTC, RFC 3339; `isoformat` form `+00:00`).
- `sha2` (SHA-256 digests), `blake2` (array fingerprints in tiler geometry;
  only needed for exact identity compatibility, D-1).
- `nix` (`flock`) or `fs4` for advisory locks. `tempfile` or manual `mkstemp`
  equivalent in the target dir. `uuid` for staging names.
- GRIB2: `eccodes` (bindings to ecCodes, which pygrib wraps) is the closest
  equivalent. It needs `libeccodes` and possibly OpenJPEG/libaec for packing.
  Verify on a real file. A pure-Rust GRIB2 reader would need to implement the
  packing used by the NAQFC product.
- Projection: `proj` (PROJ bindings). Mirror `CRS.from_user_input(projparams)` by
  passing a PROJ string or WKT. Use `always_xy` semantics.
- PNG: `png` crate for indexed (mode P + tRNS) and RGBA writing. For byte-level
  fidelity with Pillow, `flate2` with the `zlib` feature. Pillow's output depends
  on the zlib version and level (D-3).
- Numerics: `ndarray` for 2D grids; manual bilinear and nearest sampling (see
  12.2.3-12.2.4). Use `f32` for sampling where Python uses float32.
- Errors: `thiserror` for typed errors; map to `ApplicationError`-like results
  with `retryable` flag (open-meteo uses `non_retryable=False`).
- Logging: `tracing`.

### 16.2 Module layout

```
aux/
  schedules.rs         # ScheduleDef table, seeding (section 1)
  aqm/                 # find, download, grib, render, publish, cleanup
  lightning/           # picker, ws loop, lzw decode, buffer, writer, seed
  tropical/            # feed, gis, geojson build, writer
  tile_cleanup/        # sweep, grid retention, cleanup.sh-compat not needed
  open_meteo/          # subprocess, classification, cancel-safe kill
  alerts/              # fetch, diff, state, point-in-polygon, signal fan-out
shared/
  manifest.rs          # read/update/replace, atomic write, flock
  tiles/               # class model, geometry, legacy renderer, indexed (later),
                       #   publish protocol, marker, digests (optional)
  grid_dump.rs         # write_grid, prune
  processed_set.rs
  palettes.rs          # include_str! palettes/*.json
  png.rs               # chunk replacement if indexed path ever ships
  heartbeat.rs         # run_sync_with_heartbeat equivalent
```

### 16.3 Concurrency

- AQM: a semaphore of 2 for chunk activities. Activities are the unit of
  retry, so each chunk is independent.
- Lightning: one long activity. Inside, one websocket task reads, one beat task
  flushes every 30 s and heartbeats. Write the JSON off the reader's hot path
  (`spawn_blocking` with a snapshot), as the Python does.
- Alerts: fan-out with `join_all`. Worker slots bound the real concurrency.
- Open-meteo: `tokio::process::Command` with piped stdio, two line readers, a
  `VecDeque<String>` tail of 50 per stream, and a 30 s heartbeat. `kill_on_drop(true)`.

### 16.4 Compatibility and cutover

- See D-1. The simplest safe cutover: the Rust aux worker takes over the
  `ingest-airquality` schedule only after the Python aux worker is drained, and
  it writes to a new `TILE_DIR` root (or a new `runs/` namespace). Never render
  the same `run_id` with both implementations.
- Lightning and tropical outputs are plain JSON files. Cutover is a file swap
  with no coordination needed, but keep the write atomic (rename on the same
  filesystem).
- Manifest: the Rust code must use the same lock file (`manifest.lock`) and the
  same atomic write protocol, or the Python tile cleanup can race it.

---

## 17. Acceptance and verification plan

1. **Golden vectors (generate from Python, check in as fixtures):**
   - LZW decode: the six vectors in 6.6, plus random strings (property test
     against Python output).
   - Picker delays: the table in 6.3 with several rand values.
   - Round: `round_ties_even` for 0.5, 1.5, 2.5, and 115.078 / 86.3085.
   - Saffir-Simpson table; NHC GeoJSON for the test feed in
     `backend/ingest_tropical/test_activities.py`.
   - Open-meteo `critical_error` lines.
   - Point-in-polygon cases from `test_storm_watch_activities.py`.
   - Manifest JSON for a synthetic AQM run (72 frames, two palettes): compare key
     sets, types, and compact JSON bytes. Generate with the Python
     `replace_layer_manifest`, then compare the Rust output after normalizing
     `updated_at`.
   - Tile geometry: for a fixed grid and zoom, compare the set of tile coordinates
     and the first row of fractional indices.
2. **Replay tests:** feed a recorded websocket session through the Rust decoder
   and compare `lightning.json` (minus `generated_at`, `age_s`).
3. **Render parity:** for one real AQM GRIB2 message (pm25, first hour), compare
   Rust output tile sets (paths, counts) to Python's. Compare PNG bytes only if
   D-3 is "byte-identical". Otherwise compare decoded RGBA per tile with a
   tolerance of 0 for classify-equal pixels, and report the mismatch percentage.
4. **Failure injection:** kill during chunk download (heartbeat timeout),
   corrupt manifest (must refuse, not wipe), partial chunks (no publish, no
   processed mark), open-meteo critical line in the last 50 lines (must fail).
5. **Schedule parity:** after seeding, list schedules and compare
   `spec.intervals`, `policy.overlap`, `policy.catch_up_window`,
   `action.execution_timeout`, `action.task_queue`, and args (JSON) for the eight
   schedules in section 1.

---

## Appendix A. Palette edge table (shared by classic, muted, vivid)

pm25 edges (µg/m³): 0, 4.5, 9.1, 20, 35.5, 45, 55.5, 90, 125.5, 175, 225.5, 500, 20000
(12 ranges, the last ends at 20000).

ozone edges (ppb): 0, 30, 55, 63, 71, 78, 86, 95, 106, 150, 200, 600
(11 ranges, the last ends at 600).

Each palette JSON has `pm25`, `ozone` (plus reflectivity, temperature, etc.).
Verified: edges are identical across `classic`, `muted`, `vivid`.

## Appendix B. Digest algorithms (needed only for D-1 interop)

All digests are SHA-256 hex strings.

- `stable_json(value)`: recursive: numpy array -> `{array_digest, dtype, shape}`;
  bytes -> `{"bytes": hex}`; Mapping -> keys stringified and sorted; list/tuple ->
  list; non-finite float -> `{"float": repr}`; None/str/int/float/bool as is;
  other -> `str(value)`. Then `json.dumps(sort_keys=True, separators=(",", ":"))`.
- `array_content_digest(a)`: sha256 over `json({"dtype": a.dtype.str, "shape": list(a.shape)})`
  then the raw bytes in C order (chunked for non-contiguous arrays).
- `source_digest = array_content_digest(source_data)` (float32 data after orientation).
- `grid_spec_digest = stable_digest({kind, height, width, lat_bounds, lon_bounds, coordinate_identity})`
  where for projected grids `coordinate_identity = (str(source_crs), fp(source_x), fp(source_y))`
  and `fp(a) = (shape, dtype.str, blake2b16(a bytes))`.
- `tile_spec_digest = stable_digest({"tile_size": 256, "zoom_levels": [4,5,6]})`.
- `palette_digest = stable_digest(palette_table_as_loaded)` (keeps `_comment`).
- Legacy: `semantic = {"kind":"continuous","nodata_value":null,"min_valid_weight":1.0}`,
  `policy = {"sampling":"bilinear-rgba","category_map":null,"png_compress_level":1}`,
  `algorithm = "rgba-bilinear-v1"`, `renderer = "legacy"`.
- Indexed (for reference): `algorithm = "physical-bilinear-classify-v1"`,
  `policy` adds `model_edges`, `model_remap` arrays and `png_compress_level`.
- `source_id = "aqm:{layer}:runs/{run_id}/{ts}"` (stable frame identity).
- Marker: `tile_path_digest = sha256("\0".join(sorted(relpaths)))`;
  `tile_content_digest = sha256(for each relpath in sorted order: be32(len(path)) ||
  path || be64(size) || bytes)`.

## Appendix C. Worked examples (verified by running the Python)

- Decode: `"aĀ"` -> `"aaa"`; `"abĀ"` -> `"abab"`; `"abĀā"` -> `"ababba"`.
- Wind: `round(100*1.15078) = 115`; `round(75*1.15078) = 86`.
- Round half-even: `round(0.5)=0`, `round(1.5)=2`, `round(2.5)=2`.
- Picker (rand=0.5): delays 1.5, 3.0, 6.0, 12.0, 22.5, 22.5, ...
- Frame lead minutes for the first AQM frame: 0; last: 4260.
- Tile path for frame `2026-07-16T12:00:00+00:00` at z4 x3 y5:
  `TILE_DIR/air-quality/classic/runs/20260716_12/2026-07-16T12:00:00+00:00/4/3/5.png`.

## Appendix D. Source file map

| Spec section | Python source |
|---|---|
| 1, 2, 3 | `temporal/schedules/seed.py`, `temporal/worker.py`, `temporal/task_queues.py`, `temporal/workflows/{ingest_airquality,ingest_lightning,ingest_tropical,tile_cleanup,open_meteo_sync,poll_alerts}.py` |
| 5 | `backend/ingest_airquality/activities.py`, `backend/shared/{tiler,palettes,png_palette,grid_dump,manifest,state,activity_heartbeat}.py`, `backend/shared/palettes/classic.json` (and `muted.json`, `vivid.json`) |
| 6 | `backend/ingest_lightning/activities.py` |
| 7 | `backend/ingest_tropical/activities.py` |
| 8 | `backend/tile_cleanup/activities.py`, `backend/tile_cleanup/cleanup.sh`, `backend/shared/grid_dump.py` |
| 9 | `backend/open_meteo_sync/activities.py`, `temporal/open_meteo_worker.py`, `deploy/k8s/open-meteo-worker-deployment.yaml` |
| 10 | `backend/api/api/storm_watch_activities.py`, `temporal/workflows/poll_alerts.py` |
| 12 | as listed |
| 13 | `docs/configuration.md`, `deploy/k8s/configmap-temporal-config.yaml`, `deploy/docker-compose.yml`, `deploy/.env.example` |

Not covered (needs its own spec): tiler internals beyond 12.2 (`render_footprint_max_tiles`,
`render_*_tiles_atomic` wrappers used by other pipelines, overview pooling),
`backend/shared/grid_dump.py` generation markers used by nowcast, `temporal/schedules/watchdog.py`,
the WatchStormWorkflow, and the API client singleton.
