# radar-ng full-repo review — 2026-06-10

Deep review across the four subsystems (Temporal orchestration, Python ingest
pipeline, API/serving/deploy, Expo frontend). This doc records **what was
fixed in this pass**, the **verified backlog** of remaining issues, and
**Temporal learning notes** keyed to this codebase.

Findings below were verified against the code — several plausible-sounding
issues from the first review pass turned out to be wrong and are listed at the
bottom so they don't get "re-found" later.

---

## 1. Fixes applied in this pass

### Nowcast manifest accumulation (correctness, user-visible)
**Files:** `backend/shared/manifest.py`, `backend/nowcast/activities.py`,
`backend/nowcast/nowcast.py`, `backend/shared/test_manifest.py`

Every nowcast run `add`ed its 12 future-timestamped frames to the manifest and
nothing removed the previous run's frames (tile-cleanup only prunes frames
whose *valid time* is >60 min past). With a run every ~2 min, the manifest's
nowcast layer accumulated frames from ~30 anchor runs, and the app's forecast
window (`now..+60min` in `useManifest.ts`) interleaved prediction vintages —
a frame every ~2-3 min, most of them stale predictions from older model runs.

Fix: new `replace_layer_manifest()` atomically swaps a layer's entire
timestamp list under the same fcntl lock. Nowcast now renders all leadtimes
first, then publishes once. Side benefits: a half-finished run is never
visible, and old tile dirs simply age out via the existing cleanup sweep.

### gRPC channel leak in alert signaling
**File:** `backend/api/api/storm_watch_activities.py`

`signal_matching_storm_watches` called `Client.connect()` per invocation
(once per new NWS alert, forever). The Python SDK has no `Client.close()` —
channels die only with the process. Now reuses the
`backend/api/api/temporal_client.py` singleton.

### Alert seen-state: torn writes + arbitrary eviction
**File:** `backend/api/api/storm_watch_activities.py`

`_save_seen` was a non-atomic `write_text` (activity retries could tear it,
re-firing every active alert), and capped with `list(set)[-5000:]` — sets are
unordered, so the cap could evict a *currently active* alert id, which would
then re-notify as "new". Now: atomic tempfile+`os.replace`, and active ids
always survive the cap (expired ids fill the remainder).

### PollAlertsWorkflow determinism + wasted activities
**File:** `temporal/workflows/poll_alerts.py`

Replaced `hasattr` duck-typing in the workflow body with up-front
normalization (payload-shape sniffing in workflow code is a replay-safety
smell), and skip geometry-less alerts entirely — the activity returned
`matched=0` for those anyway, so each was a pointless activity execution.

### MRMS workflow: one bad frame no longer kills the run
**File:** `temporal/workflows/ingest_mrms.py`

- A frame that exhausted retries raised through `asyncio.gather`, failing the
  workflow and cancelling sibling frames mid-render. Now caught per-key
  (`ActivityError` → `rendered=False`); the frame stays unmarked so a later
  run can retry it.
- Added `schedule_to_close_timeout=30min` to `mrms_process_frame`: total
  budget across retries + queue wait. Without it, 3 × 20-min attempts could
  pin one run for ~an hour while `OverlapPolicy.SKIP` dropped every newer
  frame — stale radar for the duration.

### Graceful worker shutdown
**Files:** `temporal/worker.py`, `temporal/open_meteo_worker.py`

k8s SIGTERM previously killed the event loop outright — in-flight renders died
mid-write. Both workers now trap SIGTERM/SIGINT → `worker.shutdown()` with
`graceful_shutdown_timeout=25s` (env-tunable via `TEMPORAL_GRACEFUL_SHUTDOWN_S`
on the main worker; keep it under `terminationGracePeriodSeconds`).

### Cleanup ordering: manifest before rmtree
**Files:** `backend/tile_cleanup/activities.py`,
`backend/ingest_mrms/activities.py`, `backend/ingest_mrms/ingest.py`,
`backend/ingest_hrrr/activities.py`

All sweeps deleted tiles *then* de-listed them from the manifest — a window
where the app fetches a manifest advertising tiles that 404. Order swapped
everywhere.

### API input validation
**File:** `backend/api/api/routes_workflows.py`

`/v1/push-tokens` and `/v1/watches` are unauthenticated and internet-facing
with zero bounds: arbitrary-size `user_id`/`token` flowed into workflow IDs,
the push-token DB, and Temporal history. Now: length caps (user_id ≤128,
token 16–512), `Literal["ios","android"]`, lat/lng range checks.

### Frontend: error boundary + query GC + stale node_modules
**Files:** `frontend/src/app/_layout.tsx`, `frontend/package-lock.json` (new)

- Root `export { ErrorBoundary } from "expo-router"` — previously any throw
  (Skia worklet edge case, MapLibre native error) red-screened the app.
