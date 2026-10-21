# Rust migration: handoff notes

> **Working in `rust/` today?** Read the **`rust-dev` skill** first
> ([`.claude/commands/rust-dev.md`](../../.claude/commands/rust-dev.md)): toolchain, the push gate,
> replay compatibility, concurrency traps and the image contract, in one place. This page is the
> history behind those rules.

Read this first if you're an engineer or an LLM picking up radar-ng after the Python → Rust migration
(October 2026). It records what changed, where things live now, and the gotchas the migration hit.

## State (2026-10-14)

- **The backend is Rust only.** The Python backend (`backend/`, `temporal/`, `tools/debug_harness`),
  Caddy, FastAPI/uvicorn and the go-pmtiles sidecar in the api pod are gone from the repo. To read the
  old Python, check out `5b3cb39`, the last commit before removal (PR #92).
- **Production runs the Rust images**: `ghcr.io/mitchross/radar-ng-{temporal-worker,tile-server,open-meteo-worker}`.
  The `v2.x` line is Rust; `v1.x` was Python.
- **The app (`frontend/`) and web radar (`web/`) did not change.** The Rust API serves the same URLs, JSON
  and tiles.

| PR | What |
|---|---|
| radar-ng #90 | the port: `rust/` workspace, deploy assets, docs |
| radar-ng #91 | GHCR versions start at `v2.0.0` and print `tag@digest` |
| radar-ng #92 | Python removed; repo, CI and docs made Rust-only |
| radar-ng #94 | web nginx MIME fix (unrelated to Rust, found the same day) |
| talos-argocd-proxmox #2943, #2944 | cluster moved to `v2.0.0`, then `v2.0.1` |

## Where things are

| Need | Look at |
|---|---|
| The binary and its subcommands | `rust/README.md` (`radar api`, `worker --role …`, `seed`, `watchdog`, `debug …`, `render-once`, `parity`) |
| What each crate does, env vars it reads | `rust/crates/<crate>/README.md` (env var names are the Python names, unchanged) |
| Workflow/activity names, queues, schedules, payloads (the Temporal wire contract) | `rust/crates/radar-types` |
| Images | `deploy/Dockerfile`, targets `api`, `worker`, `open-meteo` |
| Single-machine stack | `deploy/docker-compose.yml`, `deploy/README.md` |
| Release procedure | `deploy/RUNBOOK.md`, `docs/develop/releasing.md` |
| Live cluster manifests | `mitchross/talos-argocd-proxmox`, `my-apps/personal-projects/radar-ng/` |
| Why decisions were made | `docs/archive/rust-migration/PLAN.md`; the behavioural specs of the Python in `specs/`; review findings in `REVIEW.md` |
| Before/after numbers | `docs/archive/rust-migration/benchmark-results.md` (tiles 12.9×, storms 64×, nowcast 4×, API ~13k rps vs ~570) |

## How releases work (do not deviate)

1. Merge to `master` in radar-ng. `ci-rust.yml` checks:
   - `lint-test`: fmt, clippy `-D warnings`, the tests, and the offline replay gate.
   - `temporal`: integration tests against a real server.
   - `live-noaa`: the newest real MRMS and HRRR files are decoded and rendered.
   - `images`: every target builds and starts.
2. `ghcr-*.yml` publishes the next `vX.Y.Z` and prints `ghcr.io/...:vX.Y.Z@sha256:...` as a notice.
3. Open a PR in talos-argocd-proxmox that changes only the `image:` lines. Pin tag **and** digest; never
   `latest`. Use the worker pools in `temporal-workers/worker-pools.yaml` (5 lines), `deployment-tile-server.yaml`
   and `deployment-open-meteo.yaml` (2 lines). The owner merges, and Argo CD syncs.
4. **GitOps only:** no `kubectl apply` and no hand edits to the cluster.
5. **Forward-only:** no rollbacks. Fix a bad release with a new release.

## Gotchas found during the migration

- **MRMS basic angle.** Real MRMS GRIB2 sets template 3.0 basic angle 1 with 10^6 subdivisions. Synthetic
  fixtures used 0. `radar-grib` now follows eccodes' rule.
- **HRRR precip-type codes.** HRRR now encodes categorical rain/snow/freezing/ice as WMO 0.1.33-36. The
  older NCEP-local numbering was 0.1.192-195. The selectors accept both.
- **Ground surface level.** HRRR leaves the surface (type 1) value missing. Selectors treat missing as 0.
- **Two HRRR messages are not bitwise equal to pygrib:** cloud ceiling and cloud top (0.3.5, complex
  packing, bitmap, E<0). radar-ng doesn't read them.
- **Temporal replay compatibility.** Every Python-recorded history in
  `rust/crates/radar-worker/testdata/replay_fixtures` must replay on the Rust workflows (`tests/replay.rs`,
  also run inside the worker image build). Two Rust bugs were found this way:
  - The SDK schedules a timer when `ctx.timer()` is called, so the timer must be cancelled explicitly.
  - MRMS must accept a start with no input payload.

  Keep this gate when changing workflow command order.
- **Drop-in images.** The cluster manifests didn't change. The images keep:
  - the same ports, uids (1000; 999 for open-meteo) and probe files (`/tmp/worker-alive`,
    `/tmp/temporal-healthy`);
  - busybox `sh`, `find` and `test` for the exec probes.
- **Basemap tiles.** The api reads a local `.pmtiles` archive (`BASEMAP_PMTILES`). When none is mounted, it
  proxies `/basemap/tiles/*` to the go-pmtiles `basemap` service (`BASEMAP_TILES_UPSTREAM`, default
  `http://basemap:8081`). That fallback is how production works today.
- **protoc.** The Temporal SDK build needs `protoc` and the well-known types: `protobuf-compiler` plus
  `libprotobuf-dev` on Debian/Ubuntu.
- **Toolchain.** `rust/rust-toolchain.toml` pins Rust 1.97.0. CI, the Docker builder and local builds all
  use it, so clippy sees the same lints everywhere.
- **Fixture generators.** The golden-fixture generator scripts (`rust/crates/*/testdata/gen_*.py`) ran the
  Python code. To regenerate a fixture, check out `5b3cb39`.
- **Web nginx MIME map.** A server-level `types {}` block replaces nginx's whole MIME map; include
  `mime.types` first. When that was missed, radar.vanillax.me downloaded as a `.dms` file. `ghcr-web.yml` now
  smoke-tests `Content-Type` before publishing.

- **Publication locks and Rayon.** `radar_tiler::render_frame_palettes` holds a file lock while it
  renders tiles in parallel. Calling it from parallel tasks on one Rayon pool deadlocks: nowcast did this
  until radar-ng #97. Call it from a plain thread, or one frame at a time.
- **Timeouts don't stop blocking work.** When a Temporal activity times out, its `spawn_blocking` render keeps
  running, which can hold memory across retries (`REVIEW-2026-10-14.md`).

## Known follow-ups

- `frontend` CI: `expo-doctor` fails on master, independent of this work (a label-only PR failed the same
  way). The cause hasn't been diagnosed.
- In talos-argocd-proxmox, `docs/domains/temporal/safe-deployments.md` still describes rollback steps. Policy
  is now forward-only.
- The `basemap` Deployment in the cluster can go once the api mounts the `pmtiles` volume and sets
  `BASEMAP_PMTILES`.
- There is no generator for Rust-recorded replay histories yet. Today's fixtures are Python-recorded.
