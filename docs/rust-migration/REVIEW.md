# radar-ng Rust port: independent review

Reviewer did not write any of the port. Scope: the client contract (app and web against `radar-api`), the
on-disk contract between the Rust writers and readers, Temporal wire compatibility and determinism,
concurrency and durability, and the deploy manifests. Each finding was checked by reading both sides. Where a
cheap test was possible it was run. A temporary probe test (`radar-api/tests/zz_review_probe.rs`) was used
for F1 and the manifest check, then deleted. `cargo test -p radar-api` passes (78 unit, 26 HTTP).

Severity: **High** = breaks production or clients. **Medium** = degrades production or a likely incident.
**Low** = correctness or ops nit worth fixing. Confidence: Confirmed (reproduced or proven from code on both
sides), Likely (code-proven, depends on an environment fact).

## Findings, ranked

### F1. High, Confirmed. The per-client rate limiter now throttles tile, basemap and font requests

- Where: `rust/crates/radar-api/src/lib.rs:56` applies `edge::edge` to the whole router, including
  `basemap::static_router` (`/tiles`, `/basemap/styles`, `/basemap/fonts`, `/basemap/tiles`,
  `/basemap/imagery`). `edge.rs:152` exempts only `/api/livez`.
- Python: the token bucket was FastAPI middleware (`server.py` `request_controls`). Caddy served `/tiles/*`,
  `/basemap/*` and the go-pmtiles proxy without going through FastAPI, so static traffic was never limited.
- Failure: the default limit is 60 burst and 20 rps per client IP. One map view plus playback fetches about
  13 radar frames x 12 to 20 raster tiles, plus 20 to 40 vector tiles and glyph ranges, within a few
  seconds. Probe: 200 rapid GETs of one existing tile from one client gave `{200: 60, 429: 140}`. Radar
  playback gets holes, the basemap gets holes, and `/api/manifest.json` polls starve behind tile traffic in
  the same bucket.
- Fix: limit only the API lanes. Either compute `exempt = path == "/api/livez" || !(path.starts_with("/api/")
  || path.starts_with("/v1/"))` in `edge`, or attach the limiter layer to the `api` router only and keep
  metrics and access logging global. Add an HTTP test with the default limits that requests more than 60
  tiles.

### F2. High, Confirmed. The open-meteo worker never creates `/tmp/worker-alive`, and its image has no busybox, but the liveness probe needs both

- Where: `rust/crates/radar-worker/src/worker.rs:407` (`run_open_meteo_pool`) starts no `health_file_loop`
  (README deviation 7 says so). `deploy/k8s/open-meteo-worker-deployment.yaml:70` adds an exec liveness
  probe `busybox test -f /tmp/worker-alive`. `deploy/Dockerfile.open-meteo` copies only `radar` into the
  Open-Meteo image, not busybox. Compose reuses the same healthcheck (`docker-compose.yml:206`). The Python
  open-meteo TWD had no liveness probe.
- Failure: in k8s the probe fails from 60 s on, for two reasons: exec of a missing binary, and a missing file.
  The kubelet restarts the container about every 3 to 4 minutes. `open_meteo_sync` runs up to 35 minutes,
  so no sync ever finishes. The self-hosted Open-Meteo data goes stale, and `/api/forecast` (which points
  at it in k8s) serves nulls. In compose the service is only marked unhealthy.
- Fix: start `health_file_loop` (or at least the alive-file touch) in `run_open_meteo_pool`. Also
  `COPY --from=busybox` in `Dockerfile.open-meteo`, or use an exec probe the image can run, or drop the
  probe as Python did.

### F3. Medium, Confirmed. `workflows::bounded` uses ordered `buffered`, so one slow item blocks the free slots (HRRR, AQM, MRMS)

- Where: `rust/crates/radar-worker/src/workflows/mod.rs:72`
  (`stream::iter(items).map(f).buffered(limit)`). In futures-util 0.3.34, `Buffered` refills only while
  `FuturesOrdered::len()` is below the limit, and `len()` counts completed but not-yet-yielded outputs
  (`futures_ordered.rs:128`). The doc comment ("a new one starts whenever one finishes") is wrong.
