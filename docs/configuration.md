# Configuration reference

Every knob radar-ng reads from the environment. For Docker Compose, copy
[`deploy/.env.example`](../deploy/.env.example) to `.env` and edit; for
Kubernetes the same values live in the `configmap-temporal-config.yaml` (see
[docs/kubernetes.md](kubernetes.md)).

**Only two values must be set** — everything else has a working default:

| Must set | Why |
|---|---|
| `NWS_USER_AGENT` | `api.weather.gov` rejects requests without a real contact in the User-Agent. Use your email. |
| `BASEMAP_PMTILES_URL` | The basemap download URL — see the note below; the example date will 404 within days. |

---

## The `BASEMAP_PMTILES_URL` gotcha

This is the **Protomaps basemap** — the vector map (roads, water, city labels)
rendered *under* the radar. The one-time `basemap-bootstrap` step does HTTP
range-reads against this URL to extract your region into a local
`basemap.pmtiles`.

`build.protomaps.com` serves **date-stamped daily planet builds**
(`YYYYMMDD.pmtiles`) with **~6-day retention** and **no `latest` alias**. So:

- **Don't hardcode an old date** — pick the newest from <https://build.protomaps.com/>.
- `BASEMAP_BBOX` clips the extract (default = CONUS, ~1–2 GB). Narrow it to shrink the download.
- It's used **once**; after `basemap.pmtiles` exists it isn't read again until you re-bootstrap.

```bash
# newest date, then bootstrap once:
BASEMAP_PMTILES_URL=https://build.protomaps.com/YYYYMMDD.pmtiles
docker compose run --rm basemap-bootstrap
```

> **Docker Compose only.** On **Kubernetes you don't set a date at all** — the
> `basemap-bootstrap` Job runs a self-healing image that resolves the *newest
> available* Protomaps build at run time, so there's nothing to rot. To pull a
> fresher planet, bump the Job's `refresh:` label (ArgoCD recreates the Job →
> newest build re-resolved) then `kubectl -n radar-ng rollout restart deploy
> basemap`. To change the area, edit the Job's `BBOX`. Never pin a `YYYYMMDD`
> in the k8s Job — it 404s within ~6 days.

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
| `TEMPORAL_ADDRESS` | `temporal:7233` | Point at an external cluster to skip the bundled `temporal` + `temporal-postgres`. |
| `TEMPORAL_NAMESPACE` | `default` | Namespace workers + API use. |
| `SKIP_SCHEDULE_SEED` | `0` | `1` stops the worker from (re)seeding Temporal Schedules on boot. |
| `WORKER_ROLE` | `legacy` | Activity bundle and task queue: `legacy`, `mrms`, `nowcast`, `hrrr`, `aux`, `alerts`, `open-meteo`, or `all`. |
| `USE_ISOLATED_TASK_QUEUES` | `0` | Route schedules to role queues only after matching role workers are deployed. |

### Upstream identification
| Var | Default | Notes |
|---|---|---|
| `NWS_USER_AGENT` | `(radar-ng, you@example.com)` | **Set this.** Real contact required by api.weather.gov. |
| `NWS_ALERTS_URL` | `https://api.weather.gov/alerts/active?status=actual` | Active-alerts feed, polled every 5 min. |
| `TROPICAL_FEED_URL` | `https://www.nhc.noaa.gov/CurrentStorms.json` | NHC tropical cyclone feed, hourly. |

### Basemap
| Var | Default | Notes |
|---|---|---|
| `BASEMAP_PMTILES_URL` | dated example | **Set this** to a recent build (see above). |
| `BASEMAP_BBOX` | `-125,24,-66,50` | `minLon,minLat,maxLon,maxLat`; default = CONUS. |

### Frontend basemap provider (bundled vs external)

