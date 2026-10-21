# Troubleshooting

Problems people hit when running radar-ng, and how to fix them. Find your symptom, check the cause,
apply the fix. Commands are run from `deploy/` (Docker Compose) unless they say `kubectl` (Kubernetes,
namespace `radar-ng`).

A full health sweep that names most of these problems for you:
`docker compose exec worker radar debug doctor --server http://api:8080`
(see [debug-harness.md](../reference/debug-harness.md)).

## `up.sh` stops before starting anything

| It says | Fix |
|---|---|
| "Docker isn't installed" / "not running" | Install Docker Engine 24+ (Linux) or Docker Desktop / OrbStack (Mac) and start it. On Linux, add yourself to the `docker` group (`sudo usermod -aG docker $USER`, then log out and back in) or run with `sudo`. |
| "The 'docker compose' plugin (Compose v2) is missing" | Install the Compose plugin (`docker-compose-plugin` on Debian/Ubuntu, `docker-compose` on Arch). The old `docker-compose` Python tool won't do. |
| "Set your contact email in deploy/.env" | Open `deploy/.env` and set `NWS_USER_AGENT=(radar-ng, you@yourmail)`. |
| "This is an arm64 machine…" | Run `./up.sh --build`; see [arm64](#apple-silicon-raspberry-pi-and-other-arm64-machines). |

## Starting fails

**`port is already allocated` / `address already in use`.** Something else is using port 8080 (the
API), 7233 (Temporal) or 8233 (Temporal UI). Pick other host ports in `deploy/.env`
(`TILE_SERVER_PORT=8090`, `TEMPORAL_PORT=7234`, `TEMPORAL_UI_PORT=8234`) and run again. To see what's
on a port: `sudo ss -ltnp | grep 8080` (Linux) or `lsof -i :8080` (Mac).

**`pull access denied` / `manifest unknown` / `no matching manifest for linux/arm64`.** The image name or
platform is wrong. If you set `RADAR_*_IMAGE` in `.env`, check the tag exists. On arm64, use
`./up.sh --build`.

**`temporal` stays `unhealthy` / the worker waits forever.** Temporal creates its database on the first
start, which takes a minute or two. Check `docker compose ps` and `docker compose logs temporal`. If
`temporal-postgres` keeps restarting, a half-initialised database volume from an earlier attempt is the
usual cause. Starting over (`docker compose down -v`, which **deletes all radar-ng data**) fixes it.

**`volume-init` fails.** It sets file ownership on the data volumes. Rootless Docker or Podman can refuse
that. Use rootful Docker, or check `docker compose logs volume-init`.

## Blank / gray basemap

The map file is missing, or doesn't cover where you're looking.

```bash
docker compose run --rm basemap-bootstrap        # safe to re-run; prints what it did
curl -sI http://localhost:8080/basemap/tiles/4/4/6.mvt   # expect 200
```

- **"no Protomaps build found in the last 14 days"**: the machine can't reach `build.protomaps.com`
  (proxy, firewall, DNS), or Protomaps is down. Try `curl -sI https://build.protomaps.com/` from the
  host.
- **HTTP 404 during the extract**: you pinned `BASEMAP_PMTILES_URL` to a build that expired (they
  last about 6 days). Empty it in `.env` so the newest build is used.
- **sha256 mismatch** when downloading `go-pmtiles`: the download was corrupted or intercepted. Retry;
  if it persists, check for a proxy rewriting GitHub downloads.
- **You're looking outside the US**: the default `BASEMAP_BBOX` is the lower 48 states. Widen it in
  `.env` and re-run the bootstrap (it notices the change and downloads again).

## `/api/health` stays `degraded` (no radar)

`degraded` (HTTP 503) means the newest radar frame is older than `MRMS_MAX_AGE_S` (10 minutes). In
order of likelihood:

1. **It just started.** The first frame takes about 2 minutes. Wait 5.
2. **The worker isn't running or is stuck**: `docker compose ps worker`, then
   `docker compose logs --tail 100 worker`. A restart (`docker compose restart worker`) re-seeds the
   schedules and catches up.
3. **No internet from the containers**: errors mentioning `noaa-mrms-pds` / S3 / DNS in the worker log.
   Check proxies and DNS for Docker.
4. **The machine is too slow** to render each frame before the next arrives (every 2 minutes). See
   [tuning.md](tuning.md): render fewer palettes (`PALETTES=classic`).
5. **NOAA is late.** It happens. Nothing to do but wait; the app shows a "data delayed" banner.

## No forecast / temperatures show 0°

The forecast server (`open-meteo`) is empty until its first data sync. It runs within the hour on
its own; to start one now:

```bash
docker compose exec temporal temporal schedule trigger --schedule-id open-meteo-sync-gfs
```

Then `curl http://localhost:8080/api/forecast/40.0/-83.0` should return numbers.

## A layer is missing

Each layer appears only after its first successful run. Radar is first (~2 min), nowcast needs a few
radar frames (~10 min), HRRR and air quality fill in over 15–60 minutes. If one never appears, the
worker log names the failing job; `radar debug doctor` shows which schedule is failing and why.

## The phone app can't connect

- Use the right address: `http://localhost:8080` only works in the iOS simulator on the same machine;
  the Android emulator needs `http://10.0.2.2:8080`; a real phone needs the computer's LAN IP
  (`http://192.168.x.y:8080`, printed by `up.sh`).
- The computer's firewall must allow port 8080 from your Wi-Fi.
- iOS can refuse plain `http://` to a domain name (App Transport Security). For anything beyond your
  home network, put the server behind HTTPS (see "Reach it from outside your home network" in
  [compose.md](compose.md)).

## Apple Silicon, Raspberry Pi and other arm64 machines

The published images are linux/amd64 only, so `./up.sh` stops on arm64 and `./up.sh --build` builds
the three radar images on your machine from `deploy/Dockerfile`.

- The first build compiles the whole Rust workspace, which needs a lot of memory and several GB of disk.
  If the build is killed (`exit code 137`, "killed"), raise Docker's memory limit (Docker Desktop →
  Settings → Resources) and run again. Later builds are cached.
- Third-party images (Temporal, Postgres, Open-Meteo, Alpine) publish arm64 versions.
- arm64 builds aren't tested in CI yet. If one fails, please open an issue with the build log.

## Disk fills up

Steady state is about 5 GB of tiles and grids (old frames are cleaned up hourly), plus a few GB for
forecast data and 1–2 GB for the map. `docker system df -v` shows volume sizes. To shrink: fewer
palettes, a smaller `BASEMAP_BBOX`, or shorter retention ([tuning.md](tuning.md#tile-retention--cleanup)).

## Kubernetes

The Kubernetes manifests have their own "If it didn't work" table in
[deploy/kubernetes/README.md](../../deploy/kubernetes/README.md). The radar, forecast and basemap
sections above apply there too: use `kubectl -n radar-ng logs deploy/radar-ng-worker` instead of
`docker compose logs worker`.

## Start over

```bash
docker compose down -v     # stops everything and DELETES all radar-ng data (tiles, map, Temporal)
./up.sh
```
