# radar-ng Rust runbook

How the Rust build reaches production, and how to roll back. `docs/rust-migration/PLAN.md` has the
strategy.

## 0. Production cutover (what actually ships)

Production manifests live in `mitchross/talos-argocd-proxmox`
(`my-apps/personal-projects/radar-ng/`). The Rust images are **drop-ins for the existing GHCR
image names**, so the cutover is an image-tag bump in that repo, and nothing else changes:

| GHCR image | Built from | What runs it in the cluster | What changes there |
|---|---|---|---|
| `radar-ng-temporal-worker` | `deploy/rust/Dockerfile --target worker` | the five WorkerDeployments `radar-ng-worker-{mrms,nowcast,hrrr,aux,alerts}` (role from `WORKER_ROLE`, isolated queues, `AllAtOnce` behind the `RadarDeploymentSmokeWorkflow` gate) | image tag only |
| `radar-ng-tile-server` | `--target api` | `deployment-tile-server.yaml` (port 8080, `/api/livez`, `Recreate`, read-only PVCs) | image tag only |
| `radar-ng-open-meteo-worker` | `--target open-meteo` | `deployment-open-meteo.yaml`: the `open-meteo` serve container (command `/app/openmeteo-api`) and the `sync-worker` sidecar (default command) | image tag only |

Why no manifest changes are needed:
- Same env names and defaults (every variable those manifests set is read by the Rust code except
  `MRMS_RENDER_WORKERS`, which has no meaning in Rust, and `WATCH_RADIUS_KM`, which matches the
  Rust constant 20 km).
- Same uids (1000; 999 for open-meteo), ports (8080, metrics 9464), and probe files
  (`/tmp/worker-alive`, `/tmp/temporal-healthy`). The worker image carries busybox `sh`, `find` and
  `test` because the probes exec `sh -c "test $(find ... -mmin -3)"`.
- `/basemap/tiles/*` is proxied to the existing go-pmtiles `basemap` Service when no local
  `.pmtiles` archive is mounted (`BASEMAP_TILES_UPSTREAM`, default `http://basemap:8081`), exactly
  as Caddy did. Mounting the archive in the api later lets the basemap Deployment go.
- The aux pool still seeds Schedules (`SEED_SCHEDULES=1`). The Rust seeder writes the same
  definitions with sorted-key args, so it doesn't churn them.

Release path: merge to `master` -> `ghcr-*.yml` builds and pushes the next semver (the worker build
runs the offline replay gate first) -> Renovate opens the tag bump in talos-argocd-proxmox -> merge it
-> Argo CD syncs. Each worker pool runs the smoke workflow on the new build before the controller
promotes it; a pool whose smoke fails stays on Python.

To de-risk, merge the bumps one image at a time (workers, then tile-server, then open-meteo), or edit
the Renovate PR so only one pool moves first (start with `aux`, then `mrms`).

**Rollback:** revert the bump commit in talos-argocd-proxmox. The Python images
(`radar-ng-temporal-worker:v1.1.50`, `radar-ng-tile-server:v1.1.27`,
`radar-ng-open-meteo-worker:v1.1.11`) stay in GHCR. Pause Renovate for radar-ng if it would re-propose
the Rust tag.

**Check after each bump:** `radar debug doctor` (or the Grafana radar dashboard): MRMS frame age
under 10 min, nowcast and HRRR publishing, `/api/health` 200, no restarts.

The rest of this runbook (sections 1-12) was written for the single-TWD layout in `deploy/k8s/` and
for fresh installs from `deploy/rust/k8s/`; production uses section 0.

## 2. Open items that block the cutover

### Pre-cutover check (run first)

Everything that could not be run where the port was built (no NOAA, no Docker daemon, no Temporal
server) is one command. Run it from the repository checkout on a machine with network access and
Docker (CachyOS/Arch desktop: `sudo pacman -S rustup protobuf docker docker-compose docker-buildx uv`):

```bash
rust/scripts/preflight.sh               # all steps; exit 0 = every step passed
rust/scripts/preflight.sh --dry-run     # print the commands only
rust/scripts/preflight.sh --only noaa   # rerun one step (tools, test, temporal, noaa, docker)
```

