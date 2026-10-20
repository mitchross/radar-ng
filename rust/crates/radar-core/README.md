# radar-core

The radar-ng data-plane library (ported from the former Python `backend/shared/`). It covers file locking, durable writes,
environment parsing, JSON logging, the frame manifest, the processed-frame set, the grid store and reader,
push-token storage, the nowcast skill log, and the blocking-work heartbeat.

Out of scope for this crate: the tiler, palettes and PNG output (`radar-tiler`), and the storm tracker and
storm prefetch (`radar-storms`). Spec sections 6 to 8 are not implemented here.

The source of truth is `docs/develop/specs/spec-shared.md`. Invariant IDs (MAN-, GRD-, NSK-, HBT-,
DRV-) refer to its section 13 and are cited in the test names.

## Modules

| Module | Spec | Contents |
| --- | --- | --- |
| `pycompat` | 15 | `Json` value, `dumps` (Python `json.dumps` styles, `ensure_ascii`, `NaN`/`Infinity`), `float_repr` (Python `repr(float)`), `py_int` and related coercions |
| `time` | 15.2 | `isoformat_utc`, `parse_iso`, `parse_iso_as_utc`, `canonical_utc_timestamp`, `round_half_even`, `round_digits` |
| `env` | 2 | `var`, `int_var`, `float_var`, `bool_var`, `path_var` (strict parsing; empty means default), `Paths` |
| `fsutil` | 1 | `FlockGuard` (exclusive, shared), `atomic_write`, `durable_mkdir`, `fsync_dir` |
| `logging` | 11 | `init(service)`, JSON-lines `FormatEvent`, `render_line`, `retry` with backoff |
| `manifest` | 3 | v2 manifest read, parse (fail-closed), `add_frame`, `remove_frame`, `replace_layer`, v1 migration |
| `processed_set` | 4 | Bounded, durable set of processed keys (`ProcessedSet`) |
| `grid_dump` | 5 | Byte-exact grid writer, generations, retention (`cleanup_old_grids`, `prune_grid_layer`) |
| `grid_read` | 5, 9.2 | Metadata and `.bin` reader, bilinear sampler (`GridDump::sample`) |
| `push_tokens` | 9 | SQLite push-token store (`PushTokenStore`), schema as in the Python |
| `nowcast_skill` | 10 | Skill log append (24 h window), `summarize`, `prometheus_lines` |
| `heartbeat` | 12 | `run_blocking_with_heartbeat` for blocking work on tokio |

## Public API summary

- `env::Paths::from_env()` reads `STATE_DIR`, `GRID_DIR` and `TILE_DIR` (defaults `/data/state`,
  `/data/grids`, `/data/tiles`).
- `logging::init("service-name")` installs the stdout JSON logger at `LOG_LEVEL` (default `INFO`).
- `manifest::{read_manifest, add_frame, remove_frame, replace_layer, render_manifest}`; every writer takes
  `flock` on `manifest.lock`, then writes atomically.
- `processed_set::ProcessedSet::{open, contains, add, update, items, flush}`.
- `grid_dump::{GridConfig, GridInput, write_grid, finalize_grid_generation, prune_grid_layer,
  cleanup_old_grids}`.
- `grid_read::{read_meta, read_grid, GridDump::{at, sample}}`.
- `push_tokens::PushTokenStore::{upsert, delete_by_token, delete_for_user, list_for_user}`.
- `nowcast_skill::{SkillConfig, append_entries, read_log, summarize, prometheus_lines, ratio}`.
- `heartbeat::run_blocking_with_heartbeat(f, interval, beat)`.

Errors are `thiserror` enums per module (`ManifestError`, `GridError`, `GridReadError`, `SkillError`,
`ProcessedSetError`, `PushTokenError`, `EnvError`). `anyhow` is used only at `logging::init`.

## Tests

`cargo test -p radar-core` runs the unit tests and the golden replays. Golden replays compare Rust output with
files produced by the Python originals, byte for byte where the format is byte-exact.

- `manifest`: MAN-01..10, the malformed-manifest fail-closed cases, and a golden replay of a v1 manifest.
- `grid_dump`: GRD-01..13, DRV-01..04, fault injection for directory fsync (GRD-04..06), the layer-lock
  blocking test (GRD-11), and a byte-exact replay of seven write and finalize scenarios.
- `grid_read`: sampler arithmetic, the NW-corner fill quirk, NoData, out-of-bounds and degenerate grids, and
  metadata and short-read errors.
- `nowcast_skill`: NSK-01..07, DRV-10, and a golden replay (log, summary, Prometheus text, 600 ratio cases).
- `logging`: golden lines, reserved-key prefixing, the `exc` slot, a live subscriber, and the retry schedule.
- `processed_set`, `pycompat`, `push_tokens`: golden replays of the flushed file, float reprs, JSON string
  escapes, and the SQLite schema text.
