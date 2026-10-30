# radar-ng runbook

How a release reaches production and how to check it. Images are built by GitHub Actions; the cluster
changes only through PRs to `mitchross/talos-argocd-proxmox` that Argo CD syncs. No `kubectl apply`, no
hand-edited cluster state.

Releases are forward-only. A bad release is fixed by a new release: merge the fix, let a new
`vX.Y.Z` publish, bump the GitOps tag. The old Python images are not a fallback.

## 1. What ships

| GHCR image | `deploy/Dockerfile` target | Runs in (talos-argocd-proxmox `my-apps/personal-projects/radar-ng/`) |
|---|---|---|
| `radar-ng-temporal-worker` | `worker` | the five WorkerDeployments `radar-ng-worker-{mrms,nowcast,hrrr,aux,alerts}` (`temporal-workers/worker-pools.yaml`) |
| `radar-ng-tile-server` | `api` | `deployment-tile-server.yaml` |
| `radar-ng-open-meteo-worker` | `open-meteo` | `deployment-open-meteo.yaml` (serve container and `sync-worker` sidecar) |

All three run the same `radar` binary. Ports: 8080 (api), 9464 (worker metrics). Users: 1000, and 999
for open-meteo. Worker probes read `/tmp/worker-alive` and `/tmp/temporal-healthy`; the image carries
busybox `sh`, `find` and `test` for the exec probes.

The api serves `/basemap/tiles/*` from a local archive when `BASEMAP_PMTILES` exists, and otherwise
proxies to `BASEMAP_TILES_UPSTREAM` (default `http://basemap:8081`). The reference manifests in
`kubernetes/` set that upstream and run a go-pmtiles `basemap` Deployment; a cluster whose api pod
has the archive at `BASEMAP_PMTILES` serves it locally and never uses the proxy.

## 2. Release path

1. Merge to `radar-ng` `master` with `ci-rust.yml` green: fmt, clippy, tests (including the offline
   replay gate), Temporal integration tests, the live-NOAA check, and all three image builds with their
   smoke runs.
2. `ghcr-backend.yml` compiles the `radar` binary once and pushes all three images at the next
   semver (`:vX.Y.Z`, never below v2.0.0), `:sha-…` and `:latest`. Each image's run prints the
   published `tag@digest` in its summary. The replay gate runs once, before the binary is built.
3. Bump the `image:` tag and `@sha256` digest in `talos-argocd-proxmox` by PR, on every line listed in
   section 1 (Renovate may open it). Cluster CI validates it.
4. Merge it. Argo CD syncs `my-apps-radar-ng`.
   - Each worker pool runs `RadarDeploymentSmokeWorkflow` on the new build before the Temporal Worker
     Controller promotes it (`rollout.gate`). A pool whose smoke fails stays on its current build until
     a fixed release replaces the candidate.
   - `tile-server` is a `Recreate` Deployment: tile serving pauses briefly while the pod swaps.

## 3. Checks after each bump

The debug checks are read-only HTTP and Temporal client calls and run from any machine:

```bash
radar debug doctor --server https://radar.vanillax.me --temporal-address <temporal-frontend>:7233 --namespace radar-ng
# without Temporal access
radar debug doctor --server https://radar.vanillax.me --skip-temporal
```

Or the Grafana radar dashboard: MRMS frame age under 10 minutes, nowcast and HRRR publishing,
`/api/health` 200, no pod restarts, Argo app Healthy. [docs/reference/debug-harness.md](../docs/reference/debug-harness.md)
explains each check.

## 4. Temporal notes

- Workflows are `PINNED`: a run stays on the build that started it. Old executions are disposable. Runs
  pinned to a retired build either finish during the controller's `scaledownDelay` (10 min) or may be
  terminated; the next Schedule fire starts fresh on the current build.
- With `OverlapPolicy.SKIP`, one orphaned pinned run blocks every later fire of its Schedule. The
  GitOps repo's `my-apps/personal-projects/radar-ng/RUNBOOK.md` covers finding and clearing orphaned
  pinned executions and controller-ownership errors.
- Long-lived `WatchStormWorkflow` runs are off in production (`DISABLE_WORKFLOW_ROUTES=1`).
- Exactly one pool seeds Schedules (`aux`, `SEED_SCHEDULES=1`). The seeder writes canonical JSON with
  sorted keys, so re-seeding does not churn unchanged Schedules.

## 5. Tile parity

`radar parity` decodes every PNG of two tile roots and compares them offline (no server, no Temporal),
on all cores. Use it to compare two builds' output for the same frames, for example before and after a
renderer change. The flags `--python-root` and `--rust-root` are historical names for the reference and
the candidate root. Both roots use the `TILE_DIR` layout
(`<root>/<layer>/<palette>/<frame path>/<z>/<x>/<y>.png`).

```bash
# Same input, same renderer on both sides: everything must match
radar parity --python-root /mnt/ref-tiles --rust-root /mnt/new-tiles --exact
# One layer and frame, zooms 4 to 7, with manifests, as JSON
radar parity --python-root A --rust-root B --layer radar --frame 2026-10-14T06:00:00Z \
  --min-zoom 4 --max-zoom 7 --python-manifest /mnt/ref-state --rust-manifest /mnt/new-state --json
```

Per frame it reports tiles on one side only (compared against a transparent tile, which is what a
client gets for a 404), identical tiles and pixels, the largest channel delta, the kind of each
differing pixel, palette indices when both tiles are indexed, and manifest differences when both
manifests are given. Without `--exact` a frame fails when more than `--max-diff-pct` (default 10) of
its pixels differ visibly. Exit status: 0 pass (warnings allowed), 1 a frame failed, 2 nothing to
compare or a root could not be read.

For a one-frame spot check, `radar render-once <mrms.grib2.gz> <out>` renders one MRMS frame into
`<out>/<palette>/bench/...` and prints timings.

## 6. Build checks and base images

CI (`ci-rust.yml`, job `images`) checks on every PR: the glibc floor (the open-meteo target puts a
bookworm-built binary on the Open-Meteo base, so `radar --version` must run there), image users
(`1000:1000`, `999:999`), the replay-gate label, the worker probe helpers, an api smoke run on empty
data, and the Open-Meteo image contract.

Third-party base images in `deploy/` are pinned as `name:tag@sha256:<index digest>`. Renovate
(`.github/renovate.json5`) bumps the tag and digest together.

## 7. Local stack

`deploy/docker-compose.yml` runs the whole pipeline on one machine (Temporal, workers, api,
open-meteo). It is for development and self-hosting, not the cluster. See [README.md](README.md).
