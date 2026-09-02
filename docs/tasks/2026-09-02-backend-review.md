# radar-ng Python data plane — backend review (2026-09-02)

Repo: `/home/vanillax/programming/radar-ng` @ `6d2ce28`. All paths below are relative to `backend/` unless prefixed. Every claim is anchored to file:line in that checkout.

**Tests:** the ambient interpreter cannot run them (no `scipy`; a globally-installed `langsmith` pytest plugin also breaks collection on `urllib3.contrib.appengine`). In an isolated env they pass:
`PYTEST_DISABLE_PLUGIN_AUTOLOAD=1 uv run --no-project --python 3.12 --with pytest --with 'numpy<2' --with scipy --with Pillow --with 'temporalio==1.30.0' --with httpx --with fastapi --with pydantic --with websockets --with pyproj python -m pytest backend temporal -q` → **45 passed, 11 subtests passed**. There is no lockfile/requirements-dev that makes this reproducible; that is itself a finding (§5).

Legend: **[tonight]** = small, testable, low blast radius. **[design]** = needs a PR-level discussion.

---

## 1. Status of the 2026-07-04 findings

| ID | Status | Evidence (current code) |
|---|---|---|
| **C1** ghost frames | **FIXED** | MRMS: all-or-nothing palette gate, partials rmtree'd, `rendered=False` → not marked (`ingest_mrms/activities.py:359-381`, workflow `temporal/workflows/ingest_mrms.py:107-116`). Nowcast: per-leadtime gate + run-level `_commit` raises if `< n_lead` (`nowcast/activities.py:339-377, 381-385`). HRRR: `_render_per_palette` all-or-nothing (`ingest_hrrr/activities.py:428-431`), publish only when every hour has `radar-hrrr` (`:650-651`). Bonus: manifest `palettes` is now the *intersection* across frames (`shared/manifest.py:90-92`), which closes the "accepted limitation" from the prior doc. |
| **C2** nowcast 2.5× time-stretch | **FIXED** | Fractional pysteps timesteps from the measured input cadence (`nowcast/activities.py:262, 274`). |
| **C3** storms.json torn write / cross-layer race | **FIXED** | Only `layer_name == "radar"` writes storms (`ingest_mrms/activities.py:306-308`); `write_storms_json` takes `storms.lock` + `mkstemp` + `os.replace`, and rejects out-of-order timestamps (`shared/storms.py:280-305`). |
| **C4** HRRR run marked processed before NOAA finishes | **FIXED, but replaced by a worse operational bug** | Publish/mark now require `succeeded == horizon` and consecutive hours (`temporal/workflows/ingest_hrrr.py:128-135`, `ingest_hrrr/activities.py:646-651`). Consequence: while f42–f48 are still 404, every 15-min fire re-renders *all* 48 hours and 3 palettes, then discards them on `EEXIST` — and the extended run very likely never publishes at all. See **§3 B1**. |
| **C5** alert delivery | not in scope (`storm_watch_activities.py`) — not re-verified. | |
| **H1** Caddy caches 404s + rmtree-then-rename | **FIXED** | `handle_errors { header Cache-Control "no-store" }` (`api/Caddyfile:58-61`); `render_tiles_atomic` renders to a unique `.tmp-<uuid>` sibling and `os.rename`s into place, treating `EEXIST/ENOTEMPTY` as "the other writer won" (`shared/tiler.py:237-260`). |
| **H2** push-token write to RO mount / sync sqlite on the loop | **REGRESSED → STILL OPEN** | The prior fix (route → `DeletePushTokenWorkflow`) is gone. `POST /v1/push-tokens` calls `upsert` and `DELETE` calls `delete_for_user` directly from the API pod (`api/api/routes_workflows.py:139, 148`) → `push_tokens.py:48-56` does `STATE_DIR.mkdir` + `sqlite3.connect` + `CREATE TABLE IF NOT EXISTS` on `/data/state`, which the tile-server mounts **`readOnly: true`** (`talos-argocd-proxmox/my-apps/development/radar-ng/deployment-tile-server.yaml:121-122`). Every register/unregister in production must 500 ("attempt to write a readonly database" / "unable to open database file"). The worker still registers `persist_push_token`/`delete_push_token` + `RegisterPushTokenWorkflow` (`temporal/worker.py:116-117, 137`) — the route simply stopped using them. H2a (event loop) is fixed via `asyncio.to_thread`. **[tonight]**: route back through `RegisterPushTokenWorkflow` / a delete workflow on `TASK_QUEUE`, or mount state RW on the API (worse). Verify first: `kubectl -n radar-ng logs deploy/radar-ng-tile-server | grep -i "readonly database"`. |
| **H3** unbounded `_forecast_cache` | **FIXED** | `OrderedDict` LRU capped at `FORECAST_CACHE_MAX_ENTRIES=512` (`api/api/server.py:39, 86, 256-258`). |
| **H4** unauthenticated workflow endpoints | **FIXED** | HMAC bearer tokens bound to `sub` (`routes_workflows.py:95-125`), owner check (`:122-125`), token-bucket rate limit middleware (`server.py:117-147`). Caveat: the limiter keys on the *first* `X-Forwarded-For` hop (`server.py:110-114`), which Caddy appends to rather than replaces → trivially spoofable (§3 B9). |
| **H5** slot starvation / task-queue split | **PARTIAL** | Queue names + per-role worker config exist (`temporal/task_queues.py`, `temporal/worker.py:141-200`), nowcast has `schedule_to_close` (`temporal/workflows/nowcast.py:42`). But production runs `WORKER_ROLE=legacy`, `USE_ISOLATED_TASK_QUEUES=0`, one pod, 4 activity slots (`worker.py:84`) — so the memory/CPU coupling that OOM-killed the pod on 08-31 is unchanged. |
| **H6/H8/H9** | seed.py / open-meteo Dockerfile / CI — out of scope, not re-verified. | |
| **H7a** cancel leaks tmp dirs | **STILL OPEN (MRMS, HRRR); fixed in AQM** | `except Exception` around the render skips cleanup on `CancelledError` (`ingest_mrms/activities.py:344-358`, `ingest_hrrr/activities.py:609-625`). AQM uses `finally` (`ingest_airquality/activities.py:377-397`). Nothing sweeps `/tmp/{mrms,hrrr}_work`. **[tonight]**: change to `finally:`. |
| **H7b** zombie render vs retry | **FIXED** | Unique staging dir + EEXIST-tolerant rename (`tiler.py:238, 252-259`); `tile_cleanup` sweeps `.tmp-` dirs >1 h old by mtime (`tile_cleanup/activities.py:101-107`). `to_thread` work is still uncancellable (`shared/activity_heartbeat.py:23`) but now converges. |
| **H10** worker fsGroup | **FIXED** | `fsGroup: 1000` on both deployments (`temporal-worker-deployment.yaml:46`, `deployment-tile-server.yaml:51`). |
| **M6** grid dumps non-atomic | **FIXED** | Generation-suffixed `.bin` + fsync + meta-as-commit-pointer (`shared/grid_dump.py:96-127`). |
| **M7** Temporal client singleton no lock/timeout | **STILL OPEN** | `api/api/temporal_client.py:26-31` — two concurrent first requests both `Client.connect`; no connect timeout. |
| **M8** gauge counts palettes | **FIXED** | counts frames (`server.py:804-808`). `/tmp/uvicorn.log` still unbounded (`api/start.sh:6`). |
| **M9** transient manifest read error → manifest rewritten empty | **STILL OPEN** | `read_manifest_file` returns `empty_manifest()` on *any* `OSError`/decode error (`shared/manifest.py:36-41`); `update_manifest_file` then writes that back with only the new frame (`:117-146`). One EIO = every layer's history vanishes and is **never re-added** (old frames' tiles stay on disk but nothing re-lists them). Same swallow in `ProcessedSet._load` (`shared/state.py:30-32`). |
| **M10** heartbeat gaps in download | **PARTIAL** | MRMS download is ≤60 s per op (`ingest_mrms/activities.py:153`) under a 180 s heartbeat timeout — OK in practice. HRRR full-file fallback uses `timeout=300` (`ingest_hrrr/activities.py:295`) with one heartbeat before it (`:592`) and `heartbeat_timeout=180s` (`workflows/ingest_hrrr.py:109`) → a slow full download is killed as "unhealthy". |
| **M11** per-frame `ProcessPoolExecutor` forked from a threaded process | **STILL OPEN** | `ingest_mrms/activities.py:330` — new pool per frame, default start method (fork on Linux, from a process with asyncio + `to_thread` threads), ~98 MB float32 grid pickled per palette. |
| **M12** pygrib leaks / midnight gap | **STILL OPEN (MRMS, HRRR); fixed in AQM** | `grbs.close()` only on the success path (`ingest_mrms/activities.py:164-173`, `ingest_hrrr/activities.py:318-352`); AQM uses `try/finally` (`ingest_airquality/activities.py:318-336`). Midnight: yesterday is listed only if today is empty (`ingest_mrms/activities.py:134-139`) → last frame(s) before 00:00Z can be skipped. |
| **M12** tile_cleanup rmtree's a live render | **FIXED** | mtime > 1 h guard (`tile_cleanup/activities.py:101-107`). |

