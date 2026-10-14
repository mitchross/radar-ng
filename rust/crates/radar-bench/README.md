# radar-bench

Measurement binaries for the radar-ng backend, plus `noaa_dump`, which the live-NOAA check uses. None of
them is part of the service.

## Binaries

| Binary | Calls | Input |
|---|---|---|
| `bench_tile` | `radar_tiler::render_frame_palettes` (overview max, zooms 4 to 7, classic/muted/vivid, indexed-rs1) | `mrms_3500x7000_t0.f32`, `lats_3500.f64`, `lons_7000.f64` in `--input-dir` |
| `bench_grib` | `radar_grib::mrms::decode_gz` + `ensure_south_up`, or `decode_message` per message | `*.grib2.gz`, `*.grib2` |
| `bench_storms` | `radar_storms::detect_storms` with tracking, frame B with frame A as `previous` | `mrms_3500x7000_t{0,1}.f32` |
| `bench_nowcast` | `radar_nowcast::estimate_motion` (stride 2), then `forecast` (12 leads, 6 levels, 5 dBZ) | `nowcast_{H}x{W}_t{0..3}.f32` |
| `noaa_dump` | `fetch`: newest MRMS key and object, latest HRRR run with `.idx` and the ranged subset, latest NAQFC run, and a HEAD of a missing key per bucket. `dump`: MRMS decode and south-up flip, every HRRR message decoded and hashed, `.idx` picks for the 13 variables, values, axes and the Lambert mesh | real NOAA files (used by `rust/scripts/live-noaa-check.sh`) |
| `bench_api_server` | `radar_api::serve(Config::from_env())`, for timing the API from outside with a load tool | environment only: `LISTEN_ADDR`, `TILE_DIR`, `STATE_DIR`, `GRID_DIR`, `API_RATE_LIMIT_RPS` |

Common flags: `--warmup`, `--iters`, `--json <file>`. Each run is one process: warmup calls, then timed
calls. Input arrays are raw little-endian `f32`/`f64` (numpy `tofile` layout). The synthetic-input
generator and the Python comparison runners were removed with the Python backend; the recorded results
are in `docs/rust-migration/benchmark-results.md`.

```sh
cd rust
cargo build --release -p radar-bench
target/release/bench_tile --input-dir <inputs> --out-root <out> --warmup 1 --iters 5 --json tile.json
```

## Measurement

`src/lib.rs`:

- wall time: `std::time::Instant` around one call;
- CPU: `getrusage(RUSAGE_SELF)` user plus system time, through a single `libc` call (rustix 1.1 does not
  expose it); this is measurement-only `unsafe`;
- memory: `VmRSS` and `VmHWM` from `/proc/self/status`;
- median, p90 (linear interpolation), min and cold-call time.

Inputs are read in 1 MB chunks into the final vector, so peak RSS is the data, not twice the data.

## Notes

- The tile runner removes earlier iteration trees outside the timed loop; only the last tree stays.
- The storms runner writes a normalised summary (ids, peaks, pixel counts, areas, bboxes, coordinates).
  It does not time that write.
- The nowcast runner writes the forecast with NaN replaced by -9999.
- The GRIB multi-message runner splits the file by the 8-byte GRIB2 total length, because
  `decode_message` decodes one message.
