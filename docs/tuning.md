# Tuning radar-ng

Every knob that matters, and which direction to turn it. The single constraint driving all of this: **a new MRMS frame lands every 2 minutes, the ingest schedule polls for it every minute, and the render of one frame must finish well inside 2 minutes** or radar goes stale and `/api/health` flips to `degraded`.

## Where the CPU goes

Per MRMS frame: S3 list → GRIB2 download → pygrib decode → **palette render (dominant)** → storm-cell detection → atomic publication. The render rasterizes the decoded grid into a z4–z7 PNG pyramid once per palette. Measure this on your own hardware; the acceptance target is publication inside the source cadence, not a hard-coded historical timing claim.

The three levers on render cost, biggest first:

### 1. Zoom levels

`ZOOM_LEVELS = [4, 5, 6, 7]` in `backend/ingest_mrms/activities.py`. Tile count quadruples per zoom level, so the top level is roughly 75% of the pyramid. z7 is already the honest ceiling for the ~1 km source; on a small host, reduce palette count before dropping observed-radar coverage further.

The client must agree: `SOURCE_MAX_ZOOM` in `frontend/src/components/map/RadarOverlay.tsx` tells MapLibre the real pyramid ceiling (`radar: 7`, nowcast/HRRR: 6) so it upsamples the top tile instead of firing 404-bound requests at zooms that don't exist. Change one, change the other. Same story at the bottom: `SOURCE_MIN_ZOOM = 4` keeps MapLibre from requesting world-scale tiles that CONUS-only coverage would never answer.

### 2. Palette count

`PALETTES=classic,muted,vivid` (compose env / `deploy/k8s/configmap-temporal-config.yaml`). Render cost is linear in palette count — three palettes ≈ 3× the PNG encode of one, minus thread-pool overlap. On a 4-core lab box, `PALETTES=classic` is the first thing to try. Only palettes with a matching `backend/shared/palettes/<name>.json` are valid; a typo crashes the render activity with `FileNotFoundError`.

### 3. Worker parallelism

- `TEMPORAL_MAX_CONCURRENT_ACTIVITIES` — how many activities one worker runs at once (compose lab default 2; production sets it per role pool: mrms 6, nowcast 1, hrrr 4, aux 4, alerts 8). Lower it if MRMS, HRRR, and nowcast renders pile up and thrash each other; raise it only with cores to spare.
- `NOWCAST_RENDER_WORKERS` (default 4) — threads that encode the nowcast lead-time pyramids in parallel; PNG encoding releases the GIL, so this scales up to the pod's CPU limit.
- The MRMS frame render samples each tile once and derives every palette from it; there is no per-palette process pool any more (`MRMS_RENDER_WORKERS` is not read).

## Resource shapes

