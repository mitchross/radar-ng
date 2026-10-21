# Getting started with radar-ng

radar-ng is a monorepo with three parts:

- `rust/`: the backend. One Rust binary, `radar`, downloads public weather data, renders map tiles and serves them with a JSON API.
- `frontend/`: the Expo phone app (iOS and Android).
- `web/`: the web radar.

All data sources (NOAA MRMS/HRRR, NWS alerts, Open-Meteo, Blitzortung) are free, with **no API keys
and no accounts.** The only thing you must set is a contact email for the NWS API.

## Pick your path

| I want to… | Path | Time | Guide |
|---|---|---|---|
| Run the whole stack on one computer | **Docker Compose** | ~10 min, then ~2 min for the first radar frame | [Path A](#path-a-docker-compose-one-computer) → [compose.md](../self-host/compose.md) |
| Run it on my own Kubernetes cluster | **Kubernetes** | longer; you bring Temporal and storage | [Path B](#path-b-kubernetes) → [kubernetes.md](../self-host/kubernetes.md) |
| Use the phone app with a server I already run | **App only** | ~5 min | [Running the app](#running-the-app) |
| Change the code | **Develop** | | [CONTRIBUTING.md](../../CONTRIBUTING.md) |

## Prerequisites

- **Git**, and a clone of this repo.
- **For the backend:** Docker with Compose v2 (Path A) or a Kubernetes cluster (Path B). You do **not**
  need a Rust toolchain: images are published to GHCR, and Compose builds the image inside Docker.
- **For the app:** Node 20+, [Bun](https://bun.sh) and the Expo toolchain (`npx expo`). Use a phone
  with **Expo Go**, or an iOS/Android simulator.
- About **3–4 GB of disk** for the US basemap plus a working set of radar tiles.

## Images

| Image | Role |
|---|---|
| `ghcr.io/mitchross/radar-ng-tile-server` | `radar api`: serves tiles, the basemap, `/api/*` and `/v1/*` |
| `ghcr.io/mitchross/radar-ng-temporal-worker` | `radar worker`: the ingest and render jobs (MRMS, HRRR, lightning, nowcast, alerts, cleanup); `WORKER_ROLE` picks the pool |
| `ghcr.io/mitchross/radar-ng-open-meteo-worker` | the Open-Meteo image plus `radar worker --role open-meteo` (the `open_meteo_sync` activity only) |

All three run the same Rust binary, built from `deploy/Dockerfile`. Tags are `vX.Y.Z` plus `latest`;
pin a `vX.Y.Z` in production. Build details are in [deploy/README.md](../../deploy/README.md).

Third-party images you'll also run: `temporalio/auto-setup` (or your own Temporal), `postgres:16`
(Temporal's datastore), `ghcr.io/open-meteo/open-meteo`, and `protomaps/go-pmtiles` for the one-time
basemap download. The map data is fetched at deploy time, not baked into an image.

## Path A: Docker Compose (one computer)

```bash
cd deploy
cp .env.example .env
# Edit .env and set:
#   NWS_USER_AGENT=(radar-ng, you@example.com)                         # api.weather.gov needs a real contact
#   BASEMAP_PMTILES_URL=https://build.protomaps.com/YYYYMMDD.pmtiles   # a date from the last few days
docker compose run --rm basemap-bootstrap   # one-time basemap download (~1–2 GB)
docker compose run --rm basemap-publish     # move it into place
docker compose build api                    # the radar image; `worker` reuses it
docker compose up -d --build                # add --profile ui for the Temporal Web UI
curl localhost:8080/api/health              # "degraded" until the first frame, then "ok"
```

Then [point the app](#running-the-app) at `http://<your-host>:8080`. The full guide, with hardware
sizing, what each step does and how to verify it, is **[self-host/compose.md](../self-host/compose.md)**.

## Path B: Kubernetes

You provide a cluster, a Temporal server and storage the tile-server and workers can share. The
**[Kubernetes guide](../self-host/kubernetes.md)** covers what you need, the deploy steps, storage,
probes and sizing. Ready-to-apply Kustomize manifests are in
**[deploy/kubernetes/](../../deploy/kubernetes/README.md)**; to try them on kind, k3d or k3s:

```bash
kubectl apply -k deploy/kubernetes/examples/minimal
```

## Running the app

The Expo app in `frontend/` talks to your tile-server over HTTP.

```bash
cd frontend
bun install
bunx expo start        # scan the QR with Expo Go, or press i / a for a simulator
```

Point it at your server in one of two ways:

- **In the app (no rebuild):** open **Settings → Advanced Mode → Stack URL** and enter your API base
  (`http://<host>:8080` for Compose, or your ingress URL such as `https://radar-ng-api.example.com`).
- **Change the default:** edit `SELF_HOSTED.DEFAULT_URL` in
  [`frontend/src/lib/constants.ts`](../../frontend/src/lib/constants.ts) and rebuild.

Device builds, CarPlay and Apple Watch: [app/running-the-app.md](../app/running-the-app.md) and
[app/carplay-watch-setup.md](../app/carplay-watch-setup.md). The web radar: [web/README.md](../../web/README.md).

## Where to go next

Everything else is indexed in **[docs/README.md](../README.md)**.
