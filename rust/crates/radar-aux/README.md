# radar-aux

Plain async Rust bodies for four groups of ingest activities:

| Activity | Former Python module | Module |
|---|---|---|
| `aqm_find_latest_run`, `aqm_render_chunk`, `aqm_publish_run`, `aqm_mark_processed`, `aqm_cleanup` | `backend/ingest_airquality/activities.py` | `aqm` |
| `tropical_fetch_and_publish` | `backend/ingest_tropical/activities.py` | `tropical` |
| `tile_cleanup_sweep` | `backend/tile_cleanup/activities.py`, `cleanup.sh` (retired, D-2) | `tile_cleanup` |
| `open_meteo_sync` | `backend/open_meteo_sync/activities.py` | `open_meteo` |

Spec: `docs/develop/specs/spec-ingest-aux.md` sections 2-5, 7-9, 11-16. Lightning (section 6) and NWS alerts (section 10) are
not here. There are no Temporal types in this crate. The worker crate wraps these functions and owns retry
policy, scheduling and the AQM concurrency limit.

## Public API

Config structs read from the environment, with `new(..)` / struct literals for tests:

- `aqm::AqmConfig` (`TILE_DIR`, `STATE_DIR`, `GRID_DIR` via `GridConfig`, `AQM_TMP_ROOT`, palette names from
  `PALETTES`, `PACKED_TILES`). Fields include `sources: radar_grib::naqfc::AqmSources`, `heartbeat_every`
  (30 s), `read_timeout` (300 s) and `packed_tiles`. With packing on, chunk `k` packs its frames into the
  run's bucket part `k` (`aqm/packed.rs`), and `aqm_publish_run` lists the run's buckets with the frames.
- `tropical::TropicalConfig` (`feed_url`, `gis_url`, timeouts 20/15/15 s, `gis_concurrency` 8).
- `tile_cleanup::TileCleanupConfig` (`TILE_DIR`, `STATE_DIR`, `GridConfig`).
- `open_meteo::OpenMeteoConfig` (`OPENMETEO_BIN`, `OPENMETEO_DATA_DIR`, heartbeat 30 s).

Activities (all `async` unless noted):

- `aqm::aqm_find_latest_run(cfg, &reqwest::Client) -> AqmFindRunResult`, plus `find_latest_run_at` with an explicit clock.
- `aqm::aqm_render_chunk(cfg, client, run_id, layer, start_msg, beat) -> AqmChunkResult`.
- `aqm::aqm_publish_run(cfg, run_id, &[AqmChunkResult]) -> Vec<String>` (published layer names).
- `aqm::aqm_mark_processed(cfg, run_id)`.
- `aqm::aqm_cleanup(cfg, retention_hours) -> AqmCleanupResult`, plus `aqm_cleanup_at` with an explicit clock.
- `tropical::tropical_fetch_and_publish(cfg, client) -> TropicalResult`.
- `tile_cleanup::tile_cleanup_sweep(cfg) -> TileCleanupResult`, plus `tile_cleanup_sweep_at` (sync, explicit clock).
- `open_meteo::open_meteo_sync(cfg, args, beat) -> OpenMeteoSyncResult`. Errors are `OpenMeteoError { message, retryable }`.

Helpers that are useful on their own: `aqm::grib::{split_messages, valid_time}`, `aqm::layer_spec`,
`aqm::frame_path`, `aqm::parse_run_id`, `aqm::load_palettes`, `open_meteo::{command_argv, critical_error}`,
`tile_cleanup::{LAYER_RETENTION_MIN, parse_stamp, sweep_layer}`, `tropical::build_geojson` (pure, takes
the feed JSON and a GIS map), `tropical::{num, first_present, saffir_simpson, knots_to_mph}`.

Heartbeats: `beat::Beat = Arc<dyn Fn(serde_json::Value) + Send + Sync>`. Long activities call it with the
Python detail dictionaries (`{phase, run_id, layer, start_msg}` for AQM, `{model, elapsed_s}` for Open-Meteo).
`beat::with_heartbeats` covers async waits and `radar_core::heartbeat::run_blocking_with_heartbeat` covers
blocking work (`spawn_blocking`). `beat::no_beat()` is a no-op.

