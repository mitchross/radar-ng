# Backend, Temporal and Kubernetes review, 2026-09-29

Review of the radar-ng backend, the Temporal control plane and the
Kubernetes deployment (talos-argocd-proxmox `my-apps/personal-projects/radar-ng`),
against the goal of a fully self-hosted service with no API keys. Evidence:
live cluster (`kubectl`, `temporal` CLI, Prometheus, pod logs), the 2026-09-26
k6 report, and the Mink history back to the 2026-07-15 audit.

The code changes that came out of it are in the same branch as this document;
section 4 lists the changes that belong in the gitops repository.

## 1. Where production stands (measured)

| Signal | Value | Source |
|---|---|---|
| Observed radar age, 7 d | p50 205–220 s, p95 265 s, p99 275 s, max 285 s; 0 samples above 6 min | `radar_ng_mrms_age_seconds` |
| MRMS frame render | 18–35 s (base), 26–42 s (composite) | worker `frame_done` |
| Nowcast run, v1.1.43 → v1.1.45 | 170–210 s → 113–135 s per run | Temporal start/close |
| Nowcast anchor → publish | ~6 min on v1.1.45 (was ~7–8 min) | worker `nowcast_complete` vs anchor |
| HRRR forecast hour | 30–63 s; 18 h run ≈ 4 min at concurrency 4 | worker `hour_done` |
| Air quality run | ~35 min, twice daily | Temporal, `chunk_done` |
| Lightning stream | connected 90% of the time; a server close every ~6 min cost a median 32 s (p90 123 s) of dead time; layer blanked at every hourly rollover | aux worker logs, 11 h |
| API traffic | ~12k requests/day, 9 5xx in 7 days, tile-server CPU ≈ 0 | `radar_ng_http_requests_total` |
| Worker memory peak, 24 h | nowcast 6.6–9.2 GiB (limit 12), mrms 1–2.2 GiB, others < 0.5 GiB | `container_memory_working_set_bytes` |
| Volumes | tiles 15 of 98 GiB, grids 4.3 of 20, state 2 MiB, open-meteo 19 of 59; ssd-flash disk 250 GiB free | `df`, Longhorn |
| Temporal | 5 role queues with versioned pollers, schedule-to-start p95 < 0.5 s on every queue, no slot exhaustion; `numHistoryShards: 1`, one replica per service, one Postgres | SDK metrics, `temporal` CLI |

Freshness is inside the plan's target (99% under 6 min). The remaining latency
is structural: NOAA lands a frame 1.5–2.5 min after its valid time, the ingest
schedule then waited up to 2 min for its next tick, and the nowcast waited for
its own tick and for the run in flight.

Serving is far from any limit: the 2026-09-26 stress test put the API ceiling
at ~400 req/s and tiles never bottlenecked. Nothing in this review argues for
more API capacity.

## 2. Self-hosted, no-API-key audit

Runtime dependencies of the backend, all keyless:

| Dependency | Used for | Notes |
|---|---|---|
| `noaa-mrms-pds`, `noaa-hrrr-bdp-pds`, `noaa-nws-naqfc-pds` (S3) | radar, HRRR, air quality | public buckets; NOMADS is the AQM fallback |
| `api.weather.gov` | alerts (server-side poll and `/api/alerts`) | contactable User-Agent set in gitops |
| `nhc.noaa.gov`, `mapservices.weather.noaa.gov` | tropical feed, cones and tracks | |
| Blitzortung websocket | lightning | community feed; endpoint list verified in this review |
| `basemap.nationalmap.gov` | satellite imagery | proxied by Caddy so clients never contact it; Cloudflare caches the JPEGs |
| Open-Meteo open-data S3 | model files for the in-cluster Open-Meteo | the API itself is in-cluster |
| Photon, LiteLLM/vLLM, PostHog | geocoding, narration, web analytics | all in-cluster / self-hosted |

Clients (app, web, widget, Watch) talk only to `radar-ng-api.vanillax.me` and
`maps.vanillax.me`. Remaining gaps:

- **CarPlay** still uses MapKit for the basemap (`MKMapView`), place search
  (`MKLocalSearch`) and routing (`MKDirections`). Watch and widget already use
  the self-hosted raster basemap. Self-hosting routing means Valhalla or OSRM
  plus Photon; it is the only client surface left that calls Apple at runtime.
- **Remote push** needs APNs/FCM credentials by nature; it stays disabled
  (`PUSH_DISABLED=1`, `DISABLE_WORKFLOW_ROUTES=1`). Rain-start alerts in this
  branch instead use local phone notifications, with no push token or service.
- **LLM path** goes through LiteLLM with a master key from 1Password. The key
  is internal, but pointing `LLM_BASE_URL` at the vLLM service directly removes
  both the hop and the secret.
- The tile-server image still defaults `OPEN_METEO_BASE` to the public
  `api.open-meteo.com`; gitops and compose override it. Changing the image
  default to the in-cluster name would make a missing override fail loudly
  instead of silently calling a third party.

