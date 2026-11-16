# Level II shadow run

The 30-day shadow pilot for [#293](https://github.com/mitchross/radar-ng/issues/293). The
recorder holds demand for the pilot stations through the real HTTP path, and the data lands in
Postgres, because Prometheus only keeps 15 days. Public Level II stays off for the whole run.

Related: [level2-pool.md](level2-pool.md) (the pool that produces sweeps), the deploy component
in [`deploy/kubernetes/components/level2`](../../deploy/kubernetes/components/level2/), and the
design in [design/level2-close-up.md](design/level2-close-up.md).

## Pieces

| Piece | Command | Writes |
| --- | --- | --- |
| Level II pool (Temporal `level2` role) | `radar worker --role level2` | `sweep_clocks`, one row per indexed lowest sweep |
| Stateless Level II API | `radar api` (`emptyDir` only, no PVC) | nothing |
| Publisher | `radar level2-publish` | viewport revisions and tiles |
| Recorder | `radar level2-shadow record` | `shadow_sweeps`, `shadow_events`, `shadow_states`, `shadow_daily` |

All tables live in `LEVEL2_PG_SCHEMA` (default `level2_numeric`). The recorder needs
`LEVEL2_PG_URL`. It creates its own tables, and the pool creates `sweep_clocks`.

## The five clocks

Each lowest sweep gets five timestamps. Two tables hold them, and they join on `(station, sweep_id)`:

| Clock | Column | Set by |
| --- | --- | --- |
| Scan completion | `sweep_clocks.scan_end` (also `shadow_sweeps.scan_end`) | radar (from the data) |
| Source arrival | `sweep_clocks.source_arrival_at`: newest NOAA `LastModified` of the objects that completed the sweep | pool |
| First availability | `sweep_clocks.first_available_at`: when the verified artifact was indexed | pool |
| Manifest readiness | `shadow_sweeps.manifest_ready_at`: `issued_at` of the first revision whose newest frame is this sweep | recorder |
| Edge display | `shadow_sweeps.edge_display_at`: when the first tile of that frame finished downloading through `LEVEL2_SHADOW_EDGE_URL` | recorder |

The report turns these into separate durations: source lag (arrival − scan end), processing
(availability − arrival), first availability (availability − scan end), manifest wait
(manifest − availability), edge wait (display − manifest) and end to end (display − scan end).

### Field names (stable; C4 and the launch-gate report read these)

`sweep_clocks`: `station`, `sweep_id`, `volume_start`, `scan_start`, `scan_end`, `vcp`,
`elevation_number`, `source_kind` (`live` or `archive`), `source_arrival_at`,
`first_available_at`, `qc_ms`, `rays`, `missing_sectors`, `missing_deg`, `source_bytes`,
`artifact_bytes`, `poll_ms`. A repeat indexing keeps the first row. Rows older than 45 days
(by `scan_end`) are deleted.

`shadow_sweeps`: `station`, `sweep_id`, `scan_end`, `view_id`, `revision`,
`manifest_ready_at`, `manifest_ms`, `edge_display_at`, `tile_cold_ms`, `tile_cached_ms`,
`tile_status`, `tile_cache`. Timings are in milliseconds, measured at the recorder.
`tile_cold_ms` is the first fetch of the tile. `tile_cached_ms` is an immediate second fetch,
and `tile_cache` is the edge's cache header (`cf-cache-status`, `x-cache` or `age`).

`shadow_events`: `at`, `station`, `kind`, `reason`, `seconds`, `tile_ms`. `kind` is one of:

- `fallback`, when the steady view changes to a fallback, with the reason
- `cold_ready`, which has `seconds` = submit to `issued_at`, and `tile_ms`. `reason=already_ready` means the cold view was not cold.
- `cold_timeout`
- `cold_fallback`
- `error`
- `rate_limited`

`shadow_states`: `day`, `station`, `state`, `seconds`. This is observed time per state:

- `ready`
- `fallback:<reason>`
- `error`

The valid-day rule reads this table.

`shadow_daily`: `day`, `doc`. `doc` is the JSON day summary, plus `prometheus` (peak RSS and
CPU seconds per pod), `object_bytes` (the pool's stored bytes) and `edge`.

`shadow_excluded`: `day`, `reason`.

### QC time per sweep

Before this change only a running total existed (`radar_level2_qc_seconds_total`). The decoder
now sums QC time per sweep (`Sweep.qc_seconds`). The pool writes it to `sweep_clocks.qc_ms` and
the `radar_level2_qc_sweep_seconds` summary, and prints `qc_ms` in the poll result.

## Feed-delay drill switch (test only)

`LEVEL2_TEST_FEED_DELAY_S` (pool worker env, default `0` = off, max 3600). When set above 0, the
feed hides every NOAA object whose `LastModified` is newer than that many seconds. To the pool,
the feed looks that much late. The value is logged at start and exported as
`radar_level2_test_feed_delay_seconds`, so a drill shows up on dashboards.

The switch is for the NOAA-delay drill only. Never set it during the pilot. Mark the drill day
with `exclude`. In tests, use `Feed::with_delay`.

## Start the clock

1. Merge the pool, image and deploy PRs. Check that the level2 pool is indexing: `sweep_clocks`
   gains a row every few minutes per demanded station.
2. In your overlay of `components/level2`, set these, then scale `level2-publisher` and
   `level2-shadow-recorder` to 1:
   - `LEVEL2_ENABLED_STATIONS`: the D3 pilot list. It is explicit and has no default.
   - `LEVEL2_VIEWPORT_ENABLED=true`
   - `LEVEL2_TILE_ENABLED=true`
   - `LEVEL2_SHADOW_ENABLED=true`
3. Edge clock: point `LEVEL2_SHADOW_EDGE_URL` at the shadow hostname from the optional
   `components/level2-shadow-route`. Without it, the edge clock is the in-cluster Service time,
   and the cached-edge gate is only a lower bound. Say so in the report.
4. Day 1 is the first full UTC day after the recorder logs `level2_shadow_start`. The recorder
   skips the frame that was already ready when it started, so no row has a false clock.

The recorder submits each station's view about every `max(15, 2.5 × stations)` seconds. This
keeps it under the API's per-client view budget. Every `LEVEL2_SHADOW_COLD_EVERY_S` (default
1800, 0 = off) it also asks for a view nobody holds (zoom 10–13), which measures cold readiness.
Steady and cold numbers are never mixed.

## What a valid day is

A UTC day counts when both of these hold:

- The recorder observed every pilot station for at least 95% of it. That is the sum of
  `shadow_states.seconds` per station, so recorder downtime makes the day invalid.
- The day is not excluded:

  ```sh
  radar level2-shadow exclude --day 2026-12-03 --reason "NOAA delay drill"
  ```

  The rows are kept. Exclude drills and known cluster outages that are not Level II's fault.

A feed gap is not a reason to exclude a day. Gaps are evidence: they appear under `gaps`.

## Daily summary and report

The recorder writes yesterday's summary at 00:10 UTC, and you can rerun it at any time:

```sh
radar level2-shadow summary --day 2026-12-03   # stores and prints it
radar level2-shadow report --days 30           # text; add --json for the machine form
```

`summary` also captures peak RSS and CPU seconds per Level II pod from `LEVEL2_SHADOW_PROM_URL`
(the pod regex is in `LEVEL2_SHADOW_PROM_PODS`). It needs to run within 15 days, before
Prometheus drops the data. If the URL is unset, the summary says `not_configured`.

How to read the report:

- **Gates (p95)**:
  - `qc_ms_per_lowest_sweep` ≤ 25
  - `manifest_ms_edge` < 500
  - `tile_cached_ms_edge` < 800

  Each gate prints `pass`, `fail` or `no_data`. Only valid days count.
- **Clocks**: n, p50, p95, p99 and max for each of the six durations above, in seconds.
- **Cold views**: cold view readiness, the cold tile, and the steady first-tile fetch. These are
  not cached performance.
- **Gaps**:
  - feed gaps: indexed lowest sweeps more than `LEVEL2_SHADOW_GAP_S` apart (default 900)
  - undisplayed sweeps: indexed, never seen at the edge
  - missing sectors
- **Events**: counts of fallback reasons, cold timeouts, errors and rate limits.
- **State seconds**: total time in each state.
- **VCP**: the mix of VCPs.
- **Cost**:
  - `peak_rss_bytes_max`
  - `cpu_seconds_total` (from the daily summaries)
  - `object_bytes_peak`

  Only valid days count.

A day with `valid: false` lists its per-station uptime, which shows which station broke it.