- Python: `Semaphore(N)` plus `gather` releases a slot as soon as any activity finishes.
- Failure: HRRR (`FORECAST_CONCURRENCY=4`, 18 or 48 hours) and AQM (`CHUNK_CONCURRENCY=2`, about 14 min
  per chunk, a 25 min schedule-to-close with retries) run at an effective concurrency of 1 whenever the
  head item is slow or retrying. AQM's 6 chunks are already sized against a 75 min execution timeout
  (seed.py), so one retried head chunk can push the run past the timeout. The run then times out and
  publishes nothing. HRRR extended runs can likewise overrun 45 minutes. The command order also differs
  from Python, which matters only if any Python history is replayed on Rust.
- Fix: use `buffer_unordered(limit)` with each future returning `(index, result)`, then sort by index. This
  stays deterministic under the SDK because completion order comes from history.

### F4. Medium, Likely. Nowcast skill appends take `flock(LOCK_EX)` on the state directory itself, which fails on NFS

- Where: `rust/crates/radar-core/src/nowcast_skill.rs:151` calls `FlockGuard::exclusive_dir(state_dir)`, which
  opens the directory read-only and calls `flock(LOCK_EX)`. Python `nowcast_skill.py` takes no lock.
- Failure: the Linux NFS client emulates `flock` with POSIX byte-range locks, and an exclusive lock needs a
  file opened for writing. A directory cannot be opened for writing, so the call fails with `EBADF`. Every
  `append_entries` then fails. The error is caught and logged as `nowcast_skill_failed`, so
  `/api/nowcast/skill`, the `skill` headline on points and `nowcast_skill` in `/api/health` stay
  "unavailable" forever. `server.py` documents NFS-backed (TrueNAS) PVCs as a known mode. The RUNBOOK also
  mentions Longhorn, where this works.
- Fix: lock a sidecar file opened read-write (as `manifest.lock` does), or drop the lock to match Python.
  Schedule SKIP overlap already serializes nowcast runs.

### F5. Medium, Confirmed. Blocking filesystem calls on async runtime threads, which Python deliberately avoided