---

## 2. Render hot path (by reading; numbers are estimates from array sizes, not measured)

### Frame anatomy (MRMS `radar`, one frame, 3 palettes)

Input grid: MRMS CONUS 0.01° = **3500 × 7000 = 24.5 M cells**; float32 = **98 MB decimal / 93.5 MiB**. Four-channel RGBA uint8 is also **98 MB decimal / 93.5 MiB per palette**, not 392 MB; each pixel is four bytes total. (Corrected after the Phase 2 benchmark audit.)

Tile candidates per zoom for the CONUS bbox (lat 20–55, lon −130…−60), from `_lat_lon_to_tile` (`tiler.py:49-55`): z4 = 4×3 = 12, z5 = 7×5 = 35, z6 = 14×9 = 126, z7 = 26×17 = 442 → **~615 candidates per palette (z4–7), ~1,845 per frame**. Transparent tiles are skipped *before* sampling on the MRMS path (`tiler.py:156-158, 166-167, 188-191`), so the rendered count is weather-dependent (typically 20–50 %).

| Stage | Where | What happens | Est. CPU | Repeated ×3? |
|---|---|---|---|---|
| Download + gunzip + pygrib decode | `ingest_mrms/activities.py:149-177` | 1–3 MB gz → 24.5 M float64 values → `astype(float32)` | ~1.5–2.5 s | no |
| **Colorize** `apply_color_table` | `tiler.py:14-31` | 11 ranges × (`>=`, `<`, `&`, boolean fancy-assign into a 93.5 MiB RGBA) + `below` mask. Each pass touches 24.5 M cells; the masked assign writes 4 channels. | **~2.5–3.5 s per palette** | **yes (3×)** — pure waste: all 3 palettes have identical bin edges (`shared/palettes/{classic,muted,vivid}.json` `reflectivity.ranges` = 15/20/…/65/75, verified) |
| Pickle grid to pool children | `:332` | 98 MB `data` pickled per submit | ~0.5–1 s ×3 | yes |
| **Per-tile geometry** | `tiler.py:149-177` | `meshgrid` of 256² fractional indices (2 × 512 KB float64) | ~0.3 ms/tile | yes, and identical for every frame forever (MRMS grid is static) |
| **Per-tile resample** | `tiler.py:194-203` | **4 × `scipy.ndimage.map_coordinates(order=1)`** on 65,536 points — one per RGBA band | ~4–8 ms/tile | yes (×3 palettes × 4 bands) |
| PNG encode | `tiler.py:209-212` | `Image.fromarray(RGBA)` + `save(compress_level=1, optimize=False)` on 256 KB raw | ~3–5 ms/tile | yes |
| mkdir + write | `:211-212` | one `mkdir -p` and one `open/write` per tile, **no fsync** | Longhorn-dependent | yes |
| dir rename | `tiler.py:252` | `os.rename(tmp, final)` | trivial | per palette |
| Grids + storms | `ingest_mrms/activities.py:298-310, 386-397` | inspector grid (stride 8 → 437×875), storms `ndimage.label` on a 24.5 M mask, and the **nowcast science grid at stride 2 (1750×3500, 24.5 MB, fsync'd)** | ~1–2 s | no |
| Manifest update | `:400-418` | flock + read + write JSON — **on the event loop** (see B6) | ms | no |

Threads/processes: the activity is `async def`; download runs in `to_thread`; the render runs in a **fresh `ProcessPoolExecutor(max_workers=min(3, MRMS_RENDER_WORKERS=2))`** per frame (`:329-330`), so 3 palettes run as 2 parallel + 1 serial → **wall ≈ 2 × single-palette time**. Inside a palette everything is single-threaded Python looping over tiles; only the numpy/scipy kernels are vectorized. `FRAME_CONCURRENCY=2` in the workflow (`temporal/workflows/ingest_mrms.py:45`) means up to 4 render processes at once, each holding at least ~187 MiB for the grid + RGBA before masks, pickle buffers, and tile temporaries.

Rough total per palette ≈ 3 s colorize + 1 s pickle + (~250 rendered tiles × ~12 ms ≈ 3 s) + ~615 × transparency checks ≈ **7–9 s**; ×3 palettes on 2 procs ≈ 16–18 s; + decode + grids ≈ **22–28 s** — consistent with what production shows.

**Work that is done 3× but should be 1×:** colorization, per-tile coordinate geometry, per-tile *sampling*, per-tile transparency check, grid pickling. Everything except the final bytes-on-disk is palette-independent.

**PNG settings:** RGBA (`"RGBA"`, 4 B/px), `compress_level=1`. Files are ~4× larger than an indexed-color PNG of the same tile, which costs Longhorn IO, Caddy read bandwidth and mobile download. `Caddyfile:11 precompressed gzip` is inert — nothing writes `.png.gz` — so it only adds a failed `stat` per tile request.

### HRRR / AQM / nowcast path (curvilinear branch)

Same loop, but with `source_crs` the transparency check comes **after** the projection (`tiler.py:169-191`), so every candidate tile — transparent or not — pays a **pyproj `Transformer.transform` on 65,536 points** (`tiler.py:171`) — ≈ 30–80 ms each. For ~173 candidates (z4–6) × 3 palettes ≈ **15–40 s per forecast hour just in pyproj**, × 48 hours per extended run, and the projection is identical for every hour, palette and run (the HRRR grid never changes). Nowcast uses the regular branch (`nowcast/activities.py:186-191`) but still colorizes and samples 3× per lead time on a 6.1 M-cell grid.

### Ranked speedups

1. **[design, but self-contained] Classify once → uint8 index; write indexed-colour PNGs (mode `P`) with a per-palette `PLTE`/`tRNS`.** Steps: `idx = np.searchsorted(edges, data, side="right")` once (~0.3 s vs ~9 s for 3× `apply_color_table`); per tile, sample the physical field once with a product-reviewed method, classify the 256² result to a palette index, encode one 64 KB `P` image per palette — or encode **once** and emit the other two palettes by rewriting only the `PLTE`+`tRNS` chunks (IDAT is byte-identical; recompute one CRC). Since the three palettes share bin edges (verified above), palette derivation is exact. Effect: colorize and sampling happen once, encode 3 × 256 KB becomes 1 × 64 KB (+2 chunk rewrites), tile files are much smaller, and three 93.5 MiB RGBA arrays disappear. Expected RSS reduction is hundreds of MiB plus avoided copies, not 1.2 GiB from RGBA alone. Validate the resampling policy per product against reference fixtures.
2. **[tonight-ish] Cache per-tile `(rows_mapped, cols_mapped)`** keyed by `(grid signature, z, x, y)` in a process-lifetime dict (MRMS geometry is constant; HRRR/AQM too). Removes the meshgrid for MRMS and, crucially, the **pyproj transform per tile for HRRR/AQM** (~10× on that path). Memory: 615 tiles × 1 MB = ~600 MB for MRMS if kept as float64; store as float32 (~300 MB) or cache only the curvilinear layers' `rows/cols` and the MRMS `rows_1d/cols_1d` vectors (tiny: two 256-vectors per tile).
3. **[tonight]** `MRMS_RENDER_WORKERS=3` in the ConfigMap (the pod has a 12-CPU limit, `temporal-worker-deployment.yaml:86`): 3 palettes in one round instead of two → ~30 % wall-clock cut with zero code change. Superseded by (1).
4. **[tonight]** Vectorize `apply_color_table` even if you keep RGBA: `np.searchsorted` + LUT gather (`lut[idx]`) replaces 11 masked assigns — saves ~2.5 s/palette.
5. **[design]** Persistent pool created once at worker start (`spawn`, `max_workers=3`) and share the grid via `multiprocessing.shared_memory` instead of pickling 3 × 98 MB per frame. Also fixes M11's fork-from-threads hazard. Once (1) lands, a single process may suffice and you can parallelize by zoom level instead.
6. **[tonight]** In `render_tiles`, skip the alpha-max region check when `region.size > 4 M` by testing a strided subsample first — z4 tiles scan ~2 M px each. Minor.
7. **[tonight]** Science grid retention: `radar-nowcast-input` writes 24.5 MB every 2 min and `cleanup_old_grids` keeps 12 h (`shared/grid_dump.py:27, 136-160`) → **~8.8 GB steady state on the grids PVC** for a layer that only ever needs its last 4 files (`nowcast/activities.py:40, 94-95`). Add a per-layer keep-N (or `GRID_MAX_AGE_S` per layer). Disk, not CPU — check the grids PVC size.

### Nowcast

- **Config** (`nowcast/activities.py:146-161`): Lucas–Kanade optical flow (`motion.get_method("LK")`, OpenCV) over all 4 input frames; **S-PROG** with `n_cascade_levels=6`, `precip_thr=5.0` dBZ, on the last 3 frames; 12 fractional timesteps (`:274`). Grid: `NOWCAST_SCIENCE_GRID_MAX_CELLS=7e6` forces stride 2 on 3500×7000 → **1750 × 3500 = 6.125 M cells (~2 km)**, matching the z6 display ceiling (`:44-46`).
- **Where CPU goes:** (a) S-PROG cascade decomposition — FFT-based bandpass of 3 frames into 6 levels in float64 (pysteps upcasts): ~21 2-D FFTs of 6.1 M cells ≈ 10–15 s; AR(2) fits per level; then per timestep an AR update on 6 levels + recomposition + semi-Lagrangian `map_coordinates` extrapolation (~0.3–0.5 s each) ≈ 20–40 s for 12 steps. (b) **Rendering dominates**: 12 lead times × 3 palettes × (`apply_color_table` on 6.1 M cells ≈ 0.7 s + ~173 candidates/z4–6) ≈ 25 s colorize + ~40–60 s tiles, all **single-threaded** in one `to_thread` (`:328-338`). Total ≈ 2–3 min per run on a 2-min cadence → **one core permanently saturated**, plus disk churn. Speedups (1)+(2) apply directly and would cut the render half by ~3×.
- **Memory (why 12 Gi can OOM):** nowcast alone: 4 inputs × 24.5 MB + `np.where` copies (~200 MB); pysteps float64 internals — cascade of 3 frames × 6 levels × 49 MB ≈ 880 MB, bandpass filter weights 6 × 49 MB ≈ 300 MB, several complex128 FFT buffers (98 MB each), displacement/coord arrays; output 12 × 49 MB = 588 MB float64 → `.astype(float32)` copy 294 MB → `np.where` copy 294 MB (`:165-166`) → **peak ≈ 2.5–3.5 GB**. Concurrently in the same pod: up to 4 MRMS render processes × ~500 MB ≈ 2 GB + parent pickles (~300 MB); HRRR 8 concurrent hours (`workflows/ingest_hrrr.py:40`) × ~150 MB; AQM chunk decode. Steady ~7.9 Gi observed; a nowcast peak coinciding with an HRRR fan-out and two MRMS frames plausibly crosses 12 Gi. The fix is structural (per-role pods, §H5) — a memory-only VPA on a single pod cannot separate these peaks.
- **[tonight]** cadence recovery: the gap check spans all 4 inputs (`:251-261`) but S-PROG only uses the last 3 (`:153`). Checking the last 3 (or picking the newest 3 with valid spacing) recovers from a publication gap one frame sooner and would have shortened the current `invalid_input_cadence` degraded window.

---

## 3. Correctness / reliability bugs

**B1 — HRRR: hours NOAA hasn't uploaded yet are treated as hard failures; the run is re-rendered from scratch every 15 min; extended runs likely never publish. [tonight for the 404 + resume; design for progressive publish]**
`ingest_hrrr/activities.py:204-206` `_get_idx_sync` → `raise_for_status()` on the `.idx` 404 raises `HTTPStatusError` (a subclass of `HTTPError`) → caught at `:293` → "full download fallback" `:295-296` GETs the missing `.grib2` → 404 → raises out of `_download` (`:594-598`). Temporal retries 3× with 2 s/4 s backoff (`workflows/ingest_hrrr.py:48-53`), each attempt logging a traceback (the f42–f48 tracebacks in the worker log). The workflow swallows the `ActivityError` (`:112-121`), `succeeded != horizon` (`:128`) → not published, not marked processed. Next fire (15 min): `_find_latest_run_sync` returns the same run (`activities.py:187-201`, HEAD on f01 only), and **all 48 hours are downloaded and rendered again**; for f01–f41 `render_tiles_atomic` renders the whole pyramid into staging and then throws it away on `EEXIST` (`tiler.py:252-259`). Worse: once the *next* hourly run's f01 appears (~60 min after this cycle's f01), the finder switches to it and the extended run is never revisited — so a 48-h run only publishes if all 48 hours land inside that ~60-min window, which NOAA's upload cadence rarely allows. Check: `jq '.layers["radar-hrrr"] | .run_id, (.frames|length)' /data/state/manifest.json` — if `run_id` is never a 00/06/12/18 hour with 48 frames, this is confirmed.
Fix [tonight]: (a) in `_download_subset_sync` treat 404 (idx *and* grib) as "pending" → return `None` (already mapped to an empty `ForecastHourResult`, no retry, no traceback); (b) at the top of `hrrr_process_forecast_hour`, if `tile_base/<layer>/<palette>/runs/<run_id>/<ts>` exists for every palette, return `rendered_layers=["radar-hrrr"]` immediately (idempotent resume — the paths are already declared immutable). (c) [design] publish a *consecutive prefix* (`f01..fN`) with `complete: false` and extend it as hours land — the manifest schema already carries `complete` (`:696`); or remember per-run progress in `ProcessedSet` and keep polling the run until horizon or the run is >3 h old.

