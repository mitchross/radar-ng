# radar-ng backend: Rust

This is the Rust port of the whole radar-ng backend. It replaces `backend/`, `temporal/`,
`tools/debug_harness`, Caddy and the go-pmtiles sidecar with a single binary, `radar`. The app (`frontend/`)
and the web radar (`web/`) are unchanged. They see the same URLs, JSON and tiles.

The plan is in `docs/rust-migration/PLAN.md`, the cutover steps in `deploy/rust/RUNBOOK.md`, and the
before/after numbers in `bench/results/latest.md`.

## Build and test

Requirements: Rust ≥ 1.92 and `protoc`. The Temporal SDK's proto crate needs `protoc`; install it with
`apt install protobuf-compiler libprotobuf-dev` or `brew install protobuf`.

```sh
cd rust
cargo build --release -p radar          # → target/release/radar
cargo test --workspace                   # ~890 tests
cargo clippy --workspace --all-targets -- -D warnings
```

Workflow integration tests need a Temporal dev server:

```sh
temporal server start-dev &
TEMPORAL_TEST_ADDRESS=localhost:7233 cargo test -p radar-worker --test integration
```

## Run

```sh
radar api                          # :8080 (LISTEN_ADDR). Serves /tiles, /basemap (pmtiles), /api, /v1, /metrics
radar worker --role legacy         # Temporal worker. Roles: legacy|all|mrms|nowcast|hrrr|aux|alerts|open-meteo
radar seed                         # create-or-update every schedule and exit
radar watchdog                     # schedule stall watchdog
radar debug doctor                 # operator diagnostics (port of tools/debug_harness)
radar render-once <mrms.grib2.gz> <out-dir>   # one-frame decode + render + storms timing
radar parity --python-root A --rust-root B    # decode + compare two tile pyramids (--exact vs Python indexed)
```

Environment variables keep the Python names: `TILE_DIR`, `STATE_DIR`, `GRID_DIR`, `TEMPORAL_ADDRESS`,
`WORKER_ROLE`, and so on. Each crate's README lists the variables it reads.

## Crates

| crate | replaces |
|---|---|
| `radar-types` | Temporal wire contract: workflow/activity/signal names, payload structs, schedules, retry specs |
| `radar-geo` | grid + projection types (LCC, web mercator) |
| `radar-core` | `backend/shared`: env, logging, fs/flock helpers, manifest v2, ProcessedSet, grid dumps, push tokens, skill log |
| `radar-tiler` | `backend/shared/tiler.py`, palettes, indexed PNG, atomic publish |
| `radar-grib` | pygrib/eccodes/pyproj/boto, using the pure-Rust `grib` crate plus anonymous S3 |
| `radar-storms` | storm detection, tracking, prefetch, storm-watch change detection |
| `radar-ingest` | `backend/ingest_mrms`, `backend/ingest_hrrr` |
| `radar-aux` | air quality, NHC tropical, tile cleanup, Open-Meteo sync |
| `radar-alerts` | Blitzortung lightning, NWS alerts, storm-watch activities, APNs/FCM push |
| `radar-nowcast` | `backend/nowcast`, a from-scratch Rust replacement for the pysteps LK + S-PROG pieces |
| `radar-worker` | `temporal/`: 13 workflows, activity adapters, seeder, watchdog, roles |
| `radar-api` | FastAPI + Caddy + go-pmtiles |
| `radar-debug` | `tools/debug_harness` |
| `radar-bench` | Rust side of the before/after benchmarks (`bench/`) |
| `radar` | the binary |

## Real-data and image checks

`.github/workflows/ci-rust.yml` runs all of this on every PR on GitHub's runners. To run it locally:

The port was built and tested in a sandbox that could not reach NOAA S3, Temporal's CLI downloads, or a
Docker daemon. Before the cutover, run the preflight from the repository root:

```sh
rust/scripts/preflight.sh              # everything; exits 1 if any step failed
rust/scripts/preflight.sh --dry-run    # show the commands first
```

It needs cargo, protoc, Docker with compose and buildx, and uv (Arch:
`sudo pacman -S rustup protobuf docker docker-compose docker-buildx uv`). Steps:

1. **tools**: the toolchain is present, with install hints.
2. **test**: `cargo test --workspace`, including the offline replay gate.
3. **temporal**: starts postgres and temporal from `deploy/temporal-dev.yml`, runs the `radar-worker`
   integration tests against it, and stops it again.
4. **noaa**: downloads the newest MRMS file and the latest HRRR f01 (with its `.idx` and the ranged
   subset) and compares production Python with Rust: values bitwise, axes, the south-up flip, `.idx`
   picks and message selection for all 13 HRRR variables, Lambert lat/lon. It also reports whether NOAA
   answers 403 or 404 for a missing key, and renders the MRMS frame with both and runs
   `radar parity --exact`. This covers items 1 to 10 and 12 of `crates/radar-grib/README.md`.
5. **docker**: builds both images, then runs the glibc floor and image-user checks and a smoke run of
   `radar api`.

`--help` lists the options, and `deploy/rust/RUNBOOK.md` section 2 explains how to read the results. The
end-to-end MRMS timing and the k6 load test against the cluster are still separate (`bench/README.md`).
