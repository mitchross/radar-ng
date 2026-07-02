# Tuning radar-ng

Every knob that matters, and which direction to turn it. The single constraint driving all of this: **a new MRMS frame lands every 2 minutes, and the render of one frame must finish in under 2 minutes** or radar goes stale and `/api/health` flips to `degraded`.

## Where the CPU goes

Per MRMS frame: S3 list (~200 ms) → GRIB2 download (~1 s) → pygrib decode (~800 ms) → **palette render (~2 min, dominates everything)** → storm-cell detect (~300 ms). The render rasterizes the decoded grid into a full z4–z8 PNG pyramid, once per palette, parallelized across a thread pool (PIL releases the GIL during PNG encode).

The three levers on render cost, biggest first:

### 1. Zoom levels

`ZOOM_LEVELS = [4, 5, 6, 7, 8]` in `backend/ingest_mrms/activities.py`. Tile count quadruples per zoom level, so **the top level is ~75% of total render cost** — that's exactly why z9 was dropped (frames were arriving ~15 min stale with it; without it, ≈2 min). If you're on a small host and radar still can't keep up, dropping to z7 max cuts the work by ~4× again.

The client must agree: `SOURCE_MAX_ZOOM` in `frontend/src/components/map/RadarOverlay.tsx` tells MapLibre the real pyramid ceiling (`radar: 8`) so it upsamples the top tile instead of firing 404-bound requests at zooms that don't exist. Change one, change the other. Same story at the bottom: `SOURCE_MIN_ZOOM = 4` keeps MapLibre from requesting world-scale tiles that CONUS-only coverage would never answer.

### 2. Palette count

`PALETTES=classic,muted,vivid` (compose env / `deploy/k8s/configmap-temporal-config.yaml`). Render cost is linear in palette count — three palettes ≈ 3× the PNG encode of one, minus thread-pool overlap. On a 4-core lab box, `PALETTES=classic` is the first thing to try. Only palettes with a matching `backend/shared/palettes/<name>.json` are valid; a typo crashes the render activity with `FileNotFoundError`.

### 3. Worker parallelism

- `MRMS_RENDER_WORKERS` (default 2) — parallel palette renders per MRMS frame.
- `TEMPORAL_MAX_CONCURRENT_ACTIVITIES` — how many activities one worker runs at once (compose lab default 2, prod 4). Lower it if MRMS, HRRR, and nowcast renders pile up and thrash each other; raise it only with cores to spare.

## Resource shapes

The worker (or standalone `ingest-mrms` in older setups) is sized **1 cpu / 1 Gi requests, 6 cpu / 6 Gi limits** in production — the burst headroom is what the parallel palette render uses. When this was under-provisioned the symptom was OOMKills (7/day → 0 after the bump) and stale frames. Full table in [kubernetes.md](kubernetes.md#resource-shapes). On compose, just make sure the host has the cores; there are no per-container limits by default.

## Ingest cadence + catch-up

- `BACKLOG_PER_CYCLE` (compose lab default 2, prod 3) — after downtime, each 2-min cycle processes up to N unprocessed S3 keys, **newest-first**, committing state per frame. Users get the current picture immediately and history backfills. Raising it clears backlogs faster at the cost of longer cycles; not worth touching unless you have long nightly outages.
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

**HPA:** tile-server scales 2 → 6 on CPU. Static-file serving scales near-linearly; 2 replicas cover a household, the ceiling exists to cap damage. Ingest is *not* horizontally scaled — its throughput is a per-worker concern (above).

**Caddy cache headers** (`backend/api/Caddyfile`) encode a data-model fact worth understanding before touching them:

| path | Cache-Control | why |
|---|---|---|
| `/tiles/radar/*`, `/tiles/radar-composite/*` | `public, max-age=86400, immutable` | **observed** frames are written exactly once per timestamp dir and never change — hard caching is what lets playback loop past frames without re-fetching every tile each cycle |
| all other `/tiles/*` (nowcast, radar-hrrr, temperature, …) | `public, max-age=120` | **forecast** layers rewrite the *same* valid-time path on every model run — a long TTL pins stale predictions |
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
