# Load test and bottlenecks, 2026-09-26

Two in-cluster k6 runs of `load/k6-radar.js` via `load/k8s/k6-job.yaml`: 10 pods, 10 → 20 virtual users each, 11 minutes. The traffic mix is app sessions (manifest, forecast, alerts, nowcast, geocode, 2 FPS playback of a 3×3 z6 viewport) plus widget refreshes, across 15 US places. Each pod is its own client IP, so the per-IP API rate limit (20 rps, burst 60) is never the limit.

## Results

| | Morning (tile-server CPU limit 1) | Evening (limit 2, talos #2577) |
|---|---:|---:|
| Requests | 729,412 | 810,807 |
| Throughput | 1,100 req/s | 1,223 req/s |
| 5xx | 0 | 4 (geocode, see below) |
| 429 | 0 | 0 |
| Tile p50 / p95 | 4.8 / 85 ms | 1.9 / 6.7 ms |
| Playback frame p50 / p95 | 12 / 217 ms | 5 / 16 ms |
| Manifest p95 | 537 ms | 63 ms |
| Forecast p95 | — | 39 ms |
| Nowcast point p95 | — | 66 ms |
| Tile-server CPU under load | throttled at 1 core | 0.93 of 2 cores, 0% throttled |

p95 values are the worst pod's. All k6 thresholds passed in the evening run.

- **The 4 errors** were `/api/geocode` returning 502 between 19:38:26 and 19:38:37. Photon had just restarted on its new, smaller volume (talos #2578) and its index was cold, so answers took longer than `PLACES_UPSTREAM_TIMEOUT_S` (6 s). Only 12 requests took Caddy more than 2 s, all in the first 20 s: 9 geocode, and 3 alerts that were the first NWS fetch for a city.
- **k6's 7 s maximums** on tiles don't match anything in Caddy's access log, which shows no tile slower than 2 s. They happened on the client or network side while the load pods started.

## Serving is not the bottleneck

At 1,200 req/s one tile-server pod uses under 1 core. Caddy serves tiles from disk, and the Python API's p95 is under 70 ms on every endpoint. A Rust API or tile server would not change what users see.

## The bottleneck: nowcast freshness

The nowcast (0–60 min "future radar") publishes about 8 minutes after the MRMS scan it starts from. One traced cycle (scan valid 19:40:03):

| Stage | Time |
|---|---:|
| MRMS grid arrives, then waits for the running nowcast to finish | 4 m 50 s |
| pySTEPS S-PROG | 148 s |
| Tile render | 53 s |
| Published | 19:48:14 |

The schedule fires every 2 min with `OverlapPolicy.Skip`. A run takes about 3.6 min (median 219 s), so a new scan usually waits for the current run and then its own. Cutting run time cuts both terms.

A local profile of S-PROG on the real inputs (1750 × 3500 cells, 48 s on one core) shows where the time goes:

| Hot spot | Share | Code |
|---|---:|---|
| Semi-Lagrangian extrapolation | 45% | `scipy.ndimage` `geometric_transform` (C) |
| Percentile mask and probability matching | 30% | `numpy` `sort` / `argsort` of 6 M cells (C) |
| Everything else | 25% | |

Both hot spots already run in C. S-PROG runs at every 2-minute input step (31 steps) although only 12 lead times are published.

Tested and ruled out:

- **Parallel pySTEPS** (`fft_method="pyfftw"`, `num_workers` 4 and 8, with dask): 47–50 s against 48 s serial, with identical output. The expensive parts don't run in parallel. The pod has an 8-CPU limit and uses 1.
- **Rust**: it would be rewriting compiled C loops, so no worthwhile gain. The savings come from doing less work.

Options, most promising first:

1. **Run S-PROG on 4-minute input spacing** (every other MRMS scan). That halves the internal steps (31 → 15), so pySTEPS and mask time drop about 2×. The MRMS worker must keep about 8 input grids instead of 4. Needs a side-by-side check of forecast quality.
2. **Start the nowcast when MRMS publishes a grid** (a workflow start or signal) instead of every 2 minutes with skip. That removes most of the wait.
3. **Shard extrapolation across cores**: split the domain into overlapping bands and run `map_coordinates` in threads. This needs a small fork of pySTEPS' semi-Lagrangian step.
4. **Coarser science grid**: about 4× faster at 4 km, but visibly blockier at z6. Not recommended.

## Other observations

- **Cluster CPUs are about 3× slower per core than an Apple M-series Mac** on these workloads: nowcast S-PROG 148 s against 48 s, and an air-quality chunk 11.5 min against 3.6 min. That's where most of the gap between local tests and production comes from.
- **Air-quality ingest** renders 6 chunks, 2 at a time, about 35 min per run, twice a day. Each chunk downloads the whole 70–90 MB GRIB file again (3 downloads per layer). That's acceptable at this cadence.
- **MRMS**: `mrms_process_frame` median 35 s (p95 58 s). Radar data was 246 s old during the evening run; the 10-minute target was never at risk.
- **HRRR**: after radar-ng#67, the published forecast reaches +48 h (2026-09-28T18:00 for the 18z run).

## Stress test (evening, 21:01–21:14 UTC)

`PROFILE=stress`: 60 pods climb 5 → 10 → 15 → 20 users in 3-minute steps (300 → 1,200 users). Load pods are kept off the tile-server's node, and their nodes peaked at 62% CPU. There were 1.81 M requests, zero 429s and 26 5xx.

| Users | Tile req/s | API req/s | API p95 (Caddy) | Tile-server CPU (limit 2) |
|---:|---:|---:|---:|---:|
| 300 | 1,800–2,470 | 150–197 | 0.3 s | 1.94, 49% of periods throttled |
| 600 | 2,220–2,430 | 180–196 | 3 s | 1.89 |
| 900 | 2,120–2,250 | 171–182 | 7 s | 1.83 |
| 1,200 | 1,700–2,190 | 138–178 | 10–12 s | 1.82 |

- **The API caps at about 190 req/s.** It's one uvicorn process, about 1 core, and it saturates between 300 and 600 users. Caddy uses the other core.
- **Liveness killed the container twice** (exit 137). `/api/livez` queues behind real API requests and missed the default 1 s timeout. Readiness on the same path had already emptied the one-replica Service, so clients got `connection refused`; that caused most of the 17,011 failed requests. The fix is in talos (branch `radar-ng/tile-server-probes-stress`): TCP readiness, a 5 s × 4 liveness allowance, and CPU limit 3.
- **The API's CPU goes to avoidable work:**
  - `/api/nowcast/{lat}/{lon}` parses the 130 KB manifest from disk on every call, bypassing the 15 s manifest cache.
  - `/api/manifest.json` re-serializes the cached dict on every call.
  - uvicorn's access log duplicates Caddy's.
- **Tiles never bottlenecked** (p95 30 ms). They plateaued only because each simulated user's session waited on the API.
