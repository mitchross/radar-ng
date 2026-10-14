# radar-ng before/after benchmark

Measures the original Python backend against the Rust port on identical inputs, on the same
machine. Five benchmarks, all run: tile render, GRIB decode, storms, nowcast and API load. Results go to `results/<UTC timestamp>.md` (tables) and `.json` (raw
per-iteration records, machine info, verification).

Headline run: `results/20261014T063808Z.md` (also `results/latest.md`). It is the idle-machine rerun of
benches 1 to 4 and the API load run `results/load/20261014T062447Z/`. The earlier full run
`results/20261014T044324Z.md` was made under heavy contention from other builds on this VM; it is kept
for reference, and its ratios are not used.

## What is compared

Each benchmark has a Python runner and a Rust runner that read the same bytes:

| Benchmark | "Before" (Python, production code) | "After" (Rust crate) |
|---|---|---|
| 1. Tile render, 3500 x 7000 MRMS grid, zooms 4 to 7, palettes classic/muted/vivid | `backend.shared.tiler.render_frame_palettes` with `renderer="legacy"` (production default, `TILE_RENDERER=legacy`), and with `renderer="indexed"` (the algorithm the Rust port implements) | `radar_tiler::render::render_frame_palettes`, renderer `indexed-rs1`, rayon |
| 2. GRIB2 decode | `backend.ingest_mrms.activities._download_and_decode_sync` (pygrib, gunzip to a temp file) plus the caller's south-up flip; multi-message files through pygrib | `radar_grib::mrms::decode_gz` + `ensure_south_up`; `decode_message` per message |
| 3. Storms and tracking, two frames | `backend.shared.storms.detect_storms` with `previous` | `radar_storms::detect_storms` with `previous` |
| 4. Nowcast, 4 frames, 12 leads | `backend.nowcast.activities._run_nowcast` (pysteps 1.21.5 LK on a stride-2 stack, scipy upsampling, S-PROG with the fast CDF match) | `radar_nowcast::estimate_motion` (stride 2) + `radar_nowcast::forecast` |
| 5. API load, same fixture and request mix | FastAPI app `backend/api/api/server.py` under uvicorn (`--no-access-log`, as `start.sh`), with a StaticFiles mount standing in for Caddy | `radar_api::serve` (`bench_api_server` binary), which serves `/tiles` itself |

The Python side imports the production modules from the read-only tree (`RADAR_NG`). Nothing
is copied and nothing is written into it. Only `_write_nowcast_status` is stubbed in the nowcast runner, because it writes to `/data`;
it is not part of the forecast computation and is not timed.

The "legacy" tile row is the honest "before" for the deployed system. The Rust port only
implements the indexed algorithm, so "Python indexed" is the like-for-like comparison. The
difference between the two Python rows is the algorithm change, and the difference between
"Python indexed" and Rust is the implementation change (language, rayon, memory layout).

## Layout

```
bench/
  run.sh                    runs everything available, verifies, writes results/
  py/common.py              timing, RSS, percentile helpers (same rules as the Rust side)
  py/gen_inputs.py          deterministic synthetic inputs and the GRIB file (seeded)
  py/bench_tile.py          tile runner (Python)
  py/bench_grib.py          GRIB runner (Python)
  py/bench_storms.py        storms runner (Python)
  py/bench_nowcast.py       nowcast runner (Python)
  py/verify.py              cross-checks Python and Rust outputs -> work/raw/verify.json
  py/report.py              -> results/<ts>.json and results/<ts>.md
  py/verify_noaa.py         real-NOAA check for rust/scripts/preflight.sh step 4: compares the production
                            Python decode with `noaa_dump dump`, renders MRMS with the indexed renderer
  py/requirements.txt       Python dependencies (versions as in production where they overlap)
  load/                     API load (bench 5): make_api_fixture.py (tiles, manifest, nowcast grids),
                            mix.py (oha URL list), run_load.sh (driver, core pinning),
                            sample_proc.py (/proc CPU and RSS), py_server.py (FastAPI + /tiles stand-in),
                            parity_check.py (both servers return the same content),
                            summarize_load.py, requirements.txt, K6_CLUSTER.md
  work/                     generated inputs and scratch outputs (gitignored)
  results/                  reports (kept)
  .venv/                    Python environment for benches 1-4 (gitignored, created by you)
  .venv-api/                Python environment for bench 5: backend/api/api/requirements.txt (gitignored)
crates/radar-bench          Rust runners: src/lib.rs (timing), src/bin/bench_{tile,grib,storms,nowcast,api_server}.rs,
                            src/bin/noaa_dump.rs (fetch and dump for verify_noaa.py)
```

