# Debug harness

`tools/debug_harness` is a zero-dependency CLI that inspects a **live** radar-ng
stack for performance and health issues — the pipeline, the serving path, the
Temporal control plane, and the data volumes. It probes the stack the same way
the app does (through Caddy on the public port), so its numbers are
client-perceived numbers.

```bash
# from the repo root, pointed at any running stack
python -m tools.debug_harness doctor --server http://radar.example.com:8080
```

`doctor` runs every section and exits non-zero if anything is red — usable
directly as a CI smoke test or a cron'd health sweep (`--json` for machines).

## Commands

| command | what it answers | needs |
|---|---|---|
| `doctor` | everything below, one sweep + summary | — |
| `pipeline` | is the data fresh? per-layer staleness vs schedule cadence, radar frame completeness over the trailing hour, forecast-horizon collapse, lightning feed age | server URL |
| `api` | are the endpoints fast? p50/p95 latency per endpoint vs budget; distinguishes a *degraded* `/api/health` (503 by design on stale radar) from a *down* one | server URL |
| `tiles` | is the serving path right? samples real tiles for every manifest layer at z4–z8: latency, 404 rate, and the Cache-Control contract (observed layers `max-age=86400, immutable`, forecast layers `max-age=120`) — a wrong header here silently re-downloads the whole playback loop or pins stale forecasts | server URL |
| `client` | can a cold phone keep up? replays the app's playback pattern (3×3 viewport × last N frames) against the 420 ms tick (`PLAYBACK_MS` in `TimelineBar.tsx`) and reports worst-frame latency + bytes | server URL |
| `temporal` | is the control plane healthy? all 10 seeded schedules present/unpaused/recently-fired, last-run result per schedule, failed runs in the last 6 h **with the terminal failure message extracted from history**, workflows stuck >2 h, activities on attempt >1 with their live heartbeat phase | `temporalio` installed |
| `disk` | is the storage sane? PVC headroom, orphaned `<ts>.tmp` staging dirs from crashed renders, leaked `/tmp/{mrms,hrrr}_work` attempt dirs, per-palette frame-count skew, and **manifest-vs-disk drift** (manifest timestamps with no tiles on disk = frames every client 404s) | run inside a pod/container that mounts the volumes |
| `watch` | `doctor` on a loop (`--interval 60`) for watching a deploy or an incident live | — |

Every command takes `--json`, `--server`, `--lat/--lon` (probe location,
default CONUS center), and reads `RADAR_DEBUG_SERVER`, `TEMPORAL_ADDRESS`,
`TEMPORAL_NAMESPACE` from the environment.

## Where to run it

- **Laptop → remote stack**: everything except `disk` works over plain HTTP.
  `temporal` needs `pip install temporalio` and network reach to the Temporal
  frontend (port-forward in k8s: `kubectl port-forward svc/temporal-frontend 7233`).
- **Inside the tile-server or worker container**: all commands work; the worker
  image already has `temporalio`. `disk` picks up `TILE_DIR`/`GRID_DIR`/`STATE_DIR`
  from the container env automatically.
  ```bash
  docker compose exec worker python -m tools.debug_harness doctor --server http://tile-server:8080
  kubectl exec deploy/radar-ng-worker -- python -m tools.debug_harness disk
  ```

## Reading the output

Each check is `✓ ok / ⚠ warn / ✗ fail` with a one-line diagnosis. The checks
encode the system's real invariants, so a red line maps to a known failure
mode:

- `pipeline.radar: latest 14m00s old` — ingest stalled; check
  `temporal.schedule.ingest-mrms-base` next, then worker logs.
- `pipeline.nowcast: forecast horizon +0s` — every nowcast timestamp is in the
  past; the app's future scrubber is showing stale extrapolations.
- `tiles.radar: cache-control mismatch` — Caddy routing changed; playback will
  re-fetch every tile every loop.
- `tiles.<layer>: 0/45 tiles found for latest=...` — the manifest advertises a
  frame whose pyramid never landed (crashed render, or manifest committed
  before tiles); confirm with `disk` → `disk.manifest.<layer>`.
- `client.playback: worst frame 900ms vs 420ms` — a cold client will see blank
  frames on its first loop at this location/zoom; look at `tiles` p95 and the
  render backlog.
- `temporal.failure.sched-nowcast-…: … ← pysteps_failed` — the terminal
  failure cause, extracted from workflow history so you don't have to open the
  Temporal UI.
- `disk.tmp.mrms_work: 3 leaked work dir(s)` — cancelled activities skipped
  their cleanup handler; they accumulate until pod restart.

## Extending

- New tile layer → add a row to `LAYER_EXPECTATIONS` in
  `tools/debug_harness/checks.py` (`observed` with a max age, or `forecast`
  with a minimum horizon). Unknown layers show up as a warn so they can't be
  silently unmonitored.
- New schedule → nothing to do: the expected set is imported from
  `temporal/schedules/seed.py` at runtime (with a hardcoded fallback for
  environments without the repo on `sys.path` — keep that list in sync).
- New endpoint → add `(name, path, budget_ms)` to `API_ENDPOINTS`.

Tests: `python -m pytest tools/debug_harness/test_checks.py`.
