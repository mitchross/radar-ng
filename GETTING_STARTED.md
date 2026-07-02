# Getting started with radar-ng

radar-ng is a monorepo: a Python ingest/tile pipeline (`backend/`, `temporal/`)
and an Expo mobile app (`frontend/`). This guide gets you from clone → running
stack → app on your phone.

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

You do **not** need to build the backend images yourself — they're published to
GHCR (see [Images](#images)). Compose can also build them locally.

---

## Images

The three first-party application images are published to GitHub Container Registry:

| Image | Role |
|---|---|
| `ghcr.io/mitchross/radar-ng-tile-server` | Caddy + FastAPI: serves `/api/*`, `/v1/*`, and tiles |
| `ghcr.io/mitchross/radar-ng-temporal-worker` | Main Temporal worker — all ingest/render activities (MRMS, HRRR, lightning, nowcast, alerts, cleanup) |
| `ghcr.io/mitchross/radar-ng-open-meteo-worker` | Secondary worker — the `open_meteo_sync` activity only |

Third-party images you'll also run: `temporalio/auto-setup` (or your own
Temporal), `postgres:16` (Temporal's datastore), `ghcr.io/open-meteo/open-meteo`,
and **`protomaps/go-pmtiles`** for the basemap (radar-ng ships a
`basemap-bootstrap` step that downloads a CONUS PMTiles extract into a volume —
see Path A / [docs/self-hosting.md](docs/self-hosting.md); the basemap isn't a
prebuilt GHCR image because the map data is fetched at deploy time, not baked
into a public image).

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
docker compose run --rm basemap-bootstrap    # one-time
docker compose up -d                          # add --profile ui for the Temporal Web UI
```

Verify (first radar frame lands ~2 min after boot):

```bash
curl localhost:8080/api/livez     # 200 once Caddy + FastAPI answer
curl localhost:8080/api/health    # "degraded" until the first frame, then "ok"
curl localhost:8080/api/manifest  # layers appearing
```

Then [point the app](#running-the-app) at `http://<your-host>:8080`.

---

## Path B — run it in your own Kubernetes cluster

The compose stack maps 1:1 to k8s. Reference manifests live in
**[`deploy/k8s/`](deploy/k8s/)** (and see its
[README](deploy/k8s/README.md)); the canonical live manifests are in the
[talos-argocd-proxmox repo](https://github.com/mitchross/talos-argocd-proxmox/tree/main/my-apps/development/radar-ng).

### What you must provide

1. **A Kubernetes cluster.** Anything works (this project runs on Talos + ArgoCD, but nothing is Talos-specific).

2. **A Temporal server**, reachable at `temporal-frontend.<ns>:7233`. Options:
   - Deploy the [Temporal Helm chart](https://github.com/temporalio/helm-charts), **or**
   - Run `temporalio/auto-setup` for a quick single-binary Temporal, **or**
   - Point at an existing Temporal you already run.
   - Temporal needs a **datastore** — **PostgreSQL** (a `postgres:16` StatefulSet, CloudNativePG, or managed PG) or Cassandra. The Helm chart / auto-setup can provision this for you.
   - Create the namespace the workers use (default: `default`).

3. **Storage (this is the part people miss):**
   - **Shared tile storage.** The tile-server *serves* tiles that the worker *writes*, so they need shared access to the same volume. Easiest is an **RWX** class — **NFS**, CephFS, or Longhorn-RWX. If you only have **RWO**, co-locate the tile-server and worker on one node (nodeAffinity/`podAffinity`) so they can share the volume.
   - **Open-Meteo data volume** (`openmeteo-data`): the open-meteo serve pod and its sync sidecar share one volume — **RWO is fine**, but they must be co-located (the reference manifest runs the sync worker as a **sidecar** in the serve pod for exactly this reason).
   - **Basemap volume** (~1–2 GB for CONUS), plus small volumes for `grids`/`state`.
   - Rough sizing: basemap 1–2 GB, tiles working set a few GB (a `TileCleanup` workflow prunes old frames), open-meteo data a few GB.

4. **Ingress/Gateway** to expose the tile-server as your public API (e.g. `radar-ng-api.example.com` → tile-server `:8080`).

### Deploy steps

1. **Get the images** — pull the three `ghcr.io/mitchross/radar-ng-*` application images (or mirror them into your own registry and update the manifest image refs). For the basemap, use `protomaps/go-pmtiles` + the bootstrap fetch.
2. **Temporal** — stand up Temporal + Postgres and the `TemporalConnection` CR ([`deploy/k8s/temporal-connection.yaml`](deploy/k8s/temporal-connection.yaml)).
3. **Config** — apply [`configmap-temporal-config.yaml`](deploy/k8s/configmap-temporal-config.yaml). **Set `NWS_USER_AGENT` to your email.** All other tunables have sane defaults (palettes, retention, nowcast horizon, forecast hours) — see [docs/tuning.md](docs/tuning.md).
4. **Workers** — apply the two worker deployments. They poll the same task queue; the main worker registers every activity except `open_meteo_sync`, the open-meteo worker registers only that one. Temporal dispatches by registration — nothing else to wire.
5. **Tile-server + open-meteo + basemap** — apply the serve deployments, services, PVCs, and your ingress/Gateway. (These live in the talos repo's `my-apps/development/radar-ng/`; use them as templates.)
6. **Basemap bootstrap** — run the one-time basemap fetch (a Job) so the map isn't gray.
7. **Schedules** — the worker seeds Temporal Schedules on boot (set `SKIP_SCHEDULE_SEED=1` to disable). No cron needed.

Verify the same endpoints as Path A against your ingress host:
`/api/livez`, `/api/health`, `/api/manifest`. Deeper k8s notes (probes, HPA,
PVCs) are in [docs/kubernetes.md](docs/kubernetes.md).

---

## Running the app

The Expo app in `frontend/` talks to your tile-server over HTTP.

```bash
cd frontend
bun install
bunx expo start        # scan the QR with Expo Go, or press i / a for a simulator
```

Point it at your server one of two ways:

- **In-app (no rebuild):** open **Settings → Server URL** and enter your API base (`http://<host>:8080` for compose, or your ingress URL like `https://radar-ng-api.example.com`).
- **Change the default:** edit `SELF_HOSTED.DEFAULT_URL` in [`frontend/src/lib/constants.ts`](frontend/src/lib/constants.ts) and rebuild.

For device builds, CarPlay, and Apple Watch, see
[docs/running-the-app.md](docs/running-the-app.md) and
[docs/carplay-watch-setup.md](docs/carplay-watch-setup.md).

---

## Where to go next

| Topic | Doc |
|---|---|
| How it all fits together | [ARCHITECTURE.md](ARCHITECTURE.md) |
| Self-hosting (compose) in depth | [docs/self-hosting.md](docs/self-hosting.md) |
| Kubernetes specifics | [docs/kubernetes.md](docs/kubernetes.md) |
| Making it faster / cheaper / fresher | [docs/tuning.md](docs/tuning.md) |
| Cutting a release / image tags | [docs/releasing.md](docs/releasing.md) |
