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

### Ingest / render tunables
| Var | Default | Notes |
|---|---|---|
| `PALETTES` | `classic,muted,vivid` | Each needs `backend/shared/palettes/<name>.json`. |
| `BACKLOG_PER_CYCLE` | `2` | Max MRMS frames back-filled per 2-min cycle (prod: 3). |
| `TEMPORAL_MAX_CONCURRENT_ACTIVITIES` | `2` | Concurrent activities in the worker (prod: 4). |
| `TEMPORAL_MAX_CONCURRENT_ACTIVITY_TASK_POLLS` | `1` | Concurrent activity task polls (prod: 2). |
| `MRMS_RENDER_WORKERS` | `2` | Parallel palette-render processes per MRMS frame. |
| `FORECAST_HOURS` | `18` | HRRR forecast hours rendered per model run. |
| `EXTENDED_FORECAST_HOURS` | `48` | Extended sub-hourly HRRR hours. |

### Nowcast (pysteps S-PROG)
| Var | Default | Notes |
|---|---|---|
| `NOWCAST_HORIZON_MIN` | `60` | Minutes of extrapolated nowcast. |
| `NOWCAST_STEP_MIN` | `5` | Minutes between nowcast frames. |
| `NOWCAST_INPUT_FRAMES` | `4` | Recent MRMS frames fed to the motion field. |

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
| `FORECAST_TTL_S` | `900` | In-process forecast response cache TTL (s). Compose sets `300`. |
| `FORECAST_CACHE_MAX_ENTRIES` | `1024` | Max cached forecast cells (0.1° snapped); oldest evicted beyond this. |

---

See also: [GETTING_STARTED.md](../GETTING_STARTED.md) ·
[self-hosting.md](self-hosting.md) · [tuning.md](tuning.md) (what to change for
faster/cheaper/fresher).