## 3. Changes in this branch

| Change | Where | Expected effect |
|---|---|---|
| MRMS schedules poll every minute (NOAA publishes every ~2 min) | `temporal/schedules/seed.py` | average wait for a new key 60 s → 30 s; radar age p50 ≈ 205 s → ≈ 175 s |
| Every new base-reflectivity science grid kicks the nowcast Schedule with `BUFFER_ONE`; the 2-min timer stays as a backstop | `backend/ingest_mrms/activities.py` | nowcast starts seconds after the grid lands; a busy nowcast queues exactly one follow-up; anchor → publish ≈ 6 min → ≈ 4–4.5 min |
| Nowcast exits before reading ~100 MB of inputs when its newest grid is already forecast | `backend/nowcast/activities.py` | kicked and buffered no-op runs cost milliseconds |
| Lightning: verified `wss` endpoints first, legacy ports last, per-endpoint demotion and capped backoff, 1 s reconnect after a healthy close | `backend/ingest_lightning/activities.py` | dead time per server close ≈ 32 s → ≈ 1 s; ~65 min/11 h of gaps removed |
| Lightning buffer seeded from the last published file | same | no blank layer at the hourly rollover or after a worker restart |
| `/api/manifest.json` sends a strong `ETag` and answers `304` to `If-None-Match` | `backend/api/api/server.py` | the 30-s client poll becomes an empty 304 until a frame changes |
| Retained forecasts verified against later nowcast anchor observations at 20/35 dBZ, alongside a stationary baseline | `backend/nowcast/skill.py`, `backend/shared/nowcast_skill.py` | rolling 24-hour POD, FAR and CSI by lead; `/api/nowcast/skill`, health, point responses and Prometheus expose actual performance |
| Accuracy headline on Nowcast and the Home rain banner | `frontend/src/lib/nowcastSkill.ts`, `frontend/src/screens/NowcastScreen.tsx`, `frontend/src/lib/forecastView.ts` | shows the hit rate and false-alarm ratio once six runs at the selected lead are scored; headline defaults to 30 minutes |
| Local rain-start alerts with 5/10/15/20-minute warning and optional OS background refresh | `frontend/src/lib/rainAlerts.ts`, `frontend/src/lib/rainAlertScheduler.ts`, `frontend/src/tasks/rainAlertsTask.ts`, Settings | phone schedules, replaces and cancels alerts from the point nowcast; no push service, credentials or account |
| Web accuracy headline and opt-in browser rain alerts using the same planner and formatter | `web/src/main.ts`, `web/src/rainAlerts.ts`, `web/src/shared/` | same warmup and lead options; browser alerts require the page to remain open |
| Dead `MRMS_RENDER_WORKERS` knob removed from compose, env example and docs | `deploy/`, `docs/` | no phantom tunable |

Rollout: the worker image change reseeds the MRMS schedules automatically when
the `aux` pool restarts (it is the only seeder). The kick uses the
controller-injected `TEMPORAL_ADDRESS`/`TEMPORAL_NAMESPACE` on the `mrms` pool;
`NOWCAST_KICK_ENABLED=0` disables it. The lightning change takes effect on the
next hourly fire. Verification keeps 36 complete runs by default (12 forecast
grids plus the anchor observation per run), approximately 700 MiB at the current
point-grid size. Scoring runs after nowcast publication; failures are logged
without failing publication. API health uses `nowcast_skill`, while point
responses use `skill`.

Rain alerts require a fresh Expo prebuild and native rebuild because this
branch adds `expo-notifications`, `expo-background-task` and `expo-task-manager`.
Background refresh follows the OS schedule (15 minutes at best, often longer),
so it does not guarantee an alert for every storm while the app is closed.

Verified on 2026-09-30: the full CI worker suite against mounted branch source
in the release worker image passed 251 tests, 496 subtests, with one skipped;
the replay/discovery gate also passed separately. API: 51 passed. Frontend:
305 unit tests and 13 render tests passed; Expo lint and Ruff clean. Web:
production build/type-check and 22 tests passed. Both iOS and Android production
JS bundles export successfully, including the shared planner. The latest master
was merged, retaining both the early-exit and motion-stride regression tests.

Type-checking reports three existing `MapChromeSurface.tsx` errors: Expo's web
augmentation permits `position: "fixed"`/`"sticky"`, but React Native's generated
View props accept only native positions. Confirmed on clean HEAD `76d09ac` in
an isolated worktree using the same dependencies and generated `expo-env.d.ts`
and `.expo/types`; the generated Expo files are needed to reproduce the errors.
No new type errors were introduced.

Scheduler regression covered: a new nowcast run changes the alert key. Previously
deduplication could cancel an undelivered notification for the same storm and
skip its replacement. Pending notifications are now cancelled before checking
delivered-event deduplication, allowing a new forecast or warning lead to replace
them while suppressing repeat alerts after delivery.

## 4. Recommended gitops changes, in order

