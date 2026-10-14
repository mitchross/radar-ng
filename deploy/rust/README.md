# radar-ng deploy (Rust build)

Deployment assets for the Rust build of the production topology: one radar image for the api and
the workers, an Open-Meteo worker image, a compose stack for a single node, Kubernetes manifests
that mirror the Python `deploy/k8s/` tree, CI, and the cutover runbook.

This directory is copied into the repository as `deploy/` (and `github-workflows/rust.yml` as
`.github/workflows/rust.yml`). Paths inside the files assume that layout: the build context is
the repository root, and the Cargo workspace lives in `rust/`.

Start with [RUNBOOK.md](RUNBOOK.md). Its section 2 lists the open items that block the cutover.

## Model

The Rust build is the next build of the same production TemporalWorkerDeployments. Names, queues,
schedules, PVCs, and env stay as they are. Only the image changes. The Temporal Worker Controller
ramps new executions to the Rust build, pinned Python executions drain, and rollback is an image
revert.

| TWD | Queue | Role | Image |
|---|---|---|---|
| `radar-ng-worker` | `radar-ng` | `legacy` | radar image, `radar worker --role legacy` |
| `radar-ng-open-meteo-worker` | `radar-ng-open-meteo` | `open-meteo` | open-meteo image, `radar worker --role open-meteo` |

Per-role isolated queues are a future option, not the default (RUNBOOK section 9).

## Files

| Path | What it is |
|---|---|
| `Dockerfile` | Multi-stage radar image. Builder `rust:1.92-bookworm` with BuildKit cache mounts, `cargo build --release --locked -p radar`. Runtime distroless `cc-debian12`, uid 1000, busybox for healthchecks |
| `Dockerfile.dockerignore` | Build context excludes for `Dockerfile` (keeps `rust/` and `backend/basemap/`; drops Python, data, `target/`) |
| `Dockerfile.open-meteo` | The radar binary on the pinned Open-Meteo image (uid 999), for the open-meteo worker |
| `docker-compose.yml` | Single-node stack: Temporal, `worker` (legacy), `open-meteo-worker`, `open-meteo`, `api`, and a one-shot basemap bootstrap. Per-role services are commented as the optional layout |
| `k8s/configmap-temporal-config.yaml` | Shared tunables for the api and the worker. Same name as the Python configmap |
| `k8s/temporal-connection.yaml` | `TemporalConnection` for the cluster frontend |
| `k8s/temporal-worker-deployment.yaml` | `radar-ng-worker` TWD on the Rust image, role `legacy`. Same securityContext, PVCs, env, and envFrom as the Python TWD |
| `k8s/open-meteo-worker-deployment.yaml` | `radar-ng-open-meteo-worker` TWD on the Open-Meteo-based image, role `open-meteo` |
| `k8s/tile-server-deployment.yaml`, `k8s/tile-server-service.yaml` | The api: single container, probes on `/api/livez`, read-only PVCs |
| `k8s/secret-temporal-worker.yaml.template` | Optional push secret (APNS and FCM), mounted by the worker when push is enabled |
| `k8s/README.md` | Manifest layout, placeholders, probes, storage |
| `github-workflows/rust.yml` | CI: fmt, clippy with `-D warnings`, test (with protoc), image builds, glibc floor check, image user check |
| `RUNBOOK.md` | Pre-flight, ramp, verification, rollback, decommissioning, open items |

## Build and run

Compose (single node, from `deploy/`):

```bash
cp .env.example .env                       # NWS_USER_AGENT and BASEMAP_PMTILES_URL at minimum
docker compose run --rm basemap-bootstrap  # one-shot basemap download to basemap.pmtiles.partial
docker compose run --rm basemap-publish    # rename it to basemap.pmtiles (the api reloads it, no restart)
docker compose build api                   # the radar image must exist before open-meteo-worker
docker compose up -d --build
curl http://localhost:8080/api/livez
radar debug doctor                         # acceptance check; see RUNBOOK.md section 6
```

Images by hand (from the repository root):

```bash
docker build -f deploy/rust/Dockerfile -t radar-ng-radar:local .
docker build -f deploy/rust/Dockerfile.open-meteo --build-arg RADAR_IMAGE=radar-ng-radar:local \
  -t radar-ng-open-meteo-worker:local .
```

## Image contract

| Image | Entrypoint | Default command | User |
|---|---|---|---|
| `radar-ng-radar` | `/usr/local/bin/radar` | `api` | `1000:1000` |
| `radar-ng-open-meteo-worker` | `/usr/local/bin/radar` | `worker --role open-meteo` | `999:999` |

The worker command is `worker --role <role>`, and `WORKER_ROLE` is set to the same value
(the smoke activity reads the env var). The `radar` subcommands are `api`, `worker --role <r>`,
`seed`, `watchdog`, `debug ...`, and `render-once <grib> <out>`.

## Environment