- `heartbeat`: HBT-01, plus panic isolation for the beat and the work.
- `time`, `env`, `fsutil`: unit tests for each helper.

## Deviations from the Python

These are deliberate. Each is also a Python defect fixed in the port, or an explicit choice where the spec
allows one.

1. **Grid shape and stride.** The stride loop uses floor division (`(h // s) * (w // s) <= cap`), as Python
   does. The stored shape is `ceil(n / stride)` (numpy's `[::s]` behaviour). Python's shape and the cap
   check disagree at non-multiple sizes.
2. **Non-finite coordinates.** NaN or infinite latitudes or longitudes are rejected with `InvalidInput`.
3. **Fill handling.** Fill is stored as given. The Python `fill or NaN` coercion is not replicated, so a fill of
   `0.0` stays `0.0`. NaN cells are rewritten to `-9999.0` only when the fill is NaN (D10).
4. **Run-directory sweep (D11).** `cleanup_old_grids` also removes `runs/<generation>` directories older than
   `GRID_MAX_AGE_S`, complete or not.
5. **Processed set (D9).** Duplicates are collapsed on load. Non-string items are dropped. A failed flush
   rolls back the in-memory state. `max_entries = 0` is an error (`ZeroCapacity`).
6. **Logger reserved keys (D13).** Extra fields named `ts`, `level`, `service`, `msg` or `exc` are written as
   `x_<name>`, so they cannot overwrite the reserved fields.
7. **Nowcast skill log (D15, F4).** `append_entries` takes an exclusive `flock` on the sidecar
   `nowcast-skill.lock` in the state directory. The sidecar is opened read-write, as `manifest.lock` is. The
   directory-lock version failed on NFS with `EBADF`, because a directory cannot be opened for writing. Deviation
   from the earlier "no extra file" rule: the state directory now holds `nowcast-skill.json` and
   `nowcast-skill.lock`. NSK-02 and the golden test assert both names. A lock failure is logged at `error` and
   returned, never ignored. A corrupt existing log is an error and is not overwritten. `read_log` still returns an
   empty log for readers. The directory-lock helper (`FlockGuard::exclusive_dir`) is removed.
8. **Timestamps (D19).** Manifest and skill timestamps are canonicalised to UTC `+00:00` form, or rejected
   when naive or in a non-UTC offset.
9. **Malformed manifests fail closed (PLAN D5 / spec D7).** A malformed frame or layer is an error for
   writers. Python silently dropped it.
10. **Manifest defaults.** Missing path fields are filled on read. Metadata keys `frames`, `timestamps`,
    `latest` and `palettes` are ignored when merging metadata, because they are derived.
11. **`add_frame` timestamp.** `add_frame` takes its timestamp from the `timestamp` argument. The
    `FrameSpec.timestamp` field is ignored.
12. **Sampler edge cases.** Grids with fewer than two rows or columns, or a zero-width extent, return
    `GridReadError::Degenerate`. Python raises or reads out of range. NaN coordinates return
    `Sample::OutOfBounds`. Python raises `ValueError`.
13. **Numeric coercion.** Stored numeric fields in the skill log are coerced with `py_int`, and a missing or
    non-numeric count counts as zero (`py_int_or_zero`).
14. **Strict environment parsing.** Invalid values fail startup with `EnvError`. Empty means "use the default".
    Booleans accept `1/true/yes/on` and `0/false/no/off` only. An unknown `LOG_LEVEL` fails `init`.
15. **Push-token primary key (D14).** `token` remains the global primary key, as in Python. Re-registering a
    token under another user moves it. This is documented, not changed.
16. **Float repr tie rule.** Python's `repr` breaks ties between equally short round-trip strings toward the
    even digit. Rust's shortest formatter can pick the other. `pycompat::float_repr` re-formats at the same
    width with exact rounding and keeps that string when it round-trips. Found by the golden float replay.

## Not implemented or deferred

- The storm tracker, storm prefetch and storm watch (`radar-storms`), and the tiler, palettes and PNG
  (`radar-tiler`). Their invariants (DRV-05..09) are not tested here.
- No grid writer for storms.
- `radar-geo` is not a dependency. Nothing in the in-scope modules needs it.
- `retry` is synchronous only. Async callers need their own backoff.

## Golden fixtures

`testdata/gen_golden.py` writes every fixture under `testdata/`. It loads the Python modules by file path from
`backend/shared` of a radar-ng checkout from before the Python removal (for example commit `5b3cb39`), passed as `RADAR_NG_SHARED`. The
script needs numpy, sets `TZ=UTC`, and uses fixed datetimes and uuids so the output is deterministic.

Regenerate from `crates/radar-core`:

```sh
RADAR_NG_SHARED=/path/to/old-checkout/backend/shared \
  <venv>/bin/python testdata/gen_golden.py
```

The venv needs numpy. After
regenerating, run `cargo test -p radar-core`. Any change in a fixture is a change in the Python behaviour and
should be reviewed before the Rust side is changed to match.
