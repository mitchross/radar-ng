# radar-ng

> Your own weather radar app, running on your own hardware, using only free public weather data.

[![expo](https://img.shields.io/badge/expo-SDK%2058-000)](https://expo.dev) [![rust](https://img.shields.io/badge/rust-1.97-000)](https://www.rust-lang.org/) [![temporal](https://img.shields.io/badge/orchestration-temporal-7c53ff)](https://temporal.io) [![k8s](https://img.shields.io/badge/deploy-talos%20%2B%20argocd-326CE5)](https://talos.dev)

<p align="center">
  <img src="docs/screenshots/home.png" width="19%" alt="Home" />
  <img src="docs/screenshots/nowcast.png" width="19%" alt="Nowcast" />
  <img src="docs/screenshots/radar.png" width="19%" alt="Radar" />
  <img src="docs/screenshots/alerts.png" width="19%" alt="Alerts" />
  <img src="docs/screenshots/settings.png" width="19%" alt="Settings" />
</p>

radar-ng is a weather radar app for your phone and browser, like the big weather apps, but the
data comes from a server **you** run. That server downloads radar, forecasts, alerts and lightning
straight from public sources such as NOAA, turns them into map images, and serves them to the app.
There are no API keys, no accounts and no ads, and analytics stay off unless you turn them on.

## What you get

- **Live radar** across the US, refreshed about every 2 minutes.
- **The next hour of rain**, predicted from how the radar is moving (a "nowcast"). It checks itself
  against what actually happened and shows you how accurate it's been.
- **Forecasts**, hourly and 7-day, plus **air quality**, **lightning**, **severe weather alerts**
  and **hurricanes**.
- **"Rain starting soon" notifications**, planned on the phone itself: no push service, no account.
- **An iPhone/Android app** (with Apple Watch and CarPlay extras) and **a web version**.

New to the weather terms (MRMS, HRRR, nowcast, tiles…)? See the
[glossary](docs/start/glossary.md).

## Pick your path

| I want to… | Start here |
|---|---|
| Run the server on one computer (Docker Compose) | [docs/self-host/compose.md](docs/self-host/compose.md) |
| Run the server on my Kubernetes cluster | [deploy/kubernetes/README.md](deploy/kubernetes/README.md) |
| Run the phone app against my server | [docs/app/running-the-app.md](docs/app/running-the-app.md) |
| Run the web radar against my server | [docs/app/web-radar.md](docs/app/web-radar.md) |
| Change the code | [CONTRIBUTING.md](CONTRIBUTING.md) |
| Fork it and ship my own app | [FORKING.md](FORKING.md) |
| Not sure yet | [docs/start/getting-started.md](docs/start/getting-started.md) |

Every other doc (configuration, tuning, releasing, debugging, architecture, specs) is listed in
**[docs/README.md](docs/README.md)**.

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

The first radar frame shows up about 2 minutes later. Point the app at `http://<host>:8080`
(**Settings → Advanced Mode → Stack URL**). The full walk-through, with how to check it worked and
what to do if it didn't, is [docs/self-host/compose.md](docs/self-host/compose.md).

## How it works

<img src="docs/diagrams/system-architecture.svg" width="100%">

One Rust program, `radar`, does all the server work. Its **workers** download each new radar scan
and forecast run, render them into map tiles and write them to disk. Its **API** serves those
tiles, the basemap and a small JSON API to the app on one port. [Temporal](https://temporal.io)
schedules and retries every step (no cron).

The nowcast extrapolates the latest MRMS radar scans along their motion field (Lucas-Kanade
optical flow plus S-PROG extrapolation) for the next 60 minutes, then scores itself against the
radar that arrives later.

The component-by-component breakdown, the per-frame pipeline and the caching story are in
**[docs/reference/architecture.md](docs/reference/architecture.md)**; the backend code is in
[`rust/`](rust/README.md).

## Data sources

All free and public, no keys: **NOAA MRMS** (radar) · **NOAA HRRR** + **Open-Meteo**
(forecast, wind, precipitation) · **NOAA NAQFC/AQM** (air quality: PM2.5 + ozone,
current + 3-day forecast) · **NWS** (alerts) · **NHC** (tropical) ·
**Blitzortung** (lightning) · **Protomaps** (basemap).

Prebuilt server images: `ghcr.io/mitchross/radar-ng-{tile-server,temporal-worker,open-meteo-worker}`.

## Stack

Expo SDK 58 / TypeScript · MapLibre · Skia · Rust (tokio · axum · Temporal Rust
SDK · pure-Rust GRIB2) · Temporal · Open-Meteo · Protomaps · Kubernetes
(Talos + ArgoCD) · OpenTelemetry.

## License

See [LICENSE](LICENSE).
