# Run radar-ng on one computer (Docker Compose)

The easiest way to run your own radar server: one computer, Docker, about 15 minutes of setup and
2 minutes of waiting for the first radar picture. Running Kubernetes instead? See
[deploy/kubernetes](../../deploy/kubernetes/README.md).

**In short:** you copy a settings file, set two values, download a map once, and start everything
with `docker compose up`. Words you don't know are in the [glossary](../start/glossary.md).

## What you get

A complete weather server on hardware you own. Every data source is **free, with no API key and no
account**:

| layer | source | cadence | resolution |
|---|---|---|---|
| radar (base + composite) | NOAA MRMS S3 | 2 min (polled every minute) | ~1 km |
| radar-hrrr simulated reflectivity | NOAA HRRR S3 | hourly, 18–48 h out | 3 km |
| nowcast (+60 min) | Lucas-Kanade motion + S-PROG extrapolation of MRMS | 2 min input cadence | ~2 km science grid |
| lightning | Blitzortung websocket | ~1 min | strikes |
| tropical | NHC GIS feeds | 6 h | tracks + cones |
| forecast | self-hosted Open-Meteo (GFS + HRRR) | 1–6 h | point forecast |
| basemap | Protomaps PMTiles | static | vector tiles |

Tile retention per layer is in [tuning.md](tuning.md#tile-retention--cleanup).

## What you need

### Hardware

| profile | what it runs | min | recommended |
|---|---|---|---|
| **lab** | docker compose, single node, MRMS radar + nowcast + forecast | 4 cores · 4 GB · 30 GB SSD | 8 cores · 8 GB · 50 GB SSD |
| **prod** | K8s, MRMS + nowcast + HRRR reflectivity | 16 cores · 16 GB · 100 GB SSD | 24 cores · 32 GB · 200 GB NVMe |

Most of the work is drawing radar pictures: every new radar scan is turned into map tiles at several zoom levels, once per color scheme. More CPU cores means fresher radar. See [tuning.md](tuning.md) before cutting the pipeline down for small hosts.

### Software

| requirement | version |
|---|---|
| A computer running Docker | Linux x86-64 runs the ready-made images. Apple Silicon, Raspberry Pi and other arm64 machines build their own (`./up.sh --build`); macOS works with Docker Desktop or OrbStack |
| Docker Engine | 24+ |
| Docker Compose | v2 (the `docker compose` plugin, not legacy `docker-compose`) |
| `curl` + `jq` | for verification |

### Network + disk

- **Egress:** ~50–100 MB/h steady-state pull from NOAA S3 buckets. Blitzortung is a low-volume websocket. Open-Meteo model syncs add bursts (GFS every 6 h, HRRR hourly).
- **Disk:** steady-state ≈ 5 GB of tiles/grids (the cleanup activity sweeps hourly), spiking to ~10 GB during HRRR runs. The Open-Meteo data volume grows with the models you sync (a few GB for GFS + HRRR CONUS). The basemap PMTiles extract is a one-time ~9 GB download (CONUS to zoom 14; about 3 minutes on a fast link).
- **Inbound:** nothing required. The stack is pull-only; the only public listener is the api on `:8080` (compose also publishes Temporal's ports on the host).

## Steps

### The short way

```bash
git clone <your-radar-ng-clone-url> radar-ng
cd radar-ng/deploy
./up.sh
```

The first run creates `deploy/.env` and stops, asking you to set your contact email in it
(`NWS_USER_AGENT=(radar-ng, you@example.org)`; the National Weather Service asks every app for one).
Run `./up.sh` again. It downloads the background map once, starts everything, waits for the first
radar picture, and prints the address to put in the app.

**On a Mac with Apple Silicon, a Raspberry Pi or another arm64 machine**, use `./up.sh --build`. The
ready-made images are for x86-64 (amd64) only, so it builds them on your machine instead. The first
build takes a while and needs several GB of RAM.

The rest of this section is what `up.sh` does, step by step, if you'd rather type it yourself.

### 1. Get the code and the settings file

```bash
git clone <your-radar-ng-clone-url> radar-ng
cd radar-ng/deploy
cp .env.example .env
```

Open `.env` in any text editor and set **`NWS_USER_AGENT`** to your own contact:
`NWS_USER_AGENT=(radar-ng, you@example.org)`. Everything else has a working default.

### 2. Download the background map (once, ~9 GB)

```bash
docker compose run --rm basemap-bootstrap
```

This finds the newest daily map build from [Protomaps](https://build.protomaps.com/) by itself (they
keep each one for only about 6 days), downloads just your area (`BASEMAP_BBOX`, default the lower 48
US states up to zoom 14, ~9 GB rather than the ~120 GB planet), and swaps it into place in one step, so the server
never sees a half-written file. Running it again does nothing unless you change `BASEMAP_BBOX` or `BASEMAP_MAXZOOM`. To pin
one specific build, set `BASEMAP_PMTILES_URL` in `.env`.

### 3. Start everything

```bash
docker compose up -d
```

This pulls the ready-made server images from GHCR (`ghcr.io/mitchross/radar-ng-*`, pinned by version and
digest), so there's nothing to compile. To build them from your checkout instead (arm64 machines, or
after you change the Rust code):

```bash
docker compose -f docker-compose.yml -f docker-compose.build.yml up -d --build
```

To use images from your own registry or fork, set `RADAR_API_IMAGE`, `RADAR_WORKER_IMAGE` and
`RADAR_OPEN_METEO_WORKER_IMAGE` in `.env`.

What starts:

| service | role |
|---|---|
| `temporal` (+ `temporal-postgres`) | Temporal server — schedules and dispatches all ingest work (gRPC on `:7233` for the `temporal` CLI) |
| `temporal-ui` | optional web UI — `docker compose --profile ui up -d`, then <http://localhost:8233> |
| `volume-init` | one-shot chown of the named volumes to the images' users |
| `worker` | `radar worker --role legacy` — runs every workflow and activity on task queue `radar-ng` and seeds the schedules; production splits roles onto isolated queues |
| `open-meteo` | self-hosted forecast API |
| `open-meteo-worker` | `radar worker --role open-meteo` (task queue `radar-ng-open-meteo`), runs the model syncs |
| `api` | `radar api` — tiles, basemap, `/api`, `/v1`; the only public port (`:8080`) |

Named volumes: `tiles`, `grids`, `state`, `openmeteo-data`, `basemap-data`, `temporal-pg-data`. There is no cron anywhere — the worker registers Temporal Schedules (MRMS polled every minute, HRRR every 15 min, nowcast every 2 min plus a kick from every new MRMS frame, tile-cleanup hourly, Open-Meteo syncs every 1–6 h) idempotently on startup.

The `.env.example` ingest defaults are deliberately lab-sized (`BACKLOG_PER_CYCLE=2`, `TEMPORAL_MAX_CONCURRENT_ACTIVITIES=2`); production values are noted inline in the file and in [tuning.md](tuning.md).

## How to know it worked

```bash
# process up? (pure liveness — 200 as soon as the api answers)
curl http://localhost:8080/api/livez

# data fresh? (reports "degraded" + 503 until the first frame lands)
curl http://localhost:8080/api/health

# layers appearing?
curl -s http://localhost:8080/api/manifest.json | jq '.layers | keys'

# basemap serving?
curl -sI http://localhost:8080/basemap/styles/positron.json
```

**The first radar picture takes about 2 minutes**: the worker's `ingest-mrms-base` schedule fires, downloads the latest MRMS GRIB2 (~8 MB), decodes it, and renders the tile pyramid. Once `/api/health` returns `{"status":"ok","mrms_age_s":<~120>,...}` you're live. HRRR-derived layers fill in over the next 15–30 minutes; the first forecast appears after the first Open-Meteo sync completes.

Watch the worker if you're impatient:

```bash
docker compose logs -f worker
radar debug doctor               # if you have the binary; see debug-harness.md
```

## Connect the phone app

Build and run the app per [running-the-app.md](../app/running-the-app.md), then in the app:

**Settings → Advanced Mode → Stack URL** and enter your server URL:

| where the app runs | server URL |
|---|---|
| iOS simulator (same machine as the backend) | `http://localhost:8080` |
| Android emulator | `http://10.0.2.2:8080` (emulator loopback magic) |
| Physical device on your LAN | `http://<your-LAN-IP>:8080` |
| Anywhere (public exposure) | `https://radar.your-domain.example` |

The manifest fetch determines which layers show up — if a layer hasn't been ingested yet, it simply won't be offered.

## Reach it from outside your home network

The api speaks plain HTTP on `:8080`. To reach it from outside your LAN, put a TLS-terminating proxy in front — any of:

- **Reverse proxy** (Caddy, nginx, Traefik) with a Let's Encrypt cert, forwarding to `your-server:8080`
- **Cloudflare Tunnel** (`cloudflared`) — no inbound port opening at all
- **Tailscale / WireGuard** — keep it private but reachable from your phone anywhere

**Security note:** the mutating `/v1/*` routes (push tokens, storm watches) need a per-user HMAC bearer token signed with `WORKFLOW_AUTH_SIGNING_KEY`; with no key they answer 503, and `DISABLE_WORKFLOW_ROUTES=1` turns them off. Blocking `/v1/*` at your proxy as well costs nothing if you don't use them. The `GET`-only `/api/*` and `/tiles/*` surface is harmless to expose.

Example nginx location block for read-only public exposure:

```nginx
location /v1/ { deny all; }
location / { proxy_pass http://your-server:8080; }
```

## If it didn't work

More problems and fixes (port already in use, image pulls, Temporal, arm64, disk) are in
**[troubleshooting.md](troubleshooting.md)**.

### `/api/health` says `degraded` / radar is stale

`mrms_age_s` exceeded `MRMS_MAX_AGE_S` (default 600 s). Causes, in order of likelihood:

1. **It just started.** No radar picture yet. Wait 2–3 minutes.
2. **Worker down or wedged** — `docker compose logs worker`. Schedules keep firing in Temporal but nothing picks them up if the worker is dead; a restart re-seeds and catches up (`BACKLOG_PER_CYCLE`, newest-first).
3. **Underpowered host** — the palette render can't keep up with the 2-min cadence. See [tuning.md](tuning.md): fewer palettes.
4. **NOAA having a slow day** — check whether new objects are actually landing in the MRMS S3 bucket. Nothing to do but wait; the app shows a "data delayed" banner off the same health endpoint.

### Blank / gray basemap

The PMTiles archive is missing — without it the api proxies `/basemap/tiles/*` to `BASEMAP_TILES_UPSTREAM`, which compose does not run. Verify:

```bash
curl -sI http://localhost:8080/basemap/tiles/4/4/6.mvt   # expect 200 with content
docker compose logs api
```

Fixes:

- You skipped the bootstrap: `docker compose run --rm basemap-bootstrap`
- The bootstrap 404'd: you pinned `BASEMAP_PMTILES_URL` to a build that has expired (~6-day retention). Empty it to use the newest build, and re-run
- Your map view is outside the extracted `BASEMAP_BBOX` (default CONUS) — widen the bbox and re-run the bootstrap

### No forecast / temperatures show 0°

The `open-meteo` service serves nothing until its data volume has been populated by a sync. The `open-meteo-sync-gfs` (6 h) and `open-meteo-sync-hrrr` (1 h) schedules handle this, but on a fresh install you may be waiting on the first fire. Trigger one by hand:

```bash
docker compose exec temporal \
  temporal schedule trigger --schedule-id open-meteo-sync-gfs
```

Then confirm the proxy path works: `curl http://localhost:8080/api/forecast/40.0/-83.0`.

### Layers missing from the manifest

Each layer appears only after its first successful ingest. `nowcast` additionally needs `NOWCAST_INPUT_FRAMES` (default 4) MRMS grids on disk before it can extrapolate, so expect it ~10 minutes after the radar layer.
