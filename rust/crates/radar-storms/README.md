# radar-storms

The radar-ng storm-cell pipeline:

- Storm detection: threshold mask, connected components, tracking, and the `storms.json` write (spec-shared 6, 7, 8).
- Storm prefetch: the prefetch plan for the three storm bboxes (spec-shared 13).
- Storm watch: frame sampling and change detection (spec-shared 14, 15; spec-api 5.8 to 5.9.1, 13.5).

The crate has no I/O beyond the `storms.json` file, its lock file, and reading the nowcast manifest and grid directory. It has no HTTP, Temporal, or database code.

## Public API

Re-exported from `src/lib.rs`:

| Item | Module | Purpose |
| --- | --- | --- |
| `StormConfig` | `config` | Thresholds and limits. `from_env` / `from_lookup`, plus `grid_dir_from_env` (default `/data/grids`) and `state_dir_from_env` (default `/data/state`). |
| `Grid<'a>` | `grid` | Borrowed reflectivity grid with lat and lon axes. `Grid::new` validates shapes. |
| `detect_storms`, `detect_storms_with_clock` | `detect` | Mask, label, filter, sort, track, build a `StormsDoc`. The `_with_clock` form takes the wall clock as an argument so tests are deterministic. |
| `CellFeature`, `StormsDoc` | `doc` | Serde types. Field order matches the `storms.json` key order. |
| `write_storms_json`, `write_storms_json_with_clock`, `WriteOutcome` | `storage` | Locked, atomic, monotonic write of `storms.json`. Returns `Written` or `SkippedStale`. |
| `PrefetchPlan`, `PrefetchRequest`, `build_storm_prefetch_plan` | `prefetch` | Tile URL plan for the selected storm, from the manifest and the storms file. |
| `detect_change`, `sample_window`, `ChangeEvent`, `ChangeKind`, `FrameSample`, `WatchConfig` | `watch` | Storm-watch sampling and change classification. |
| `StormsError` | `error` | `Config`, `InvalidGrid`, `Manifest`, `Io`, `Json`. |

The `watch` module (`pub mod watch`) also exposes `compare_radar_frames`, `detect_storm_change`, and `latest_radar_meta`. The `prefetch` module exposes `read_manifest_value`, radar-core's manifest reader as a JSON value.

## Defect fixes (PLAN.md)

The golden fixtures record the behavior of the Python implementation this crate replaced. For D1 to D3 the Rust tests assert the corrected behavior and how it differs from the fixtures.

- **D1, nowcast tile paths.** The prefetch plan takes a nowcast frame's tile path from the manifest frame's `path` field. The old Python rebuilt it from the layer and timestamp, which broke for run-scoped paths (`runs/<anchor>/<valid>`). The code percent-encodes path segments one at a time, so `/` survives, and the quote-safe set is `:+-.T_`. An oracle in `testdata/gen_golden.py` checks the fixed output. Covered by `golden::nowcast_run_paths_produce_tiles_d1` and `invariants::d1_nowcast_paths_come_from_the_manifest_frame`.
- **D2, no-data samples.** A window with no valid pixels, or one outside the grid, produces a `-inf` max sample with `has_data == false`. `detect_change` returns no event for a sample without data, so no-data never reports dissipating or intensifying. Covered by `golden::detect_change_matches_python_table_except_d2`, `golden::compare_radar_frames_matches_python_except_d2`, and `invariants::d2_no_data_never_produces_a_change`.
- **D3, monotonic cell IDs.** `storms.json` carries `next_cell_id`. New IDs are `max(previous IDs) + 1`, raised to the stored counter, so an ID is never reused after its track disappears. Covered by `golden::d3_reuse_reference_ids_are_monotonic` and `invariants::drv06_matched_keep_id_and_new_cells_take_max_plus_one`.

Other behavior and decisions:

