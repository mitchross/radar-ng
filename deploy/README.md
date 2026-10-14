# radar-ng deploy

Build and run assets for the Rust backend: one Dockerfile with three targets, a single-node compose
stack, and the release runbook. The build context is the repository root; the Cargo workspace is in
`rust/`. Cluster manifests live in `mitchross/talos-argocd-proxmox`
(`my-apps/personal-projects/radar-ng/`) and change only through GitOps PRs synced by Argo CD.

Releases and post-release checks: [RUNBOOK.md](RUNBOOK.md).

## Files

| Path | What it is |
|---|---|
| `Dockerfile` | Builder `debian:bookworm-slim` with the toolchain from `rust/rust-toolchain.toml` and `protoc`; runs the offline replay gate (`REPLAY_GATE=1`), then `cargo build --release --locked -p radar`. Runtime distroless `cc-debian12`, uid 1000, static busybox for probes, bundled styles and glyphs under `/srv/basemap` |
| `Dockerfile.dockerignore` | Build-context excludes (keeps `rust/`, drops `target/`, app and data) |
| `docker-compose.yml` | Single-node stack: Temporal, `worker` (role `legacy`), `open-meteo-worker`, `open-meteo`, `api`, and one-shot `basemap-bootstrap` / `basemap-publish` |
| `.env.example` | Compose settings; copy to `.env` |
| `temporal-dev.yml` | Throwaway Temporal (Postgres, auto-setup, UI) for the worker integration tests |
| `pin-images.sh` | Pins third-party base images in `deploy/` to their current digests |
| `RUNBOOK.md` | What ships, release path, checks, Temporal notes, tile parity |

## Images

| GHCR image | Target | Default command | User | Port |
|---|---|---|---|---|
| `radar-ng-tile-server` | `api` | `radar api` | `1000:1000` | 8080 |
| `radar-ng-temporal-worker` | `worker` | `radar worker` (role from `WORKER_ROLE`, default `legacy`) | `1000:1000` | 9464 (metrics) |
| `radar-ng-open-meteo-worker` | `open-meteo` | `radar worker --role open-meteo` on the pinned Open-Meteo image | `999:999` | — |

The open-meteo image also contains `/app/openmeteo-api`; the cluster's serve container overrides the
command to run it. `radar --help` lists the subcommands: `api`, `worker`, `seed`, `watchdog`, `debug`,
`render-once`, `parity`.

Build by hand from the repository root:

```bash
docker build -f deploy/Dockerfile --target api        -t radar-ng-radar:local .
docker build -f deploy/Dockerfile --target worker     -t radar-ng-temporal-worker:local .
docker build -f deploy/Dockerfile --target open-meteo -t radar-ng-open-meteo-worker:local .
```

## Compose stack

From `deploy/`:

```bash
cp .env.example .env                       # NWS_USER_AGENT and BASEMAP_PMTILES_URL at minimum
docker compose run --rm basemap-bootstrap  # one-shot basemap download to basemap.pmtiles.partial
docker compose run --rm basemap-publish    # rename it to basemap.pmtiles (the api reopens it, no restart)
docker compose build api                   # the radar image; `worker` runs it but does not build it
docker compose up -d --build
curl http://localhost:8080/api/livez
radar debug doctor                         # see docs/debug-harness.md
```

Compose runs one `legacy` worker that registers every workflow on task queue `radar-ng` and seeds the
Schedules. Production runs five role pools on isolated queues (`USE_ISOLATED_TASK_QUEUES=1`); the
commented block at the end of `docker-compose.yml` shows that layout for compose.

## Behaviour worth knowing

- **Readiness is `/api/livez`, not `/api/health`.** `/api/health` returns 503 when radar data is stale,
  which would drain every replica during an ingest outage.
- **Basemap.** The api serves `BASEMAP_PMTILES` itself and rechecks its file identity every
  `BASEMAP_PMTILES_RECHECK_S` (default 10), so renaming a new archive over it takes effect without a
  restart. Never write the served path in place. With no local archive, `/basemap/tiles/*` is proxied
  to `BASEMAP_TILES_UPSTREAM` (default `http://basemap:8081`), which is how the cluster runs.
- **Push tokens.** `/v1/push-tokens` answers 503 `push token store is read-only` when `STATE_DIR` is not
  writable. Compose and the cluster mount `state` read-only in the api, so enabling push tokens needs a
  writable mount.
- **Workers run as uid 1000.** The compose `volume-init` service chowns the named volumes.
- **Worker liveness** is `/tmp/worker-alive`, touched every 30 s from startup, including during a long
  Open-Meteo sync.
- **Environment names** are the ones the crates read; each crate README lists them.
  `WATCH_RADIUS_KM` and `MRMS_RENDER_WORKERS` are not read.