1. **Turn on the indexed renderer, one role at a time.** `TILE_RENDERER` is
   unset in production, so every role still renders the legacy RGBA path: the
   render-once work (Gate 2) never got its canary. The benchmark on a full
   CONUS frame is 7.8 s → 2.1 s cold, 6.9 MiB → 0.9 MiB per pyramid, 134 → 7
   MiB peak. Smaller PNGs also cut tile bytes for every client. Start with
   `TILE_RENDERER=indexed` + `TILE_RENDERER_CANARY_ROLE=mrms` on the mrms pool
   only, following the two-key procedure in `docs/reliability-and-scale-plan.md`,
   then nowcast, hrrr, aux.
2. **Cloudflare Cache Rule for vector basemap tiles.** `/basemap/tiles/*.mvt`
   returns `cf-cache-status: DYNAMIC` on both `radar-ng-api.vanillax.me` and
   `radar.vanillax.me`; `.mvt` is not in Cloudflare's default cacheable
   extensions, so every basemap tile reaches go-pmtiles at home. Add a rule
   matching those hostnames and `/basemap/tiles/*` set to eligible-for-cache,
   respecting the origin `max-age=86400`. Radar tiles and imagery already HIT.
3. **Let in-flight work finish on pod termination.** Nowcast (≈2 min) and HRRR
   hours (≈1 min) exceed the 25 s graceful shutdown; failures on 2026-09-29
   05:18 (nowcast) and 05:00 (lightning, whole hour lost) were `WorkerShutdown`.
   Set `terminationGracePeriodSeconds: 180` and
   `TEMPORAL_GRACEFUL_SHUTDOWN_S=150` on the nowcast and hrrr pools.
4. **Remove `MRMS_RENDER_WORKERS`** from `release-env-patch.yaml` and the
   frozen configmap; the code has not read it since render-once landed.
5. **Nowcast pool sizing.** With 12 internal steps the run is ≈2 min at one
   slot; re-measure the memory peak after 24 h on v1.1.45 (6.6 GiB so far
   against an 8 GiB request and 12 GiB limit) before changing anything.
6. **tile-server autoscaling objects.** The HPA is pinned to 1 replica and only
   emits `FailedGetResourceMetric` events on every rollout; delete it or accept
   the noise. The VPA runs `InPlaceOrRecreate` with `minReplicas: 1` on a
   `Recreate` deployment, so a memory recommendation can evict the only serving
   pod; `updateMode: Initial` keeps the recommendation without the eviction.
7. **Composite reflectivity is a product decision.** The app uses
   `radar-composite` only as a fallback when `radar` is absent, and the web
   never uses it, yet it costs ≈30 s of CPU per frame, 396 PNGs per palette and
   5 GiB of the tile volume. Keep it, or pause its schedule and drop its
   retention.
8. **Rollout overlap.** The sunset window keeps both worker versions running
   (10 min after drain, up to ~1 h for `aux` because the pinned 50-min lightning
   activity keeps the old version alive), doubling ≈15 GiB of memory requests on
   a node already at 73–86% requested memory. Keep that headroom, or shorten the
   lightning activity so `aux` drains faster.
9. **Longhorn trim.** The tiles volume holds 35 GiB on disk for 15 GiB of files
   because deleted tiles are never trimmed; a Longhorn recurring `fstrim` job
   reclaims it.
10. **Temporal platform** (unchanged from the plan): one history shard, single
    replicas, single Postgres. Server metrics are scraped now (578 series), so
    the Gate 6 alerts on schedule-to-start, persistence errors and missed
    catch-ups can be added before the sharded database exists.

## 5. Temporal health details

- Every failed radar execution in the last 7 days has an external cause:
  `WorkerShutdown` during rollouts (nowcast 09-29 05:18, lightning 09-29 05:00),
  NWS `503` on the alert poll (7 in 7 days), a cluster DNS blip at 09-26 10:00
  (mrms, tropical, poll-alerts), a nowcast heartbeat timeout on 09-25 before
  #76, and the 09-25 03:2x `TimedOut` group during the Temporal Postgres
  restart. Schedules fire on time; the watchdog reported no stall.
- `temporal_activity_execution_failed` showed 112 HRRR hour failures in 24 h
  while logs and histories show none; the by-pod 3 h view is empty, so treat it
  as a counter-reset artefact across the rollouts and re-check after a quiet day.
- Temporal SDK histograms are in milliseconds; `temporal task-queue describe`
  prints no pollers for versioned workers unless `--select-all-active` is
  passed. Both tripped this review before the numbers made sense.

## 6. Follow-ups not taken here

- A workflow-level nowcast start (child workflow or signal) if triggering the
  Schedule from the MRMS activity is unwanted; it needs replay fixtures.
- Air quality downloads each 70–90 MB GRIB three times per layer; caching the
  file per run on the aux pod's local disk removes ~0.5 GB/day of egress.
- Serve `/api/manifest.json` straight from the state volume through Caddy if
  the API ever approaches its ~400 req/s ceiling.
- Self-hosted CarPlay search and routing.