- **D5, empty frames.** When no pixel reaches the threshold, the writer emits a short payload with no `threshold_dbz` or `cell_count`. A frame where `min_pixels` filters out every component keeps the full shape. `next_cell_id` is written on every payload so the counter survives empty frames. Covered by `invariants::empty_frame_keeps_the_python_short_shape_and_the_counter` and `invariants::components_all_filtered_by_size_keep_the_full_shape`.
- **D6 and D16, naive timestamps.** The crate reads a timestamp with no offset as UTC. Mixed naive and aware timestamps do not crash. Covered by `invariants::mixed_naive_and_aware_timestamps_do_not_crash`.
- **D7, manifest without layers.** A manifest that is not an object, has no `layers`, or cannot be parsed returns `StormsError::Manifest` and fails closed. A missing manifest file is read as empty. Covered by `invariants::d7_manifest_without_layers_fails_closed`.
- **D8, durability.** The writer fsyncs the `storms.json` temp file before the rename and the directory after it. `EINVAL` on the directory fsync is ignored.
- **D4, 111.32 versus 110.574 km per degree.** Not unified. Storm area and distance use 110.574 km per degree of latitude. The watch sampler uses 111.32 for latitude. This still needs a decision.
- **D17, appeared and disappeared events.** The crate does not produce them.
- **Mode 0600.** The writer gives `storms.json` mode 0600.

## Notes on numerics and ordering

- Connected components use 4-connectivity, numbered in raster order, to match `scipy.ndimage.label`. Components are sorted stably by peak, descending, so ties keep raster order.
- Optimal track assignment uses `pathfinding::kuhn_munkres_min` on integer costs (km times 1e6). Missing predictions cost 1e9 km. The matrix is transposed when there are more rows than columns.
- `sample_window` means use float32 and `fit_motion` sums use float64. Both reproduce numpy's pairwise summation (8-lane blocks up to 128, recursive split above that). Checked bit-exact against numpy 2.5.4 in `golden::numerics_match_numpy_and_python`.
- `round(x)` is round-half-even, as in Python. `round(x, n)` is reproduced by format and parse, not multiply and divide.
- Haversine takes (lon, lat). `distance_latlon_km` takes (lat, lon).

## Known caveats

- **Kuhn-Munkres ties.** Integer costs at 1 mm resolution can break ties differently from scipy's `linear_sum_assignment`. Only exact ties are affected. No golden case hits one.
- **Counter key.** `next_cell_id` is a key in `storms.json` that the old Python writer did not have.
- **Style URL in radar-api.** The prefetch plan resolves the nowcast path correctly. The radar-api `style.json` builder must also read the manifest frame `path`, or D1 is fixed only in the plan.
- **Manifest reader.** `read_manifest_value` wraps radar-core's `read_manifest` and returns it as JSON, so a structurally invalid layer or frame fails closed too.
- **Naive timestamps.** Treating them as UTC is a policy choice. The crate misreads a naive timestamp from a non-UTC writer.
- **Single-row or single-column grids.** Spacing cannot be measured from one sample, so the detector uses fixed fallback spacing (1 km latitude, 0.01 degrees longitude). No golden case covers this.

## Deferred

- Point-in-GeoJSON check used by the storm-watch activity.
- The alert and mark-seen storm-watch activities. They have side effects and are not pure logic.
- Wire structs for storms in `radar-types`. The crate defines its own serde types for now.
- radar-core integration for the manifest reader (see above).
- `style.json` path resolution in radar-api (see Known caveats).
- Readers of `storms.json` in radar-api (spec-api 9). These live outside this crate.

## Tests

Run with `cd rust && cargo test -p radar-storms`.

- Unit tests (`src/`): 27 pass. Config defaults and env parsing, geo reference values, label numbering, numeric rounding and pairwise association, time parsing, storage (no regression, compaction, mode 0600, corrupt previous file, flock exclusivity), tracking (fit, assignment, mixed timezones, invalid cell ID).
- Golden (`tests/golden.rs`): 10 pass. Compares against fixtures in `testdata/` generated by `testdata/gen_golden.py` running the original Python. Checks IDs, ordering, bboxes, and centroids to 1e-9.
- Invariants (`tests/invariants.rs`): 20 pass. Covers the STM, DRV, and D1 to D7 invariants from the checklist.

Regenerate the fixtures with `python3 -I testdata/gen_golden.py <checkout> <out dir>` inside a venv that has numpy and scipy, where `<checkout>` is a radar-ng checkout from before the Python removal (for example commit `5b3cb39`). The generator stubs `temporalio` and `httpx`. Copy the fixtures into `testdata/` after review.

Lint: `cargo clippy -p radar-storms --all-targets` is clean.
