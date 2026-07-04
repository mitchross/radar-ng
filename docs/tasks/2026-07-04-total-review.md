# radar-ng — total review (2026-07-04)

Full-stack review: frontend, backend API, ingest pipeline, Temporal
orchestration, deploy/CI. Every finding below was verified against the code at
`84015b8` (file:line refs). Companion deliverable: the reusable debug harness
at `tools/debug_harness/` ([docs/debug-harness.md](../debug-harness.md)),
whose checks are designed to catch the operational failure modes listed here.

## Docs-vs-reality drift (read first — it reframes two "known" facts)

1. **The 5-slot frame carousel does not exist.** ARCHITECTURE.md describes
   `frontend/src/lib/radarCarousel.ts` + a 5-slot opacity-swap carousel in
   detail; the file has never existed (`git log` empty, no "carousel" anywhere
   in `src/`). Playback is still the old design: `RadarOverlay.tsx:54-62`
   remounts a single `RasterSource` per 420 ms tick (`key` embeds
   `frame.path`), with no prefetch and `raster-fade-duration: 0`. The map-jank
   diagnosis in `docs/tasks/2026-07-02-radar-ng-followups.md` (P2) is still
   the live state.
2. **Backend OTel does not exist.** README/ARCHITECTURE mention OTel; only the
   Temporal workers have a tracing interceptor (`temporal/shared/otel.py`,
   enabled by `OTEL_EXPORTER_OTLP_ENDPOINT`) and the mobile app has a client
   (`src/lib/telemetry.ts`). `backend/api` has zero tracing/metrics middleware
   — the API's observability surface is `/api/health`, `/api/metrics`
   (in-process counters), and Caddy/uvicorn stdout.

---

## Critical

**C1 — Manifest can advertise frames that have zero tiles; frame still marked
processed.** `backend/ingest_mrms/activities.py:337-341,362,365-371` +
`backend/shared/tiler.py:246-249`. `_render_all` counts a palette as rendered
whenever the future doesn't raise, but `render_tiles_atomic` returns 0 tiles
for a fully-transparent frame *without creating the output dir*; and if every
palette raises (ENOSPC, PIL error), the manifest add still runs with
`palettes=[]` (falsy → defaults to `["classic"]`), the activity returns
`rendered=True`, and `mrms_mark_processed` permanently skips the frame. Same
pattern in HRRR (`ingest_hrrr/activities.py:397-409`) and nowcast
(`nowcast/activities.py:152-153`). Client impact: permanently blank/404
frames in the playback loop. *(Harness: `disk.manifest.<layer>` ghost check,
`tiles.<layer>` 0-hits check.)*

**C2 — Nowcast frames are time-stretched 2.5×.**
`backend/nowcast/activities.py:33-35,76-77,231-233`. pysteps extrapolation
steps inherit the input frame interval (MRMS = 2 min), but leadtimes are
labeled `latest_dt + (i+1)×NOWCAST_STEP_MIN` (5 min). The published "+60 min"
frame is really a +24 min extrapolation; storm motion in the nowcast layer
plays ~2.5× slower than reality. Fix: resample inputs to 5-min spacing or
label with 2-min steps.

**C3 — `storms.json` torn-write race between base/composite schedules.**
`backend/ingest_mrms/activities.py:308` + `backend/shared/storms.py:100-105`.
Both MRMS schedules (and 2 concurrent frames within each) write storms via the
same fixed `storms.json.tmp` with non-atomic `write_text` + `replace` — torn
JSON can be published, and base/composite cells overwrite each other every
cycle; an out-of-order backlog frame can replace newer storms with older ones.
Storm detection should run for one canonical layer only, with `mkstemp`.

**C4 — HRRR run marked processed before NOAA finishes uploading it.**
`backend/ingest_hrrr/activities.py:171-185` +
`temporal/workflows/ingest_hrrr.py:121-143`. Run detection HEADs f01 only;
NOAA uploads hours progressively over ~40-90 min. Hours not yet uploaded
exhaust retries, are swallowed, and the run is marked processed on
`succeeded > 0` — a recurring multi-hour hole in the extended forecast that
never backfills.

