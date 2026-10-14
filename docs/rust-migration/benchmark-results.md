# radar-ng before/after benchmark (20261014T063808Z)

Machine: Intel(R) Xeon(R) Processor @ 2.10GHz, 2 logical CPUs, 8029 MB RAM, kernel 6.18.44-fc-v114.

Rust source: git 6dd7973584ed; 2 modified or new files in the measured crates at build time (see provenance in the JSON). Python 3.12.3, pysteps 1.21.5, numpy 1.26.4.

## 1. Tile render (MRMS 3500 x 7000, zooms 4-7, classic + muted + vivid)

| Implementation | Wall median (s) | Wall p90 (s) | Cold first call (s) | CPU median (s) | Peak RSS (MB) | PNG files | PNG bytes (MB) |
|---|---:|---:|---:|---:|---:|---:|---:|
| Python (production default, legacy RGBA) | 11.84 | 12.41 | 11.81 | 11.50 | 390 | 648 | 12.9 |
| Python (indexed, same algorithm as Rust) | 3.96 | 4.08 | 3.95 | 3.69 | 195 | 585 | 1.3 |
| Rust (radar-tiler, indexed-rs1, rayon) | 0.92 | 0.99 | 0.83 | 1.46 | 200 | 585 | 1.4 |

Load during these runs (1-min loadavg, 2 CPUs): mean 1.32, max 1.50.

| Rust compared with | Wall speedup | CPU speedup | Peak RSS ratio (Python / Rust) |
|---|---:|---:|---:|
| python-legacy | 12.86x | 7.85x | 1.95x |
| python-indexed | 4.31x | 2.52x | 0.97x |

Verification (indexed Python vs Rust): tile sets equal = **True**, palette indices identical in **585 / 585** tiles, RGBA identical in 585 tiles.

Legacy RGBA vs Rust (informational, different sampling order by design): 92.1% of pixels identical over 585 tiles.

## 2. GRIB2 decode

| Case | Python median (s) | Rust median (s) | Speedup | Python peak RSS (MB) | Rust peak RSS (MB) | CPU speedup | Values bitwise equal |
|---|---:|---:|---:|---:|---:|---:|---|
| MRMS fixture (35 x 70, gzip) | 0.0048 | 0.0001 | 48.13x | 122 | 4 | 41.73x | yes |
| Lambert HRRR-like mini (6 messages) | 0.0249 | 0.0041 | 6.11x | 91 | 4 | 5.69x | yes |
| MRMS synthetic (3500 x 7000, gzip, grid_png) | 1.1272 | 0.9581 | 1.18x | 437 | 587 | 1.18x | yes |

Load during these runs (1-min loadavg, 2 CPUs): mean 1.20, max 1.23.

## 3. Storm detection and tracking (2 frames, 3500 x 7000)

| Implementation | Wall median (s) | Wall p90 (s) | CPU median (s) | Peak RSS (MB) | Features A / B |
|---|---:|---:|---:|---:|---:|
| Python (detect_storms + tracking) | 8.777 | 9.347 | 8.602 | 750 | 86 / 72 |
| Rust (radar-storms) | 0.136 | 0.148 | 0.136 | 244 | 86 / 72 |

Speedup 64.47x, CPU 63.35x, peak RSS ratio (Python / Rust) 3.07x.

Load during these runs (1-min loadavg, 2 CPUs): mean 1.11, max 1.16.

Verification: cell ids equal in order = True, pixel counts equal = True, max peak diff 0.0, max bbox diff 0.0, max centroid diff 0.0 (degrees).

## 4. Nowcast (LK motion stride 2 + S-PROG, 4 input frames, 12 leads)

| Science grid | Python median (s) | Python cold (s) | Rust median (s) | Rust cold (s) | Speedup | Python peak RSS (MB) | Rust peak RSS (MB) | Iterations (warmup + timed) |
|---|---:|---:|---:|---:|---:|---:|---:|---|
| 875x1750 | 17.19 | 19.35 | 3.61 | 3.82 | 4.76x | 1300 | 533 | 1 + 3 |
| 1750x3500 | 83.74 | n/a | 20.87 | n/a | 4.01x | 4243 | 1802 | 0 + 1 |