| Step | What it checks | Fails when |
|---|---|---|
| 1 tools | cargo/rustc ≥ 1.92, protoc, curl, docker + compose + buildx with the daemon up, uv (or python3.12), with Arch install hints | a tool a selected step needs is missing |
| 2 test | `cargo test --workspace` in `rust/`, which includes the offline replay gate (`radar-worker --test replay`) | any test fails |
| 3 temporal | `docker compose -f deploy/temporal-dev.yml up -d postgres temporal`, waits for :7233 and namespace `default`, runs `TEMPORAL_TEST_ADDRESS=localhost:7233 cargo test -p radar-worker --test integration -- --test-threads=1`, then `down` (volumes kept; `--keep-temporal` leaves it up) | the server is not ready in 240 s, or an integration test fails |
| 4 noaa | Downloads the newest MRMS `.grib2.gz`, the latest HRRR `wrfsfcf01.grib2` + `.idx`, the 13-variable ranged subset and a NAQFC file with the `radar-grib` network code (`noaa_dump fetch`, cached in `bench/work/noaa/`), then compares production Python (pygrib, `backend.*`) with Rust (`bench/py/verify_noaa.py` against `noaa_dump dump`): MRMS values bitwise, axes ≤ 1e-9, south-up flip; HRRR `.idx` format and `IDX_MATCHERS` vs `idx_matcher` picks, name selectors vs code selectors for all 13 variables, values bitwise, Lambert lat/lon ≤ 1e-9, every message of the file decoded and hashed, negative E/D, the ranged subset vs the whole file; NAQFC grid 227 (informational). Then renders the MRMS frame with Python's indexed renderer and with `radar render-once` and runs `radar parity --exact` | any FAIL row (WARN rows are for reading) |
| 5 docker | Builds the `api` and `open-meteo` targets of `deploy/rust/Dockerfile` (tags `:preflight`, context = repo root, needs `backend/basemap/styles/`), the glibc floor (8.1, plus `radar --version` inside the open-meteo image), image users (8.2), and a smoke run of `radar api` on empty data: `/api/livez` 200 and `/api/health` 503 `no_mrms_tiles` | a build fails, glibc too old, wrong user, or a wrong status |

Read these lines in the output before going on:

- **S3 status for a missing key** (step 4, printed at the end): radar-grib README item 1. 404 on
  `noaa-hrrr-bdp-pds` and `noaa-nws-naqfc-pds` means the pending and `.idx`-fallback paths work as
  written. 403 means they do not (`GribError::is_not_found()` is 404 only), and the 403 mapping must
  be decided before the cutover.
- **`select <var>` WARN rows** (step 4): Python's name selector matched nothing where Rust's code
  selector matched a message. Production Python skips that layer and Rust renders it. That is a
  behavior change to accept or fix, not a decode error. The values are still compared bitwise.

Logs and a summary go to `bench/work/preflight/<timestamp>/`. `--noaa-cached` reuses the downloads,
`--skip-docker`, `--skip-temporal`, `--skip-noaa`, `--skip-test` and `--skip-naqfc` skip parts, and
`--help` lists the rest. Not covered: radar-grib README items 11, 13 and 14, the k6 cluster load test
(`bench/README.md`), and the cluster pre-flight in section 4.

### Open items

1. **Open-meteo API home.** The Python tile-server pod ran open-meteo as a sidecar on the RWO
   `openmeteo-data` PVC. The Rust api is a single container. Decide where `open-meteo serve`
   runs and set `OPEN_METEO_BASE` in `k8s/tile-server-deployment.yaml` to match.
2. **Shared PVCs during the ramp (blocking).** During a ramp the Python and Rust pods run at
   the same time, and both mount `tiles`, `grids`, `state`, and `openmeteo-data`. A RWO volume
   attaches to one node at a time. Either the ramp pods share a node, or the PVCs are RWX
   (Longhorn). Confirm the access mode before step 5.2. Check it with
   `kubectl -n radar-ng get pvc tiles grids state openmeteo-data basemap -o wide`.
3. **Watch-start queue.** The api starts `WatchStormWorkflow` on `TEMPORAL_ALERTS_TASK_QUEUE`.
   The Python default is `radar-ng-alerts`, and no pool polls it in the single-TWD model (the
   legacy worker polls `radar-ng`). This build sets `radar-ng` in the configmap. Confirm the
   production api's current value. The Python tile-server manifest is not in the upload. Check
   the running WatchStorm count before and after the cutover (section 4.4).
