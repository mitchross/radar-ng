# Configuration reference

The environment variables operators usually set. For Docker Compose, copy
[`deploy/.env.example`](../deploy/.env.example) to `.env` and edit; on
Kubernetes the worker values go in a ConfigMap and the api values in the
tile-server Deployment (see [docs/kubernetes.md](kubernetes.md)). Defaults below
are the code defaults unless marked "compose". Each crate README under
`rust/crates/` lists every variable that crate reads.

**Only two values must be set** — everything else has a working default:

| Must set | Why |
|---|---|
| `NWS_USER_AGENT` | `api.weather.gov` rejects requests without a real contact in the User-Agent. Use your email. |
| `BASEMAP_PMTILES_URL` | The basemap download URL — see the note below; the example date will 404 within days. |

---

## The `BASEMAP_PMTILES_URL` gotcha

This is the **Protomaps basemap** — the vector map (roads, water, city labels)
rendered *under* the radar. The one-time `basemap-bootstrap` step does HTTP
range-reads against this URL to extract your region into
`basemap.pmtiles.partial`; `basemap-publish` renames it to `basemap.pmtiles`,
which the api serves.

`build.protomaps.com` serves **date-stamped daily planet builds**
(`YYYYMMDD.pmtiles`) with **~6-day retention** and **no `latest` alias**. So:

- **Don't hardcode an old date** — pick the newest from <https://build.protomaps.com/>.
- `BASEMAP_BBOX` clips the extract (default = CONUS, ~1–2 GB). Narrow it to shrink the download.
- It's used **once**; after `basemap.pmtiles` exists it isn't read again until you re-bootstrap.

```bash
# newest date, then bootstrap once:
BASEMAP_PMTILES_URL=https://build.protomaps.com/YYYYMMDD.pmtiles
docker compose run --rm basemap-bootstrap
docker compose run --rm basemap-publish
```

> **Docker Compose only.** The reference Kubernetes manifests (talos-argocd-proxmox)
> don't set a date: the `basemap` Deployment's `fetch-basemap` init container
> resolves the newest available Protomaps build when its archive is missing or
> its `BBOX`/`REFRESH` env changed. Bump `REFRESH` (a GitOps change) to pull a
> fresher planet. Never pin a `YYYYMMDD` there — it 404s within ~6 days.

---

## All values

### Ports (host side)
| Var | Default | Notes |
|---|---|---|
| `TILE_SERVER_PORT` | `8080` | Public API / tiles (container `:8080`). |
| `TEMPORAL_PORT` | `7233` | Temporal gRPC. |
| `TEMPORAL_UI_PORT` | `8233` | Temporal Web UI (compose `--profile ui`). |

### Temporal
| Var | Default | Notes |
|---|---|---|
| `TEMPORAL_ADDRESS` | `temporal:7233` (compose) | Point at an external cluster to skip the bundled `temporal` + `temporal-postgres`. |
| `TEMPORAL_NAMESPACE` | `default` | Namespace workers + API use. |
| `WORKER_ROLE` | `legacy` | Activity bundle and task queue: `legacy`, `mrms`, `nowcast`, `hrrr`, `aux`, `alerts`, `open-meteo`, or `all`. `radar worker --role` overrides it. |
| `SEED_SCHEDULES` | `1` for `legacy`/`all`, else `0` | Whether this worker (re)seeds the Temporal Schedules on boot. Exactly one worker should. |
| `SKIP_SCHEDULE_SEED` | `0` | `1` stops the worker from seeding. |
| `USE_ISOLATED_TASK_QUEUES` | `0` | `1` puts each role on its own queue (`radar-ng-<role>`) and seeds Schedules onto them; production uses `1` with five role pools. Deploy every role worker before switching. |
| `TEMPORAL_MAX_CONCURRENT_ACTIVITIES` | `4` (compose `2`) | Concurrent activities per worker. |
| `TEMPORAL_MAX_CONCURRENT_ACTIVITY_TASK_POLLS` | `2` (compose `1`) | Concurrent activity task polls. |
| `TEMPORAL_GRACEFUL_SHUTDOWN_S` | `25` | Drain time for in-flight activities on SIGTERM. |
| `TEMPORAL_METRICS_BIND` | `0.0.0.0:9464` | Worker Prometheus endpoint; `off` disables it. |

