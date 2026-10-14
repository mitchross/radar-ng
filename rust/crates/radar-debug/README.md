# radar-debug

Live-stack diagnostics behind `radar debug <check>`, plus the offline tile-pyramid comparison behind
`radar parity` (see below). It is a library with one entry point, `run(DebugArgs) -> anyhow::Result<i32>`,
which the `radar` binary calls. It is a port of the former Python debug harness, with the same checks,
human and JSON output, and exit codes (0 healthy, 1 any `fail`). Operator guide:
`docs/debug-harness.md`.

It inspects a **live** stack the way the app does. HTTP checks go through the public serving path
(`radar api` on the tile-server port), so the numbers are client-perceived numbers.

## Commands

| command | what it answers | needs |
|---|---|---|
| `doctor` | everything below, one sweep and a summary line | server, Temporal (unless the check is run elsewhere) |
| `pipeline` | is the data fresh? per-layer staleness against schedule cadence, radar frame completeness over the trailing hour, forecast-horizon collapse, lightning feed age | server URL |
| `api` | are the endpoints fast? p50 and p95 per endpoint against a budget. Tells a *degraded* `/api/health` (503 by design on stale radar) from a *down* one | server URL |
| `tiles` | is the serving path right? real tiles for every manifest layer at z4 to z8: latency, 404 rate, and the `Cache-Control` contract (`max-age=86400` on every tile) | server URL |
| `client` | can a cold phone keep up? replays the app's playback pattern (3x3 viewport, last N frames) against the 420 ms tick and reports worst-frame latency and bytes | server URL |
| `temporal` | is the control plane healthy? every schedule present, unpaused, recently fired, with the last run's result. Failed runs in the last 6 h with the terminal failure message from history. Workflows running more than 2 h. Activities on attempt > 1 with their live heartbeat | Temporal frontend |
| `disk` | is the storage sane? PVC headroom, orphaned `<ts>.tmp` staging dirs, leaked `MRMS_TMP_ROOT` and `HRRR_TMP_ROOT` work dirs, per-palette frame-count skew, and manifest-vs-disk drift (a manifest timestamp with no tiles on disk is a frame every client 404s) | run where the volumes are mounted |
| `watch` | `doctor` on a loop (`--interval 60`, `--skip-temporal`), for watching a deploy or an incident. Ctrl-C exits 0 | — |

Every command accepts `--json`, `--server`, `--temporal-address`, `--namespace`, `--lat` and `--lon`
(probe location, default CONUS centre). The environment supplies the defaults: `RADAR_DEBUG_SERVER`
(`http://localhost:8080`), `TEMPORAL_ADDRESS` (`localhost:7233`), `TEMPORAL_NAMESPACE` (`default`).
The disk checks read `TILE_DIR`, `GRID_DIR`, `STATE_DIR` (through `radar_core::env::Paths`) and
`MRMS_TMP_ROOT`, `HRRR_TMP_ROOT`.

```sh
radar debug doctor --server http://radar.example.com:8080
radar debug temporal --temporal-address temporal-frontend.temporal:7233 --json
radar debug disk                       # inside a pod that mounts /data/*
```

`doctor` skips `disk` unless `TILE_DIR` or `STATE_DIR` exists on the machine it runs on, so a laptop
pointed at a remote server does not print a spurious failure.

## `radar parity`: tile-pyramid parity

`radar parity` is not a live-stack check. It compares two tile roots offline and lives in the `parity`
module; the `radar` binary wires it as a top-level subcommand. It was built to compare the Python and
Rust builds, which is where the flag names come from: `--python-root` is the reference root and
`--rust-root` the candidate.

```sh
radar parity --python-root A --rust-root B                 # default threshold (10% of pixels)
radar parity --python-root A --rust-root B --exact         # same renderer on both sides: must match exactly
radar parity --python-root A --rust-root B --layer radar --palette classic \
    --frame 2026-10-14T06:00:00Z --min-zoom 4 --max-zoom 7 --json
```