**B2 — `/v1/push-tokens` writes to a read-only mount → 500s.** See H2 above (`routes_workflows.py:139,148`; `push_tokens.py:48-56`; `deployment-tile-server.yaml:121-122`). [tonight]

**B3 — Manifest wiped by a transient read error.** `shared/manifest.py:36-41` + `:117` (M9). Failure: one `EIO`/short read under the lock → the manifest is rewritten with a single layer; radar/composite/HRRR/nowcast history is gone and only re-fills as new frames land (4 h for radar; HRRR until the next run publishes). [tonight]: in `read_manifest_file` only return `empty_manifest()` on `FileNotFoundError`; re-raise other `OSError`/`JSONDecodeError` so the activity retries; optionally write `manifest.json.bak` on each publish and fall back to it.

**B4 — MRMS/HRRR temp dirs leak on cancellation.** `ingest_mrms/activities.py:344-358`, `ingest_hrrr/activities.py:609-625` (`except Exception` misses `CancelledError`). ~50 MB per HRRR attempt in `/tmp/hrrr_work` on the node's ephemeral disk. [tonight]: `try/finally`.

**B5 — Lightning heartbeat task dies silently on any write error.** `ingest_lightning/activities.py:137-143` — `_beat` calls `_write_geojson`; if it raises (ENOSPC, EIO, or the fixed-name `.tmp` racing another writer at `:109-111`), the task ends, heartbeats stop, and Temporal kills a healthy activity after `heartbeat_timeout`. Also `_write_geojson` (json.dumps of ≤5000 features + write + replace) runs **on the worker's event loop** every 2 s (`:198-201`), stalling every other activity's heartbeats/IO in the process. [tonight]: wrap the body of `_beat` in try/except+log; run `_write_geojson` via `asyncio.to_thread`; use `mkstemp`.