### Upstream identification
| Var | Default | Notes |
|---|---|---|
| `NWS_USER_AGENT` | `(radar-ng, you@example.com)` | **Set this.** Real contact required by api.weather.gov. |
| `NWS_ALERTS_URL` | `https://api.weather.gov/alerts/active?status=actual` | Active-alerts feed, polled every 5 min. |
| `TROPICAL_FEED_URL` | `https://www.nhc.noaa.gov/CurrentStorms.json` | NHC tropical cyclone feed, hourly. |

### Basemap
| Var | Default | Notes |
|---|---|---|
| `BASEMAP_PMTILES_URL` | dated example | Compose bootstrap only. **Set this** to a recent build (see above). |
| `BASEMAP_BBOX` | `-125,24,-66,50` | Compose bootstrap only. `minLon,minLat,maxLon,maxLat`; default = CONUS. |
| `BASEMAP_PMTILES` | `/data/basemap.pmtiles` | api: the archive it serves at `/basemap/tiles/*`. |
| `BASEMAP_TILES_UPSTREAM` | `http://basemap:8081` | api: go-pmtiles server to proxy `/basemap/tiles/*` to when `BASEMAP_PMTILES` does not exist. |
| `BASEMAP_PMTILES_RECHECK_S` | `10` | api: how often the archive's file identity is checked, so a renamed-in replacement is picked up. |

### Frontend basemap provider (bundled vs external)

The app ships **batteries-included**: by default the map renders the **bundled**
Protomaps basemap served by your own tile-server (`/basemap/styles/*` and
`/basemap/tiles/*`, backed by the `basemap-bootstrap` archive above). Forks and upstream users set nothing.