| flag | meaning |
|---|---|
| `--python-root`, `--rust-root` | tile roots in the `TILE_DIR` layout: `<root>/<layer>/<palette>/<frame path>/<z>/<x>/<y>.png` |
| `--layer` (alias `--product`), `--palette`, `--frame` | restrict the comparison (each repeatable). `--frame` matches the frame path, its last segment (HRRR's `runs/<run>/<ts>`), or the timestamp in `Z` or `+00:00` form |
| `--min-zoom`, `--max-zoom` | zoom range |
| `--max-diff-pct` | per-frame threshold: percentage of pixels that differ visibly (default 10) |
| `--exact` | threshold 0; tile sets, palette indices and manifest entries must match |
| `--python-manifest`, `--rust-manifest` | `manifest.json` or the state dir holding it. Default: `<root>/manifest.json` when present |
| `--json` | one JSON document (`frames`, `frames_only_python`, `frames_only_rust`, `manifest`, `summary`) |
| `--threads` | decoder threads (default: all cores) |

**Discovery.** Hidden entries are skipped: `.render-publish-locks`, the `.<ts>.tmp-<hex>` staging dirs,
the completion markers. Any `.png` deeper than `<layer>/<palette>/` whose last three components are
`<z>/<x>/<y>.png` is a tile, and the components before them are the frame path, so multi-segment
paths (HRRR) work. Other `.png` files are counted in `summary.ignored_files`. Symlinks are followed.

**Per frame** (each `(layer, palette, frame)` present on both sides):

- tile sets: counts and the first 20 tiles on one side only. A one-sided tile is compared against a
  fully transparent tile, because the renderers only elide fully transparent tiles and a client sees
  a 404 as transparent.
- every PNG decoded to 8-bit RGBA on rayon, whatever its encoding (indexed with PLTE and tRNS, grey,
  RGB, RGBA, 16 bit): byte-identical tiles, visibly identical tiles, identical pixels, largest
  channel delta.
- differing pixels by kind: `alpha_edge` (transparent on one side only), `alpha_only` (same RGB),
  `color`, and `transparent_rgb_only` (both alpha 0, invisible, not counted). `transparency_only` is
  true when no pixel changed colour.
- palette indices, when both tiles are indexed.
- size-mismatched and undecodable tiles (always a failure).

**Manifests** (when both sides have one): `schema_version` and the URL templates, layers on one side,
layer `palettes` and metadata keys, frame timestamps on one side (timing, reported only), and per
common frame its `path`, `palettes`, key set and differing values (values are informational).

**Threshold.** `diff_pct` is visibly differing pixels over every pixel compared in the frame. The default
10% was calibrated on the Python build's legacy RGBA tiles, which sample colour before classifying and are
about 92% pixel-identical to the indexed tiles by design. Pyramids rendered from the same input by the
same renderer should match exactly, so compare them with `--exact` (the Python indexed renderer and Rust matched 585/585 tiles on the
benchmark pyramid, and 675/675 on a live MRMS frame).

**Status and exit code.** A frame is `fail` when it is over the threshold, or has an undecodable or
size-mismatched tile, or (with `--exact`) its tile sets or palette indices differ. Otherwise it is
`warn` when anything differs, else `ok`. Exit 0 when no frame fails (warnings allowed), 1 when a frame
fails or `--exact` finds a manifest entry mismatch, 2 when no frame is present on both sides or a root
cannot be read.

**Speed.** Release build, 2 cores: 2000 tile pairs (256 x 256, RGBA against indexed) in about 1.1 s.

## Public API

```rust
pub async fn run(args: DebugArgs) -> anyhow::Result<i32>;   // the radar debug entry point
pub struct DebugArgs { pub check: DebugCheck }               // clap Args, flattened into `radar`
pub enum DebugCheck { Doctor { .. }, Api { .. }, Tiles { .. }, Pipeline { .. }, Client { .. },
                      Temporal { .. }, Disk { .. }, Watch { .. } }
pub struct Common { server, temporal_address, namespace, json, lat, lon }
```

```rust
// parity (radar parity)
pub fn parity::run(args: &ParityArgs) -> anyhow::Result<i32>;          // print and return the exit code
pub fn parity::compare_roots(python: &Path, rust: &Path, opts: &CompareOptions) -> anyhow::Result<ParityReport>;
pub fn parity::scan_tree(root: &Path, filter: &Filter) -> anyhow::Result<TreeScan>;
pub fn parity::decode_rgba(png: &[u8]) -> anyhow::Result<DecodedTile>;
pub fn parity::compare_rgba(a: &[u8], b: &[u8]) -> PixelStats;
pub fn parity::render_human(report: &ParityReport) -> String;
```

Modules: `model` (checks, status, HTTP client, percentiles, Python-compatible formatting), `checks`
(API, tiles, pipeline, client sim), `disk`, `temporal`. Each module's pure logic is public so tests
can reach it: `checks::pipeline_checks`, `checks::slippy`, `checks::tile_url`, `disk::check_disk`,
`model::percentiles`, `model::age_str`.

## Output

Human output matches the Python: `== section ==`, then `  ✓|⚠|✗ name: detail`, then `N checks · F fail ·
W warn` for `doctor`. The JSON form is one document per section, `json.dumps(indent=2)` with ASCII escapes
and Python float reprs (`1500`, `10.0`, `1e-05`):

```json
{"section": "...", "status": "ok|warn|fail", "checked_at": "...", "checks": [{"name", "status", "detail", "data"}]}
```

`pretty_json` is a Python `indent=2` writer, so a diff against the Python output is a byte diff.

## Tests

```sh
cd rust
cargo test -p radar-debug      # 47 unit tests + 12 parity integration tests, no server, no Temporal
cargo test -p radar-debug --test parity -- --ignored --nocapture   # 1200-pair throughput check
```

- `parity` (unit): CLI flags and `--exact` conflicting with `--max-diff-pct`, tile path splitting
  (including HRRR run paths), the frame filter (`Z` and `+00:00`, path suffix), pixel kinds, non-PNG input.
- `tests/parity.rs`: synthetic pyramids in a tempdir. Identical indexed pyramids pass `--exact`;
  indexed and RGBA encodings of the same image are identical; a missing tile counts against
  transparent (warn under a raised threshold, fail by default and with `--exact`); a missing
  transparent tile fails only with `--exact`; colour and coverage-edge pixels are classified and
  thresholded; a recoloured frame fails the default threshold; frames on one side and the
  nothing-to-compare exit 2; layer, palette, frame and zoom filters; undecodable and size-mismatched
  tiles; manifest comparison (timing vs entry mismatches, malformed manifest); JSON shape. Staging
  dirs, lock dirs and markers are present in every fixture and must be ignored.

- `model`: percentiles (half-even index rounding), ISO parsing, `quote_ts`, `age_str` (including negative
  ages with Python's floor division), exit code, Python reprs, pretty JSON byte layout.
- `checks`: slippy tile at z4, template filling, `pipeline_checks` (the Python `test_pipeline_freshness_logic`
  case, unknown layers, health reasons, zero horizon), and the cache contract constants.
- `disk`: tempdir fixtures for missing mounts, usage, ghost and hidden frames, uneven palettes, stale and
  fresh `.tmp` dirs, malformed manifests (fail closed), state-file age, leaked scratch dirs.
- `temporal`: first-seen ordering of failure counts, Python enum names, the expected schedule set, the stall
  threshold. Live Temporal behaviour is not covered by unit tests (see Deviations and Not verified).
- lib: clap parsing of every subcommand and flag, and `Command::debug_assert`.

## Deviations from the Python

Each is deliberate. Where a Python defect is fixed, it is marked as such.

1. **Tile URL template uses `{path}` (defect fixed).** The live manifest advertises
   `/tiles/{layer}/{palette}/{path}/{z}/{x}/{y}.png`. The Python formatted only `{timestamp}`, so `tiles`,
   `client` and `doctor` raised `KeyError` on the real stack. The Rust fills `{path}` with the frame's `path`
   (defaulting to its timestamp) and still accepts `{timestamp}`.
2. **Schedule set from `radar-types` (defect fixed).** The Python fell back to a hard-coded list of 10 when
   the repo was not on `sys.path`, and that list had already drifted (it lacked `ingest-airquality` and
   `open-meteo-sync-gfs025`). The Rust reads the 12 schedules of `radar_types::schedules::all_schedules()`,
   the same table the seeder uses.
3. **Disk palettes.** The per-layer palette set is the stored `palettes` key (radar-core keeps it verbatim,
   as the Python reader does). If that key is absent, the frames' palettes are used, then `classic`. The
   Python used `["classic"]` in that case.
4. **Deterministic ordering.** Palette and file lists are sorted. The Python followed `iterdir()` order,
   which is arbitrary, so the frame-count dict in `disk.tiles.<layer>` prints palettes in name order.
5. **Disk reads through radar-core.** `Paths::from_env` for the roots and `read_manifest` for the manifest.
   The manifest read fails closed, which reports `disk.manifest` FAIL as the Python did for unparseable JSON.
   `processed_set` and `grid_dump` are not read: no Python check looks at them. Adding a check for them would
   change the output, so it is left for a deliberate decision.
6. **Temporal last-action handle.** The Python described the last action's workflow by its
   `first_execution_run_id`. The Rust API exposes the action's `run_id`, so a continued-as-new chain is
   described at the run that was started. The status line stays the same kind of answer.
7. **Heartbeat detail.** Rendered with Python-style reprs of the decoded JSON value, cut to 120 characters.
   Python's `str()` of a dict is the same for the simple values the ingest heartbeats send.
8. **Transport error labels.** `URLError`, `TimeoutError` and `Error` are the Python exception names that
   `reqwest` errors map onto. The wording after the label is reqwest's, not urllib's.
9. **Request timeout.** 15 s total per request (`reqwest`), where urllib's timeout applied per socket
   operation. For a healthy server the difference does not matter.
10. **Temporal connect timeout.** 10 s on the frontend connection, so an unreachable address fails instead of
    hanging. The Python had no timeout.
11. **Unknown-layer hint.** Points at `crates/radar-debug/src/checks.rs` (where `LAYER_EXPECTATIONS` now lives).
12. **Client sim `--frames 0`.** Replicates the Python `timestamps[-0:]` quirk: zero selects every frame.
13. **`temporal` warning when `temporalio` is missing** is gone. The Rust build has the client compiled in.
14. **Transport.** TLS certificate verification is off for probes, as in the Python (self-signed stacks).

## Not verified

- **Live Temporal.** Tests cover the connect-failure path (FAIL, exit 1) only. Schedule describe, list
  workflows, failure history and pending-activity decoding are not covered by tests.
- **Real PVCs.** The HTTP checks were tested end to end against a fake stack (manifest, tiles with the cache
  contract, the point endpoints). The disk checks are tested against tempdir fixtures only.

## Golden fixtures

None. The output format is pinned by `model::tests::pretty_json_matches_python_indent_2` and the
`python_reprs` and `age_str` tests, which encode the former Python harness's formatting.