## Running it

### Here (this sandbox, 2 cores)

```
python3.12 -m venv /path/to/venv && /path/to/venv/bin/pip install -r bench/py/requirements.txt
BENCH_PYTHON=/path/to/venv/bin/python bench/run.sh
```

`BENCH_ENV` is sourced before cargo (it sets the proxy and the shared target directory here).
Run it on an otherwise idle machine. `run.sh` samples the 1-minute load every 5 s, marks each
step in `work/raw/steps.log`, and prints the load seen during each section of the report.

Smoke test (one timed iteration, no production nowcast): `BENCH_QUICK=1 bench/run.sh`.
Skip groups with `BENCH_SKIP=tile,grib,storms,nowcast`. Bench 5 is not part of `run.sh`; see below.

### On the real hardware or cluster

1. Build the Rust side with a release profile on the target machine: `cargo build --release -p radar-bench`.
2. Create a Python 3.12 venv with `bench/py/requirements.txt`, the same Python used in the
   production image (`backend/api/Dockerfile`, `backend/nowcast/requirements.txt`).
3. Point `RADAR_NG` at the checkout of the original repository and run `bench/run.sh`.
4. Run the "before" and "after" on the same host, one after another, with no other
   workload. For the API, run the cluster load (`load/K6_CLUSTER.md`) against the Python
   deployment, then against the Rust one, in the same time window and with the same pod count.
5. Keep the `results/<ts>.json` files. They hold the machine description (CPU model, logical
   CPUs, memory, kernel) so the numbers can be compared with the production hosts later.

Set `BENCH_WORK` to put the 300 MB of inputs and the scratch trees on a disk with space.

### API load (bench 5)

```
python3.12 -m venv bench/.venv-api
NO_PROXY=localhost,127.0.0.1,::1 bench/.venv-api/bin/pip install -r backend/api/api/requirements.txt numpy==1.26.4 Pillow
cargo build --release -p radar-bench --bin bench_api_server          # from rust/, env.sh sourced
bench/.venv-api/bin/python bench/load/make_api_fixture.py            # needs bench/work/inputs and bench/work/out/nowcast
bench/load/run_load.sh                                               # ~5 min; writes results/load/<ts>/
bench/.venv-api/bin/python bench/load/summarize_load.py bench/results/load/<ts>/
```

`oha` is needed too (installed at `$(which oha)`; elsewhere
`cargo install oha --locked --root <dir>`). `run.sh` picks the newest load run up into section 5 of the report.

### Real NOAA files (pre-cutover check)

The benchmarks use synthetic inputs. The comparison on real MRMS, HRRR and NAQFC files is step 4 of
`rust/scripts/preflight.sh` (run it with `--only noaa`). It uses the same venv (`bench/.venv`, Python
3.12, `py/requirements.txt`) and caches downloads in `work/noaa/`. By hand:

```
target/release/noaa_dump fetch --cache bench/work/noaa --naqfc --json fetch.json   # radar-grib network code
target/release/noaa_dump dump --mrms <gz> --hrrr <grib2> --hrrr-idx <idx> [--hrrr-subset <f>] --out out/rust
RADAR_NG=. bench/.venv/bin/python bench/py/verify_noaa.py compare --rust-dir out/rust --mrms <gz> \
  --hrrr <grib2> --hrrr-idx <idx> --fetch-json fetch.json --json verify.json
RADAR_NG=. bench/.venv/bin/python bench/py/verify_noaa.py render --mrms <gz> --out out/tiles-py
target/release/radar render-once <gz> out/tiles-rs/radar
target/release/radar parity --python-root out/tiles-py --rust-root out/tiles-rs --exact
```

