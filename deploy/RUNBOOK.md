# radar-ng Rust runbook

How the Rust build reaches production and how to roll it back. Everything goes through Git: images
are built by GitHub Actions, and the cluster changes only by commits to `mitchross/talos-argocd-proxmox`
that Argo CD syncs. No `kubectl apply`, no hand-edited cluster state. `docs/rust-migration/PLAN.md`
has the strategy.

## 1. What ships

The Rust images are drop-ins for the GHCR images the cluster already runs, under the same names:

| GHCR image | `deploy/Dockerfile` target | Runs in (talos-argocd-proxmox `my-apps/personal-projects/radar-ng/`) |
|---|---|---|
| `radar-ng-temporal-worker` | `worker` | the five WorkerDeployments `radar-ng-worker-{mrms,nowcast,hrrr,aux,alerts}` (`temporal-workers/worker-pools.yaml`) |
| `radar-ng-tile-server` | `api` | `deployment-tile-server.yaml` |
| `radar-ng-open-meteo-worker` | `open-meteo` | `deployment-open-meteo.yaml` (serve container + `sync-worker` sidecar) |

Nothing else in the GitOps repo changes:
- Env names and defaults are the same. Every variable the manifests set is read by Rust, except
  `MRMS_RENDER_WORKERS` (meaningless in Rust) and `WATCH_RADIUS_KM` (20, equal to the Rust constant).
- Same uids (1000; 999 for open-meteo), ports (8080; worker metrics 9464), probe files
  (`/tmp/worker-alive`, `/tmp/temporal-healthy`). The worker image carries busybox `sh`, `find`, `test`
  for the `sh -c "test $(find ...)"` exec probes.
- The api proxies `/basemap/tiles/*` to the existing go-pmtiles `basemap` Service when no local
  archive is mounted (`BASEMAP_TILES_UPSTREAM`, default `http://basemap:8081`), as Caddy did.
- The aux pool keeps seeding Schedules (`SEED_SCHEDULES=1`); the Rust seeder writes the same
  definitions with sorted-key args, so the Schedules don't churn.

## 2. Release path

1. Merge the change to `radar-ng` `master`. `ci-rust.yml` must be green first: tests, clippy,
   the offline replay gate, Temporal integration tests, the real-NOAA comparison against production
   Python, and all three image builds with smoke runs.
2. `ghcr-temporal-worker.yml`, `ghcr-tile-server.yml`, `ghcr-open-meteo-worker.yml` push the next
   semver (`:vX.Y.Z`, `:sha-…`, `:latest`). The worker build runs the replay gate again inside the image build.
3. Renovate opens the tag bump in `talos-argocd-proxmox` (or open it by hand: change the `image:`
   tag and `@sha256` digest on the lines listed in section 1). Cluster CI validates it.
4. Merge it. Argo CD syncs `my-apps-radar-ng`.
   - Each worker pool runs `RadarDeploymentSmokeWorkflow` on the new build before the Temporal
     Worker Controller promotes it (`rollout.gate`). A pool whose smoke fails stays on the old build.
   - `tile-server` is a `Recreate` Deployment: a short gap in tile serving while the pod swaps.

To go slower, bump one image per GitOps PR (workers first, then tile-server, then open-meteo), or edit
the Renovate PR so one pool moves first (`aux`, then `mrms`).

Old workflow executions don't need to be kept. Executions pinned to the Python build either finish on
the old pods during the controller's sunset (`scaledownDelay: 10m`) or are dropped with them; the next
Schedule fire starts fresh on Rust. Long-lived `WatchStormWorkflow` runs are disabled in production
(`DISABLE_WORKFLOW_ROUTES=1`), so nothing user-facing is lost.

## 3. Check after each bump

From any machine (the debug checks are HTTP and Temporal client calls, read-only):

```bash
radar debug doctor --server https://radar.vanillax.me --temporal-address <temporal-frontend>:7233 --namespace radar-ng
radar debug doctor --server https://radar.vanillax.me --skip-temporal   # HTTP checks only
```

or the Grafana radar dashboard: MRMS frame age under 10 minutes, nowcast and HRRR publishing,
`/api/health` 200, no pod restarts, Argo app Healthy.

## 4. Rollback

Revert the bump commit in `talos-argocd-proxmox` and let Argo sync. The Python images stay in GHCR
(`radar-ng-temporal-worker:v1.1.50`, `radar-ng-tile-server:v1.1.27`,
`radar-ng-open-meteo-worker:v1.1.11`). If Renovate would re-propose the Rust tag, close its PR or add
an `ignoreVersions` rule for radar-ng in the Renovate config (also a GitOps commit).