**C5 — At-most-once severe-weather alert delivery.**
`backend/api/api/storm_watch_activities.py:262` +
`temporal/workflows/poll_alerts.py:64-76`. Alert IDs are committed to
`alerts_seen.json` inside `fetch_nws_active_alerts` *before* watches are
signaled, and signaling has no per-alert failure isolation — a failure on
alert N permanently drops alerts N+1..M. Compounded by
`fan_out_push_to_user` (`storm_watch_activities.py:171-179`) swallowing every
per-token push exception, so transient APNS/FCM 5xx are never retried. This is
the push path for tornado-class alerts; it should be at-least-once.

## High

**H1 — Caddy caches tile 404s hard.** `backend/api/Caddyfile:14-18` sets
`Cache-Control` unconditionally (non-deferred `header` runs before
`file_server`), so a 404 for an observed-radar tile goes out
`max-age=86400, immutable`. Combined with the non-atomic replace window in
`render_tiles_atomic` (`tiler.py:250-252` does `rmtree` **then** `rename`) a
client that fetches during a re-render pins a 404 for a day. Fix: `header
@notfound` via `handle_errors` or defer + `status 404` matcher; make replace
atomic (rename old aside, rename new in, rmtree old).

**H2 — `DELETE /v1/push-tokens` writes to a read-only mount → 500 forever.**
`deploy/docker-compose.yml:256` (`state:/data/state:ro` on tile-server, same
in docs/kubernetes.md:42) vs `routes_workflows.py:92-94` →
`push_tokens.py:47-73` (sqlite write). Every unregister 500s; stale tokens
never purge. Also H2a: that same handler is `async def` doing sync sqlite on
the event loop — the one violation of the project's own NFS-hang rule
(`server.py:113-118`), and it stalls `/api/livez`, the k8s probe. And H2b:
sqlite on an NFS-backed PVC written by two pods with no `timeout=` is a
corruption risk (`push_tokens.py:21-56`).

**H3 — Unbounded `_forecast_cache` → OOM.** `server.py:58,169`. Dict keyed by
0.1° coords, no eviction (~6.5M possible keys × tens of kB). Unauthenticated
+ CORS `*`: a coordinate sweep OOM-kills the API. Cap it (LRU) and evict on
expiry.