- `gcTime` 10 min → 5 min (weather data is stale in minutes; inactive queries
  were piling up on low-end devices).
- **`node_modules` was stale vs `package.json`** (OTEL pinned at 0.56/1.30
  installed vs 0.218/2.7.1 declared, MapLibre types drifted too). On the old
  installed sdk-logs, the `processors:` constructor option doesn't exist —
  the app was **silently dropping every OTLP log**. `npm install` + dedupe
  fixed it; 22 tsc errors → 0, jest 5/6 suites → 6/6 (26 tests).
  **Commit `package-lock.json`** so the tree can't drift again.

---

## 1b. Deploy/CI verification pass (2026-06-11)

Verified against the LIVE talos-argocd-proxmox manifests (fetched from
GitHub), not just the reference copies in `deploy/k8s/`.

**Confirmed healthy:**
- PVCs are `ReadWriteMany` on `truenas-nfs` — the "RWO with 2 replicas"
  concern from the first pass is definitively false.
- Temporal cutover is fully live: kustomization includes the worker CRs and
  the legacy ingest deployments are gone.
- Worker pods: non-root, seccomp, dropped caps, sane requests/limits.
  Progressive rollout (10% → 50%) with sunset delays via the Worker
  Controller. Default 30s `terminationGracePeriodSeconds` fits the new 25s
  graceful drain.
- tile-server: HPA 3-8 @70% CPU, topology spread, PDB, memory limits tuned
  after real OOM history.

**Fixed in this pass:**
- **`build-api.yml` restored** (`.gitea/workflows/build-api.yml`). Commit
  95767c3 removed it as a "zombie" alongside the genuinely-dead per-service
  builds — but `radar-ng-tile-server` is the live public API image (talos
  pins v1.0.10, Renovate bumps from registry tags). Since May 22, changes
  under `backend/api/**` built nothing and could never ship. Also fixed a
  pre-existing gap: the old workflow didn't watch `backend/shared/**`,
  which the image COPYs.
- **`/v1/*` was unreachable through Caddy** (`backend/api/Caddyfile`). The
  workflow router mounts at `/v1` but Caddy only proxied `/api/*` —
  `/v1/push-tokens` and `/v1/watches` died at Caddy with an empty 200,
  never reaching FastAPI. Unnoticed because the mobile app doesn't call
  `/v1` yet. Added the `handle /v1/*` reverse-proxy block.
- **`/api/livez` added** (`backend/api/api/server.py`) as the correct probe
  target: 200 iff the FastAPI process answers, probed via Caddy:8080 so it
  exercises both processes in the pod. `/api/health` must NOT be a probe —
  it reports degraded on stale radar *data*, a condition shared by every
  replica; wiring it to liveness/readiness would restart or drain the whole
  fleet when NOAA is slow.

**Needs a talos-repo change (user action):**
- tile-server has **zero probes** today. `/start.sh` backgrounds uvicorn
  under Caddy — if uvicorn dies, the pod stays Ready and 502s `/api/*`
  forever. Once a tile-server image with `/api/livez` is deployed, add:
  ```yaml
  livenessProbe:
    httpGet: { path: /api/livez, port: 8080 }
    initialDelaySeconds: 10
    periodSeconds: 15
  readinessProbe:
    httpGet: { path: /api/livez, port: 8080 }
    periodSeconds: 10
  ```
- Temporal worker pods also have no probes (lower stakes — Temporal
  server-side timeouts catch dead workers; a file-touch exec probe is the
  cheap option if wanted).

## 2. Verified backlog (not applied — recommend in this order)

1. **Delete (or extract) the legacy sync runners.** Each ingest service has a
   near-duplicate `ingest.py` (standalone loop) and `activities.py` (Temporal)
   — ~100-150 duplicated lines apiece, already diverging (the sync MRMS path
   has no ProcessPool). Temporal owns orchestration now; the sync runners are
   drift liabilities. Either delete them or thin both onto shared helpers.
2. **Atomic tile-pyramid publish.** `mrms_process_frame` writes PNGs in place;
   a crash leaves a partial pyramid, and a palette that fails while another
   succeeds publishes a manifest entry missing that palette's tiles. Render to
   `.{ts}.tmp/` and `os.rename` per palette before the manifest add.
3. **Rate limiting + auth.** Nothing limits `/api/*` or `/v1/*`. Cloudflare is
   in front — a WAF rate rule is the cheapest win; a static bearer header
   checked in Caddy would cover the mutating `/v1/*` routes.
4. **Worker liveness probes.** `deploy/k8s/temporal-worker-deployment.yaml`
   has no probes; a deadlocked worker pod sits Ready forever. Cheap version:
   touch a file in an activity interceptor + `exec` probe on its mtime.
