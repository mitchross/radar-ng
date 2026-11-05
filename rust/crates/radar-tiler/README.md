# radar-tiler

The radar-ng tile renderer. It renders one source grid into one Web Mercator XYZ pyramid per palette,
published atomically, with indexed (palette-mode) PNG tiles. Behavior follows the design history in
`docs/archive/rust-migration/specs/spec-tiler.md`; the decisions in `docs/archive/rust-migration/PLAN.md` override it where they differ.

Renderer identity: `"indexed-rs1"` (PLAN option B). The crate never reuses or adopts pyramids
from another renderer, such as the old Python one; see Behavior notes.

## Coverage

| Area | Spec | Status |
|---|---|---|
| Tile math, pixel centers, tile ranges | 3.1 to 3.6 | done (`geometry.rs`) |
| Nearest sampling, bilinear physical sampling (f32 storage, f64 weights) | 4.1, 4.2 | done (`sample.rs`) |
| Max-pooled footprint overview | 4.4, 4.5 | done, `overview = max` only |
| Class model, categorical model, transparent collapse | 5.1 to 5.3 | done (`classify.rs`) |
| Palette loader, validation, shipped JSON | 6 | done (`palettes.rs`, assets embedded) |
| Indexed PNG encode, palette rewrite, chunk iteration | 7.1, 7.3, 7.4 | done (`png.rs`); 7.2 legacy RGBA not implemented |
| Tile emission and elision | 8 | done |
| Render orchestration (`render_frame_palettes`) | 9.2 to 9.5 | done (`render.rs`); no legacy branch |
| Canonical JSON, digests, identity documents | 10 | done (`identity.rs`), renderer `indexed-rs1` |
| Completion marker v2 (write, inspect, tamper detection) | 11 | done (`marker.rs`) |
| On-disk layout, staging names, lock stripes | 12, 13.1 | done (`publish.rs`) |
| Atomic publish decision table, durability order | 13.2 to 13.8 | done with per-palette collision rule (see Behavior notes) |
| Environment parsing | 14 | done (`env.rs`), pure functions, no `set_var` |
| Geometry cache, single flight, negative caching | 15.1 | done (`cache.rs`) |
| rayon tile parallelism, per-thread scratch | 15.3 | done (`render.rs`, `map_init`) |

## Public API

- `render_frame_palettes(grid, kind, palettes, output_dirs, zoom_levels, options)` returns
  `Result<MultiPaletteRenderResult>`. This is the production entry point.
- `FrameKind::{Continuous { nodata_value, min_valid_weight }, Categorical { category_map }}`
- `FrameOptions { tile_size, source_id, publication_lock_root, overview, png_compress_level, types }`
  (`Default`: tile 256, source `unspecified-frame`, overview off, level from
  `TILE_PNG_COMPRESS_LEVEL`, no types). `Overview::Max` is the only mode. `types` is a per-cell
  `TypePlane` (rain, snow, mix) that picks each pixel's ramp from the palette's `types` (`typed.rs`).
- `PublishStatus::{Created, ExistingValid, ExistingForeign, Empty}`, `PalettePublishOutcome`,
  `MultiPaletteRenderResult` (`counts`, `rendered_palettes`, `outcome`).
- `build_class_model`, `build_categorical_class_model`, `ClassModel`, `PaletteLut`.
- `load_palette`, `palette_names`, `ColorTable`.
- `encode_indexed`, `replace_palette`, `decode_indexed`, `png::iter_chunks`.
- `complete_pyramid_identity`, `is_complete_pyramid`, `frame_identity_is_compatible`,
  `PyramidIdentity`, `RENDERER`.
- `resolve_tile_renderer`, `tile_renderer_for_role`, `TileRenderer`.
- `tile_geometry_cache_stats`, `clear_tile_geometry_cache`.
- `packed` (`docs/reference/packed-tiles.md`): `ValueSampler` (physical values per tile, same routes as the
  colored tiles), `sample_tiles`, `Quantizer`, `slot_position`, `atlas_dims`, and `pack_frames`, which
  merges slots into a bucket under `<layer dir>/.packed.lock` and writes `.packed-bucket-v1.json` last.
