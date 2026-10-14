# radar-bench

Rust runners for the radar-ng before/after benchmark. They are measurement binaries, not part of
the service. The driver, the synthetic inputs, the Python runners and the report are in
`bench` (see `bench/README.md`).

## Binaries

| Binary | Calls | Input from `bench/work/inputs` |
|---|---|---|
| `bench_tile` | `radar_tiler::render_frame_palettes` (overview max, zooms 4 to 7, classic/muted/vivid, indexed-rs1) | `mrms_3500x7000_t0.f32`, `lats_3500.f64`, `lons_7000.f64` |
| `bench_grib` | `radar_grib::mrms::decode_gz` + `ensure_south_up`, or `decode_message` per message | `*.grib2.gz`, `*.grib2` |
| `bench_storms` | `radar_storms::detect_storms` with tracking, frame B with frame A as `previous` | `mrms_3500x7000_t{0,1}.f32` |
| `bench_nowcast` | `radar_nowcast::estimate_motion` (stride 2), then `forecast` (12 leads, 6 levels, 5 dBZ) | `nowcast_{H}x{W}_t{0..3}.f32` |
| `noaa_dump` | `fetch`: `radar_grib::mrms::list_recent_keys` + `fetch_gz`, `hrrr::find_latest_run` + whole-object GETs + `fetch_hour` (ranged), `naqfc::find_latest_run` + `download_product`, and HEAD of a missing key per bucket. `dump`: `decode_gz` + `ensure_south_up`, `list_messages`, every message decoded and hashed, `parse_idx` + `pick_ranges` and `HrrrVar::selector` picks for the 13 variables, values, axes and the Lambert mesh | real NOAA files in `bench/work/noaa` (not a benchmark: the pre-cutover check, `rust/scripts/preflight.sh` step 4, compares its output with `bench/py/verify_noaa.py`) |
| `bench_api_server` | `radar_api::serve(Config::from_env())`; no timing of its own (bench 5 measures it from outside with oha) | environment only: `LISTEN_ADDR`, `TILE_DIR`, `STATE_DIR`, `GRID_DIR`, `API_RATE_LIMIT_RPS` |

Common flags: `--warmup`, `--iters`, `--json <file>`. Each run is one process: warmup calls,
then timed calls. Output has the same JSON shape as the Python runners.

Build and run (from `rust`, with `env.sh` sourced):

```
cargo build --release -p radar-bench
target/release/bench_tile --input-dir ../bench/work/inputs --out-root ../bench/work/out/tile/rust \
  --warmup 1 --iters 5 --json ../bench/work/raw/tile_rust.json
```

## Measurement

`src/lib.rs` implements the same rules as `bench/py/common.py`:

- wall time: `std::time::Instant` around one call;
- CPU: `getrusage(RUSAGE_SELF)` user plus system time, through a single `libc` call (rustix 1.1
  does not expose it); this is measurement-only `unsafe`;
- memory: `VmRSS` and `VmHWM` from `/proc/self/status`;
- median, p90 (linear interpolation), min and cold-call time.

Inputs are read in 1 MB chunks into the final vector, so the peak RSS is the data, not twice
the data. The `f32` files are little-endian, as numpy writes them.

## Dependencies

`radar-core` (declared, not used by the runners yet), `radar-geo`, `radar-tiler`, `radar-grib`,
`radar-storms`, `radar-nowcast` (all `workspace = true`), `anyhow`, `clap`, `serde_json`,
`sha2`, `tokio` and `radar-api` (for `bench_api_server`), `chrono` and `reqwest` (for `noaa_dump`:
the clock the radar-grib finders take, and the client type of the NAQFC finder), and `libc = "0.2"`
for `getrusage`.

## Deviations and notes

- The tile runner removes the earlier iteration trees outside the timed loop, as the Python
  runner does. Only the last tree stays for `bench/py/verify.py`.
- The storms runner writes the normalised summary (ids, peaks, pixel counts, areas, bboxes,
  coordinates) that `verify.py` compares with the Python output. It does not time that write.
- The nowcast runner writes the forecast with NaN replaced by -9999, as the Python activity does
  before it publishes; `verify.py` compares those values.
- The GRIB multi-message runner splits the file by the 8-byte GRIB2 total length, because
  `decode_message` decodes one message. Python uses pygrib on the whole file.
- No `todo!()` or `unimplemented!()` in the runners.
- `bench_api_server` is a thin wrapper so the bench 5 driver (`bench/load/run_load.sh`) can start the
  Rust API the same way it starts the Python one. It has no measurement code of its own.