**B6 — MRMS updates the manifest on the event loop.** `ingest_mrms/activities.py:400-418` calls `update_manifest_file` (blocking `fcntl.flock` + read + write) directly in the `async def` activity. If `tile_cleanup`/`mrms_cleanup` (which call it per directory from threads, `tile_cleanup/activities.py:117`, `ingest_mrms/activities.py:462`) hold the lock, the whole worker's loop blocks — heartbeats for nowcast/HRRR included. Same class: `_load_palette_tables()` at `:275`. [tonight]: `await asyncio.to_thread(update_manifest_file, ...)`.

**B7 — Unbounded label cardinality in API metrics.** `server.py:141-146`: for unmatched routes `request.scope["route"]` is `None`, so the counter key is the **raw URL path**. A 404 scan grows `_request_counts`/`_request_duration_sums` without bound and bloats `/api/metrics`. Also `_metrics[...] += 1` is a non-atomic read-modify-write from threadpool threads (`:215, 356`), so counts drift under load. [tonight]: key unmatched requests as `path="<unmatched>"`; use `itertools.count`/a lock or `prometheus_client`.

**B8 — Retention race with API-side caching.** Cleanup de-lists then deletes tiles for frames older than exactly `retention` (`tile_cleanup/activities.py:109-119`, `ingest_mrms/activities.py:455-464`), while the API caches the manifest 15 s (`server.py:94, 205-209`) and clients cache it another 15 s (`:206`). A client on the frame at the 4-h edge gets `no-store` 404s for up to 30 s. [tonight]: delete tiles at `retention + 5 min`, de-list at `retention`.

