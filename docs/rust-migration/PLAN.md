# radar-ng → Rust: migration plan

Status: **complete: production runs the Rust backend (v2.0.0, 2026-10-14); Python removed.** Owner: Mitch. Specs are in `specs/` and the independent review is in `REVIEW.md`; both are historical. Current docs: `rust/README.md`, `deploy/README.md`, `ARCHITECTURE.md`.

## Where it stands

- `rust/` is the 15-crate workspace that builds one binary, `radar`. All 13 workflows and all 30 activities are
  ported, along with the schedule seeder and watchdog, the API (which replaces Caddy, uvicorn and go-pmtiles) and
  the debug harness.
- **892 tests pass** and `cargo clippy --workspace --all-targets -D warnings` is clean.
- Golden fixtures were produced by running the original Python. Tiles match pixel-for-pixel, storms exactly,
  GRIB decoding bitwise, and nowcast at a correlation of at least 0.999 against pysteps.
- An independent review found 10 defects (`REVIEW.md`). All are fixed and have regression tests.
- Measured on the same 2-core box with identical inputs (`benchmark-results.md`):
  - tile render **12.9×** faster than production Python (4.3× vs Python's indexed renderer)
  - storms **64×** faster
  - nowcast **4×** faster, using 2.4× less RAM
  - the API sustains 13k rps on one core, where FastAPI tops out at ~550 rps; p99 at 1000 rps is 2.6 ms vs 194 ms,
    with 12 MB RSS vs 78 MB
- Real NOAA files, a live Temporal server and the Docker image builds are checked by `ci-rust.yml` on every
  PR (jobs `live-noaa`, `temporal`, `images`).

## Goal

Replace every Python process in the backend (the FastAPI API, all Temporal workers, every ingest/render
activity, nowcast, the schedule seeder and watchdog, and the debug harness) with Rust, for
throughput, latency and footprint. The React Native app (`frontend/`) and the web radar (`web/`)
do not change. They see the same URLs, the same JSON and the same tiles. Everything stays self-hosted on
Kubernetes with Temporal as the only orchestrator.

Non-goals: changing the client contract, changing the data products, or switching away from Temporal.

## What the research settled

| Question | Answer |
|---|---|
| Is there a Temporal Rust SDK? | Yes. `temporalio-sdk` **1.1.0** on crates.io (1.0 shipped 2026-09-04, 1.1 on 2026-10-06; MSRV 1.92). It has `#[workflow]`/`#[activities]` macros, signals, queries, updates, `patched()`, continue-as-new, typed schedules client (`create_schedule`, overlap policy, catchup window, describe/update), heartbeats with details, `WorkerDeploymentOptions` (worker versioning), Prometheus/OTel telemetry, a replayer, and a `testing` feature that spins up a dev server. A hello-world worker compiles against it here. |
| GRIB2 in pure Rust? | Yes. The `grib` crate (0.19) decodes template 5.0, 5.2, **5.3** (HRRR complex packing with spatial differencing), **5.41** (MRMS PNG packing), and 5.40 (JPEG2000, pure Rust via `hayro`). It also handles grid templates 3.0 (lat/lon) and **3.30** (Lambert conformal, HRRR) with built-in lat/lon computation, so we don't need eccodes or PROJ. |
| pysteps? | No Rust equivalent. Port the narrow slice we use: dense Lucas-Kanade on a stride-2 grid, semi-Lagrangian advection, the 6-level FFT bandpass cascade, AR(2) per level, CDF probability matching (spec-nowcast §6–7). |
| Caddy? | **Dropped.** Caddy only existed because Python shouldn't serve ~12k static PNGs per frame, and Envoy can't serve files. `radar api` serves `/tiles` itself (tower-http `ServeDir`, precompressed gzip, per-path cache headers, `no-store` plain-text error override, strict right-to-left `X-Forwarded-For` trusted-proxy rule, JSON access log for Loki). It also serves the basemap **directly from the `.pmtiles` file** (`pmtiles` crate, mmap), which **removes the go-pmtiles sidecar** too. One binary per pod. |
| Payload compatibility | The Python side uses the default JSON converter (`json/plain`, snake_case dataclass fields). Rust payload structs use `serde` with identical field names, `#[serde(default)]` wherever Python has a default, and tolerate unknown keys. |

## Architecture

One Cargo workspace at `rust/`, one container image, and one binary `radar` with subcommands:

```
radar api                       # axum: /api/*, /v1/*, /tiles/* (static), /basemap (pmtiles + styles + imagery proxy), /metrics
radar worker --role <role>      # Temporal worker: legacy|all|mrms|nowcast|hrrr|aux|alerts|open-meteo
radar seed                      # create-or-update all schedules (also runs at worker boot)
radar debug <check>             # port of tools/debug_harness
radar render-once <grib>        # benchmark / parity helper (port of scripts/bench_render_once.py)
```

Crates (dependency order):

| crate | contents | replaces |
|---|---|---|
| `radar-types` | Every Temporal payload struct, wire names (workflow/activity/signal/query/patch IDs, queues, schedule IDs), shared enums | dataclasses across `temporal/` + `backend/` |
| `radar-core` | env/config, JSON logging, durable fs helpers (flock, tmp+rename, fsync), palettes + class model, tiler (geometry, sampling, indexed PNG, identity, completion marker, atomic publish), manifest v2, ProcessedSet, grid dumps, storms + tracking, storm prefetch, storm-watch change detection, push-token store (SQLite), nowcast-skill log | `backend/shared/*` |
| `radar-grib` | `grib` crate wrapper → `Grid { values: Vec<f32>, w, h, geo }`, `.idx` parsing + HTTP range fetch, anonymous S3 ListObjectsV2 | pygrib + boto calls |
| `radar-ingest` | Activity bodies as plain async fns, with no Temporal types: MRMS, HRRR, air quality, lightning (Blitzortung WS), tropical (NHC), NWS alerts, tile cleanup, Open-Meteo sync (still shells out to the Swift binary), APNs/FCM push | `backend/ingest_*`, `tile_cleanup`, `open_meteo_sync`, `storm_watch_activities`, `temporal/shared/push.py` |
| `radar-nowcast` | LK motion, S-PROG, rendering hand-off, skill scoring | `backend/nowcast` |
| `radar-worker` | `#[activities]` adapters over `radar-ingest`, all 13 workflows, schedule seeder, watchdog, roles, health file, deployment smoke | `temporal/` |
| `radar-api` | axum + tower-http, moka caches, a token-bucket rate limiter identical to the Python one, the HMAC bearer verifier, the Temporal client for `/v1`, static tiles, PMTiles basemap | `backend/api` + Caddy + go-pmtiles |
| `radar` (bin) | clap CLI that wires the above | `start.sh`, `worker.py` entrypoints |

Runtime model: tokio for I/O. CPU work (decode, render, nowcast) runs on a dedicated rayon pool behind
`spawn_blocking`, with a heartbeat ticker running alongside it. This mirrors `activity_heartbeat.py` without the GIL.

## Contracts that must not change

These are frozen. Every crate's tests assert them.

1. **HTTP:** every route, query param, status code, JSON key and cache header in `specs/spec-api.md` §2–§10.
2. **Disk:** directory layout, manifest v2 schema, grid `.bin` + `.meta.json` byte format, `storms.json`,
   `nowcast-skill.json`, ProcessedSet files, and the `flock` sidecar names (`specs/spec-shared.md` §2–§5).
   Rust and Python must be able to run against the same PVCs during cutover.
3. **Temporal:** the 13 workflow type names, the activity names, signal/query names, patch IDs, task-queue names,
   schedule IDs, the `sched-` action prefix, schedule args JSON, the watch ID scheme
   `watch:` + sha256(user\0cell), and `WatchStormHandoff` schema v1 (`specs/spec-temporal.md` §1, §10.3).
4. **Tile pixels:** decoded tile pixels must match. PNG bytes need not, because flate2 output differs from zlib.

## Cutover strategy: same GHCR images, bumped through GitOps

Production runs from `mitchross/talos-argocd-proxmox` via Argo CD: five isolated worker pools
(`radar-ng-worker-{mrms,nowcast,hrrr,aux,alerts}`, `AllAtOnce` behind the `RadarDeploymentSmokeWorkflow`
gate), the `tile-server` Deployment, and the open-meteo Deployment. The Rust images are drop-ins
published under the same GHCR names (`radar-ng-temporal-worker`, `radar-ng-tile-server`,
`radar-ng-open-meteo-worker`), so the cutover is the usual Renovate tag bump in the GitOps repo, merged
and synced by Argo. No manifest changes, no `kubectl apply`. Old workflow executions are disposable.
Releases are forward-only: a problem is fixed by a new release. Details: `deploy/RUNBOOK.md`.

## Python defects found by the specs

Default rule: **fix** a defect if the fix does not change a client-visible contract, and **replicate** it
otherwise. Each spec's §14–16 lists its own. The ones that change behavior:

| ref | defect | decision |
|---|---|---|
| shared D1 | storm_prefetch builds nowcast tile paths from timestamp, not `runs/<anchor>/<valid>` → +5/+10 prefetch silently empty | fix |
| shared D2 | storm_watch samples `-inf` outside coverage → false "dissipating/intensifying" pushes | fix (treat as no data) |
| shared D3 | storm cell IDs can be reused | fix (monotonic counter in state) |
| shared D5 | malformed manifest is read as empty, then the next write wipes all layers | fix (fail closed, keep last good) |
| api D* | 500s on non-JSON upstream / NaN body / Temporal down at first connect | fix → 502/400/503 |
| temporal D5 | `PUSH_DISABLED` is truthy for anything but `"1"` | fix (accept 1/true/yes) |
| nowcast D1/D2/D4 | IndexError at low wet-area ratio; dry window reports ok; status written before commit | fix |
| mrms/hrrr D3/D5 | tmp-dir leak on download failure; range reads don't check for 206 | fix |

Config drift to resolve (spec-api §0): `FORECAST_TTL_S` (code 900 vs docs 300), `TEMPORAL_ADDRESS`
default, `NWS_USER_AGENT`, `OPEN_METEO_BASE`. Rust follows the **code** values, and the docs get fixed.

## Phases

| phase | deliverable | done when |
|---|---|---|
| 0 | Workspace scaffold, `radar-types`, `radar-core` fs/env/logging, CI (`cargo fmt/clippy/test`) | `cargo test` green |
| 1 | `radar-core` tiler + manifest/state/grid_dump/storms. Golden fixtures produced by running the Python on synthetic grids (`testdata/`) | the 79 tiler invariants + shared invariants pass; pixel-identical tiles on the synthetic reflectivity fixture |
| 2 | `radar-grib` + MRMS activities + `IngestMrmsWorkflow` + worker + seeder | the parity harness on ≥20 real MRMS frames gives identical decoded pixels; frame wall-clock is measured against Python |
| 3 | `radar-api` (every route + static tiles + PMTiles basemap), replacing Caddy, uvicorn and go-pmtiles | the API test checklist (spec-api §15) is ported and green; k6 `load/k6-radar.js` p95 ≤ Python |
| 4 | HRRR, air quality, tropical, lightning, alerts, cleanup, Open-Meteo, push, WatchStorm | each role's invariants pass; one role at a time is promoted via `set-current-version` |
| 5 | Nowcast (LK + S-PROG) | golden-value tolerances (spec-nowcast App. A); 7-day POD/CSI from `nowcast-skill.json` within 2 pts of Python |
| 6 | Port the debug harness, Dockerfile targets + compose, CI, docs | GHCR images built from Rust; Python, Caddy and go-pmtiles retired |

## Benchmarks and load test (before/after)

Every comparison ran Python and Rust **on the same inputs on the same machine**. The harness (`bench/`) was
removed with the Python code; the results are in `benchmark-results.md`.

| bench | Python baseline | Rust | metric |
|---|---|---|---|
| tile render | `backend/scripts/bench_render_once.py` | `radar render-once` | wall, CPU-s, peak RSS per frame (3 palettes, z4–z7) |
| GRIB decode | pygrib on the same `.grib2` | `radar-grib` | ms per message |
| storms | `detect_storms` + tracking | `radar-storms` | ms per frame |
| nowcast | pysteps `nowcast_run` core | `radar-nowcast` | wall per run (motion + 12 leads) |
| MRMS end-to-end | `mrms_process_frame` | Rust activity | seconds from object listed to manifest commit (real NOAA data, run where S3 is reachable) |
| API load | Caddy+uvicorn+FastAPI | `radar api` | `oha`/k6 (`load/k6-radar.js` mix): RPS, p50/p95/p99, errors, CPU + RSS at fixed RPS |
| idle footprint | Python pods | Rust pods | RSS and CPU at idle per role |

Micro-benches run in CI-like conditions here. The end-to-end and k6 runs happen against the cluster before and after each role cutover.

## Verification

- **Unit/golden:** a Python script (`rust/testdata/gen_golden.py`) runs the original functions on
  synthetic inputs and writes fixtures. Rust tests compare against them exactly for integers and bytes, and
  within ≤1 ulp for floats where the spec allows.
- **Parity harness:** `radar parity --python-root A --rust-root B` decodes every PNG in both pyramids and
  compares the palette-index rasters. It runs on real NOAA data on a machine that can reach S3.
- **Workflow tests:** the SDK `testing` feature (local dev server) runs each workflow's required scenarios
  from spec-temporal §10.1 (success, partial, activity-error, …).
- **Perf:** `radar render-once` vs `bench_render_once.py` on the same GRIB; MRMS end-to-end per frame;
  RSS per worker pod.

## Known risks

1. **Nowcast parity.** pysteps' LK uses OpenCV's Shi-Tomasi and pyramidal LK. Exact replication is out of
   scope, so it is validated statistically with skill scores (`/api/nowcast/skill`).
2. **GRIB edge cases.** Verify against real files before trusting the decoder: MRMS 5.41 bitmap handling,
   HRRR 5.3 with bitmap, NAQFC templates.
3. **Temporal server version** for Worker Deployments; fall back to separate queues if needed.
4. **Replay fixtures** were not in the staged snapshot. Rust doesn't need to replay Python histories
   (pinned runs drain on Python), but WatchStorm handoff must round-trip.