`compare` prints one PASS/FAIL/WARN/SKIP/INFO row per check (radar-grib README "Must verify against
real NOAA files", items 1 to 10 and 12) and exits 1 on any FAIL. `make-idx` writes a wgrib2-style
`.idx` for a local GRIB2 file, so the comparison can run offline on the test fixtures
(`preflight.sh --only noaa --local-mrms <gz> --local-hrrr <grib2>`).

## Bench 5 results (API load)

Run `results/load/20261014T062447Z/summary.md`, produced by `run_load.sh`. Each cell is one
30-second run. Latency in ms. CPU is the server's own utime plus stime over the run, as a percentage
of its one pinned core. Requests are the local-endpoint mix, 95% tiles.

| target | run | achieved rps | success | p50 | p95 | p99 | server CPU % of core | CPU ms/req | RSS peak MB |
|---|---|---:|---:|---:|---:|---:|---:|---:|---:|
| FastAPI + StaticFiles | 200 rps | 200.4 | 100% | 1.86 | 3.54 | 8.23 | 44.0 | 2.27 | 70.5 |
| FastAPI + StaticFiles | 500 rps | 493.9 | 100% | 83.88 | 172.25 | 209.77 | 94.2 | 1.98 | 77.2 |
| FastAPI + StaticFiles | 1000 rps | 571.7 | 100% | 106.65 | 160.79 | 193.80 | 95.6 | 1.73 | 78.0 |
| FastAPI + StaticFiles | max (64 conn) | 538.0 | 100% | 111.88 | 181.91 | 214.13 | 94.9 | 1.83 | 78.5 |
| radar-api | 200 rps | 200.5 | 100% | 0.37 | 0.63 | 1.90 | 4.6 | 0.24 | 11.7 |
| radar-api | 500 rps | 500.0 | 100% | 0.38 | 0.76 | 2.23 | 11.1 | 0.23 | 12.1 |
| radar-api | 1000 rps | 1000.0 | 100% | 0.31 | 0.61 | 2.55 | 16.7 | 0.17 | 12.5 |
| radar-api | max (64 conn) | 13039.4 | 100% | 4.05 | 6.72 | 45.81 | 94.9 | 0.08 | 14.1 |

No non-200 responses on either side. Requests still in flight when the 30 s window closed are
oha's "aborted due to deadline" count (at most 64 per run) and are not in the success figure.

Reading it: the Python stand-in reaches one core's limit somewhere around 400 to 550 rps on this mix
(94 to 96% CPU at the 500 and 1000 rps steps and at max). 500 rps sits on the knee: that step had p50
3 ms in the earlier run (`20261014T061843Z`) and 84 ms in the headline run, so read the Python latency
at 500 rps and above as a saturated queue rather than a fixed figure. The
radar-api server uses 17% of its core at 1000 rps with p99 2.6 ms, about 10 times less CPU per request
at a fixed rate (0.17 to 0.24 ms against 1.7 to 2.3 ms), and about 24 times less at max. Its RSS is
about 12 to 14 MB against 70 to 78 MB. The Python-side figures include the tile work the production
stack hands to Caddy (see caveats), so they are an upper bound on the Python API cost, not a measure
of it.

Parity: `load/parity_check.py` requests the same 300 unique URLs from both servers. All 284 tile
responses are byte-identical, and all 16 JSON responses (the manifest, the nowcast points and
`/api/livez`) are equal after removing the per-request `updated_at` field (`parity.txt` in the run
directory).

### How the fixture and the mix are built

- `load/make_api_fixture.py` writes one served tree: `tiles/radar/<palette>/<ts>/<z>/<x>/<y>.png`
  for classic, muted and vivid at zooms 4 to 7, from the Rust tile runner on the bench 1 input
  (4,680 PNGs for eight frames, hardlinked to one render); `state/manifest.json` in the production
  schema (eight radar frames two minutes apart and a nowcast layer with three leads at 5, 10 and
  15 minutes); and `grids/nowcast/runs/<run>/<ts>.*` written through the production
  `backend.shared.grid_dump.write_grid` from the bench 4 Rust forecast at 875 x 1750.
- `load/mix.py` builds the oha URL list from the manifest. It follows the k6 script (`load/k6-radar.js` in the original repository)
  `./load/k6-radar.js` (widget and app sessions, playback tiles at zoom 6, manifest and nowcast point
  requests), with two changes. The upstream routes (forecast, alerts, geocode) are left out
  because they call Open-Meteo, NWS and Photon, which this sandbox cannot reach (`--with-upstream`
  adds them back). Tile URLs with no file in the served tree are dropped (`--tiles-root`). The
  result is 300 unique URLs and 3,845 list lines per 100 sessions, 95.3% tiles.
- Both servers get the same manifest, tree and grids (`STATE_DIR`, `TILE_DIR`, `GRID_DIR`). The
  Python server is `load/py_server.py` (the production `backend.api.api.server.app` with a
  StaticFiles mount at `/tiles`), run by uvicorn with one worker and `--no-access-log`. The Rust
  server is `bench_api_server`, which calls `radar_api::serve(Config::from_env())`.
- Pinning: the server runs under `taskset -c 0`; oha and the `/proc` sampler run under
  `taskset -c 1`. Each run is 30 s at a fixed rate (`-q`, 64 connections) or at max (`-c 64`); the
  sampler reads utime, stime, VmRSS and VmHWM once a second.

## What each measurement is

- **Wall time** is `perf_counter` around one call, in process. The warmup iterations are not in
  the summary; the first warmup call is reported as "cold".
- **CPU time** is user plus system time of the whole process (`getrusage(RUSAGE_SELF)`), so rayon
  threads are included. A value above the wall time means parallel work.
- **Peak RSS** is `VmHWM` from `/proc/self/status` at the end of the run. For Python it includes
  the interpreter and numpy/scipy (the "after load" RSS in the JSON shows the baseline).
- **Median and p90** are over the timed iterations. p90 uses linear interpolation, so with 5
  samples it sits close to the maximum.
- Each runner runs in a single process: warmup, then timed iterations. The geometry cache
  (tiler) and the warm caches are therefore warm for the timed iterations, which matches a
  long-lived worker.

Iteration counts (headline run): tile 1 warmup + 5 timed per renderer; GRIB 2 + 20 on the fixtures
and 1 + 5 on the 3500 x 7000 file; storms 1 + 5; nowcast 875 x 1750 1 + 3; nowcast 1750 x 3500
**0 + 1 per side** (one Python iteration takes minutes and the process holds several GB, so the
production-size run is capped to one iteration; the table says so).

## Verification

`verify.py` runs after the timed runs and writes `work/raw/verify.json`:

- Tile: the Python indexed tree and the Rust tree have the same file set, and every tile of
  every palette decodes to identical palette indices and identical RGBA. The legacy RGBA tree is
  compared by pixel only (informational; it samples colour first, so it differs by design).
- Storms: per-cell ids, pixel counts, peaks, areas, bboxes and centroids for both frames.
- Nowcast: per-lead correlation, mean and largest absolute difference, and 5 dBZ rain-mask
  agreement. The Rust motion field re-implements OpenCV's LK and Shi-Tomasi, so the fields are
  close (mean difference of a few hundredths of a dBZ), not bit-identical.
- GRIB: sha256 of the little-endian float32 values (and the axes for MRMS), per message for the
  multi-message file.
- Assets: the palette JSON embedded in `radar-tiler` against the production files.

## Caveats

- **Shared machine, two cores.** The sandbox VM is shared with other jobs (builds and tests).
  The first full run (`20261014T044324Z`) ran with a 1-minute load of 2.5 to 6.2 on 2 CPUs from
  other agents' builds; its wall times are inflated and it is kept only as a reference. The
  headline run is `20261014T063808Z`, started when the load was 0.2 and nothing else was using the
  CPU. Its load during the timed sections was 1.1 to 1.5 (the benchmark's own single-threaded work
  plus rayon). Per-section load is in the "Load during these runs" lines. CPU time is reported
  alongside wall time because the Rust runs use two rayon threads.
- **CPU versus wall.** The CPU/wall ratio shows how much parallelism each side used. In the latest
  run the Python tile runs used about 0.7 to 0.9 of one core on average; the Rust tile run used
  about 1.5 cores. The production host's core count is not known here.
- **Source state.** The Rust crates were being edited by other work during the run. The
  provenance block in the report lists the git head and the modified files; `radar-nowcast`
  had uncommitted changes at build time, so the nowcast Rust numbers measure that state.
- **Synthetic data.** The fields are generated (60 moving Gaussian cells, seed 20261014), not
  real MRMS or HRRR. NOAA is not reachable from here. Real data has more coverage, noise and
  larger bright areas, so tile and storm times will differ.
- **Legacy tile output differs from the indexed output.** The legacy renderer writes RGBA tiles
  and emits a different set of tiles (648 vs 585 here) because it does not elide the same tiles.
  Size and count comparisons therefore compare the production output with the Rust output.
- **Memory is not always lower in Rust.** Tile render: Rust peak RSS equals the Python indexed
  path (about 200 MB, dominated by the 98 MB input grid), and it is about half the production
  legacy path. GRIB decode of the 3500 x 7000 file: Rust peaks higher (about 590 MB against 437 MB
  for Python, headline run). The Rust decode holds the gunzipped message and the decoded arrays; this was not
  profiled further and is a candidate for a follow-up in `radar-grib`.
- **Python GRIB includes file IO.** The production path writes the gunzipped GRIB and the
  message to a temporary directory and then removes it; the Rust decode is in memory. Both are
  what each side actually does. The 35 x 70 fixtures are dominated by per-call overhead.
- **Eccodes and pygrib.** pygrib 2.1.8 is the production decoder. The synthetic file uses 16-bit
  `grid_png` without a bitmap, because bitmap-coded grid_png did not encode at this size with
  the installed eccodes (2.49.0); the round trip is verified during generation.
- **Nowcast is not bit-exact.** pysteps uses OpenCV's LK; the Rust port re-implements it (see the
  radar-nowcast README). The fields correlate above 0.999 and the mean difference is a few
  hundredths of a dBZ, but single cells differ by up to about 17 dBZ.