**B9 — Rate-limit bypass via `X-Forwarded-For`.** `server.py:110-114` trusts the first XFF hop; Caddy's `reverse_proxy` *appends* to an incoming XFF, so a client sets a random XFF per request and never shares a bucket. [tonight]: take the *last* hop (Caddy's), or have Caddy set `header_up X-Forwarded-For {remote_host}` (overwrite) in the `/api/*` and `/v1/*` handles.

**B10 — No fsync on any state file or tile.** `manifest.py:205-216`, `state.py:65-78`, `storms.py:294-304`, `nowcast/activities.py:111-121`, `lightning:109-111`, `tiler.py:212` write+`os.replace`/`os.rename` with no `fsync` (only `grid_dump.py:100-101,124` does it). A node power loss inside the window can leave a renamed tile dir whose PNGs are zero-length; Caddy serves a 200 with an empty body under `immutable, max-age=86400`. Low probability, ugly consequence. [design-lite]: `fsync` the staging directory + `fdatasync` PNGs once per pyramid before rename (or accept and add a `Content-Length: 0` → error rewrite in Caddy).

**B11 — pygrib handles leak on exception paths** (`ingest_mrms/activities.py:164-173`, `ingest_hrrr/activities.py:318-352`). [tonight]: `try/finally: grbs.close()` as AQM does.

**B12 — HRRR full-download fallback can outlive the heartbeat.** `timeout=300` (`ingest_hrrr/activities.py:295`) vs `heartbeat_timeout=180s` (`workflows/ingest_hrrr.py:109`) with only one heartbeat before the download (`:592`). [tonight]: stream with `client.stream` and heartbeat per chunk, or run it under `run_sync_with_heartbeat`.

**B13 — Temporal client singleton** (`api/api/temporal_client.py:26-31`): no lock, no connect timeout; concurrent cold requests leak channels; first-connect failure surfaces as a 500 not 503. [tonight]: `asyncio.Lock` + `asyncio.wait_for(Client.connect(...), 5)`.

**B14 — uvicorn listens on `0.0.0.0:8000`** (`api/start.sh:6`), reachable on the pod IP bypassing Caddy (and its error/no-store chain). [tonight]: `--host 127.0.0.1`.