- Errors: `TilerError::{Invalid, Conflict, Unbound, Corrupt, Io}`. Callers branch on the
  variant. `Unbound` means a pre-marker tree sits at a final path. The crate does not adopt it.

## Behavior notes

These items record where the crate departs from the spec or from the old Python renderer that produced the golden fixtures.

1. **No legacy RGBA renderer.** `render_tiles`, `apply_color_table`,
   `apply_categorical_color_table`, `render_tiles_atomic` and the `rgba-bilinear-v1`
   identity are not implemented. `TileRenderer::Legacy` exists only so role routing can
   name the legacy renderer.
2. **Renderer identity `indexed-rs1` (PLAN option B).** A complete `indexed` pyramid
   from the old Python renderer is a foreign winner, not a reusable one.
3. **Collision rule (PLAN).** For each palette, a final with a complete marker of any
   identity is `ExistingValid` (exact match) or `ExistingForeign` (anything else). The result reports the tile
   count of the existing tree. Foreign trees are never rewritten, and no
   `PyramidIdentityConflict` is raised. The spec's rules for mixed or partial palette sets
   (13.2) are therefore not implemented. The crate renders the absent palettes.
   `existing_compatible` is never returned.
4. **No adoption of unbound trees (H6).** A non-empty final without a v2 marker (including
   a v1 `.render-complete.json`) fails with `TilerError::Unbound` before anything is
   rendered or renamed. The crate never adopts such a tree.
5. **Frame model over the full palette set (H3).** Class numbering uses every
   palette of the frame, including the ones already bound. The old Python overview path
   built it from the staging subset only, which renumbers classes. This crate does not.
6. **Overview modes.** Only `max` exists, as the `Overview` enum. The old Python `"mean"` error
   case cannot be expressed.
7. **Value type.** Continuous values are `f32` (`Grid` storage). The f64 classify variant of
   the checklist is not ported. Spec 4.2 casts values to f32 before sampling anyway.
8. **Categorical source digest.** The categorical source digest hashes the `f32` storage
   (`<f4`), not the caller's integer dtype. Categorical identities therefore do not match
   the old Python digests. They are foreign to Python in any case.
9. **Projected geometry uses `radar_geo`'s spherical formulas, not pyproj (spec 19.2).**
   Transforms may differ from pyproj by about one ULP, which can flip a rare nearest or
   classify edge case. The golden projected fixture is EPSG:3857 only. The HRRR
   Lambert-conformal path has no golden fixture yet.
10. **zlib bytes.** PNG bytes come from `flate2` at the configured level with adaptive
    row filters. Byte parity with Pillow is not a goal (spec 7.1). Decoded class indices
    match the Python-generated fixture exactly on every golden fixture.
11. **Shared IDAT across palettes.** The tile is encoded once, and each palette file is
    `header + PLTE/tRNS + shared IDAT/IEND`. These are the same bytes the old Python
    `replace_palette` path produced, with fresh CRCs.
12. **Footprint allocation.** The overview path allocates two small vectors per tile in the
    reduction helpers. The native paths reuse per-worker buffers.
13. **Geometry cache env.** The process cache sizes itself from
    `TILE_GEOMETRY_CACHE_MB` and `TILE_GEOMETRY_CACHE_ENTRIES` on first use
    (`OnceLock`). Later environment changes do not resize it.
14. **Publish staging.** The crate creates the final's parent durably before rendering, so the
    staging sibling can be renamed into place. It removes staging on every failure path.
    Renamed palettes are not rolled back (13.7).
15. **Categorical frames with `overview`** fail up front, as the spec requires.
16. **Anyhow.** The crate does not use it. All errors are `TilerError`, which implements `std::error::Error`
    and converts into `anyhow::Error` at call sites.