- **Production-size nowcast is one iteration per side.** One Python iteration at 1750 x 3500
  takes about 84 s and peaks at 4.2 GB (Rust: 21 s, 1.8 GB) in the headline run. Repeating it needs
  a host with more RAM headroom than an 8 GB VM shared with other work.
- **API load is a stand-in on the Python side.** Caddy is not installable here. The Python server
  serves `/tiles` itself with StaticFiles (no Cache-Control, no gzip, no precompressed files); in
  production Caddy serves those files and Python never sees them. So the Python tile cost in bench 5
  is higher than the production Python path, and the bench 5 Python numbers are not a measure of
  the production API routes alone. The Rust server serves `/tiles` in-process too (`ServeDir`), so
  both sides carry the tile work. A no-tile mix (API routes only) would isolate the Python routes;
  it was not run.
- **Access logging is inside the Rust process.** `radar-api` writes one JSON line per request
  (except `/api/livez` and `/api/health`) and has no switch for it. The rerun wrote 442,164 lines
  (about 100 MB) to the log. Production Caddy writes its access log outside the app. The Rust CPU
  figures therefore include that logging; the Python server runs with `--no-access-log` as in
  `start.sh`. The logging cost is not separated.
- **Max throughput is one core.** The server is pinned to core 0 and reached 95% of it at the
  max run, so the Rust 13,000 rps figure is the core's limit. oha and the sampler run on core 1
  and their own CPU was not sampled, so oha may also have been near its limit; the figure is a
  lower bound for the server, not a measured ceiling.