Error types: `aqm::AqmError`, `tropical::TropicalError`, `tile_cleanup::CleanupError`, `open_meteo::OpenMeteoError`.
All are `thiserror` enums.

## Behaviour

**AQM.** Each chunk renders messages `start..min(start+24, 72)` (starts 0, 24, 48) of one layer
(`air-quality` = `ave_1hr_pm25_bc`, `ozone` = `ave_1hr_o3_bc`). Messages are split by hand from the GRIB2 file.
Each message's valid time comes from its reference time, product template and forecast time (template 4.0),
or from the end of the time range (template 4.8, one range only). The AQI breakpoint tables come from radar-tiler
palettes, keyed by `pm25` or `ozone`. A frame is recorded only when every palette rendered. Grids are written to
`GRID_DIR` with ascending longitudes. Publication writes a layer only when it has exactly 72 frames, with
`runs/<run>/<ts>` paths, `lead_minutes` from the run, `source: "aqm"` and `complete: true`. A run is marked
processed only by `aqm_mark_processed`. The worker must call it after the air-quality layer is published.

**Tropical.** The feed is required. GIS layers are best effort: index failure gives positions only, and one failed
layer drops only that layer. Output is compact JSON, `tropical.json`, mode 0644, written atomically with fsync.

**Tile cleanup.** Follows spec 8.2-8.6. For each expired timestamp directory the manifest entry is removed
first (`remove_frame`, which takes `manifest.lock`), then the directory is deleted. If the manifest write fails
the directory stays and the sweep errors. A corrupt manifest aborts before anything is removed. Versioned runs in
`<layer>/<palette>/runs/` are removed when older than the cutoff, unless they are the layer's current manifest run.
Grid retention runs last, through radar-core `cleanup_old_grids`.

**Open-Meteo.** Runs `OPENMETEO_BIN sync <model> <variables> --past-days <n>` in the parent of
`OPENMETEO_DATA_DIR`, with the worker environment. Stdout and stderr are drained concurrently, each keeping its
last 50 lines. A `[ CRITICAL ]` or `[ ERROR ]` line in the kept lines fails the sync even if the exit code is 0.
A non-zero exit fails with the last 500 characters of stderr. The child has `kill_on_drop`, so cancelling the
future kills it.

## Deviations from the spec / Python

- **D-5** AQM download falls through to the next source on any non-2xx or transport error. radar-grib's
  `download_product` falls back only on 404, while NOAA may answer 403 for a missing key. The fallback is implemented
  locally in `aqm/download.rs`.
- **D-6** The per-attempt temp directory is replaced by a per-run cache, `AQM_TMP_ROOT/cache/<run>/<product>.grib2`.
  One fetch covers all three chunks of a layer. A `flock` on `.<product>.lock` makes concurrent chunks wait for the
  single download. The partial file is `<product>.grib2.part`, renamed on success and removed on failure. Cache
  retention keeps only the newest run directory and is run from `aqm_cleanup`.
- **D-7** radar-core `write_grid` sorts longitudes only when the maximum is above 180. The AQM grid is already
  normalised, so the ascending-column reorder is done in `aqm/render.rs` before writing.
- **D-8** The HTTP read timeout is applied per read (`read_timeout`), not as a total request deadline. This matches
  the Python httpx behaviour the spec describes.
- **D-9** Cancellation of `open_meteo_sync` kills the child process (SIGKILL through `kill_on_drop`). The Python
  activity had no kill path.
- **D-10** Tropical writes through a unique temp name with `tempfile` and fsyncs the file and directory. Python
  used a fixed `tropical.tmp` with no fsync.
- **D-11** Non-finite numbers (NaN, inf) are treated as missing. A numeric string is accepted. A `0` is kept, which
  matters for the `first_present` fallback. A latitude or longitude that fails to parse skips the position feature,
  but the storm is still counted.
- **D-13** Top-level staging entries (`.<name>.tmp-*` or `*.tmp`) directly under a layer directory are swept by
  mtime after one hour, as spec 8.5 says. The 8.2 pseudocode would treat them as palette directories and never
  remove them.