The app ships **batteries-included**: by default the map renders the **bundled**
Protomaps basemap served by your own tile-server (`/basemap/styles/*`, backed by
`basemap-bootstrap` + go-pmtiles above). Forks and upstream users set nothing.

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
| `EXPO_PUBLIC_BASEMAP_SATELLITE_STYLE_URL` | _(unset → bundled Esri)_ | Optional; omit to keep the bundled no-key satellite style. |

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
| `PALETTES` | `classic,muted,vivid` | Each needs `backend/shared/palettes/<name>.json`. |
| `BACKLOG_PER_CYCLE` | `2` | Max MRMS frames back-filled per 2-min cycle (prod: 3). |
| `TEMPORAL_MAX_CONCURRENT_ACTIVITIES` | `2` | Concurrent activities in the worker (prod: 4). |
| `TEMPORAL_MAX_CONCURRENT_ACTIVITY_TASK_POLLS` | `1` | Concurrent activity task polls (prod: 2). |
| `MRMS_RENDER_WORKERS` | `2` | Parallel palette-render processes per MRMS frame. |
| `STORM_THRESHOLD_DBZ` | `40` | Reflectivity threshold used to form tracked storm cells. |
| `STORM_MIN_PIXELS` | `5` | Minimum connected MRMS pixels in a storm cell. |
| `STORM_MAX_CELLS` | `500` | Strongest storm cells retained per frame. |
| `STORM_MAX_TRACK_SPEED_KMH` | `160` | Association and extrapolation speed ceiling. |
| `STORM_MIN_TRACK_RADIUS_KM` | `20` | Minimum consecutive-frame cell matching radius. |
| `STORM_PREFETCH_PADDING_KM` | `12` | Padding around each predicted storm bbox. |
| `STORM_PREFETCH_MAX_DISTANCE_KM` | `500` | Do not prefetch when the nearest storm is farther from the user. |
| `FORECAST_HOURS` | `18` | HRRR forecast hours rendered per model run. |
| `EXTENDED_FORECAST_HOURS` | `48` | Extended sub-hourly HRRR hours. |
| `HRRR_ENABLED_LAYERS` | `radar-hrrr` | Comma-separated HRRR layers. Reflectivity-only is the production-safe default. |

### Nowcast (pysteps S-PROG)
| Var | Default | Notes |
|---|---|---|
| `NOWCAST_HORIZON_MIN` | `60` | Minutes of extrapolated nowcast. |
| `NOWCAST_STEP_MIN` | `5` | Minutes between nowcast frames. |
| `NOWCAST_INPUT_FRAMES` | `4` | Recent MRMS frames fed to the motion field; values below pySTEPS' minimum of 3 are clamped. |
| `NOWCAST_GRID_INPUT_LAYER` | `radar-nowcast-input` | Higher-fidelity grid dedicated to motion estimation. |
| `NOWCAST_MAX_INPUT_GAP_MIN` | `6` | Fail closed when consecutive input frames exceed this cadence. |
| `NOWCAST_ALLOW_PERSISTENCE_FALLBACK` | `0` | Opt-in stationary fallback; disabled so degraded output is not presented as a motion forecast. |

### Storm watch
| Var | Default | Notes |
|---|---|---|
| `WATCH_RADIUS_KM` | `20` | Radius around a watched location that triggers storm alerts. |
| `INTENSIFY_DBZ_DELTA` | `10` | dBZ increase over previous frame = "intensifying". |
| `DISSIPATE_DBZ_DELTA` | `10` | dBZ decrease = "dissipating". |
| `HAIL_DBZ_THRESHOLD` | `60` | dBZ at/above which hail is flagged. |
| `PUSH_DISABLED` | `1` | `1` = never touch APNS/FCM (needs key files wired to enable). |

### Lightning (Blitzortung)
| Var | Default | Notes |
|---|---|---|
| `LIGHTNING_RETENTION_MIN` | `15` | Minutes of strikes kept in the rolling buffer. |
| `LIGHTNING_FLUSH_S` | `2.0` | Seconds between buffer flushes to the state dir. |

### Tile-server API
| Var | Default | Notes |
|---|---|---|
| `OPEN_METEO_BASE` | `http://open-meteo:8080/v1/forecast` | Forecast upstream; default is the bundled self-hosted Open-Meteo. |
| `MRMS_MAX_AGE_S` | `600` | Radar age (s) past which `/api/health` reports `degraded`. |
| `FORECAST_TTL_S` | `300` | In-process forecast response cache TTL (s). |
| `FORECAST_CACHE_MAX_ENTRIES` | `512` | Bound for the in-process point-forecast cache. |
| `WIND_CACHE_MAX_ENTRIES` | `48` | Bound for decoded wind-grid cache entries. |
| `API_RATE_LIMIT_RPS` | `20` | Per-client token refill rate for API routes. |
| `API_RATE_LIMIT_BURST` | `60` | Per-client API token bucket capacity. |
| `DISABLE_WORKFLOW_ROUTES` | `0` | Disable watch/push mutation routes until an identity issuer and signing key are configured. |
| `WORKFLOW_AUTH_SIGNING_KEY` | unset | HMAC key for scoped workflow-route bearer tokens. |

---

See also: [GETTING_STARTED.md](../GETTING_STARTED.md) ·
[self-hosting.md](self-hosting.md) · [tuning.md](tuning.md) (what to change for
faster/cheaper/fresher).
