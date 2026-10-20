# Debugging a live stack: `radar debug`

`radar debug` inspects a **live** radar-ng stack for health and performance problems: the pipeline,
the serving path, the Temporal control plane, and the data volumes. It probes the stack the way the
app does, through the public port, so its numbers are client-perceived numbers. It is part of the
`radar` binary (crate `rust/crates/radar-debug`), so it is in every image and needs nothing else.

```bash
radar debug doctor --server https://radar.example.com --temporal-address temporal:7233 --namespace radar-ng
```

`doctor` runs every section and exits 1 if anything fails, so it works as a CI smoke test or a
scheduled health sweep (`--json` for machines).

## Commands

| command | what it answers | needs |
|---|---|---|
| `doctor` | everything below, one sweep and a summary | server; Temporal for that section; `disk` only runs where `TILE_DIR` or `STATE_DIR` exists |
| `pipeline` | is the data fresh? per-layer staleness against schedule cadence, radar frame completeness over the trailing hour, forecast-horizon collapse, lightning feed age | server |
| `api` | are the endpoints fast? p50/p95 per endpoint against a budget (`--samples`, default 5); tells a *degraded* `/api/health` (503 by design on stale radar) from a *down* one | server |
| `tiles` | is the serving path right? real tiles for every manifest layer at z4–z8: latency, 404 rate, and the `Cache-Control` contract (`max-age=86400` on every tile) | server |
| `client` | can a cold phone keep up? replays the app's playback pattern (3×3 viewport, last `--frames` frames at `--zoom`, `--layer`) against a 420 ms tick and reports worst-frame latency and bytes | server |
| `temporal` | is the control plane healthy? every seeded schedule present, unpaused and recently fired, with its last result; failed runs in the last `--failure-window-h` hours (default 6) **with the terminal failure message from history**; workflows running more than 2 h; activities on attempt > 1 with their live heartbeat | Temporal frontend |
| `disk` | is the storage sane? PVC headroom, orphaned `<ts>.tmp` staging dirs from crashed renders, leaked `MRMS_TMP_ROOT`/`HRRR_TMP_ROOT` work dirs, per-palette frame-count skew, and **manifest-vs-disk drift** (a manifest timestamp with no tiles on disk is a frame every client 404s) | run where the volumes are mounted |
| `watch` | `doctor` on a loop (`--interval 60`, `--skip-temporal`) for watching a deploy or an incident; Ctrl-C exits 0 | — |

Every command takes `--json`, `--server`, `--temporal-address`, `--namespace`, `--lat` and `--lon`
(probe location, default CONUS centre). Defaults come from `RADAR_DEBUG_SERVER`
(`http://localhost:8080`), `TEMPORAL_ADDRESS` (`localhost:7233`) and `TEMPORAL_NAMESPACE`
(`default`). `disk` reads `TILE_DIR`, `GRID_DIR`, `STATE_DIR`, `MRMS_TMP_ROOT` and `HRRR_TMP_ROOT`.

## Where to run it

- **Laptop → remote stack**: everything except `disk` works over HTTP and gRPC. Build it with
  `cargo build --release -p radar` (see [rust/README.md](../../rust/README.md)). `temporal` needs network
  reach to the Temporal frontend. Without it: `radar debug doctor --server … --skip-temporal`.
- **Inside a container that mounts the volumes**: all commands work, and `disk` picks up the paths from
  the container env. The images are distroless, so call the binary directly:
  ```bash
  docker compose exec worker radar debug doctor --server http://api:8080
  ```
  In the cluster, use an ephemeral debug container or a pod spec in GitOps; do not change workloads by
  hand.

## Reading the output

Each check is `✓ ok / ⚠ warn / ✗ fail` with a one-line diagnosis. The checks encode the system's
invariants, so a red line maps to a known failure mode:

- `pipeline.radar: latest 14m00s old` — ingest stalled; check `temporal.schedule.ingest-mrms-base`
  next, then the mrms worker logs.
- `pipeline.nowcast: forecast horizon +0s` — every nowcast timestamp is in the past; the app's future
  scrubber is showing stale extrapolations.
- `tiles.radar: cache-control mismatch` — the static tile lane is not sending the immutable header;
  playback will re-fetch every tile every loop. The api sends `max-age=86400, immutable` for every
  `/tiles/*` path (frame and forecast-run paths are unique).
- `tiles.<layer>: 0/45 tiles found for latest=...` — the manifest advertises a frame whose pyramid
  never landed (crashed render, or manifest committed before tiles); confirm with `disk` →
  `disk.manifest.<layer>`.
- `client.playback: worst frame 900ms vs 420ms` — a cold client will see blank frames on its first
  loop at this location and zoom; look at `tiles` p95 and the render backlog.
- `temporal.failure.<workflow-id>: … ← <cause>` — the terminal failure cause, extracted from workflow
  history so you don't have to open the Temporal UI.
- `disk.tmp.mrms_work: 3 leaked work dir(s)` — cancelled activities skipped their cleanup; they
  accumulate until the pod restarts.

## Extending

- New tile layer → add a row to `LAYER_EXPECTATIONS` in `rust/crates/radar-debug/src/checks.rs`
  (observed with a max age, or forecast with a minimum horizon). Unknown layers show up as a warning
  so they can't be silently unmonitored.
- New schedule → nothing to do: the expected set is `radar_types::schedules::all_schedules()`, the same
  table the seeder uses.
- New endpoint → add it to the endpoint list in `checks.rs` with its latency budget.

Tests: `cargo test -p radar-debug` (no server, no Temporal).