Every variable in `docker-compose.yml` and `k8s/*.yaml` was checked against the names the Rust
crates read (string literals in `rust/crates/*/src`). Result:

- **Rust-read:** every radar variable carried over from the Python configmap and TWDs.
- **Infra-only (not read by radar):** Temporal auto-setup and Postgres settings, the Temporal UI
  CORS setting, and `OTEL_EXPORTER_OTLP_ENDPOINT` (read by the OpenTelemetry SDK).
- **Dropped:** `WATCH_RADIUS_KM` and `MRMS_RENDER_WORKERS`. No Rust crate reads them. RUNBOOK open
  item 7 asks to confirm before cutover.
- **Deliberately not set:** `USE_ISOLATED_TASK_QUEUES`. The legacy model keeps Schedules on
  `radar-ng`, as Python does.

Config drift follows the code (`FORECAST_TTL_S` 900, not the old compose value of 300). See
RUNBOOK section 10.

## Checks performed

- All YAML parses: `docker-compose.yml`, `github-workflows/rust.yml`, every `k8s/*.yaml`, and the
  secret template (PyYAML `safe_load_all`).
- Manifest consistency: `radar-ng-worker` and `radar-ng-open-meteo-worker` each have the Python
  name, queue, and role, the Rust args, and no `TEMPORAL_TASK_QUEUE` or `USE_ISOLATED_TASK_QUEUES`.
  Compose `worker` and `open-meteo-worker` have the matching command and `WORKER_ROLE`. The removed
  services (`basemap`, `tile-server`) are absent. Every `depends_on` target exists.
- Env audit (script not committed; re-run by grepping the crates for the names).

## Not checked

- No image was built. Docker was not available. The Dockerfile syntax and cache-mount usage are
  unverified.
- `docker compose config`, `kubeconform`, `hadolint`, and the GitHub workflow have not been run.
- `cargo build -p radar` has not been run for the deploy path.
- The glibc floor (`Dockerfile.open-meteo`) is unverified (RUNBOOK section 8.1).
- The worker's freshness behavior (alive file age) is not checked by the probe.

## Known problems in the inputs

- **`backend/basemap/styles/` is not in the uploaded tree.** The Python api Dockerfile copies it
  from there, and the Rust image does too. Restore it before the first build.
- **`cargo clippy --workspace -- -D warnings` fails until existing warnings are fixed.** Clippy
  reported warnings in `rust/crates/radar-core/src/grid_dump.rs` (outside radar-debug, which is
  clean). CI runs this gate, so the job fails until those are fixed.
- **Image tags are placeholders** in `k8s/` (`REPLACE_ME`). They must come from CI builds.
- **The Python reliability plan expects `radar-ng-alerts` pollers** for the watch-start queue.
  The single-TWD model has none. This build sets `TEMPORAL_ALERTS_TASK_QUEUE=radar-ng` (RUNBOOK
  open item 3).

## Deviations from the Python deploy

- One radar image for the api and the worker (was: Python api image, Python worker image, Caddy,
  go-pmtiles). The Python TWD names, queues, and PVCs are unchanged.
- The api is one container with no Caddy, uvicorn, or sidecar. It serves PMTiles and styles
  itself. The go-pmtiles service is gone. `basemap-bootstrap` still fills the volume, and it writes
  `basemap.pmtiles.partial`. `basemap-publish` renames that file into place (F6), so the api never
  reads a half-written archive. The api checks the archive's file identity every
  `BASEMAP_PMTILES_RECHECK_S` (default 10) and reopens it after a rename.
- Workers run as uid 1000 (were root in compose). `volume-init` chowns the named volumes.
- The worker and open-meteo TWDs have a liveness probe on `/tmp/worker-alive`. The Python TWDs had
  none. The open-meteo image copies in a static busybox for that probe (F2). Its probe waits 120 s
  before the first check, runs every 60 s, and fails after 5 misses. The health task runs from
  startup, so a sync that takes up to 35 minutes does not trip it.
- Push-token registration (`/v1`) returns 503 `push token store is read-only` when `STATE_DIR` is not
  writable (F9). Compose and k8s mount `state` read-only in the api, so `/v1` push tokens need a
  writable state mount before they can be enabled. Until then the 503 is the expected answer.
- The api's readiness is `/api/livez`, not `/api/health`. `/api/health` returns 503 when radar data
  is stale, and that would drain every replica during an ingest outage.
- `SEED_SCHEDULES=1` is set on the compose `worker` (legacy seeds by default in Python too). The
  k8s TWD leaves it unset, matching Python.

## Future: per-role queues (not the default)

Per-role isolated queues (`USE_ISOLATED_TASK_QUEUES=1`, queues `radar-ng-<role>`, one TWD per role)
are supported by `radar-types` and `radar-worker`. The commented block in `docker-compose.yml`
shows the shape. RUNBOOK section 9 has the procedure. This is a separate change after the legacy
cutover is stable.