**H4 — Unauthenticated workflow-start endpoints.**
`routes_workflows.py:76-125`. `POST /v1/watches` creates arbitrarily many
long-running `WatchStormWorkflow`s from attacker-chosen IDs; `POST
/v1/push-tokens` lets anyone register their device under any `user_id`
(receive that user's alerts); `GET /v1/watches/{user}/{cell}` reads anyone's
state. Needs at minimum a shared-secret header + rate limit at Caddy. Related
deploy exposure: compose publishes Temporal gRPC 7233 and the UI to 0.0.0.0
unauthenticated (`docker-compose.yml:68-69,92-93`) — bind to 127.0.0.1.

**H5 — Worker-slot starvation defeats the 2-min radar budget.**
`temporal/worker.py:57` (4 slots) vs: lightning holding 1 slot ~50 min/h
(`workflows/ingest_lightning.py`), HRRR fanning out 8
(`workflows/ingest_hrrr.py:39`), plus 2×MRMS + nowcast every 2 min. During
any HRRR run, MRMS/nowcast queue behind it, burn their `schedule_to_close`
budget waiting, and SKIP drops fresh triggers — radar goes stale every hour
on the hour. Fix: raise slots, or dedicated task queues (lightning + hrrr),
or drop `FORECAST_CONCURRENCY` to ≤2. Same family: `nowcast.py:35-44` and
`open_meteo_sync.py:42-48` are missing `schedule_to_close` entirely (the
exact pin-the-schedule failure the MRMS/HRRR comments say was fixed), and
lightning's deadline is only checked per-message so an idle socket overruns
into a failed attempt + full-length retry (`ingest_lightning/activities.py:163-165`).

**H6 — Schedule reseed silently unpauses schedules.**
`temporal/schedules/seed.py:149` replaces the entire `Schedule` including
default `state` (`paused=False`). Pause `ingest-mrms-base` during an incident,
deploy or restart any worker replica → unpaused mid-incident. Preserve
`state` in the update lambda.

**H7 — Cancellation leaks: temp dirs, zombie renders, orphan subprocess.**
(a) `ingest_mrms/activities.py:346-360` / `ingest_hrrr:581-597` cleanup is
`except Exception` — `CancelledError` skips it; nothing ever sweeps
`/tmp/{mrms,hrrr}_work` (H1-class disk leak, ~50 MB per HRRR attempt).
(b) `activity_heartbeat.py:23` `to_thread` work is uncancellable — a
cancelled render keeps writing and can `os.rename` a pyramid into place while
the retry renders into the same deterministic `<ts>.tmp` (`tiler.py:234-252`)
→ interleaved/partial published pyramid. (c)
`open_meteo_sync/activities.py:60-91` never kills the Swift subprocess on
cancellation → two concurrent syncs of the same model. *(Harness:
`disk.tmp.*`, `disk.tiles.orphans`.)*

**H8 — open-meteo worker image: version pins are shell redirects.**
`temporal/open_meteo_worker.Dockerfile:38-40` — unquoted `temporalio>=1.9.0`
installs *unpinned latest* (writes junk file `=1.9.0`); the copied
requirements.txt is never installed. Main worker pins `temporalio<2`; this
image will break on the next major. Quote the specs or `pip install -r`.

**H9 — CI never runs any test or linter.** All 8 workflows
(`.github/workflows/`, `.gitea/workflows/`) are build/push only, while the
repo has backend pytest suites and frontend `tsc`/`jest`/`lint`. A red test
on master still ships a semver-tagged image that Renovate auto-deploys. Also
the Gitea tag computation (`build-*.yml`) has a read-then-push race that can
silently overwrite a tag.

**H10 — k8s worker: no `fsGroup`, no probes, undersized vs docs.**
`deploy/k8s/temporal-worker-deployment.yaml:44-49` sets `runAsUser: 1000`
without `fsGroup` (fresh PVC = root:root → first ingest write dies;
the open-meteo sibling gets this right); no liveness/readiness probe on
either worker; ships `cpu:2/mem:4Gi` while docs/kubernetes.md:57 documents
6/6Gi as the tested minimum for the render path (per-frame render peak is
~800 MB × 4 concurrent activities — OOM territory in a catch-up).

## Medium (selected — highest value)

- **M1 Frontend: `nowSec` frozen at mount** (`TimelineBar.tsx:47`) — after 30+
  min on the tab, the 1h window/NOW marker/labels drift fully into the past.
- **M2 Frontend: frame identity is a bare index across manifest polls**
  (`useManifest.ts:113-118`) — a poll that prunes head frames silently moves
  the user's paused frame / playback window forward in time.
- **M3 Frontend: stale frames retained on layer/server switch**
  (`useManifest.ts:105`) — old timestamps against the new layer/server → all
  tile fetches 404, map silently blank.
- **M4 Frontend: eyedropper + wind-field fetch storms during playback**
  (`Eyedropper.tsx:48-71` refires per 420 ms tick ≈ 2.4 req/s;
  `useWindField.ts:31-50` keys on `frame.path` → full u/v grid per tick).
- **M5 Frontend: telemetry always on to a hardcoded third-party endpoint**
  (`telemetry.ts` → `https://otel.vanillax.me`, no opt-out, includes GPS
  coords on `api.inspectPoint` spans) — privacy + self-hoster surprise;
  should default off or point at the user's server.
- **M6 API: grid dumps written non-atomically** (`grid_dump.py:90-104`) +
  `wind-field`/`inspect` decode paths raise `struct.error` → user-facing 500s
  during HRRR rewrites (`server.py:222-234,330-334`).
- **M7 API: Temporal client singleton — no connect lock/timeout; leaked gRPC
  channels on every reset** (`temporal_client.py:26-37`); first-connect
  failures are 500s not 503s.
- **M8 API: `radar_ng_tile_timestamps` gauge counts palettes, not frames**
  (`server.py:524-531` counts `{layer}/{palette}` level) — any freshness alert
  on it is blind. `/tmp/uvicorn.log` also grows unbounded (`start.sh:6`).
- **M9 Ingest: transient manifest read error rewrites manifest as empty**
  (`manifest.py:36-39` under the update lock; NFS EIO → all layers vanish
  until re-added). Same in `ProcessedSet._load` → full-day reprocess.
- **M10 Ingest: heartbeat gaps in download phases** (mrms:282-288,
  hrrr:569-575 — one beat then a long download vs 180 s heartbeat timeout;
  NOAA slow day = healthy activities killed). Downloads should use
  `run_sync_with_heartbeat` like the render phases.
- **M11 Ingest: per-frame `ProcessPoolExecutor` forked from the multithreaded
  worker** (mrms:316-344, py3.12 fork default) — fork-safety hazard +
  ~100 MB grid pickled per palette; move to spawn or a persistent pool.
- **M12 Ingest: pygrib handle leaks on exception paths** (mrms:166-176,
  hrrr:302-336); **MRMS midnight listing gap** (mrms:135-141); **tropical
  `intensity` is knots published as `wind_mph`** (ingest_tropical:45,55);
  **tile_cleanup can rmtree a live >1h render** (tile_cleanup:75-81).
- **M13 Temporal: `WatchStormWorkflow.continue_as_new` drops state + buffered
  signals** (`watch_storm.py:113` — baseline/counters reset, pending
  alert/unpin signals lost at CAN); re-watch after clean completion 409s
  forever (`routes_workflows.py:119` `ALLOW_DUPLICATE_FAILED_ONLY`).
- **M14 Deploy: compose tile-server can't self-heal a dead uvicorn**
  (`start.sh` + docker healthcheck never restarts on unhealthy); open-meteo
  serve floats `:latest` against a worker pinned 1.5.2 (the exact skew the
  pin comment warns about); k8s push Secret is documented but referenced by
  no manifest (`secret-temporal-worker.yaml.template` vs
  `temporal-worker-deployment.yaml:73-74`).

## Low (themes)

Inert basemap style-name guard (`server.py:486-490`); workflow-ID ambiguity
via `:` in `user_id` (`routes_workflows.py:100-101`); seed never prunes
removed schedules; `open_meteo_worker` ignores `TEMPORAL_TASK_QUEUE` drift
with the workflow's hardcoded queue; frontend dead settings (playback-FPS
slider, °F/°C toggle, fabricated Docker/cache stats in Settings Advanced,
`SELF_HOSTED.METRICS_PATH` defined but never fetched); unvalidated MMKV casts
crash on corrupt persisted values; hourly strip uses today's sunrise/sunset
for tomorrow's hours; `docs/configuration.md` omits ~20 consumed env vars and
documents a wrong `FORECAST_TTL_S` default (300 vs code 900);
`deploy/k8s/README.md` claims both workers poll the same task queue (false);
`backend/base/` is a zombie image nothing builds from; author emails baked
into configmap/code fallbacks.

## What got fixed in this pass

Nothing — this branch adds the review + the debug harness only
(`tools/debug_harness/`, `docs/debug-harness.md`). The harness was pointed at
the failure modes above deliberately: manifest/disk drift (C1), forecast
horizon collapse (C2's symptom), cache-header contract (H1), schedule
pause/stall/failure visibility (H5/H6), leaked tmp dirs (H7), and cold-client
playback budget (the carousel-that-isn't).

## Suggested fix order

1. C5 (alert delivery) — severity × user harm.
2. C1 + H1 together (ghost frames + cached 404s share the atomic-publish fix).
3. C2 (nowcast timing — small change, big correctness win).
4. H2 (ro-mount 500s — one-line compose/k8s change + move handler off the loop).
5. H5/H6 (Temporal capacity + reseed-unpause — operational pain).
6. H8/H9/H10 (build/CI/k8s hygiene — cheap, prevents regressions of all of the above).
