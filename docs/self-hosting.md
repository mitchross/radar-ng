# Self-hosting radar-ng

The golden path: one machine, Docker Compose, a full NOAA radar pipeline in about ten minutes of typing and two minutes of waiting. Kubernetes is the advanced path — see [kubernetes.md](kubernetes.md).

## What you get

A complete weather backend on hardware you own. Every data source is **free and requires no API key, no account, no auth**:

| layer | source | cadence | resolution |
|---|---|---|---|
| radar (base + composite) | NOAA MRMS S3 | 2 min | ~1 km |
| radar-hrrr simulated reflectivity | NOAA HRRR S3 | hourly, 18–48 h out | 3 km |
| nowcast (+60 min) | pysteps S-PROG of MRMS | 2 min input cadence | ~2 km science grid |
| lightning | Blitzortung websocket | ~1 min | strikes |
| tropical | NHC GIS feeds | 6 h | tracks + cones |
| forecast | self-hosted Open-Meteo (GFS + HRRR) | 1–6 h | point forecast |
| basemap | Protomaps PMTiles | static | vector tiles |

Full layer matrix with tile paths and retention lives in the [README](../README.md#self-hosted-layer-matrix).

## Requirements

### Hardware

| profile | what it runs | min | recommended |
|---|---|---|---|
| **lab** | docker compose, single node, MRMS radar + nowcast + forecast | 4 cores · 4 GB · 20 GB SSD | 8 cores · 8 GB · 50 GB SSD |
| **prod** | K8s, MRMS + nowcast + HRRR reflectivity | 16 cores · 16 GB · 100 GB SSD | 24 cores · 32 GB · 200 GB NVMe |

The hot spot is the MRMS palette render: every radar frame is rasterized into a z4–z7 PNG pyramid per palette. More cores = fresher radar. See [tuning.md](tuning.md) before cutting the pipeline down for small hosts.

### Software

| requirement | version |
|---|---|
| Linux host (x86_64) | any modern distro; macOS works for dev |
| Docker Engine | 24+ |
| Docker Compose | v2 (the `docker compose` plugin, not legacy `docker-compose`) |
| `curl` + `jq` | for verification |

### Network + disk

- **Egress:** ~50–100 MB/h steady-state pull from NOAA S3 buckets. Blitzortung is a low-volume websocket. Open-Meteo model syncs add bursts (GFS every 6 h, HRRR hourly).
- **Disk:** steady-state ≈ 5 GB of tiles/grids (the cleanup activity sweeps hourly), spiking to ~10 GB during HRRR runs. The Open-Meteo data volume grows with the models you sync (a few GB for GFS + HRRR CONUS). The basemap PMTiles extract is a one-time ~1–2 GB download.
- **Inbound:** nothing required. The stack is pull-only; the only listener is the tile-server on `:8080`.

## Quick start

### 1. Clone and configure

```bash
git clone <your-radar-ng-clone-url> radar-ng
cd radar-ng/deploy
cp .env.example .env
```

Edit `.env` — everything has a sane default except two values you should set:

- **`NWS_USER_AGENT`** — api.weather.gov requires a contact email in the User-Agent. Use your own.
- **`BASEMAP_PMTILES_URL`** — Protomaps publishes date-stamped daily builds (`YYYYMMDD.pmtiles`) with ~6-day retention and **no `latest` alias**; pick a recent date from <https://build.protomaps.com/> or the bootstrap below will 404.

### 2. Basemap bootstrap (one-time, ~1–2 GB)

The `basemap` service serves vector tiles from a local Protomaps PMTiles archive — the one thing that isn't fetched automatically:

```bash
docker compose run --rm basemap-bootstrap
```

This runs `pmtiles extract`, which uses HTTP range reads to pull only your bounding box (`BASEMAP_BBOX`, default CONUS, ~1–2 GB — not the ~120 GB planet) into the `basemap-data` volume. To change coverage later, adjust the bbox and re-run; `go-pmtiles` picks up the replaced file without a restart.

### 3. Bring it up

```bash
docker compose up -d --build
```

Images build locally from the repo (no registry needed). The root `.dockerignore` keeps frontend output, test caches, docs, and local captures out of the worker build context; add any new local data directories there before they become large.

What starts:

| service | role |
|---|---|
| `temporal` (+ `temporal-postgres`) | Temporal server — schedules and dispatches all ingest work (gRPC on `:7233` for the `temporal` CLI) |
| `temporal-ui` | optional web UI — `docker compose --profile ui up -d`, then <http://localhost:8233> |
| `worker` | Temporal worker — the default `legacy` role runs every ingest activity on task queue `radar-ng` and self-seeds schedules; production can split roles onto isolated queues after all role workers are deployed |
| `open-meteo` | self-hosted forecast API |
| `open-meteo-worker` | isolated worker pool (task queue `radar-ng-open-meteo`) that runs the model syncs |
| `tile-server` | Caddy + FastAPI, the only public port (`:8080`) |
| `basemap` | Protomaps `go-pmtiles` vector tile server |

Named volumes: `tiles`, `grids`, `state`, `openmeteo-data`, `basemap-data`, `temporal-pg-data`. There is no cron anywhere — the worker registers Temporal Schedules (MRMS every 2 min, HRRR every 15 min, nowcast every 2 min, tile-cleanup hourly, Open-Meteo syncs every 1–6 h) idempotently on startup.

The `.env.example` ingest defaults are deliberately lab-sized (`BACKLOG_PER_CYCLE=2`, `TEMPORAL_MAX_CONCURRENT_ACTIVITIES=2`); production values are noted inline in the file and in [tuning.md](tuning.md).

### 4. Verify

```bash
# processes up? (pure liveness — 200 as soon as Caddy + FastAPI answer)
curl http://localhost:8080/api/livez

# data fresh? (reports "degraded" + 503 until the first frame lands)
curl http://localhost:8080/api/health

# layers appearing?
curl -s http://localhost:8080/api/manifest.json | jq '.layers | keys'

# basemap serving?
curl -sI http://localhost:8080/basemap/styles/positron.json
```

**The first radar frame takes about 2 minutes**: the worker's `ingest-mrms-base` schedule fires, downloads the latest MRMS GRIB2 (~8 MB), decodes it, and renders the tile pyramid. Once `/api/health` returns `{"status":"ok","mrms_age_s":<~120>,...}` you're live. HRRR-derived layers fill in over the next 15–30 minutes; the first forecast appears after the first Open-Meteo sync completes.

Watch the worker if you're impatient:

```bash
docker compose logs -f worker   # look for "schedule seed complete" then "worker starting"
```

## Connecting the mobile app

Build and run the app per [running-the-app.md](running-the-app.md), then in the app:

**Settings → Data Source → Self-Hosted** and enter your server URL:

| where the app runs | server URL |
|---|---|
| iOS simulator (same machine as the backend) | `http://localhost:8080` |
| Android emulator | `http://10.0.2.2:8080` (emulator loopback magic) |
| Physical device on your LAN | `http://<your-LAN-IP>:8080` |
| Anywhere (public exposure) | `https://radar.your-domain.example` |

The manifest fetch determines which layers show up — if a layer hasn't been ingested yet, it simply won't be offered.

## Exposing it publicly

The tile-server speaks plain HTTP on `:8080`. To reach it from outside your LAN, put a TLS-terminating proxy in front — any of:

- **Reverse proxy** (Caddy, nginx, Traefik) with a Let's Encrypt cert, forwarding to `your-server:8080`
- **Cloudflare Tunnel** (`cloudflared`) — no inbound port opening at all
- **Tailscale / WireGuard** — keep it private but reachable from your phone anywhere

**Security note:** there is currently **no authentication on the mutating `/v1/*` routes** (push tokens, storm watches, workflow triggers). If you expose the server publicly, either keep exposure read-only (block `/v1/*` at your proxy) or restrict access to a VPN/tunnel. The `GET`-only `/api/*` and `/tiles/*` surface is harmless to expose.

Example nginx location block for read-only public exposure:

```nginx
location /v1/ { deny all; }
location / { proxy_pass http://your-server:8080; }
```

## Troubleshooting

### `/api/health` says `degraded` / radar is stale

`mrms_age_s` exceeded `MRMS_MAX_AGE_S` (default 600 s). Causes, in order of likelihood:

1. **First boot** — no frame rendered yet. Wait 2–3 minutes.
2. **Worker down or wedged** — `docker compose logs worker`. Schedules keep firing in Temporal but nothing picks them up if the worker is dead; a restart re-seeds and catches up (`BACKLOG_PER_CYCLE=3`, newest-first).
3. **Underpowered host** — the palette render can't keep up with the 2-min cadence. See [tuning.md](tuning.md): fewer palettes, fewer zoom levels.
4. **NOAA having a slow day** — check whether new objects are actually landing in the MRMS S3 bucket. Nothing to do but wait; the app shows a "data delayed" banner off the same health endpoint.

### Blank / gray basemap

The PMTiles archive is missing — the `basemap` container runs fine without it, it just serves empty tiles. Verify:

```bash
curl -sI http://localhost:8080/basemap/tiles/4/4/6.mvt   # expect 200 with content
docker compose logs basemap
```

Fixes:

- You skipped the bootstrap: `docker compose run --rm basemap-bootstrap`
- The bootstrap 404'd: your `BASEMAP_PMTILES_URL` build date expired (~6-day retention) — set a recent `YYYYMMDD` from <https://build.protomaps.com/> and re-run
- Your map view is outside the extracted `BASEMAP_BBOX` (default CONUS) — widen the bbox and re-run the bootstrap

### No forecast / temperatures show 0°

The `open-meteo` service serves nothing until its data volume has been populated by a sync. The `open-meteo-sync-gfs` (6 h) and `open-meteo-sync-hrrr` (1 h) schedules handle this, but on a fresh install you may be waiting on the first fire. Trigger one by hand:

```bash
docker compose exec temporal \
  temporal schedule trigger --schedule-id open-meteo-sync-gfs
```

Then confirm the proxy path works: `curl http://localhost:8080/api/forecast/40.0/-83.0`.

### Layers missing from the manifest

Each layer appears only after its first successful ingest. `nowcast` additionally needs ≥4 MRMS grids on disk before pysteps can extrapolate, so expect it ~10 minutes after the radar layer.