4. **Basemap PMTiles in Kubernetes.** Done: `k8s/basemap-bootstrap-job.yaml` extracts into
   `basemap.pmtiles.partial` and renames it onto the served path (F6-safe; the api reopens it within
   10 s). Run it once before the api cutover, and again to refresh the basemap. Remaining check:
   the `basemap` PVC exists and its access mode (see item 2).
5. **Parity reference.** `radar parity` (section 6) compares a Python tile root with a Rust tile
   root. During the ramp both builds publish into the one `tiles` PVC (first complete pyramid
   wins per frame), so the PVC alone is not a comparison: the two roots must be rendered
   separately from the same inputs, on a host that reaches S3 (PLAN "Verification"). Decide which
   Python renderer is the reference. Production Python writes legacy RGBA tiles, which are about
   92% pixel-identical to Rust's indexed tiles by design, and the default threshold allows for
   that. Only Python's indexed renderer is expected to match exactly (`--exact`).
6. **Image tags.** `REPLACE_ME` placeholders must be replaced by CI-built tags in the TWD
   `image:` fields. Do not hand-edit them to guessed tags.
7. **Dropped env.** `WATCH_RADIUS_KM` and `MRMS_RENDER_WORKERS` are gone from the manifests. No
   Rust crate reads them. Confirm that the storm-watch radius is a Rust code constant.
8. **Styles directory.** `backend/basemap/styles/` is missing from the uploaded tree. The Rust
   image copies it. Restore it before the first build.
