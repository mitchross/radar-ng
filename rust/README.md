# radar-ng backend (Rust)

The whole radar-ng backend is one Cargo workspace that builds one binary, `radar`: the HTTP server
(tiles, basemap, `/api`, `/v1`, `/metrics`), the Temporal workers, the schedule seeder and watchdog, and
the operator diagnostics. The app (`frontend/`) and the web radar (`web/`) talk to it over HTTP.

Images, compose and releases: [`deploy/`](../deploy/README.md). Migration history:
`docs/rust-migration/` (PLAN, REVIEW, specs, benchmark-results).

## Build and test

Requirements: the toolchain pinned in `rust-toolchain.toml` (rustup installs it) and `protoc`, which the
Temporal SDK's proto crate needs (`apt install protobuf-compiler libprotobuf-dev` or
`brew install protobuf`).

```sh
cd rust
cargo build --release -p radar          # → target/release/radar
cargo test --workspace                   # unit, golden and the offline Temporal replay gate
cargo clippy --workspace --all-targets -- -D warnings
cargo fmt --all -- --check
```

Workflow integration tests need a Temporal server, for example the throwaway one in `deploy/`:

```sh
docker compose -f deploy/temporal-dev.yml up -d postgres temporal    # from the repository root
cd rust && TEMPORAL_TEST_ADDRESS=localhost:7233 cargo test -p radar-worker --test integration
```

Without `TEMPORAL_TEST_ADDRESS` those tests pass through a skip path.

## Run

```sh
radar api                          # :8080 (LISTEN_ADDR). /tiles, /basemap, /api, /v1, /metrics
radar worker --role legacy         # Temporal worker. Roles: legacy|all|mrms|nowcast|hrrr|aux|alerts|open-meteo
radar seed                         # create or update every Temporal Schedule and exit
radar watchdog                     # schedule stall watchdog
radar debug doctor                 # live-stack diagnostics (docs/debug-harness.md)
radar render-once <mrms.grib2.gz> <out-dir>   # decode, render and detect storms for one frame; print timings
radar parity --python-root A --rust-root B    # decode and compare two tile pyramids (deploy/RUNBOOK.md)
```

Configuration is environment variables (`TILE_DIR`, `STATE_DIR`, `GRID_DIR`, `TEMPORAL_ADDRESS`,
`WORKER_ROLE`, and so on). Each crate's README lists the variables it reads; `docs/configuration.md`
covers the ones operators usually set.

## Crates

| crate | what it does |
|---|---|
| `radar-types` | Temporal wire contract: workflow, activity and signal names, payload structs, schedules, retry specs, queues |
| `radar-geo` | grid and projection types (Lambert conformal, Web Mercator) |
| `radar-core` | env parsing, JSON logging, file locks and durable writes, manifest v2, processed-frame set, grid store, push tokens, nowcast skill log, heartbeats |
| `radar-tiler` | tile renderer: palettes, indexed PNG, per-palette pyramids, atomic publish |
| `radar-grib` | GRIB2 decoding (pure-Rust `grib` crate) and anonymous NOAA S3 / HTTP access for MRMS, HRRR and NAQFC |
| `radar-storms` | storm-cell detection and tracking, storm prefetch plan, storm-watch change detection |
| `radar-ingest` | MRMS and HRRR ingest activity bodies |
| `radar-aux` | air quality (NAQFC), NHC tropical, tile cleanup, Open-Meteo sync |
| `radar-alerts` | Blitzortung lightning, NWS alerts, storm-watch activities, APNs and FCM push |
| `radar-nowcast` | radar nowcast: Lucas-Kanade motion, semi-Lagrangian advection, S-PROG, probability matching, skill scoring |
| `radar-worker` | Temporal control plane: 13 workflows, activity adapters, seeder, watchdog, roles, Worker Versioning |
| `radar-api` | the HTTP server (axum): routes, static tile and basemap lanes, rate limit, metrics |
| `radar-debug` | `radar debug` live-stack checks and `radar parity` |
| `radar-bench` | measurement binaries and `noaa_dump` (used by the live-NOAA check) |
| `radar` | the binary |

## CI and real-data checks

`.github/workflows/ci-rust.yml` runs on every PR that touches `rust/` or `deploy/`, and on `master`:

| job | what it checks |
|---|---|
| `lint-test` | `cargo fmt --check`, clippy with `-D warnings`, `cargo test --workspace` (includes the replay gate) |
| `temporal` | `radar-worker` integration tests against `deploy/temporal-dev.yml` |
| `live-noaa` | `rust/scripts/live-noaa-check.sh` |
| `images` | all three Dockerfile targets build; image users, replay-gate label, glibc floor, probe helpers, api smoke run, Open-Meteo image contract |

`rust/scripts/live-noaa-check.sh [WORK_DIR]` (default `rust/target/live-noaa`) downloads the newest
MRMS MergedBaseReflectivityQC frame and the latest HRRR `wrfsfcf01` with its `.idx` and the ranged
13-variable subset, using the production `radar-grib` code. It checks that MRMS decodes to a CONUS
lat/lon grid, south-up, with real echoes; that every HRRR variable radar-ng reads has an `.idx` record
and a matching message that decodes; and that `radar render-once` renders the MRMS frame for every
palette. It needs cargo, jq and network access to `noaa-mrms-pds` and `noaa-hrrr-bdp-pds`, and exits
non-zero on any failure. Run it locally from the repository root the same way.

The k6 load test against the cluster is in [`load/`](../load/README.md).
