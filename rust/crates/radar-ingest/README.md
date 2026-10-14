# radar-ingest

MRMS and HRRR ingest activity bodies for radar-ng, as plain async functions. The crate has no Temporal
types. The worker crate wraps these functions as activities and supplies the `ActivityInfo` and heartbeat
sink.

Source of truth: `docs/rust-migration/specs/spec-ingest-mrms-hrrr.md`. Defect decisions: `docs/rust-migration/PLAN.md` (Python defects table).

## Coverage

| Area | Tests | Result |
|---|---|---|
| MRMS end to end (fixture GRIB, fake S3, render, manifest, ProcessedSet, cleanup) | `tests/mrms_end_to_end.rs` (11) | 11 pass |
| HRRR pure logic (run selection, publishable prefix, extended tail, resampling, wind rotation H24) | `tests/hrrr_pure.rs` (10) | 10 pass |
| HRRR publication coherence (H5, H6, H7, H21) | `tests/hrrr_publish.rs` (4) | 4 pass |
| HRRR activities against fake S3 and the Lambert fixture (H1, H2, H10, H11, H12, H13, H15 to H19, D3, D5) | `tests/hrrr_activities.rs` (12) | 12 pass |

`cargo test -p radar-ingest`: 37 passed, 0 failed (the lib target has 0 unit tests; invariants are covered
by the integration tests above). No warnings at the time of writing.

Test helpers: `tests/common/mod.rs` provides `FakeS3` (ListObjectsV2, GET with Range, per-key status
overrides, a switch that ignores Range, and a request log of `"METHOD /path"`) and the fixtures:

- `testdata/mrms_regular_ll_png.grib2.gz`: 35 x 70 regular lat/lon MRMS field.
- `testdata/hrrr_lambert_mini.grib2` (6 messages: REFC, TMP, UGRD, VGRD, CAPE, APCP) with a hand-written
  `.idx` built from the message offsets.

The fixture's REFC is a uniform -10 dBZ, which is below every bin of the classic reflectivity palette. The
radar layer therefore only produces tiles from a varied grid. The radar tests use a synthetic 10 to 49 dBZ
field (`refc_grid` in `tests/hrrr_activities.rs`). Tests that run the real decode path assert on
temperature and wind, which the fixture fills.

## Public API

Crate root:

- `IngestConfig` (`from_env()` for the worker; `with_roots(paths, tmp_root)` for tests). Env names and
  defaults follow spec section 3.
- `IngestCtx::new(cfg, nowcast: Option<Arc<dyn NowcastKicker>>)`.
- `NowcastKicker` trait and `kick_nowcast(ctx, timestamp) -> bool`. The kick checks the enabled flag first,
  never raises, and returns whether it was delivered.
- `Heartbeat` (`Arc<dyn Fn(serde_json::Value) + Send + Sync>`), `heartbeat::noop()`, `heartbeat::during`
  (async I/O), `heartbeat::blocking` (CPU work on `spawn_blocking`).
- `ActivityInfo { workflow_id, run_id, activity_id, attempt }`, used for scratch directory names.
- `bench_render_once(grib_path, out_dir) -> anyhow::Result<BenchReport>`, behind `radar render-once`.

`mrms` activities:

- `list_unprocessed_keys(ctx, &IngestMrmsArgs) -> ListKeysResult`
- `process_frame(ctx, info, &ProcessFrameInput, hb) -> ProcessFrameResult` (parses the key before any download, D21)
- `mark_processed(ctx, &MarkProcessedInput)`
- `cleanup(ctx, &CleanupInput) -> CleanupResult`
- `load_mrms_palettes(names)`

`hrrr` activities:

- `find_latest_run(ctx, now) -> FindRunResult`
- `horizon_for_run(ctx, run_id) -> i64`
- `process_forecast_hour(ctx, info, run_id, fhr, hb) -> ForecastHourResult`
- `publish_run(ctx, run_id, results) -> Vec<String>`
- `mark_processed(ctx, run_id)`
- `cleanup(ctx, retention_hours) -> HrrrCleanupResult`

Payload types come from `radar-types`. Each activity returns `anyhow::Result`.

## Deviations from the Python

Renderer and palettes:

- Only the indexed renderer (`indexed-rs1`) is implemented. `TILE_RENDERER` is read by no code in this crate.
- Pyramids written by the Python renderer are never adopted by this crate. A complete pyramid from another
  renderer resumes only when its algorithm, palette digest, tile spec digest and semantic digest all match
  (policy is excluded, spec 6.8.2). Otherwise the hour re-renders. H14 (adoption of a fresh legacy render)
  is N/A because the Rust renderer does not write legacy pyramids.
- `PALETTES` names are de-duplicated, first occurrence wins. If none load, `classic` is used.

Defects (PLAN.md):

- D2 (replicated). An HRRR hour is published only when every palette that has the color key rendered.
  A partial or empty set makes the hour count as not rendered, and the publish prefix stops there. An MRMS
  frame with no visible echo logs `frame_incomplete`, reports `rendered = false`, and is not marked
  processed.
- D8 (replicated). `find_latest_run` returns None when no f01 exists in the last 12 hours, and the resume
  check is not reached.
- D3 (fixed). Scratch directories are removed on `Drop`, including on failure.
- D4 (fixed). The full-file fallback runs only on an index 404. Index 5xx and transport errors fail the hour.
- D5 (fixed). A ranged read that does not return 206 is an error. The crate never writes a whole object
  once per range.
- D21 (fixed). The MRMS key is parsed before the download.
- D6 (percent-encoding of listing prefix and continuation token) lives in `radar-grib`, not in this crate.

Other:

- The HRRR resampler has no per-run cache. Each grid dump resamples on demand.
- Bilinear resampling skips NaN contributions with zero weight. A target with no finite contributions is NaN.
- The nowcast overlap policy (BufferOne) is the worker's job. `kick_nowcast` only delivers the trigger.
- Storm detection keeps the short-shape output that `radar-storms` already produces.
- `bench_render_once` returns its report through `Result` instead of printing and exiting.
- Blocking file and lock work in async activities runs on the blocking pool (F5). This covers the processed-set
  reads and writes, the manifest reads and `add_frame` (which runs under `heartbeat::blocking`, so heartbeats
  continue while the `manifest.lock` flock is waited on), HRRR's existing-layer check, scratch creation, and the
  success-path scratch removal (`ScratchDir::remove`, awaited). Error and cancellation paths still remove the
  scratch directory in `Drop`, which runs inline. A test covers the explicit removal. The other async-path changes
  are not covered by unit tests: they need the object store and Temporal.

## Deferred

- W1 and W2 (workflow constants and the `IngestHrrrResult` default) belong to the worker crate.
- M2 (projected-axis fallback to `latlons()`) is covered at the `radar-grib` level, not here.
- H10 log assertion (exactly one `hour_pending` log line): not implemented. The test checks the return value
  and the empty scratch directory.
- Optional bench test for `bench_render_once` on the MRMS fixture: not written. The function is implemented.

## Performance note

Derived HRRR layers clone the projected geometry (lat and lon meshes) once per layer. This is simple and
correct, but it costs memory and time when many layers are derived from one decoded grid. Sharing the
geometry through an `Arc` would remove the clones.