Optionally, the frontend can point at an **external** MapLibre style URL instead
— e.g. a self-hosted [VersaTiles](https://versatiles.org) instance shared across
several apps — without deploying the bundled basemap at all. These are **frontend
build-time** vars (`EXPO_PUBLIC_*`, inlined into the app bundle by Expo, same as
the telemetry vars); they are **not** backend/compose settings. Leave them unset
for bundled mode.

| Var (build-time `EXPO_PUBLIC_*`) | Default | Notes |
|---|---|---|
| `EXPO_PUBLIC_BASEMAP_LIGHT_STYLE_URL` | _(unset → bundled)_ | Absolute MapLibre style URL for the **light** theme. |
| `EXPO_PUBLIC_BASEMAP_DARK_STYLE_URL` | _(unset → bundled)_ | Absolute MapLibre style URL for the **dark** theme. |
| `EXPO_PUBLIC_BASEMAP_SATELLITE_STYLE_URL` | _(unset → bundled)_ | Optional. The bundled style uses public-domain USGS imagery that the tile server fetches and serves at `/basemap/imagery`, so the app never contacts a third party. |

Each style resolves independently: set only light+dark to move the vector
basemap external while keeping the bundled satellite. An external style must be
a complete absolute document (its own `sources`/`glyphs`/`sprite`); the app
loads it directly instead of rewriting relative tile paths. The provider serving
it must allow the app's origin via CORS. Example (build the app with):

```bash
EXPO_PUBLIC_BASEMAP_LIGHT_STYLE_URL=https://maps.vanillax.me/styles/light.json \
EXPO_PUBLIC_BASEMAP_DARK_STYLE_URL=https://maps.vanillax.me/styles/dark.json
```

### Ingest / render tunables
| Var | Default | Notes |
|---|---|---|
| `PALETTES` | `classic,muted,vivid` | Palettes are built into the binary (`classic`, `muted`, `vivid`). |
| `BACKLOG_PER_CYCLE` | `3` (compose `2`) | Max MRMS frames back-filled per schedule fire (fires every minute; prod: 1). |
| `STORM_THRESHOLD_DBZ` | `40` | Reflectivity threshold used to form tracked storm cells. |
| `STORM_MIN_PIXELS` | `5` | Minimum connected MRMS pixels in a storm cell. |
| `STORM_MAX_CELLS` | `500` | Strongest storm cells retained per frame. |
| `STORM_MAX_TRACK_SPEED_KMH` | `160` | Association and extrapolation speed ceiling. |
| `STORM_PREFETCH_PADDING_KM` | `12` | Padding around each predicted storm bbox. |
| `STORM_PREFETCH_MAX_DISTANCE_KM` | `500` | Do not prefetch when the nearest storm is farther from the user. |
| `FORECAST_HOURS` | `18` | HRRR forecast hours rendered per model run. |
| `EXTENDED_FORECAST_HOURS` | `48` | Extended sub-hourly HRRR hours. |
| `HRRR_ENABLED_LAYERS` | `radar-hrrr` (compose: all layers) | Comma-separated HRRR layers: `radar-hrrr`, `temperature`, `feels-like`, `dewpoint`, `humidity`, `wind`, `cape`, `precip-type`, `precip-accum`, `cloud`. `feels-like` is the NWS heat index or wind chill. |
| `MRMS_QPE_24H_ENABLED` | `0` (compose: `1`) | Render the `precip-24h` layer: MRMS MultiSensor QPE Pass 1, past 24 hours, in inches, colored with `precip_accum`. Hourly; each file lands about 16 minutes after the hour. Read by the MRMS worker. The `ingest-mrms-qpe-24h` schedule is always seeded; with `0` it lists nothing and its cleanup removes old frames. |
| `PACKED_TILES` | `1` | Write packed data tile buckets (`docs/packed-tiles.md`) for `radar`, `radar-composite` and `nowcast`. `0` stops new buckets; the PNG pyramids are unchanged. Read by the MRMS and nowcast workers. |
| `PRECIP_TYPE` | `1` | Color `radar` and `radar-composite` by precipitation type (rain, snow, mix) from MRMS PrecipFlag. Packed buckets become v2 typed codes. `0` gives rain-only frames and v1 buckets. Read by the MRMS worker. |
| `PRECIP_FLAG_PREFIX` | `CONUS/PrecipFlag_00.00` | MRMS PrecipFlag product prefix in `noaa-mrms-pds`. |

### Nowcast (Lucas-Kanade motion + S-PROG)
| Var | Default | Notes |
|---|---|---|
| `NOWCAST_HORIZON_MIN` | `60` | Minutes of extrapolated nowcast. |
| `NOWCAST_STEP_MIN` | `5` | Minutes between nowcast frames. |
| `NOWCAST_INPUT_FRAMES` | `4` | Recent MRMS frames fed to the motion field; values below 3 are clamped to 3. |
| `NOWCAST_GRID_INPUT_LAYER` | `radar-nowcast-input` | Higher-fidelity grid dedicated to motion estimation. |
| `NOWCAST_MAX_INPUT_GAP_MIN` | `6` | Fail closed when consecutive input frames exceed this cadence. |
| `NOWCAST_ALLOW_PERSISTENCE_FALLBACK` | `0` | Opt-in stationary fallback; disabled so degraded output is not presented as a motion forecast. |
| `NOWCAST_INPUT_STEP_MIN` | `5` | Target spacing (minutes) between the input grids picked from the ~2-min MRMS stream. |
| `NOWCAST_RENDER_WORKERS` | `4` | Threads encoding lead-time pyramids in parallel. |
| `NOWCAST_KICK_ENABLED` | `1` | Each new base-reflectivity science grid triggers the nowcast Schedule (`BUFFER_ONE`) instead of waiting for its 2-min tick. `0` = timer only. |
| `NOWCAST_SCHEDULE_ID` | `nowcast` | Schedule the kick triggers. |
| `NOWCAST_KICK_TIMEOUT_S` | `5` | RPC budget for the kick; a slow Temporal never delays the frame. |
| `NOWCAST_POINT_GRID_RETENTION_RUNS` | `36` | Complete nowcast runs kept on the grids volume (13 grids of ~1.5 MB each); 36 covers scoring the 60-minute lead at a 2-min cadence. |
| `NOWCAST_SKILL_WINDOW_HOURS` | `24` | Rolling window the verified-accuracy summary (`/api/nowcast/skill`) aggregates. |
| `NOWCAST_SKILL_HEADLINE_LEAD_MIN` | `30` | Lead time the app's accuracy headline is taken from. |
| `NOWCAST_SKILL_MIN_RUNS` | `6` | Scored runs needed before the headline is shown at all. |

### Storm watch
| Var | Default | Notes |
|---|---|---|
| `INTENSIFY_DBZ_DELTA` | `10` | dBZ increase over previous frame = "intensifying". |
| `DISSIPATE_DBZ_DELTA` | `10` | dBZ decrease = "dissipating". |
| `HAIL_DBZ_THRESHOLD` | `60` | dBZ at/above which hail is flagged. |
| `PUSH_DISABLED` | `1` | `1` = never touch APNS/FCM (needs key files wired to enable). |

### Lightning (Blitzortung)
| Var | Default | Notes |
|---|---|---|
| `LIGHTNING_RETENTION_MIN` | `15` | Minutes of strikes kept in the rolling buffer. |
| `LIGHTNING_FLUSH_S` | `2.0` | Seconds between buffer flushes to the state dir. |
| `LIGHTNING_WS_ENDPOINTS` | unset | Comma-separated websocket endpoints to prefer over the built-in `wss://ws{1,2,7,8}.blitzortung.org:443/` list (legacy ports stay the last resort). |

### Tile-server API
| Var | Default | Notes |
|---|---|---|
| `LISTEN_ADDR` | `0.0.0.0:8080` (image) | Bind address. |
| `OPEN_METEO_BASE` | `http://open-meteo:8080/v1/forecast` | Forecast upstream; default is the bundled self-hosted Open-Meteo. |
| `MRMS_MAX_AGE_S` | `600` | Radar age (s) past which `/api/health` reports `degraded`. |
| `FORECAST_TTL_S` | `900` | In-process forecast response cache TTL (s); the cluster sets 300. |
| `FORECAST_CACHE_MAX_BYTES` | `67108864` (64 MiB) | Byte budget for cached forecast bodies (replaces `FORECAST_CACHE_MAX_ENTRIES`). |
| `WIND_CACHE_MAX_BYTES` | `33554432` (32 MiB) | Byte budget for cached wind-field bodies (replaces `WIND_CACHE_MAX_ENTRIES`). |
| `NOWCAST_POINT_CACHE_MAX_BYTES` | `16777216` (16 MiB) | Byte budget for cached nowcast point bodies. |
| `ALERTS_CACHE_MAX_BYTES` | `16777216` (16 MiB) | Byte budget for point-to-zone lookups and direct point alert bodies. |
| `GEOCODE_CACHE_MAX_BYTES` | `8388608` (8 MiB) | Byte budget for each geocode cache (forward, reverse). |
| `NWS_POINTS_URL` | `https://api.weather.gov/points` | NWS point-to-zone lookup used to match zone alerts. |
| `API_RATE_LIMIT_RPS` | `20` | Per-client token refill rate for API routes. |
| `API_RATE_LIMIT_BURST` | `60` | Per-client API token bucket capacity. |
| `DISABLE_WORKFLOW_ROUTES` | `0` | Disable watch/push mutation routes until an identity issuer and signing key are configured. |
| `WORKFLOW_AUTH_SIGNING_KEY` | unset | HMAC key for scoped workflow-route bearer tokens (`WORKFLOW_API_SIGNING_KEY` also accepted). |
| `TEMPORAL_ALERTS_TASK_QUEUE` | `radar-ng-alerts` | Queue the api starts `WatchStormWorkflow` on; compose sets `radar-ng` for the legacy worker. |
| `GEOCODER_URL` / `PHOTON_URL` | unset | Geocoder behind `/api/geocode` and `/api/reverse-geocode`; neither set means 503. |
| `LLM_BASE_URL` / `LLM_MODEL` / `LLM_API_KEY` | unset | Optional OpenAI-compatible endpoint for `/api/briefing`; unset hides the briefing. |
| `GIBS_WMTS_BASE` | `https://gibs.earthdata.nasa.gov/wmts/epsg3857/nrt` | NASA GIBS WMTS root for the GOES satellite layer (`/tiles/satellite/*` and the manifest `satellite` layer). Empty turns the layer off. |
| `SATELLITE_CACHE_MAX_BYTES` | `134217728` (128 MiB) | In-process cache for satellite tiles. Count it in the api memory limit. `0` turns the cache off. |

---

See also: [GETTING_STARTED.md](../GETTING_STARTED.md) ·
[self-hosting.md](self-hosting.md) · [tuning.md](tuning.md) (what to change for
faster/cheaper/fresher).
