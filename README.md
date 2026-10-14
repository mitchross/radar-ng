# radar-ng

> Self-hosted weather radar on a Rust + Temporal tile pipeline. Pure public data, no aggregators, and no API keys.

[![expo](https://img.shields.io/badge/expo-SDK%2058-000)](https://expo.dev) [![rust](https://img.shields.io/badge/rust-1.97-000)](https://www.rust-lang.org/) [![temporal](https://img.shields.io/badge/orchestration-temporal-7c53ff)](https://temporal.io) [![k8s](https://img.shields.io/badge/deploy-talos%20%2B%20argocd-326CE5)](https://talos.dev)

<p align="center">
  <img src="docs/screenshots/home.png" width="19%" alt="Home" />
  <img src="docs/screenshots/nowcast.png" width="19%" alt="Nowcast" />
  <img src="docs/screenshots/radar.png" width="19%" alt="Radar" />
  <img src="docs/screenshots/alerts.png" width="19%" alt="Alerts" />
  <img src="docs/screenshots/settings.png" width="19%" alt="Settings" />
</p>

radar-ng pulls radar (NOAA MRMS), forecast (HRRR + Open-Meteo), air quality
(NOAA NAQFC), alerts (NWS), lightning (Blitzortung), and tropical (NHC)
straight from the public sources,
renders map tiles on hardware you own, and serves them to a universal Expo app.
The phone is the windshield; a Kubernetes + Temporal pipeline is the engine.
The next hour is a real radar nowcast (Lucas-Kanade motion and S-PROG extrapolation of MRMS) that
scores itself against the radar that arrives later and shows you its hit rate;
"rain starting soon" alerts are local notifications planned on the phone, with no
push service and no account.

## Quick start (Docker Compose)

```bash
cd deploy
cp .env.example .env
# set two values in .env:
#   NWS_USER_AGENT=(radar-ng, you@example.com)
#   BASEMAP_PMTILES_URL=   # newest date from https://build.protomaps.com/
docker compose run --rm basemap-bootstrap   # one-time basemap fetch
docker compose run --rm basemap-publish     # move it into place
docker compose build api                    # the radar image; `worker` runs it without building
docker compose up -d --build
```

First radar frame lands ~2 min later. Point the app at `http://<host>:8080`
(Settings → Advanced Mode → Stack URL). Deploying to your own Kubernetes cluster and running the
app are covered in **[GETTING_STARTED.md](GETTING_STARTED.md)**.

Prebuilt images: `ghcr.io/mitchross/radar-ng-{tile-server,temporal-worker,open-meteo-worker}`.

## Local iOS/watch IPA

To build a signed standalone iPhone and iPad IPA with the watch app embedded, without
Metro or EAS:

```bash
cd frontend
bun run ios:ipa
```

The output lands at:

```text
artifacts/ios/export-debugging/radarng.ipa
```

This is a `Release` Xcode archive/export. It embeds the Hermes `main.jsbundle`
and `Payload/radarng.app/Watch/radar-ngWatch.app`, so the installed app does
not need a Metro server.

The default export method is `debugging`, which is standalone but still
development-signed with the Apple account configured in Xcode. For a true
distribution-style install path, use:

```bash
cd frontend
bun run ios:ipa:release-testing   # ad-hoc / release testing
bun run ios:ipa:app-store         # App Store Connect / TestFlight
```

`release-testing` requires the latest Apple Developer Program License Agreement
to be accepted, an installed Apple Distribution certificate with private key, and
provisioning profiles for both `com.vanillax.radar-ng` and
`com.vanillax.radar-ng.watch`.

## How it works

<img src="docs/diagrams/system-architecture.svg" width="100%">

Every ingest step is a Temporal activity, scheduled by Temporal Schedules (no
cron). One Rust binary, `radar`, runs the workers and the tile-server: static
tiles off disk with immutable cache headers, the basemap, and the JSON API on
one port. The component-by-component breakdown, the per-frame pipeline, and the
caching story are in **[ARCHITECTURE.md](ARCHITECTURE.md)**; the backend code is
in [`rust/`](rust/README.md).

## Docs

| Doc | For |
|---|---|
| [GETTING_STARTED.md](GETTING_STARTED.md) | Run the stack (Compose or k8s) + the app |
| [docs/configuration.md](docs/configuration.md) | Every env var + the basemap-date gotcha |
| [docs/self-hosting.md](docs/self-hosting.md) | Docker Compose golden path |
| [docs/kubernetes.md](docs/kubernetes.md) | Bring-your-own-cluster (Temporal, storage, probes) |
| [docs/releasing.md](docs/releasing.md) | Code to running pods: GHCR, GitOps, verification |
| [docs/temporal-safe-deployments.md](docs/temporal-safe-deployments.md) | Worker handoff, functional gates, pinned executions |
| [docs/tuning.md](docs/tuning.md) | Faster / cheaper / fresher, knob by knob |
| [docs/debug-harness.md](docs/debug-harness.md) | `radar debug`: freshness, latency, Temporal, disk |
| [ARCHITECTURE.md](ARCHITECTURE.md) | How it all fits together |
| [rust/README.md](rust/README.md) | Backend crates, build, tests, CI |
| [deploy/README.md](deploy/README.md) | Images, compose stack, release runbook |
| [CONTRIBUTING.md](CONTRIBUTING.md) | Dev setup, verification, conventions |

## Data sources

All free, public, no keys: **NOAA MRMS** (radar) · **NOAA HRRR** + **Open-Meteo**
(forecast, wind, precipitation) · **NOAA NAQFC/AQM** (air quality: PM2.5 + ozone,
current + 3-day forecast) · **NWS** (alerts) · **NHC** (tropical) ·
**Blitzortung** (lightning) · **Protomaps** (basemap).

## Stack

Expo SDK 58 / TypeScript · MapLibre · Skia · Rust (tokio · axum · Temporal Rust
SDK · pure-Rust GRIB2) · Temporal · Open-Meteo · Protomaps · Kubernetes
(Talos + ArgoCD) · OpenTelemetry.

## License

See [LICENSE](LICENSE).
