# Getting started with radar-ng

radar-ng has two halves:

- **The server** (`rust/`). It downloads public weather data, turns it into map images ("tiles") and
  serves them. You run it on a computer or cluster you own.
- **The apps** that show it: the phone app (`frontend/`, iPhone and Android) and the web radar (`web/`).

All the weather data (NOAA radar and forecasts, NWS alerts, Open-Meteo, Blitzortung lightning) is
free, with **no API keys and no accounts.** The only thing you must set is a contact email, because
the NWS asks for one.

Unfamiliar word? See the [glossary](glossary.md).

## Pick your path

| I want to… | Path | Time | Guide |
|---|---|---|---|
| Run the server on one computer | **Docker Compose** | ~10 min (mostly the map download), then ~2 min for the first radar frame | [Path A](#path-a-docker-compose-one-computer) → [compose.md](../self-host/compose.md) |
| Run the server on my Kubernetes cluster | **Kubernetes** | ~30 min on a test cluster (kind, k3d, k3s) | [Path B](#path-b-kubernetes) → [deploy/kubernetes](../../deploy/kubernetes/README.md) |
| See it in a browser, against a server I run | **Web radar** | ~5 min | [app/web-radar.md](../app/web-radar.md) |
| Use the phone app with a server I run | **Phone app** | 30–60 min the first time (native build) | [Running the phone app](#running-the-phone-app) |
| Change the code | **Develop** | | [CONTRIBUTING.md](../../CONTRIBUTING.md) |
| Ship my own copy of the app | **Fork** | | [FORKING.md](../../FORKING.md) |

## What you need

- **Git**, and a clone of this repo.
- **For the server:** Docker with Compose v2 (Path A) or a Kubernetes cluster (Path B). You do **not**
  need Rust installed: the images are published to GHCR, and Compose builds inside Docker.
- **For the phone app:** [Bun](https://bun.sh), plus Xcode (iPhone, macOS only) or Android Studio
  (Android). The app uses native map code, so it **can't run in Expo Go**: you build it once onto a
  simulator or your phone.
- About **3–4 GB of disk** for the US basemap and a working set of radar tiles.

## Path A: Docker Compose (one computer)

```bash
cd deploy
./up.sh          # use ./up.sh --build on Apple Silicon / arm64
```

The first run creates `deploy/.env` and asks you to set your contact email in it
(`NWS_USER_AGENT=(radar-ng, you@example.org)`; the NWS asks every app for one). Run it again: it downloads
the background map once (~1–2 GB), starts everything from ready-made images, and waits for the first
radar frame.

**How to know it worked:** `up.sh` ends by printing the address for the app. You can also check
`curl localhost:8080/api/health`: `degraded` at first, then `ok` about 2 minutes later.

The full guide, with hardware sizing, the manual steps and what to do if it didn't work, is
**[self-host/compose.md](../self-host/compose.md)**; common problems are in
[troubleshooting.md](../self-host/troubleshooting.md).

## Path B: Kubernetes

Ready-to-apply manifests live in **[deploy/kubernetes/](../../deploy/kubernetes/README.md)**. To
try them on a laptop cluster (kind, k3d, k3s):

```bash
kubectl apply -k deploy/kubernetes/examples/minimal
```

That README walks through it step by step. For your own production cluster (your Temporal, storage
and ingress), read the [Kubernetes guide](../self-host/kubernetes.md).

## Running the phone app

```bash
cd frontend
bun install
bun run ios          # builds and opens it in the iOS simulator (macOS)
bun run android      # or: Android emulator / a plugged-in Android phone
```

The first build takes a while; after that it's quick. Then point the app at your server:

- **In the app:** **Settings → Advanced Mode → Stack URL**, and enter `http://<host>:8080`
  (Compose) or your public URL (e.g. `https://radar-ng-api.example.com`). No rebuild needed.
- **As the default for every install:** set `EXPO_PUBLIC_RADAR_DEFAULT_URL` and rebuild
  (see [FORKING.md](../../FORKING.md)).

Running on a real phone, simulator tips, CarPlay and Apple Watch:
[app/running-the-app.md](../app/running-the-app.md) and
[app/carplay-watch-setup.md](../app/carplay-watch-setup.md).

## Images

| Image | What it runs |
|---|---|
| `ghcr.io/mitchross/radar-ng-tile-server` | `radar api`: serves tiles, the basemap, `/api/*` and `/v1/*` |
| `ghcr.io/mitchross/radar-ng-temporal-worker` | `radar worker`: downloads and renders radar, forecasts, lightning, nowcast and alerts; cleans up old frames. `WORKER_ROLE` picks which jobs |
| `ghcr.io/mitchross/radar-ng-open-meteo-worker` | the Open-Meteo forecast server's image plus `radar worker --role open-meteo` (keeps its forecast data up to date) |

All three run the same Rust program, built from `deploy/Dockerfile`. Tags are `vX.Y.Z` plus
`latest`; pin a `vX.Y.Z` in production. Build details are in [deploy/README.md](../../deploy/README.md).

You'll also run some third-party images: `temporalio/auto-setup` (or your own Temporal),
`postgres:16` (Temporal's database), `ghcr.io/open-meteo/open-meteo`, and `protomaps/go-pmtiles`
for the one-time basemap download. Map data is downloaded when you set up, not baked into an image.

## Where to go next

Everything else is indexed in **[docs/README.md](../README.md)**.