- **Python saturation point moves between runs.** Two runs on this box gave a Python 1000 rps step
  of 363 rps (`results/load/20261014T061843Z`, an earlier run on a fixture whose nowcast manifest
  lacked the `path` and `palettes` fields that production writes; the server code paths are the
  same) and 572 rps (`results/load/20261014T062447Z`, the headline run). Both saturate the core.
  The 500 rps step moved too (p50 3 ms against 84 ms), so Python latency above 400 rps is a queue,
  not a constant. The Rust server stayed within a few percent of target in both.
- **Nowcast is 2% of the mix and has no rain in it.** All sampled nowcast points in the fixture read
  -12 dBZ (no rain): the synthetic field has no rain at those places, and the Seattle point is outside
  the grid and returns `unavailable`. The handler's rain path is not exercised by bench 5.
- **Rate limit lifted.** Both servers run with `API_RATE_LIMIT_RPS=100000` so the limiter does not
  turn the test into a 429 test. The production limit (20 rps, burst 60, per IP) is not measured.
- **One client address.** All oha traffic comes from 127.0.0.1, so the trusted-proxy and client-IP
  logic in radar-api runs but is not exercised by distinct clients.
- **Empty tiles are removed from the mix.** The synthetic scene has no data in some tile positions
  near the sampled places. 675 of the mix's tile requests had no file and were dropped, so both
  servers only see tiles that exist (a 404 would be the same on both sides; it would only dilute the
  comparison).

## Output schema (results/<ts>.json)

`{timestamp_utc, machine, benches: {tile|storms|grib_*|nowcast_*: {...}}, verification, notes}`.
Bench 5 is written by `load/summarize_load.py` to `results/load/<ts>/summary.{json,md}` (per target
and run: achieved rps, success, latency percentiles, server CPU % of its core, CPU ms per request,
RSS peak, non-200 codes). `report.py` embeds the newest one as section 5. `parity.txt` in each load
run directory is the response-equality check between the two servers.
Each raw file under `work/raw/<name>.json` has `records` (one per call: `phase`, `index`,
`wall_s`, `cpu_s`, plus benchmark fields), `summary` (median, p90, min, cold), `memory`
(`after_load_rss_mb`, `peak_rss_mb`) and the machine description.