- **Q-12** Only the last 50 lines of each stream are kept, as Python `deque(maxlen=50)` does.
- **Q-13** Error messages keep the last 500 characters, counted in code points.
- **Time parsing.** Naive timestamps are read as UTC. Python's local-time `.timestamp()` is not reproduced.
- **Run ids** are parsed by hand as `YYYYMMDD_CC`, because chrono's `%H` pattern requires minutes.

### Bug found while testing

The template 4.8 valid-time offsets were wrong in the first version. The code read the number of time ranges as the
time unit and the missing-value count as the range length. It only passed because the fixture has a one-hour
range. The golden test caught it against eccodes. The offsets are now octets 42, 49 and 50-53, and multi-range
statistical products are rejected with `Unsupported`.

## Deferred / not done

- **D-12 (not changed here).** radar-core `cleanup_old_grids` does not protect the keep-N layers from the spec.
  The nowcast input retention is owned by radar-core and the nowcast owners. Nothing in this crate works around it.
- **TILE_RENDERER** is ignored. radar-tiler provides only the `indexed-rs1` renderer.
- **Concurrency.** The AQM `Semaphore(2)` for chunks is the worker's job, not this crate's.
- **Scheduling, retry policy and Temporal wrapping** belong to the worker crate.
- **Unverified against real data.** The GRIB fixtures are synthetic (eccodes, `testdata/gen_aqm.py`). They mimic the
  grid 227 Lambert projection from the spec, but the projection parameters are not checked against a real NOAA file.
  The real NAQFC template is unverified: the first-frame-equals-run rule holds only for template 4.0. The NHC feed
  fixtures are hand-made from the Python test FEED, not recorded. The Open-Meteo binary is replaced by shell scripts.
- **Test runtime.** `tests/aqm_service.rs` takes 45 to 80 s in a debug build, mostly tile rendering.

## Tests

`cargo test -p radar-aux`: 76 pass, 0 fail.

- Unit (19): GRIB splitting and unit codes, render helpers, publish (palette selection, skipped layer, sorted frames
  and lead minutes), Open-Meteo argv and classification, tile-cleanup retention table and stamp parsing, tropical
  rounding, number parsing, Saffir-Simpson table.
- `tests/aqm_grib.rs` (6): validDate for template 4.0 and 4.8 and the flipped scan, against eccodes.
  Decoded values match the eccodes golden bitwise. Truncated files are rejected.
- `tests/aqm_service.rs` (22): discovery (AWS before NOMADS, 403 and 500 fall through, cycles after `now`),
  chunks (24 frames per chunk, one download per run and layer, heartbeats, all-NaN frame excluded, missing messages,
  two palettes, failed download leaves no `.part`, retry), publication (72-frame invariants, 50-frame refusal,
  incomplete layer, ozone-only, two-chunk partition, corrupt manifest, bad run id), processed set (100 cap),
  cleanup (expired runs, current run kept, recent kept, cache pruned).
- `tests/tropical.rs` (8): a local axum server serves the feed and GIS. Covers outputs, mode 0644, one query per
  bin and layer, GIS index failure, one failed layer, feed failure keeps the old file, latitude 0, unparseable latitude.
- `tests/tile_cleanup.rs` (11): every retention rule, per-layer windows, palette layout, staging by mtime, non-timestamp
  names, versioned runs (old removed, current and future kept, both palettes), manifest-then-directory ordering
  (manifest lock blocked, directory survives), corrupt manifest fails closed, grid pruning.
- `tests/open_meteo.rs` (10): fake shell binaries. Covers argv and cwd, CRITICAL with exit 0, ERROR, non-zero exit,
  500-character tail, 50-line window, CRITICAL outside the window, heartbeats, cancellation kills the child (checked
  through `/proc`), missing binary.

Fixtures: `testdata/gen_aqm.py` (regenerate with `python -I gen_aqm.py` in a venv with eccodes) writes
`aqm_pm25_mini.grib2`, `aqm_o3_mini.grib2`, `aqm_pm25_stat.grib2`, `aqm_pm25_flipped.grib2`,
`aqm_pm25_blank24.grib2` and `golden/`. `testdata/tropical/` holds the NHC feed and GIS fixtures.