5. **Disk-full behavior.** A full tiles PVC fails renders mid-frame with no
   pre-check and no metric. Check `shutil.disk_usage` before render; emit a
   `radar_ng_pvc_free_bytes` gauge. (Related: registry-GC followup from
   2026-05-03 — same class of problem.)
6. **Pin Caddy in `backend/api/Dockerfile`** — it downloads "latest" from the
   Caddy CDN at build time; builds aren't reproducible.
7. **pytest gate in CI.** Gitea workflows build images without running tests.
8. **HRRR forecast hours sequentially processed** — fan out
   `hrrr_process_forecast_hour` activities from the workflow (bounded by a
   semaphore like MRMS) instead of looping inside one activity.
9. **Tile prefetch during playback** — RadarOverlay mounts one frame at a
   time; prefetching `frames[i+1]` would remove the first-play stutter.
10. **Offline fallback** — persist last-good manifest to MMKV; blank radar on
    network loss today.
11. **EAS Updates (OTA)** — no `eas.json`; UI fixes currently require a full
    store cycle.
12. **Nowcast→HRRR blend** — the +60min nowcast frame hands off to the +1h
    HRRR frame with a visible model discontinuity; a weighted blend of the
    last 2-3 nowcast leadtimes would smooth it.
13. **CARROT-style polish gap** — home/alerts tabs hit the bold-card Cumulus
    look; the radar tab is functional-dark. Inspector readouts are raw numbers
    ("42 dBZ") — phrase-ify them; consider a "Storm risk" context card on the
    timeline during active warnings.

## 3. Claims from review agents that did NOT survive verification

Recorded so future reviews don't re-flag them:

- **"HRRR range-GET partial writes corrupt files on retry"** — false: each
  attempt gets a fresh tmp dir (`attempt{n}` in the path) and the caller
  rmtree's on failure; `open("wb")` truncates besides.
- **"MRMS cleanup can delete just-written tiles"** — false: the cutoff
  compares the *timestamp parsed from the dir name*, not mtime; a fresh frame
  is by definition inside retention.
- **"S-PROG should use all 4 input frames, not `stack[-3:]`"** — false: AR(2)
  consumes exactly `ar_order+1 = 3` frames; extras are ignored by pysteps.
  The 4th frame still improves the LK optical-flow estimate, which does use
  the full stack.
- **"iOS deploymentTarget 26.0 is a typo"** — false: Apple's year-based OS
  versioning; iOS 26 is real.
- **"workflow.now().timestamp() is a determinism bug"** — false: it's the
  replay-safe clock; `.timestamp()` is a pure conversion.

---

## 4. Temporal learning notes (keyed to this repo)

What this codebase already does *right* — worth internalizing as patterns:

- **Determinism discipline**: workflow files import activity code only inside
  `workflow.unsafe.imports_passed_through()`; no wall-clock/random/I-O in
  workflow bodies; `workflow.now()` for time.
- **Schedules over cron**: every ingest is a Schedule with
  `OverlapPolicy.SKIP` + 1h catchup window — fresher data beats backfill for
  radar; a down worker doesn't cause a thundering herd on recovery.
- **Heartbeats for long activities**: `run_sync_with_heartbeat` pumps
  `activity.heartbeat()` from the event loop while CPU work runs in a thread;
  `heartbeat_timeout` is what lets Temporal detect a *hung* worker quickly
  instead of waiting out the full `start_to_close`.
- **State on disk, not in history**: frame keys live in `ProcessedSet` files
  on the PVC; workflow history carries only small dataclasses. Temporal
  history is an event log, not a database — payloads are replayed on recovery.
- **Worker versioning**: `WorkerDeploymentConfig` + `PINNED` means in-flight
  workflows finish on the build they started on; new runs go to the new build.
  This is why you mostly haven't needed `workflow.patched()` yet.

Concepts this pass added — the *why*:

- **`start_to_close` vs `schedule_to_close`**: start_to_close bounds ONE
  attempt; schedule_to_close bounds the whole scheduled lifetime including
  retries and queue wait. With only the former, retry policy multiplies your
  worst case (3 × 20 min). Set both on anything driven by a SKIP-overlap
  schedule, or one sick input starves the pipeline of fresh runs.
- **Partial failure inside a fan-out**: `asyncio.gather` in a workflow
  propagates the first `ActivityError` and cancels siblings. If items are
  independent (frames), catch per-item and return a "failed" result instead.
- **Activities must manage connections like server code**: they run in a
  long-lived worker process. Per-invocation `Client.connect()` is a leak;
  module-level singletons are the norm.
- **Graceful drain**: `worker.shutdown()` stops polling and gives in-flight
  activities `graceful_shutdown_timeout` to finish, then cancels. Without a
  SIGTERM handler none of that machinery runs — k8s just kills the loop.
- **Replace vs add for derived data**: when every run supersedes the last
  (forecasts), publish with a replace-semantics commit, not incremental adds.
  Incremental is for append-only observations (MRMS frames).