- `rust/crates/radar-api/src/health.rs:107`: `read_nowcast_status` uses `std::fs::read_to_string` inside the
  async handler. Python made `/api/health` a sync `def` so a hung NFS read parks a threadpool thread, not
  the event loop. Here each stuck `/api/health` call parks a Tokio worker. With a few callers (dashboards,
  the app's status card) every worker blocks. `/api/livez` then stops answering, and the liveness probe
  restarts the pod, which is the exact outcome Python's design comment warns against. Fix: wrap it in
  `spawn_blocking`, as `nowcast.rs:85` already does.
- `rust/crates/radar-ingest/src/mrms.rs:242`: `manifest::add_frame` (blocking `flock` on `manifest.lock`,
  read, write, fsync) runs directly in the async activity. `mrms.rs:70` (`processed_set` read) and
  `mark_processed` do the same. Python wrapped all three in `asyncio.to_thread` with an explicit comment
  about heartbeat stalls. `manifest.lock` is contended by nowcast `replace_layer`, HRRR publish and the
  cleanups. Waiting blocks a runtime worker that the Temporal core shares, which can delay heartbeats and
  polls. Fix: `spawn_blocking`.

### F6. Medium, Likely. The basemap archive is mmapped once, and an in-place refresh can SIGBUS the API

- Where: `rust/crates/radar-api/src/basemap.rs:171` opens `AsyncPmTilesReader<MmapBackend>` on first use and
  keeps it for the life of the process.
- Failure: `rust/assets/basemap/README.md` documents replacing `basemap.pmtiles` in place ("go-pmtiles picks up
  the new bytes without a restart"). In k8s the archive is now on a writable PVC filled by a bootstrap job.
  If the file is rewritten or truncated in place while mapped, a read from a truncated page raises SIGBUS
  and kills the whole API process: tiles, manifest and everything else. A rename-over is safe but serves
  the old archive until restart.
- Fix: document "replace by rename only, then restart". Better: use a file (pread) backend, or reopen when
  the inode or mtime changes.

### F7. Low, Confirmed. `forecast_requests_total` is counted twice per `/api/forecast` request

- `rust/crates/radar-api/src/forecast.rs:96` (handler) and `:55` (`forecast_for`) both increment it. Python
  increments once. Dashboards and alerts built on the cache hit ratio are skewed. Fix: remove the
  increment in the handler. The briefing path still counts through `forecast_for`.

### F8. Low, Confirmed. Schedule arguments are byte-compared, and key order differs from Python, so Python and Rust seeders keep rewriting each other's schedules

- `serde_json` is built with `preserve_order` (`rust/Cargo.toml:36`). `seed.rs:145` encodes the `json!`
  arguments in insertion order (`{"mrms_prefix":…,"layer_name":…}`). The Python SDK's JSON converter
  writes `sort_keys=True`. `comparable()` compares raw payload bytes.
- Failure: while Python and Rust legacy pods coexist (the ramp, a rollback), each pod's start-up seed sees
  the other's schedules as different and updates the 5 schedules with arguments. The churn is harmless but
  noisy, and it hides real drift in the "updated" logs. Fix: canonicalize with sorted keys (pycompat dumps
  style) in `json_payload`, or compare decoded JSON values.

### F9. Low, Confirmed. Push-token registration cannot work with the shipped mounts (D21 still open)

- `deploy/k8s/tile-server-deployment.yaml` and the compose `api` service mount `state` read-only.
  `POST /v1/push-tokens` writes `STATE_DIR/push_tokens.sqlite` from the API process, so it always returns
  500. Neither client calls `/v1` today, so nothing visible breaks, but the feature is dead as deployed.
  Python's compose had the same `:ro` mount. Fix: decide D21. Either mount `state` read-write for the API
  or move the writes into a worker activity.

### F10. Low, Confirmed. README and code disagree on the imagery cache default

- `config.rs:160` defaults `IMAGERY_CACHE_MAX_BYTES` to 128 MiB. `radar-api/README.md:90,135` say the
  default is 0 (off). The 128 MiB fits the 2 Gi limit, but operators sizing memory from the README will
  be wrong. Separately, `get_imagery` passes `None` for upstream headers, so `ETag` and `Last-Modified`
  from USGS are never forwarded (Caddy forwarded them). Fix the README, and pass the fetched headers.

## Areas reviewed and found clean

- **Route table and client calls.** Every URL in `frontend/src/lib/*.ts` and `web/src/*.ts` (manifest, tile
  template, forecast, inspect, nowcast point and skill, wind-field, storms, lightning, tropical, alerts,
  alerts/map, alerts/explain, geocode, reverse-geocode, briefing, storm-prefetch and style.json, health,
  basemap styles, fonts and tiles) is served with the same path, parameters, JSON keys and status codes as
  `server.py`, `routes_places.py` and the Caddyfile. Checked by side-by-side reading. Path-parameter
  percent-decoding of `encodeURIComponent` timestamps is equivalent.
- **Manifest shape.** A manifest written by `radar_core::manifest::add_frame` and served by `radar-api` was
  run through the app's real `parseSelfHostedManifest` (`frontend/src/lib/manifest.ts`, via `node
  --experimental-strip-types`): VALID. The ETag, 304, 15 s cache and last-good fallback match.
  MRMS, nowcast (`runs/<anchor>/<valid>`, `grid_key`), HRRR (`runs/<run>/<valid>`) and AQM frame extras
  satisfy the validator's rules on `kind`, `issued_at`, `lead_minutes` and `max_zoom`.
- **Tile cache headers.** `public, max-age=86400, immutable` plus ACAO on hits. `no-store` with a text
  body on errors, as Caddy's `handle_errors` did. Run-scoped paths with `:` and `+` resolve through
  `ServeDir`.
- **On-disk paths between writers and readers.** MRMS tiles, grids, `radar-nowcast-input` and
  `storms.json`. Nowcast `runs/<anchor>/<valid>` tiles and point grids, `nowcast-status.json` and
  `nowcast-skill.json`. HRRR `wind_u`/`wind_v` grids keyed by valid time. `lightning.json`,
  `tropical.json`, `alerts_seen.json` and `alerts_active_snapshot.json`. The `manifest.lock` flock sidecar
  matches Python's name and lock type, so mixed Python and Rust writers exclude each other. The
  manifest-first-then-delete order holds in the MRMS cleanup and `tile_cleanup`.
- **Atomic publish.** Tile pyramids are written to hidden staging siblings, marked, fsynced and renamed,
  and the manifest is updated after the tiles exist. The manifest is replaced atomically under flock and
  fsynced. Partial pyramids are never advertised.
- **Temporal wire.** All 13 workflow types, 30 activity names, signal and query names, patch IDs and queue
  names match. The registration per role matches `ROLE_CONFIG`: `legacy` and `all` register every workflow
  and all activities except `open_meteo_sync`, and the other roles add the smoke workflow and activity.
  Activity timeouts and retry policies match every Python call site. All 12 schedules match `seed.py`
  (interval, jitter, catch-up, execution timeout, queue routing, arguments, SKIP). The WatchStorm payloads
  and handoff deserialize Python-encoded inputs (`serde(default)`). The query and signal paths used by
  `signal_matching_storm_watches` work against both Python and Rust watches. The API start, query and
  unpin calls are compatible, and error classification was checked against `temporalio-client` 1.1.0.
- **Determinism.** No wall clock, `HashMap`, randomness or Tokio primitives in workflow code. Waits use the
  SDK's `select!`, `join_all` and `timer`. WatchStorm reads its `patched()` markers at the same points and
  in the same order as Python.
- **Container basics.** reqwest and tungstenite use rustls (no OpenSSL in the lockfile), so distroless
  `cc` is sufficient. Palettes are embedded with `include_str!`. `LOG_LEVEL=info` parses. Double logging
  init is idempotent. The main worker's alive file and busybox probe work.

## Resolution

All ten findings are fixed in the Rust tree. Scope: `radar-api`, `radar-worker`, `radar-core`,
`radar-ingest`, and `deploy/`. Each crate's README lists its own deviations and review fixes.

| Finding | Fix | Test |
|---|---|---|
| F1 rate limit on static tiles | `edge.rs` limits only `/api/*` and `/v1/*`. `/api/livez` stays exempt. Metrics and access logs remain global. | `radar-api/tests/http.rs` `rate_limit_applies_to_api_lanes_but_not_static_tiles` runs with the default limits and more than 60 tile requests. |
| F2 open-meteo liveness probe | `Dockerfile.open-meteo` copies in a static busybox. `run_open_meteo_pool` starts the health-file task before the worker runs. The k8s probe has `initialDelaySeconds` 120, `periodSeconds` 60, `failureThreshold` 5. | Not automated. It needs a live Temporal server. Checked by reading the code and the image build. |
| F3 `bounded` blocks on a slow head item | `workflows::bounded` is a sliding window: a finished slot starts the next item at once. Results keep input order. The window never exceeds the limit. | `radar-worker` `bounded_tests`: `a_freed_slot_starts_the_next_item_while_a_slow_head_is_pending`, `in_flight_never_exceeds_the_limit_and_results_stay_ordered`, `empty_input_and_zero_limit_are_handled`. Replay compatibility: see the next row. |
| F3 replay status | Offline gate `radar-worker/tests/replay.rs` replays all 143 recorded Python histories (v1.1.26, v1.1.27, v1.1.49, src-e8c8659) with the worker's own registrations, and enforces `REQUIRED_SCENARIOS`. MRMS, HRRR and AQM pass on the F3 window with no changes. Two other fixes were needed. WatchStorm's poll wait now cancels its timer when the condition wins, as Python's `wait_condition(timeout=)` does (20 fixtures had failed). MRMS accepts a start with no arguments (`OptionalArg`, 3 fixtures). The allowlist is empty. | `cargo test -p radar-worker --test replay`: 4 tests, 143/143 fixtures pass. The fixtures complete activities in schedule order, so the out-of-order completion case is reasoned to match Python's semaphore, not proven by a fixture. |
| F4 nowcast skill lock on the state directory | The lock moves to a sidecar file `nowcast-skill.lock`. It is not `flock` on the directory. `exclusive_dir` is removed. | `radar-core` `nowcast_skill/tests.rs` `appends_wait_on_the_read_write_lock_sidecar`. NFS behavior is not reproducible here. Deviation: NSK-02 now expects the lock file in the state directory as well (spec D15). |
| F5 blocking filesystem calls on runtime threads | Health and metrics reads, the nowcast skill summary, and ingest file work run under `spawn_blocking` or `heartbeat::blocking`. `ScratchDir::remove` is awaited on success paths. | `radar-api` `health_file_read_does_not_park_the_runtime_worker`. `radar-ingest` `scratch.rs` `remove_deletes_the_directory_before_returning`, plus the existing HRRR and MRMS suites. Ingest coverage is partial. |
| F6 mmapped basemap archive can SIGBUS | `FileBackend` reads with `pread` in `spawn_blocking`, clamped to the snapshot length. The archive's file identity is rechecked every `BASEMAP_PMTILES_RECHECK_S` (default 10). A changed file reopens, and a failed reopen keeps the old reader. Compose and k8s use replace-by-rename (`basemap-publish`). | `radar-api` unit `file_backend_clamps_reads_to_the_snapshot_length`. HTTP `replaced_archive_is_reopened_after_a_rename` and `truncating_the_archive_in_place_is_a_500_not_a_crash`. |
| F7 forecast counter counted twice | `briefing` uses the counted `forecast_for`. The handler uses `forecast_lookup`, which does not count. | `radar-api` `forecast_requests_are_counted_once_including_422s` (expects `radar_ng_forecast_requests_total 4`). |
| F8 schedule arguments compared as bytes | `canonical_json` sorts keys at every depth, as Python's `sort_keys=True` does. The seeder compares parsed values. | `radar-worker` `argument_key_order_does_not_count_as_a_change` and `canonical_json_sorts_keys_at_every_depth`. |
| F9 push tokens unusable with read-only mounts | `POST` push-token returns 503 `push token store is read-only` when `STATE_DIR` is not writable (`access(W_OK)` with ROFS, ACCESS or PERM). The deploy docs say `/v1` push tokens need a writable state mount before they can be enabled. | `radar-api` `push_token_registration_is_503_when_the_state_directory_is_read_only`. D21 is replaced by this decision. |
| F10 imagery headers and cache default | Upstream ETag and Last-Modified are forwarded on hits and misses. The cache is weighed by body size. The README default is 128 MiB, and 0 disables the cache. | `radar-api` `imagery_forwards_upstream_etag_and_last_modified_on_hits_and_misses`. |

Verification (final run):

- `cargo test -p radar-api -p radar-worker -p radar-core -p radar-ingest`: passes. radar-api 79 unit and 33 HTTP. radar-core 110 unit and 1 integration. radar-ingest 1, 12, 4, 10, and 11. radar-worker 69 unit, 4 integration and 4 replay.
- `cargo clippy -p <crate> --all-targets -- -D warnings`: clean for radar-api and radar-core. radar-worker and radar-ingest are clean with `--no-deps`. Their exact-form run fails only on lints in workspace dependencies (`radar-tiler`, and `radar-alerts` for radar-worker). Those crates are out of scope and unfixed.