The worker (or standalone `ingest-mrms` in older setups) is sized **1 cpu / 1 Gi requests, 6 cpu / 6 Gi limits** in production — the burst headroom is what the parallel palette render uses. When this was under-provisioned the symptom was OOMKills (7/day → 0 after the bump) and stale frames. Full table in [kubernetes.md](kubernetes.md#resource-shapes). On compose, just make sure the host has the cores; there are no per-container limits by default.

## Ingest cadence + catch-up

- The two MRMS schedules fire every minute (`_MRMS_POLL` in `temporal/schedules/seed.py`) although NOAA publishes a frame every ~2 minutes at an unpredictable offset: polling twice per frame halves the average wait for a new key. The list is a single S3 request, and `OverlapPolicy.SKIP` drops a tick that lands while a frame is still rendering.
- `BACKLOG_PER_CYCLE` (compose lab default 2, current cluster default 1) — after downtime, each fire processes up to N unprocessed S3 keys, newest-first. Keep it at 1 while one frame is slower than the source cadence; raising it can make freshness worse by extending each workflow run.
- `MRMS_MAX_AGE_S=600` (tile-server env) — staleness budget before `/api/health` reports `degraded` (HTTP 503) and the app shows its "data delayed" banner. 600 s tolerates ~3 missed frames. Loosen it on deliberately slow setups rather than living with a permanently red health check; don't tighten below ~300 s or normal NOAA jitter will page you.

## Tile retention + cleanup

The `tile-cleanup` schedule sweeps hourly. Per-layer retention lives in `LAYER_RETENTION_MIN` in `backend/tile_cleanup/activities.py`:

| layers | retention |
|---|---|
| radar, radar-composite | 4 h |
| nowcast | 1 h |
| all HRRR-derived (radar-hrrr, temperature, wind, cape, precip-*, cloud, …) | 12 h |

Retention × cadence × pyramid size is your steady-state disk (~5 GB). Longer radar retention = longer scrub-back history in the app's timeline, linearly more disk.

## Serving: HPA + cache headers

**HPA and storage:** the current cluster's tile-server is capped at one replica because tiles, grids, and state use ReadWriteOnce volumes. An HPA cannot create safe multi-pod serving while that remains the data plane. The north-star design moves immutable tiles to S3-compatible object storage and manifest/state to a shared service; only then should the API/tile tier scale horizontally.

**Caddy cache headers** (`backend/api/Caddyfile`) encode a data-model fact worth understanding before touching them:

| path | Cache-Control | why |
|---|---|---|
| `/tiles/radar/*`, `/tiles/radar-composite/*` | `public, max-age=86400, immutable` | **observed** frames are written exactly once per timestamp dir and never change — hard caching is what lets playback loop past frames without re-fetching every tile each cycle |
| `/tiles/nowcast/*`, `/tiles/radar-hrrr/*`, other model layers | `public, max-age=86400, immutable` | each forecast run now has immutable paths; manifest v2 points clients at the current complete run |
| `/basemap/tiles/*` | `max-age=86400` | static archive |
| `/basemap/styles/*` | `max-age=3600` | style JSON |

If you add a new observed-once layer (e.g. another MRMS product), add its path to the `@observed` matcher to get the immutable treatment; new forecast layers need nothing — the mutable default is correct.

One more server-side cache: `FORECAST_TTL_S` (default 300) is the tile-server's in-process cache for `/api/forecast/*` responses, which keeps a chatty home tab from hammering Open-Meteo.

## Client-side knobs

`frontend/src/lib/constants.ts` (`DEFAULTS`):

| knob | default | effect |
|---|---|---|
| `MANIFEST_REFETCH_MS` | 30 s | how fast new frames appear in the timeline; the server caches the manifest 15 s, so polling faster buys nothing |
| `FORECAST_REFETCH_MS` | 15 min | forecast re-poll |
| `ALERTS_REFETCH_MS` | 60 s | NWS alert re-poll |

`frontend/src/components/map/RadarOverlay.tsx`:

- `WINDOW = 5` — pre-mounted raster carousel slots. Playback flips opacity between mounted sources; each tick remounts exactly one hidden slot, giving it ~(WINDOW−1)×tick to fetch before display. Bigger = smoother scrubbing, more memory + tile fetches. `WINDOW = 1` is the kill switch that reproduces single-source behavior if a native regression shows up (the constant child count is load-bearing on iOS — see the comment block in the file before "simplifying" it).

## Nowcast

`NOWCAST_HORIZON_MIN=60`, `NOWCAST_STEP_MIN=5`, `NOWCAST_INPUT_FRAMES=4`. Cost scales with horizon/step (number of extrapolated frames to render) — and each nowcast frame pays the same palette-render bill as a radar frame, just on a smaller pyramid (client caps it at z6). Nowcast needs `NOWCAST_INPUT_FRAMES` recent grids on disk before it produces anything.

The nowcast Schedule ticks every 2 minutes as a backstop, but the normal start is the kick: every base-reflectivity frame that writes a new science grid triggers the Schedule with `BUFFER_ONE` (`kick_nowcast` in `backend/ingest_mrms/activities.py`), so an idle nowcast starts within seconds of the grid landing and a busy one queues exactly one follow-up run. `NOWCAST_KICK_ENABLED=0` returns to timer-only starts. A kicked run whose newest grid is already forecast exits before reading its inputs.

Every run is also verified. Each run keeps its twelve lead-time point grids plus the observation it started from; each new nowcast anchor observation scores every retained run that predicted that time, cell by cell at 20 dBZ (rain) and 35 dBZ (heavy): hits, misses and false alarms, next to the same counts for a "nothing moves" persistence forecast. The rolling 24-hour log is summarised by `/api/nowcast/skill`, folded into `/api/health` as `nowcast_skill` and `/api/nowcast/{lat}/{lon}` as `skill`, and exported as `radar_ng_nowcast_{pod,far,csi}` gauges. The app shows the headline (for example, "Caught 87% of rain 30 min out today, 9% false alarms") once six runs at the selected lead are scored; the headline defaults to 30 minutes. Scoring happens after publication, and scoring failures do not fail publication.

## Lightning

The Blitzortung consumer rotates over the verified `wss://ws{1,2,7,8}.blitzortung.org:443/` endpoints and only falls back to the legacy plain-ws ports after every primary has failed (`EndpointPicker`). Blitzortung closes a healthy stream every few minutes; the consumer reconnects after 1 s and backs off only on endpoints that delivered nothing. Each hourly run seeds its buffer from the last published `lightning.json`, so the layer no longer blanks at the rollover. `LIGHTNING_WS_ENDPOINTS` (comma-separated) replaces the primary list, e.g. for a relay you host.

## Watching it: observability

- **`/api/health`** — `mrms_age_s` vs `mrms_max_age_s`, nowcast status, machine-readable `reasons`. The first thing to curl, always.
- **`/api/metrics`** (Prometheus):

  ```
  radar_ng_mrms_age_seconds                 gauge   ← alert on this
  radar_ng_tile_timestamps{layer="…"}       gauge
  radar_ng_manifest_requests_total          counter
  radar_ng_forecast_requests_total          counter
  radar_ng_forecast_cache_hits_total        counter
  radar_ng_forecast_upstream_errors_total   counter
  ```

  A flat `radar_ng_mrms_age_seconds` climbing past 600 is the "ingest is dead" signature; sawtooth between ~0–180 is healthy.
- **Logs** — every service emits one-line JSON to stdout. `msg":"frame_done"` lines carry `duration_s`: if that number trends toward 120, you're about to go stale — cut a palette or a zoom level *before* it crosses.