**Checked and found OK:**
- *Path traversal:* `_sample_grid_point` strips everything but `[A-Za-z0-9:\-_+.T]` (`server.py:264-265`) — no `/`, so at worst `..` becomes the filename `...meta.json` inside the layer dir (verified with pathlib); `_grid_binary_path` rejects a `data_file` outside the meta's dir (`:167-174`); `wind_field` same charset (`:457`); `get_basemap_style` rejects `/`, `..`, `\\` (`:768-772`) and the router never passes a `/` in a path segment. Tiles are served by Caddy's `file_server`, which sanitizes.
- *httpx timeouts:* every call passes an explicit timeout (`server.py:68`; `ingest_mrms/activities.py:119,153`; `ingest_hrrr/activities.py:184,196,205,295,310`; `ingest_airquality/activities.py:116,370`).
- *API blocking the loop:* disk endpoints are plain `def` (threadpool) per the documented rule (`server.py:194-199`); only `/api/forecast` (pure httpx) and `/api/livez` are `async`. `authenticated_user` is CPU-only.
- *Processed sets:* bounded (`state.py:42-46`; 2000/200/100 entries).
- *Manifest never advertises tiles that don't exist:* MRMS adds after all palettes are renamed in (`ingest_mrms/activities.py:359-418`); nowcast/HRRR/AQM swap the layer only after every frame exists (`nowcast/activities.py:381-402`, `ingest_hrrr/activities.py:644-700`, `ingest_airquality/activities.py:412-472`); cleanup de-lists before deleting.

---

## 4. Scale-out readiness (N stateless tile-servers on different nodes)

Everything below assumes one POSIX filesystem shared by exactly one writer pod and one reader pod on the same node (RWO Longhorn + podAffinity). Inventory:

| Shared-FS assumption | Where | Object storage (RustFS S3) | Metadata store |
|---|---|---|---|
| `manifest.json` + `manifest.lock` (`fcntl.flock`) | `shared/manifest.py:109-148, 168-202`; read by API `server.py:177-179` | — | **Single document with CAS.** Redis `WATCH/MULTI` or `SET key val XX` with a version field; or Postgres one-row table `manifest(id=1, body jsonb, version int)` updated with `WHERE version = $expected`. API reads it (cached 15 s as today). The whole `update_manifest_file(fn)` contract becomes `ManifestStore.update(mutator)` retrying on CAS conflict. |
| Tile pyramids: write via `os.rename` of a dir; serve via Caddy `file_server` from `/data/tiles`; cleanup walks dirs; prefetch planner `is_file()`s tiles | `tiler.py:237-260`; `Caddyfile:8-15`; `tile_cleanup/activities.py:55-120`; `shared/storm_prefetch.py:160-163` | **Yes.** Key = `tiles/{layer}/{palette}/{path}/{z}/{x}/{y}.png` (same as today's URL). Per-object PUT is atomic; the "pyramid appears atomically" property is replaced by "manifest is the publish boundary" — which is already the contract (§3 OK list). Serve with Caddy `reverse_proxy` to RustFS on a public-read bucket (or S3 presign-less anonymous GET) — N Caddy replicas, no PVC. Retention = **bucket lifecycle rules per prefix** (`radar/` 4 h, `nowcast/` 1 h, `*/runs/` 12 h) + manifest de-listing with a grace margin; `tile_cleanup` stops walking directories. Prefetch planner stops `stat`ing: with indexed PNGs + known bbox you can list keys per pyramid once at publish and store the tile list (or a bitmap) in the manifest frame. | — |
| Grid dumps (`.bin` + `.meta.json`, generation pointer) | `shared/grid_dump.py`; API `server.py:262-336, 440-552`; nowcast `_list_recent_grids` glob (`nowcast/activities.py:86-95`) | **Yes** for the bytes (`grids/{layer}/{ts}.{gen}.bin` + meta). The API's 4-byte `seek` reads (`server.py:302-315`) become S3 `Range` GETs — too slow per point; instead have the API pull whole grids (≤3.6 MB) and cache them per `(layer, ts)` in memory (an LRU of ~50 grids ≈ 180 MB). Nowcast reads its 4 inputs by listing the prefix. | Meta JSON can live next to the object or in the metadata store keyed by `(layer, ts)`. |
| `ProcessedSet` files | `shared/state.py`; `ingest_mrms/activities.py:210-214`, `ingest_hrrr:139-140`, `ingest_airquality:92-93`, `nowcast:248,413` | — | Redis `SET`/`ZADD` (with trim) or Postgres `processed(scope, key, at)`; even simpler: **Temporal is already the source of truth** — a per-run workflow id (`hrrr-<run_id>`) with `REJECT_DUPLICATE` makes the set unnecessary for HRRR/AQM/nowcast; MRMS keeps a small keyed set. |
| `storms.json` + `storms.lock`, `nowcast-status.json`, `lightning.json`, `tropical.json`, `alerts_active_snapshot.json`, `push_tokens.sqlite` | `shared/storms.py:279-282`; `nowcast/activities.py:103-121`; `ingest_lightning:35,109-111`; `server.py:158-164, 563, 587, 613, 678`; `push_tokens.py:22` | JSON blobs could be S3 objects (`If-Match` CAS on RustFS needs verifying) but a 2-s lightning cadence argues against object storage. | **Redis keys** (`radar-ng:storms`, `radar-ng:lightning`, …) or a Postgres `kv(name, body jsonb, updated_at)` table; API reads with the same 10–30 s cache it has now. `push_tokens.sqlite` → Postgres table (or keep on the worker and route via Temporal, as the prior fix did). |
| Worker-local tmp | `/tmp/{mrms,hrrr,aqm}_work` | fine as is (ephemeral per pod). | |
| `shutil.disk_usage(TILE_DIR)` in health | `server.py:733-745` | drop, or report bucket stats. | |
| API filesystem reads generally | `server.py:29, 158-164, 262-336, 457-501, 563-594, 613-617, 678-685` | | The API becomes a pure network client (S3 + Redis/Postgres) → stateless, `replicas: N` with pod anti-affinity, HPA on CPU. |

**Minimal seam (do this first, no behaviour change):**
```python
# backend/stores/tiles.py
class TileStore(Protocol):
    def put_pyramid(self, layer: str, palette: str, path: str, tiles: Iterable[tuple[int,int,int,bytes]]) -> int: ...
    def delete_pyramid(self, layer: str, palette: str, path: str) -> None: ...
    def list_pyramids(self, layer: str, palette: str) -> list[str]: ...
# backend/stores/manifest.py
class ManifestStore(Protocol):
    def read(self) -> dict: ...
    def update(self, mutate: Callable[[dict], dict]) -> dict: ...   # CAS loop inside
# backend/stores/state.py
class StateStore(Protocol):            # processed sets + small JSON blobs
    def processed(self, scope: str) -> ProcessedSet: ...
    def put_json(self, name: str, body: dict) -> None: ...
    def get_json(self, name: str) -> dict | None: ...
```
`render_tiles` should return `(z, x, y, png_bytes)` instead of writing (it already builds each tile in memory, `tiler.py:194-212`); `render_tiles_atomic` becomes `LocalFsTileStore.put_pyramid` (staging dir + rename) and `S3TileStore.put_pyramid` (parallel PUTs, 8–16 in flight). Then:
1. **Dual-write phase:** `FanoutTileStore(LocalFs, S3)` in the worker; `ManifestStore = DualManifestStore(file, redis)`; API keeps reading files. Compare: object count per pyramid vs local file count in a nightly check.
2. **Read switch:** Caddy `handle /tiles/*` → `reverse_proxy` RustFS; API reads manifest/state from Redis. Scale tile-server to N; drop its PVC mounts.
3. **Drop local writes;** replace `tile_cleanup` with lifecycle rules + manifest de-list; per-role worker pods (`WORKER_ROLE=mrms|nowcast|hrrr|aux|alerts`) now schedule anywhere because no RWO volume pins them.

Cost model: ~1,845 tiles/frame today (≈600 at ~4× smaller size after §2.1) every 2 min × 2 MRMS layers + nowcast 36 pyramids/2 min + HRRR 144/hour → a few thousand PUTs/min; RustFS handles that, but batch them (thread pool) and keep the per-frame budget under ~10 s.

---

## 5. Refactor plan for maintainability

**Duplication (verbatim or near-verbatim):**
- `_safe_path_part` + `_current_activity_tmp_dir`: `ingest_mrms/activities.py:217-235`, `ingest_hrrr/activities.py:143-180`, `ingest_airquality/activities.py:285-303`.
- `_load_palette_tables`: `ingest_mrms:180-189`, `nowcast:296-305`, `ingest_hrrr:545-554`, `ingest_airquality:273-282` (4 copies, two of them differ in whether they index `["reflectivity"]`).
- `_normalize_lons`, `_extract_native_projection`, `ExtractedGrid`/`_Grid`, `_grid_dump_axes`, `_safe_grid_dump`, `_write_palette_tiles`, `_render_per_palette`: HRRR `:235-276, 126-133, 366-379, 382-432` vs AQM `:124-162, 82-90, 257-270, 205-254` — near-identical.
- Four cleanup walkers with drifting semantics: `mrms_cleanup` (`ingest_mrms:439-468`), `hrrr_cleanup` (`ingest_hrrr:723-776`), `aqm_cleanup` (`ingest_airquality:490-522`), `_sweep_layer` (`tile_cleanup:55-120`). Retention constants are also duplicated: `tile_cleanup.LAYER_RETENTION_MIN` (`:31-45`) vs `workflows/ingest_mrms.RETENTION_HOURS=4` (`:44`) vs `workflows/ingest_hrrr.RETENTION_HOURS=12` (`:39`) — three places to keep in sync.
- Run-finder pattern (`_find_latest_run_sync`) in HRRR `:187-201` and AQM `:104-121`.

**Config sprawl:** `TILE_DIR/STATE_DIR/GRID_DIR` are read at import time in 9 modules (`shared/manifest.py:19`, `shared/grid_dump.py:26`, `ingest_mrms:39-40`, `nowcast:32-34`, `ingest_hrrr:35-36`, `ingest_airquality:44-45`, `ingest_lightning:34`, `tile_cleanup:25-26`, `api/api/server.py:32-34`, `shared/push_tokens.py:21`), plus ~45 other `os.environ.get` sites across the scope with three different int-parsing idioms (`int(os.environ.get(..))` vs `_int_env` duplicated in `ingest_mrms:52-59` and `temporal/worker.py:218-226`). Import-time reads force tests to monkeypatch module globals and make "what does prod run with?" unanswerable from one file. `docs/configuration.md` drift was already noted last time.

**Logging:** three systems — `backend.shared.logger.get_logger` (JSON, `extra=` fields) in activities; `loguru` in `temporal/worker.py:15`; stdlib `logging.getLogger(__name__)` with no configuration in `server.py:44`/`routes_workflows.py:38` (so API warnings go out in uvicorn's default text format, unstructured, and `request` context is never attached).

**Typing/serialization:** activity I/O are plain `@dataclass`es (good for Temporal), but request bodies in the API are pydantic while manifest frames are untyped `dict[str, Any]` (`manifest.py:64-76`), so the frame schema (`path`, `kind`, `lead_minutes`, `run_id`, `complete`) exists only by convention in 4 writers. Define `Frame`/`Layer`/`Manifest` pydantic models once and have `_normalize_frame` validate.

**Suggested layout:**
```
backend/
  core/        settings.py (pydantic-settings Settings; single get_settings()), logging.py (one JSON logger, uvicorn + loguru sinks), metrics.py (prometheus_client registry + histograms)
  stores/      tiles.py, manifest.py, state.py, grids.py  (+ fs/ and s3/ implementations, §4)
  render/      classify.py (edges→index, LUTs), reproject.py (tile geometry + cache), png.py (P-mode encode, PLTE swap), pipeline.py (Grid → pyramids for N palettes)
  sources/     mrms.py, hrrr.py, aqm.py  (list/find, download, decode → Grid; nothing else)
  pipelines/   frame.py (Grid → tiles + grid dump → publish), run.py (versioned-run publish for hrrr/aqm/nowcast), cleanup.py (one walker/lifecycle policy)
  activities/  thin Temporal wrappers: tmp dir, heartbeat, call pipeline, map exceptions
  api/         unchanged surface; reads only via stores/
```
Migrate by moving code, not rewriting: `sources/hrrr.py` is today's `:183-355`; `pipelines/run.py` is `_publish_hrrr_run_sync` + `_publish_run_sync` with the layer table injected.

**Build/deps:** `backend/base/Dockerfile` is a zombie (neither `api/Dockerfile:6` nor `temporal/Dockerfile:9` uses it). `temporal/Dockerfile:18-23` leaves `build-essential gcc g++` in the runtime image (no multi-stage), has no `USER`, and installs 8 `>=`-pinned requirement files (`:47-55`) — builds aren't reproducible and Renovate can't see the effective versions; `numpy` is constrained `<2` in `base/` and `nowcast/` but not in `ingest_mrms/requirements.txt`. Adopt one `pyproject.toml` + `uv.lock` (worker and API extras), multi-stage build, and a `requirements-dev` so `pytest` runs the same way in CI as it did here.

**Test gaps** (16 test files, 45 tests; the render/ingest core is essentially untested):
- No test renders a real pyramid and asserts pixel output — `test_tiler.py:29` only checks files exist for z4–5. Add a golden-tile test (small synthetic grid → known PNG bytes) before touching the tiler.
- No test for `ingest_mrms/activities.py` at all; no HRRR test for the 404/pending path (B1), the resume path, or `_publish_hrrr_run_sync` gating.
- No concurrency test for `update_manifest_file` (two processes) or for the M9 read-error path.
- No test for `server.py` middleware (rate limit, XFF, 404 cardinality) or for `read_manifest_file` fallbacks; `test_server_concurrency.py` covers the def/async rule only.
- No test for `ingest_lightning` (decoder, prune, heartbeat task survival).
- No end-to-end "publish invariant" test: after any pipeline, every manifest frame × palette has ≥1 tile on the store — cheap to write against `LocalFsTileStore` and the fake S3 later.

---

## 6. Observability gaps

**Worker (nothing today except JSON logs):**
- Temporal SDK metrics are one line away and off: `Runtime(telemetry=TelemetryConfig(metrics=PrometheusConfig(bind_address="0.0.0.0:9464")))` before `Client.connect` (`temporal/worker.py:229-235`) gives activity latency/failures/schedule-to-start, slot usage, poll counts per task queue — exactly what was needed to see the 08-31 slot/queue stall. Add a `ServiceMonitor` in the GitOps repo.
- App metrics via `prometheus_client` in the same process: `radar_frame_render_seconds{layer,palette}` histogram, `radar_tiles_written_total{layer,zoom}`, `radar_frame_publish_lag_seconds{layer}` (= now − frame timestamp at manifest add, the real freshness SLI), `radar_backlog_total{layer}` gauge from `ListKeysResult.backlog_total` (`ingest_mrms/activities.py:89, 261`), `radar_hrrr_hours_pending{run}`, `radar_nowcast_seconds{phase=pysteps|render}` and `radar_nowcast_status{reason}` (today only in a JSON file), `radar_lightning_buffer_size`, `radar_tmp_bytes`. `ProcessFrameResult.duration_s` etc. already exist (`:98`) — they just aren't exported.
- Traces: `temporal/shared/otel.py` instruments workflows/activities; add manual spans around download/colorize/tile/publish inside `mrms_process_frame` so the flame graph in §2 becomes measured, not estimated.

**API:**
- No tracing at all; add `opentelemetry-instrumentation-fastapi` + `-httpx` (the same OTLP endpoint the worker uses) and Caddy's `tracing` directive so a request id flows Caddy → uvicorn → Open-Meteo/Temporal.
- Replace hand-rolled counters (`server.py:96-107, 141-146`) with `prometheus_client` — histograms for request duration (currently only a sum, `:146`), bounded labels (B7), and process metrics for free. Keep `/api/metrics`.
- Caddy access logs: `format console` (`Caddyfile:64-65`) → `format json` and `header X-Request-ID {http.request.uuid}` so tile 404 rates per layer/palette (the client-visible failure) can be graphed; today there is no signal for "how many tile requests 404".
- `/api/health` mixes a data-freshness SLI with an availability check and returns 503 (`server.py:757`) — fine as long as nothing probes it, but export `mrms_age_s` and `nowcast.status` as gauges so alerting doesn't scrape health.

**Alerting worth adding once the above exists:** `radar_frame_publish_lag_seconds > 300 for 5m` (the 4-hour outage would have paged in 5 min), `increase(radar_hrrr_hours_pending) with no publish in 3h`, `temporal_schedule_missed` on the three wedged schedules, worker RSS > 80 % of limit.

---

## Priority list

**Tonight (small, testable):** B2 push-token RO writes · B1(a)+(b) HRRR 404→pending + idempotent resume · B3 manifest read-error re-raise · B6 manifest update off the loop · B4/B11 `finally` cleanup + pygrib close · B5 lightning heartbeat guard + `to_thread` · B7 metrics cardinality · B9 XFF · `MRMS_RENDER_WORKERS=3` · science-grid keep-N · B12/B13/B14.

**Design change (PR review):** indexed-PNG single-classify pipeline (§2.1–2.2) · persistent spawn pool / shared memory · progressive HRRR publish · `TileStore`/`ManifestStore`/`StateStore` seams → RustFS + Redis/Postgres (§4) · per-role worker pods · module layout + settings + one lockfile (§5) · Prometheus/OTel wiring (§6).