## Checklist (spec section 17, 79 items)

Ported directly: 46. Ported with the expectation adapted to the no-adoption, per-palette
or reporting behavior: 19 (each adapted test says so in a comment). Covered by unit tests
only: 2. Skipped: 12.

Skipped, with the reason:

- **Legacy RGBA renderer (6):** `test_apply_color_table`, `test_render_tiles_creates_files`,
  `test_render_tiles_accepts_projected_source_grid`, `test_single_cell_core...[legacy]`,
  `test_legacy_renderer_is_a_marker_validated_rollback_path`,
  `test_rollback_reuses_full_indexed_winner_without_claiming_legacy`.
- **f64 classify inputs (1):** `test_class_model_matches_legacy...[float64]`.
- **Fault injection, no hooks (5):** `test_publish_orders_tile_marker_and_parent_directory_durability`,
  `test_retry_finishes_parent_sync_after_post_rename_failure`,
  `test_regular_file_fsync_errors_are_never_suppressed`,
  `test_atomic_eexist_requires_winner_completion_marker` (needs a race),
  `test_atomic_eexist_exact_pre_marker_winner_is_adopted` (adoption, H6).

Adapted items (the behavior is checked, the expectation differs): the adoption tests
(`adopts_exact_pre_marker`, `rejects_visually_equal...`, `rejects_partial...`,
`exact_unmarked_pyramid_is_adopted...`, `partial_unmarked...`, `schema_v1...` x2,
`incomplete_existing_dir...`) now expect `Unbound`; the foreign-identity tests now expect
`ExistingForeign`; `partial_foreign_renderer_set_fails_closed` now renders the absent
palette; the symlink lock test now expects an overlap error; `atomic_helpers_allow_...supersets`
now checks that extra palettes are ignored; the pyproj-skip test checks that a repeated
frame adds no misses.

The continuous, categorical
and projected reference tests (`tests/golden.rs`) use Python-generated fixtures, not a live SciPy call. Cleanup on failure is covered by a unit
test for the single-palette case only.

## Tests

`cargo test -p radar-tiler` (from `rust` with `env.sh` sourced):

- unit (`src/`): 65 passed
- `tests/golden.rs`: 4 passed (reflectivity x3 palettes, categorical x3, projected EPSG:3857, overview max)
- `tests/checklist.rs`: 29 passed
- `tests/cache.rs`: 4 passed (single process-global test sequence plus bounded and single-flight checks)
- `tests/bench.rs`: 1 ignored (micro-benchmark)

Total: 102 passed, 0 failed, 1 ignored.

Benchmark (release, 2 cores): 3500 x 7000 MRMS-like grid, zooms 4 to 7,
palettes classic/muted/vivid, 615 tiles per palette, 1.59 s wall. Run with:

    cargo test -p radar-tiler --release --test bench -- --ignored --nocapture

## Regenerating the golden fixtures

`testdata/gen_golden.py` imports the Python reference from a radar-ng checkout from before the Python removal (for example commit `5b3cb39`) and writes
`testdata/`. It never writes into that checkout:

    python3 -m venv <venv>
    <venv>/bin/pip install numpy Pillow scipy pyproj
    PYTHONDONTWRITEBYTECODE=1 <venv>/bin/python testdata/gen_golden.py /path/to/old-checkout

`assets/` holds the palettes (`classic`, `muted`, `vivid`), embedded with
`include_str!`; they are the only palette source.

## TODO

- Fault-injection hooks for the fsync order and the EIO/EINVAL/ENOTEMPTY publish paths, so
  the old Python durability and race tests can be ported.
- A golden fixture for the Lambert-conformal (HRRR) projected path.
- HRRR real-data parity. The fixtures are synthetic. (MRMS was checked against live data with
  `radar parity --exact`, see `radar-grib/README.md`.)