## 5. Tile parity

`radar parity` decodes every PNG of two tile roots and compares them. Both
roots use the `TILE_DIR` layout (`<root>/<layer>/<palette>/<frame path>/<z>/<x>/<y>.png`), and
every `(layer, palette, frame)` present on both sides is compared. It runs offline (no server,
no Temporal) and decodes on all cores.

```bash
# Against Python production (legacy RGBA tiles): default threshold, 10% differing pixels per frame
radar parity --python-root /mnt/py-tiles --rust-root /mnt/rs-tiles
# Against Python's indexed renderer: everything must match
radar parity --python-root /mnt/py-indexed --rust-root /mnt/rs-tiles --exact
# One layer and one frame (Z or +00:00), zooms 4 to 7, with manifests, as JSON
radar parity --python-root A --rust-root B --layer radar --frame 2026-10-14T06:00:00Z \
  --min-zoom 4 --max-zoom 7 --python-manifest /mnt/py-state --rust-manifest /mnt/rs-state --json
```

What is compared, per frame:

- Tile sets: tiles on one side only. A missing tile is compared against a fully transparent tile,
  which is what a client gets for a 404.
- Every tile decoded to RGBA, whether it is indexed or RGBA: identical tiles, percentage of
  identical pixels, largest channel delta, and the kind of each differing pixel (`alpha_edge`:
  transparent on one side only; `alpha_only`; `color`; `transparent_rgb_only`, which is invisible
  and not counted).
- Palette indices, when both tiles are indexed.
- Manifests, when both are given (or `<root>/manifest.json` exists): frame lists, paths, palettes,
  and frame and layer keys. Frame-list differences are timing and are reported only.

Reading the result:

- **Python legacy RGBA vs Rust (production today).** Expect about 92% identical pixels (bench:
  92.1%; `radar parity` on the bench pyramid: 92.9% per palette) and a few extra Python tiles at
  the edges (648 vs 585 on the bench). Most differing pixels are colour steps at class boundaries
  and coverage edges, because legacy samples colour before classifying. The default
  `--max-diff-pct 10` passes this. A frame over 10% is `✗`, and a frame within it is `⚠` with
  its numbers. The 10% was calibrated on the synthetic bench field (7.1% to 7.2% per palette).
  Real MRMS frames have more class boundaries, so record the per-frame numbers of the first real
  run (`--json`) and adjust `--max-diff-pct` if frames sit close to 10%. The threshold is a share
  of all tile pixels, so a frame that is mostly transparent can hide a local defect: read the
  `max Δ` and `differing_tiles` (in `--json`) too.
- **Python indexed vs Rust.** Use `--exact`. Same tile sets, same palette indices, same RGBA, and
  matching manifest entries. Any difference fails.
- Exit status: 0 pass (warnings allowed), 1 a frame failed, 2 no frame present on both sides, or a
  root could not be read.

For a one-frame spot check, `radar render-once <grib> <out>` writes `<out>/<palette>/bench/...`.
Put it and the Python render of the same GRIB (`bench/README.md`) under
`<root>/<layer>/<palette>/<frame>/` with the same frame name (symlinks are followed), then run
`radar parity` on the two roots.

## 6. Build checks

CI runs these on every PR (`ci-rust.yml`, job `images`): the glibc floor (the open-meteo target puts a
bookworm-built binary on the Open-Meteo base, so `radar --version` must run there), image users
(`1000:1000`, `999:999`), the api smoke run, and the Open-Meteo image contract. Locally:
`rust/scripts/preflight.sh --only docker`.

`deploy/pin-images.sh` pins every base image in `deploy/` to its current digest; commit the diff.

## 7. Local stack

`deploy/docker-compose.yml` runs the whole thing on one machine (Temporal, workers, api,
open-meteo). It is for development and self-hosting, not the cluster.

## 8. Decommissioning Python

After the Rust images have run in production for a while: delete `backend/api/` (Python api, Caddyfile,
start.sh), the Python worker code and Dockerfiles under `temporal/` and `backend/`, `ci-backend.yml`'s
Python jobs, the Gitea `build-*.yml` workflows, and `deploy/k8s` + `deploy/docker-compose.yml`. Keep
`rust/assets/basemap/styles/` and `fetch-glyphs.sh` (the Rust image copies them) and
`temporal/workflows/replay_fixtures/` only as long as the replay gate is wanted. In the GitOps repo,
`deployment-basemap.yaml` can go once the api mounts the `pmtiles` volume and sets `BASEMAP_PMTILES`.