9. **glibc floor and base images.** See section 8.1 (the preflight's docker step runs it).
   Run `deploy/rust/pin-images.sh` on a machine with Docker to pin every base image to its current
   digest (`name:tag@sha256:...`), then commit the diff. `--check` prints without changing files.
10. **Seeding and watchdog.** Legacy seeds Schedules by default, as Python does (the reliability
    plan keeps `SEED_SCHEDULES=1` on legacy). `radar watchdog` is the read-only stall observer,
    which runs on the seeding pool. Decide whether production runs it as its own process.

## 3. Pinned images

| Image | Pin | Where | Notes |
|---|---|---|---|
| Rust builder | `rust:1.92-bookworm` | `Dockerfile` | Matches workspace `rust-version = "1.92"` |
| Runtime | `gcr.io/distroless/cc-debian12:latest` | `Dockerfile` | Pin to a digest in CI. glibc 2.36 |
| busybox | `busybox:1.36-musl` | `Dockerfile` | Healthchecks only (`wget`, `test`, `sleep`). The open-meteo liveness probe uses it too (F2) |
| alpine | `alpine:3.22` | `Dockerfile` (`ALPINE_IMAGE`), compose `basemap-publish` | Glyph staging in `Dockerfile`; the rename step in compose |
| Open-Meteo | `ghcr.io/open-meteo/open-meteo:1.6.0@sha256:e1517a01…9dd9da` | compose, `Dockerfile` (open-meteo target) | Same digest everywhere. Bump together |
| Temporal server | `temporalio/auto-setup:1.25.2` | compose only | Production uses the cluster's server |
| Temporal UI | `temporalio/ui:2.31.2` | compose (`ui` profile) | |
| Postgres | `postgres:16-alpine` | compose | |
| go-pmtiles | `protomaps/go-pmtiles:latest` | compose (`bootstrap` profile) | Used only for `pmtiles extract`. Pin before prod |

## 4. Pre-flight

Run these before applying the Rust build. Each one must pass.

**4.1 Controller and Worker Deployments.** The production TWDs already run under the controller,
so the server and controller support Worker Deployments. Confirm the current state:

```bash
kubectl -n radar-ng get temporalworkerdeployment radar-ng-worker radar-ng-open-meteo-worker -o yaml | grep -A8 status
temporal worker deployment describe --namespace default --name radar-ng-worker
```

Record the current build ID. It is the rollback target.

**4.2 Baseline.** Run the debug harness against the Python stack and keep the output as the
reference for the ramp:

```bash
export RADAR_DEBUG_SERVER=http://tile-server.radar-ng.svc.cluster.local:8080
export TEMPORAL_ADDRESS=temporal-frontend.temporal.svc.cluster.local:7233
radar debug doctor --json > baseline-doctor.json
radar debug pipeline > baseline-pipeline.txt
radar debug temporal > baseline-temporal.txt
```

Exit status is 0 when no check fails (warnings are allowed). A baseline with failures must be
understood before the cutover, not explained away during it.

**4.3 Images.** CI has built and pushed the Rust build tags, and the glibc check passed (section 8).

**4.4 Watch-start queue and WatchStorm.** Count running watches, and confirm the api queue
value (open item 3):

```bash
kubectl -n radar-ng get cm radar-ng-temporal-config -o jsonpath='{.data.TEMPORAL_ALERTS_TASK_QUEUE}'
# The Python api's live value is set on its own pod; compare it with the line above:
kubectl -n radar-ng get pods -l app=tile-server -o jsonpath='{.items[0].spec.containers[0].env}'
temporal workflow list --namespace default --query 'WorkflowType="WatchStormWorkflow" AND ExecutionStatus="Running"' | wc -l
```

**4.5 Storage.** Open item 2 is confirmed (RWX, or old and new pods on one node).

**4.6 Smoke on the new build, before any traffic.** `RadarDeploymentSmokeWorkflow` runs the
release gate activity (`radar_deployment_smoke`, which reads `WORKER_ROLE`). Run it pinned to the
new build, so it does not depend on the ramp. The flag names below are from the Temporal CLI as
known at the time of writing. Verify them with `temporal workflow start --help` before use:

```bash
temporal workflow execute --namespace default \
  --type RadarDeploymentSmokeWorkflow \
  --task-queue radar-ng \
  --workflow-id smoke-rust-$(date +%s) \
  --versioning-override-behavior pinned \
  --versioning-override-deployment-name radar-ng-worker \
  --versioning-override-build-id <new build id>
```

Expected result: `{"ok": true, "role": "legacy"}`. If the override flags are not available in the
installed CLI, skip this step. The smoke then happens at the first ramp step (5%), and the step-5
checks cover it.

## 5. Cutover (legacy worker, then open-meteo)

The Progressive rollout in the manifests starts the ramp when the manifest is applied. Steps:
5% for 30 minutes, 25% for 1 hour, 50% for 2 hours, then 100%.

**5.1 Apply the configmap.** It is read at pod start, so applying it alone does not change the
Python pods.

```bash
kubectl -n radar-ng apply -f k8s/configmap-temporal-config.yaml
```

**5.2 Apply the worker TWD with the Rust image tag.** Replace `REPLACE_ME` in
`k8s/temporal-worker-deployment.yaml` with the CI tag first. Then:

```bash
kubectl -n radar-ng apply -f k8s/temporal-worker-deployment.yaml
kubectl -n radar-ng get pods -l app=radar-ng-worker -w
temporal worker deployment describe --namespace default --name radar-ng-worker
```

The Rust pods register a new build ID. New executions go to the Python build until the ramp starts.

**5.3 Checkpoint after each ramp step.** When a step's pause starts, run the checkpoint, and
continue only if it passes:

```bash
radar debug doctor --json | jq '.checks[] | select((.status | ascii_downcase) == "fail")'   # must be empty
radar debug pipeline                 # manifest freshness per layer, compare with baseline-pipeline.txt
radar debug temporal                 # failures, stuck, retrying: no growth vs baseline-temporal.txt
temporal worker deployment describe --namespace default --name radar-ng-worker   # ramp percentage
```

Checkpoint pass criteria:

- No `fail` in any section.
- Layer freshness is within the same thresholds as the baseline.
- `temporal.retrying` and `temporal.failures` do not grow between checkpoints.
- The WatchStorm running count (4.4) is steady, and watch starts still progress.
- `radar parity` (section 6) passes on the frames rendered since the last checkpoint.

If a checkpoint fails, roll back (section 7) before the next step.

**5.4 Steps.** After 5% passes, the controller moves to 25%, then to 50%, with the checkpoint
(5.3) after each. The controller completes the ramp to 100% after the last step. Confirm with
`temporal worker deployment describe`.

To hold at a step (for example, while investigating), change `strategy` to `Manual` and apply.
Promotion is then manual with `temporal worker deployment set-current-version`.

**5.5 After 100%.** Restore the Python steady-state rollout values (the commented block in
`temporal-worker-deployment.yaml`), and apply. Python pods drain: pinned executions finish on
them, and sunset (scaledown 10m, delete 1h) removes them. Keep the Python image in the registry
until the sunset completes, so rollback stays available.

**5.6 Open-meteo.** Repeat 5.2 to 5.5 for `k8s/open-meteo-worker-deployment.yaml` after the
legacy cutover is stable. It uses the same slow ramp. Open item 2 applies: the data PVC is mounted
by both builds during the ramp, so keep both builds on one node or on RWX.

**5.7 API.** The tile-server Deployment is a plain swap. Apply the Rust image tag. The Service and
port (`tile-server`, `:8080`) do not change. The api gate is the k6 run in `load/` (job in
`load/k8s/k6-job.yaml`) and the API checklist (spec-api section 15). p95 must not exceed the
Python baseline.

### Coexistence during the ramp

These are expected, and the checkpoints must tolerate them.

- **Tiles from both builds.** Both builds write the same pyramid. Per PLAN's collision rule, a
  final directory with a complete marker from any identity counts as published, so the Rust
  publisher logs `existing_foreign_identity` and does not retry. The checkpoint watches for
  retries, not for that log line.
- **`storms.json` next_cell_id.** A Python write during the ramp can overwrite the counter Rust
  advanced. The result is at most one cell ID reuse per overlap. This is accepted and documented
  (PLAN D3 is fixed in Rust only). The ramp is the only window where it can happen.
- **WatchStorm handoff.** Runs started on Python finish on Python. AUTO_UPGRADE continue-as-new
  runs move to Rust only with a `WatchStormHandoff` v1 marker (PLAN, "WatchStorm"). Check the
  WatchStorm running count before 25% and after 100%.
- **Manifest.** Both builds write `manifest.json`. Rust reads it fail-closed (keeps the last good
  state, does not wipe layers). `radar debug disk` reports `disk.manifest` and ghost or hidden
  layers. Any `FAIL` there is a rollback signal.

## 6. Verification commands

```bash
radar debug doctor                    # full sweep; exit 0 required, warnings allowed
radar debug pipeline                  # manifest freshness per layer
radar debug temporal                  # schedules, failures, running, stuck, retrying
radar debug api                       # api routes and latency
radar debug tiles                     # tile sampling (zooms, 3x3)
radar debug client                    # client simulation (lat/lon sampling)
radar debug disk                      # disk usage, orphans, manifest vs layers
radar debug watch --interval 60       # repeated sweep while the ramp runs
```

**Tile parity.** `radar parity` decodes every PNG of two tile roots and compares them. Both
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

## 7. Rollback

**Normal rollback (preferred).** Put the Python image tag back in the TWD and apply. The
controller re-promotes the previous build. Do this for each TWD that was cut over.

**Emergency rollback (faster, no manifest change).** Set the Python build as current with the
CLI, and switch the TWD to `Manual` so the controller does not fight the change:

```bash
temporal worker deployment set-current-version --namespace default \
  --deployment-name radar-ng-worker --build-id <python build id>
```

Then restore the manifest to the Python image and the steady-state rollout, so the repository
matches the cluster again.

**Drain, do not kill.** Rust executions already running keep running on the Rust pods until they
finish. Do not delete those pods. The sunset removes them after `scaledownDelay`.

**Open-meteo.** Same procedure with the open-meteo TWD. A sync in progress finishes on its pod.

**API.** Revert the image tag on the tile-server Deployment. The api is stateless. Revert
`TEMPORAL_ALERTS_TASK_QUEUE` with the configmap if it was changed.

**Rust-written state.** Python reads the state Rust wrote (`manifest.json`, `storms.json`, and
the nowcast files). The contracts are unchanged (PLAN "Contracts that must not change"), so
rollback leaves that state in place. Check `radar debug disk` after a rollback.

**Compose.** `docker compose down` keeps the named volumes. Bring the Python stack back from git.

## 8. Build checks

### 8.1 glibc floor

```bash
cid=$(docker create radar-ng-radar:ci); docker cp "$cid:/usr/local/bin/radar" ./radar-check; docker rm "$cid"
objdump -T ./radar-check | grep -o 'GLIBC_[0-9.]*' | sed 's/GLIBC_//' | sort -uV | tail -1
docker run --rm --entrypoint /bin/sh ghcr.io/open-meteo/open-meteo:1.6.0 -c 'ldd --version | head -1'
```

The highest GLIBC symbol the radar binary needs must be at or below the open-meteo base's glibc.
If it is not, build the radar image on a matching base, or on musl, and re-check. CI runs the same
check (`.github/workflows/ci-rust.yml`).

### 8.2 Image users

```bash
docker image inspect radar-ng-radar:ci --format '{{.Config.User}}'               # 1000:1000
docker image inspect radar-ng-open-meteo-worker:ci --format '{{.Config.User}}'   # 999:999
```

## 9. Future option: per-role isolated queues (not the default)

Isolated queues are supported (`USE_ISOLATED_TASK_QUEUES=1`, `radar-types` and `radar-worker`),
and the Python reliability plan describes the same step (its step 7). Do this only after the
legacy cutover has been stable for a week, and only with a plan for the queue move:

1. Add the per-role TWDs (one per role, each on its own isolated queue). The commented compose
   block shows the shape.
2. Set `USE_ISOLATED_TASK_QUEUES=1` on the seeding pool. The one seeder rewrites every Schedule to
   its declared queue, after destination pollers are proven.
3. Move the watch-start queue to `radar-ng-alerts` with `alerts` pollers in place.
4. Keep the legacy TWD until its pinned executions are drained.

This is a separate change. It is not part of the cutover in section 5.

## 10. Config drift

Rust follows the code defaults. Where the old compose or docs differ, the code wins (PLAN,
"Config drift to resolve"):

| Variable | Code default (Rust) | Old compose / docs | Action |
|---|---|---|---|
| `FORECAST_TTL_S` | 900 | 300 (compose) | Use 900. Fix the docs |
| `TEMPORAL_ADDRESS` | `localhost:7233` | `temporal-frontend.temporal:7233` (api image) | Set in every manifest and compose service |
| `NWS_USER_AGENT` | none (required) | placeholder in compose | Set in `.env` (compose) or the configmap (k8s). Must carry a contact |
| `OPEN_METEO_BASE` | none (required) | `http://open-meteo:8080/v1/forecast` | Set to the open-meteo home (open item 1) |

## 11. Decommissioning Python, Caddy, and go-pmtiles

Only after the cutover is complete, the Python image has drained and passed its sunset, and the
parity checks have passed on real MRMS frames. Keep the PVCs. Delete these from the Python
repository, in one reviewed change:

- `backend/api/Dockerfile`, `backend/api/Caddyfile`, `backend/api/start.sh`, and the Python api
  package (`backend/api/api/`). Also `backend/shared/` once no Python code imports it.
- `backend/basemap/Dockerfile` (go-pmtiles image). Keep `backend/basemap/styles/` and
  `fetch-glyphs.sh`, which the Rust image copies.
- `temporal/Dockerfile`, `temporal/open_meteo_worker.Dockerfile`, and the Python workers.
- `deploy/docker-compose.yml` and `deploy/k8s/*` (replaced by this directory).

Then fix the config drift in the docs (section 10).

## 12. Known gaps

- Replica counts are 1 for the api and both TWDs. Raise them only after open item 2.
- Worker resource requests and limits are the Python values, not measured for Rust.
- `readOnlyRootFilesystem` is not set on the workers. Rust writes to `/tmp` for the health and
  alive files, and the MRMS and HRRR temp roots (`MRMS_TMP_ROOT`, `HRRR_TMP_ROOT`) are not mapped
  to an emptyDir yet.
- Worker liveness is file existence (`/tmp/worker-alive`). The health task runs apart from the
  workflow loop, so a worker whose runtime is alive but whose workflows are stuck still passes.
  Add a freshness check tied to workflow progress if that matters.
- No Kubernetes bootstrap Job for basemap PMTiles (open item 4).
- Push tokens (`/v1`) cannot be enabled on k8s or compose until the api gets a writable state
  mount. Until then registration returns 503 (F9).
- The open-meteo API has no manifest yet (open item 1).