Load during these runs (1-min loadavg, 2 CPUs): mean 1.13, max 1.36.

Verification 875x1750 (not bit-exact by design, see radar-nowcast README): correlation min 0.9991, mean absolute difference up to 0.033 dBZ, largest single-cell difference 17.6 dBZ, 5 dBZ rain-mask agreement min 99.94%.
Verification 1750x3500 (not bit-exact by design, see radar-nowcast README): correlation min 1.0000, mean absolute difference up to 0.001 dBZ, largest single-cell difference 15.0 dBZ, 5 dBZ rain-mask agreement min 100.00%.

## 5. API load (FastAPI + StaticFiles vs radar-api, same fixture and request list)

Run `20261014T062447Z`. One core per side: server on core 0, oha and the /proc sampler on core 1. Fixed-rate steps are 30 s each; the max run uses 64 connections. Python serves /tiles with StaticFiles in place of Caddy (no Cache-Control, no gzip), so its tile numbers are a lower bound for the Caddy setup. Per-IP rate limit lifted for both.

| target | run | achieved rps | success | p50 | p90 | p95 | p99 | server CPU % core | CPU ms/req | RSS peak MB | non-200 |
|---|---|---:|---:|---:|---:|---:|---:|---:|---:|---:|---|
| FastAPI + StaticFiles | 200 rps | 200.4 | 100.00% | 1.86 | 2.97 | 3.54 | 8.23 | 44.0 | 2.273 | 70.5 | none; 11 in flight at deadline |
| FastAPI + StaticFiles | 500 rps | 493.9 | 100.00% | 83.88 | 154.22 | 172.25 | 209.77 | 94.2 | 1.979 | 77.2 | none; 62 in flight at deadline |
| FastAPI + StaticFiles | 1000 rps | 571.7 | 100.00% | 106.65 | 146.72 | 160.79 | 193.8 | 95.6 | 1.734 | 78.0 | none; 64 in flight at deadline |
| FastAPI + StaticFiles | max (-c) | 538.0 | 100.00% | 111.88 | 160.06 | 181.91 | 214.13 | 94.9 | 1.832 | 78.5 | none; 64 in flight at deadline |
| radar-api | 200 rps | 200.5 | 100.00% | 0.37 | 0.52 | 0.63 | 1.9 | 4.6 | 0.238 | 11.7 | none; 15 in flight at deadline |
| radar-api | 500 rps | 500.0 | 100.00% | 0.38 | 0.59 | 0.76 | 2.23 | 11.1 | 0.229 | 12.1 | none; 2 in flight at deadline |
| radar-api | 1000 rps | 1000.0 | 100.00% | 0.31 | 0.49 | 0.61 | 2.55 | 16.7 | 0.173 | 12.5 | none |
| radar-api | max (-c) | 13039.4 | 100.00% | 4.05 | 5.83 | 6.72 | 45.81 | 94.9 | 0.075 | 14.1 | none; 64 in flight at deadline |

Full caveats and the request mix: bench/README.md, section 'Bench 5'.

## Notes

- Wall = median over timed iterations of one call, in-process (warmup excluded).
- CPU = user + sys of the process, all threads.
- Peak RSS = VmHWM of the process after the run (includes interpreter and numpy for Python).
- p90 uses linear interpolation; with 5 samples it is close to the maximum.
- GRIB rows on the 35 x 70 fixtures are dominated by per-call overhead; the 3500 x 7000 row is the realistic one.
- Python GRIB decode is the production code path: gunzip to a temp file and pygrib open. Rust decodes in memory.
- Nowcast verification: the Rust motion field is a re-implementation of OpenCV's Lucas-Kanade and Shi-Tomasi, so it is close to, not identical with, pysteps' output.

