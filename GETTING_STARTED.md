# Getting started with radar-ng

radar-ng is a monorepo: a Rust ingest/tile pipeline and API (`rust/`), an Expo
mobile app (`frontend/`), and a web radar (`web/`). This guide gets you from
clone → running stack → app on your phone.

There are two paths:

1. **[Try it locally (Docker Compose)](#path-a--try-it-locally-docker-compose)** — one host, ~10 min, good for evaluating.
2. **[Run it in your own Kubernetes cluster](#path-b--run-it-in-your-own-kubernetes-cluster)** — the real deployment. You bring Temporal, a datastore, and shared storage.

All data sources (NOAA MRMS/HRRR, NWS alerts, Open-Meteo, Blitzortung) are free —
**no API keys, no accounts.** The only thing you must set is a contact email in
the NWS User-Agent.

---

## Prerequisites (both paths)

- **Git**, and a clone of this repo.
- For the app: **Node 20+** and **[Bun](https://bun.sh)** (the frontend uses Bun), plus the **Expo** toolchain (`npx expo`). A physical device with **Expo Go**, or an iOS/Android simulator.
- Roughly **3–4 GB disk** for the CONUS basemap + a working set of radar tiles.

You do **not** need a Rust toolchain to run the stack — the images are published
to GHCR (see [Images](#images)), and Compose builds them locally inside Docker.

---

## Images

The three first-party application images are published to GitHub Container Registry:

| Image | Role |
|---|---|
| `ghcr.io/mitchross/radar-ng-tile-server` | `radar api`: serves tiles, the basemap, `/api/*` and `/v1/*` |
| `ghcr.io/mitchross/radar-ng-temporal-worker` | `radar worker`: the ingest/render activities (MRMS, HRRR, lightning, nowcast, alerts, cleanup); `WORKER_ROLE` picks the pool |
| `ghcr.io/mitchross/radar-ng-open-meteo-worker` | the Open-Meteo image plus `radar worker --role open-meteo` (the `open_meteo_sync` activity only) |

All three run the same Rust binary, built from `deploy/Dockerfile`.

Third-party images you'll also run: `temporalio/auto-setup` (or your own
Temporal), `postgres:16` (Temporal's datastore), `ghcr.io/open-meteo/open-meteo`,
and **`protomaps/go-pmtiles`** for the one-time `basemap-bootstrap` step that
downloads a CONUS PMTiles extract into a volume (see Path A /
[docs/self-hosting.md](docs/self-hosting.md)). The api serves that archive
itself; the map data is fetched at deploy time, not baked into a public image.

> Tags are auto-semver (`vX.Y.Z`) plus `latest`. Pin a `vX.Y.Z` in production.

---

## Path A — try it locally (Docker Compose)

Fastest way to see radar. Full detail lives in **[docs/self-hosting.md](docs/self-hosting.md)**; the short version:

```bash
cd deploy
cp .env.example .env
# EDIT .env — set at minimum:
#   NWS_USER_AGENT=(radar-ng, you@example.com)   # api.weather.gov requires a real contact
#   BASEMAP_PMTILES_URL=https://build.protomaps.com/YYYYMMDD.pmtiles  # a recent date
```

Bootstrap the basemap once (~1–2 GB CONUS extract), then bring the stack up:

```bash
docker compose run --rm basemap-bootstrap    # one-time download
docker compose run --rm basemap-publish      # move it into place
docker compose build api                     # the radar image; `worker` runs it without building
docker compose up -d --build                 # add --profile ui for the Temporal Web UI
```

Verify (first radar frame lands ~2 min after boot):

```bash
curl localhost:8080/api/livez     # 200 once the api answers
curl localhost:8080/api/health    # "degraded" until the first frame, then "ok"
curl localhost:8080/api/manifest.json  # layers appearing
```

Then [point the app](#running-the-app) at `http://<your-host>:8080`.

---

## Path B — run it in your own Kubernetes cluster

The compose stack maps 1:1 to k8s. There are no ready-to-apply manifests in
this repo; the live manifests in the
[talos-argocd-proxmox repo](https://github.com/mitchross/talos-argocd-proxmox/tree/main/my-apps/personal-projects/radar-ng)
(`my-apps/personal-projects/radar-ng/`) are the reference to copy and adapt.

### What you must provide

1. **A Kubernetes cluster.** Anything works (this project runs on Talos + ArgoCD, but nothing is Talos-specific).

2. **A Temporal server**, reachable at `temporal-frontend.<ns>:7233`. Options:
   - Deploy the [Temporal Helm chart](https://github.com/temporalio/helm-charts), **or**
   - Run `temporalio/auto-setup` for a quick single-binary Temporal, **or**
   - Point at an existing Temporal you already run.
   - Temporal needs a **datastore** — **PostgreSQL** (a `postgres:16` StatefulSet, CloudNativePG, or managed PG) or Cassandra. The Helm chart / auto-setup can provision this for you.
   - Create the namespace the workers use (default: `default`; production uses `radar-ng`).

3. **Storage (this is the part people miss):**
   - **Shared tile storage.** The tile-server *serves* tiles that the workers *write*, so they need shared access to the same volume. Easiest is an **RWX** class — **NFS**, CephFS, or Longhorn-RWX. If you only have **RWO**, co-locate the tile-server and workers on one node (nodeAffinity/`podAffinity`) so they can share the volume.
   - **Open-Meteo data volume** (`openmeteo-data`): the open-meteo serve container and its sync worker share one volume — **RWO is fine**, but they must be co-located (the reference manifest runs the sync worker as a **sidecar** in the serve pod for exactly this reason).
   - **Basemap volume** (~1–2 GB for CONUS), plus small volumes for `grids`/`state`.
   - Rough sizing: basemap 1–2 GB, tiles working set a few GB (a `TileCleanup` workflow prunes old frames), open-meteo data a few GB.

4. **Ingress/Gateway** to expose the tile-server as your public API (e.g. `radar-ng-api.example.com` → tile-server `:8080`).

### Deploy steps

1. **Get the images** — the three `ghcr.io/mitchross/radar-ng-*` images, pinned by `vX.Y.Z@sha256:…` (or mirror them into your own registry).
2. **Temporal** — stand up Temporal + Postgres. The reference uses the Temporal Worker Controller (`TemporalConnection` + `WorkerDeployment` CRs) so each release is smoke-tested before promotion; plain Deployments running `radar worker` also work.
3. **Config** — the worker tunables go in a ConfigMap. **Set `NWS_USER_AGENT` to your email.** Everything else has working defaults (palettes, retention, nowcast horizon, forecast hours) — see [docs/configuration.md](docs/configuration.md) and [docs/tuning.md](docs/tuning.md).
4. **Workers** — either one worker with `WORKER_ROLE=legacy` (every workflow on queue `radar-ng`), or five role pools (`mrms`, `nowcast`, `hrrr`, `aux`, `alerts`) with `USE_ISOLATED_TASK_QUEUES=1`, as production runs. Add the open-meteo worker (`--role open-meteo`). Temporal dispatches by registration — nothing else to wire.
5. **Tile-server + open-meteo + basemap** — the serve Deployments, Services, PVCs, and your ingress/Gateway. The api serves the basemap from `BASEMAP_PMTILES` if mounted, or proxies `/basemap/tiles/*` to a go-pmtiles service at `BASEMAP_TILES_UPSTREAM`.
6. **Basemap** — fetch the PMTiles extract once into the basemap volume so the map isn't gray.
7. **Schedules** — exactly one worker seeds Temporal Schedules on boot (`SEED_SCHEDULES=1`; `legacy` seeds by default, `SKIP_SCHEDULE_SEED=1` disables it). No cron needed.

Verify the same endpoints as Path A against your ingress host:
`/api/livez`, `/api/health`, `/api/manifest.json`, or run
`radar debug doctor --server https://<host>` ([docs/debug-harness.md](docs/debug-harness.md)).
Deeper k8s notes (probes, HPA, PVCs) are in [docs/kubernetes.md](docs/kubernetes.md).

---

## Running the app

The Expo app in `frontend/` talks to your tile-server over HTTP.

```bash
cd frontend
bun install
bunx expo start        # scan the QR with Expo Go, or press i / a for a simulator
```

Point it at your server one of two ways:

- **In-app (no rebuild):** open **Settings → Advanced Mode → Stack URL** and enter your API base (`http://<host>:8080` for compose, or your ingress URL like `https://radar-ng-api.example.com`).
- **Change the default:** edit `SELF_HOSTED.DEFAULT_URL` in [`frontend/src/lib/constants.ts`](frontend/src/lib/constants.ts) and rebuild.

For device builds, CarPlay, and Apple Watch, see
[docs/running-the-app.md](docs/running-the-app.md) and
[docs/carplay-watch-setup.md](docs/carplay-watch-setup.md).

---

## Where to go next

| Topic | Doc |
|---|---|
| How it all fits together | [ARCHITECTURE.md](ARCHITECTURE.md) |
| Every config value (env vars) | [docs/configuration.md](docs/configuration.md) |
| Self-hosting (compose) in depth | [docs/self-hosting.md](docs/self-hosting.md) |
| Kubernetes specifics | [docs/kubernetes.md](docs/kubernetes.md) |
| Making it faster / cheaper / fresher | [docs/tuning.md](docs/tuning.md) |
| Cutting a release / image tags | [docs/releasing.md](docs/releasing.md) |
