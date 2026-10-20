# Full audit: UX, performance, setup, backend, Temporal, cluster, 2026-10-10

> **Historical record.** Written against the Python backend, which was replaced by the Rust backend
> (`rust/`, v2.0.0, 2026-10-14). Paths under `backend/`, `temporal/` and `tools/`, and the Caddy and
> FastAPI tile-server, no longer exist; see [ARCHITECTURE.md](../../reference/architecture.md) for the current layout.

Scope: live cluster and API (kubectl, Prometheus, Loki, Temporal CLI, curl),
the gitops manifests in `talos-argocd-proxmox/my-apps/personal-projects/radar-ng`,
and read-only code reviews of `backend/`, `temporal/`, `frontend/`, `web/`,
`deploy/` and `docs/`. Master at `917d89a`. Production images: worker
v1.1.48, tile-server v1.1.26, open-meteo-worker v1.1.11, web v0.1.4.

Each finding is VERIFIED (traced in code, reproduced live, or measured) or
PLAUSIBLE (read only). Items from the 2026-09-29 review that are unchanged
are marked "still open".

## 1. Production state (measured today)

| Signal | Value | Source |
|---|---|---|
| Radar age, 7 d | p50 178 s, p95 254 s, p99 290 s | `radar_ng_mrms_age_seconds` |
| Radar age max, 7 d | 5,704 s on 10-07 14:28 to 16:02 UTC | NOAA MRMS bucket had no files for 93 min (verified by listing `noaa-mrms-pds`) |
| Nowcast run | p50 79 s, p95 116 s | Temporal end-to-end histogram |
| Nowcast skill, 30 min lead, 24 h | POD 0.61, FAR 0.37, CSI 0.45 vs persistence 0.39, 714 runs | `/api/health` |
| MRMS run | p95 41 s | Temporal |
| API, 24 h | 8,640 requests, 0 5xx, 0 429, 0 304 | `radar_ng_http_requests_total` |
| Schedule-to-start p95 | under 0.5 s on all five queues | SDK metrics |
| Non-completed workflows, 24 h | 3: MRMS timed out 02:05 UTC (node reboot), PollAlerts failed 04:20 UTC (NWS), one unrelated app | `temporal workflow list` |
| Worker memory peak, 24 h | nowcast 5.4 GiB (limit 12, request 8); mrms 3.7; hrrr 4.0; aux 1.5; alerts 0.2 | cAdvisor |
| CPU throttling, 24 h | none above 0.4% | cAdvisor |
| Disk | tiles 18%, grids 26%, state 1%, open-meteo 46% | `df` in pods |
| Longhorn tiles volume | 34.9 GB on disk for 18 GB of files (no trim, still open) | Longhorn |
| GPU node | 89% memory requested, 63% CPU requested; every radar-ng pod pinned here | kube-state-metrics |
| Firing alerts | none | Prometheus |

Photon geocode returned a 502 after 12 s at 18:45 UTC; a retry 20 s later
took 0.5 s. Cause is in finding C2.

One cold Open-Meteo forecast took 9.9 s at 18:44 UTC; three other cold
locations took 11 to 217 ms and warm calls 17 ms. The slow call coincided with
the dell node going down. Not a finding; re-check if users report slow first
loads.

## 2. Cluster incidents today

- **Operator node reboots.** The owner rebooted nodes by hand today:
  control-plane and hp-sff at 02:08 UTC, hp-micro at 16:38 UTC, dell from
  18:40 UTC (still cordoned at the end of this audit). Not an incident. The
  Temporal outage below shows what one reboot of hp-sff costs. If gpu-workers
  reboots, every radar-ng pod and Photon go down together (finding C1).
- **Temporal control plane outage at 02:08 UTC.** temporal-matching and
  temporal-worker ran on hp-sff. The frontend and history logged about
  1,600 "Not enough hosts to serve the request" errors and DNS failures for
  temporal-postgres in that hour. The watchdog logged 13 schedules x 9
  passes of `RPCError[CANCELLED]` (finding T3). Everything recovered without
  intervention; one MRMS run timed out.
- The web pod on dell was rescheduled to hp-elite; radar.vanillax.me stayed
  up (6 of 6 probes 200 in 80 to 130 ms).

## 3. Findings ranked by severity

### High

| # | Area | Finding | Where | Status |
|---|---|---|---|---|
| H1 | Backend | Absolute URLs the API builds are `http://` in production. Caddy has no `trusted_proxies`, so it overwrites `X-Forwarded-Proto`. Live: `/api/basemap/style/positron` returns `http://radar-ng-api.vanillax.me/...` tiles; storm-prefetch style URLs too. The app hands these to MapLibre offline packs. | `backend/api/api/server.py:789`, `:957-959`, `backend/api/Caddyfile` | VERIFIED live |
| H2 | App | Forecast times are wrong for a chosen city in another time zone. The API asks Open-Meteo for `timezone=auto` and gets offset-less local strings; the app parses them with `new Date()` in the device zone and labels with `getHours()`. Device-location mode is fine; a city two zones away shifts the hourly strip, NOW index, night flag and rain window by two hours. | `backend/api/api/server.py:355`, `frontend/src/lib/forecastView.ts:17-21,52-58` | VERIFIED code |
| H3 | App | The app is pinned to a pre-release SDK: `expo 58.0.0` and `react-native 0.88.0-rc.3`. Expo's changelog has only an SDK 58 beta post; SDK 57 is the latest stable. `expo install --check` lists 17 outdated packages. README badge says SDK 57. | `frontend/package.json:37,54` | VERIFIED (web search 2026-10-10) |
| H4 | Temporal / gitops | Worker pools have no `terminationGracePeriodSeconds` (default 30 s) and `TEMPORAL_GRACEFUL_SHUTDOWN_S` is never set (default 25 s). Nowcast runs 2 min and HRRR hours 1 min, so every rollout kills in-flight work. Still open (09-29 #3). | gitops `temporal-workers/worker-pools.yaml`, `temporal/worker.py:413-415` | VERIFIED |
| H5 | Temporal | Heartbeats stop during blocking downloads. Air quality streams 50 to 90 MB GRIBs with no heartbeat against a 120 s heartbeat timeout; MRMS downloads under `to_thread` against a 60 s timeout. A slow but healthy download is killed and retried. HRRR already uses `run_sync_with_heartbeat`. | `backend/ingest_airquality/activities.py:387-405`, `backend/ingest_mrms/activities.py:339-346` | VERIFIED code, impact PLAUSIBLE |
| H6 | Web | Boot has no error path when the API is down. `loadStyle` has no try/catch; the page stays on a blank map with "Loading forecast…" forever. | `web/src/main.ts:126-139`, `:992-996` | VERIFIED code |
| H7 | Web | Browser rain alerts silently fail on Chrome for Android: `new Notification()` is rejected there, and the site ships no service worker or web manifest. iOS Safari needs both plus Home Screen install. | `web/src/rainAlerts.ts:17-19,69` | VERIFIED |
| H8 | Docs | Quick-start verify step uses a route that does not exist: `curl localhost:8080/api/manifest` (404). The route is `/api/manifest.json`. | `README.md:36`, `GETTING_STARTED.md:72` | VERIFIED |
| H9 | Docs | App server-URL instructions do not match the app. Docs say "Settings → Server URL" or "Data Source → Self-Hosted / Free tier (IEM)". The app has a "Stack URL" field only in Advanced Mode and no data-source toggle. | `GETTING_STARTED.md:171`, `docs/self-hosting.md:124`, `docs/running-the-app.md:57-60`, `frontend/src/app/(tabs)/settings.tsx:275-290` | VERIFIED |

### Cluster and gitops (medium)

| # | Finding | Where | Status |
|---|---|---|---|
| C1 | Single point of failure: all radar-ng pods, Photon and both Longhorn flash volumes sit on gpu-workers (required affinity for the flash disk). Node is at 89% memory requested. A rollout doubles about 16 GiB of worker requests for the 10 min sunset window (1 h for aux). Lower nowcast request 8 to 6 Gi and raise hrrr 2 to 4 Gi to match peaks; plan a second flash disk on another node if availability matters. | `worker-pools.yaml:39-51,95-97,194-196` | VERIFIED, sizing PLAUSIBLE |
| C2 | Photon probes have no `timeoutSeconds` (default 1 s). Photon is a JVM at 6.4 GiB RSS; a GC pause fails readiness, the endpoint drops, and `/api/geocode` returns 502. Set `timeoutSeconds: 5` on readiness and liveness. | gitops `my-apps/development-infra/photon/deployment.yaml:55-65` | VERIFIED live |
| C3 | Cloudflare still does not cache vector basemap tiles: `/basemap/tiles/*.mvt` is `cf-cache-status: DYNAMIC` on both hostnames. Still open (09-29 #2). Add a Cache Rule for `/basemap/tiles/*`. | Cloudflare dashboard | VERIFIED live |
| C4 | `TILE_RENDERER` is unset, so every role still renders the legacy RGBA path. Still open (09-29 #1). Start the canary on mrms. | gitops `worker-pools.yaml` | VERIFIED |
| C5 | Dead `MRMS_RENDER_WORKERS=3` is still shipped in the release patch and the frozen configmap, and the configmap is no longer referenced by any pool. `HRRR_ENABLED_LAYERS` is duplicated with different values. Still open (09-29 #4). | `release-env-patch.yaml:17-21`, `configmap-temporal-config.yaml:17`, `worker-pools.yaml:73` | VERIFIED |
| C6 | PrometheusRule gaps: no alert for lightning stream down, nowcast skill stale, tiles or grids PVC fill, Temporal schedule overdue, zero pollers per queue, or the open-meteo `sync-worker` sidecar OOM (rules filter `container="worker"`). Also no alert for "Temporal control plane unreachable" (today's outage fired nothing). | `prometheusrule.yaml` | VERIFIED |
| C7 | VPA `InPlaceOrRecreate` with `minReplicas: 1` on single-replica Recreate deployments can evict the only tile-server or open-meteo pod. HPA pinned 1/1 is noise. Still open (09-29 #6). Use `updateMode: Initial`, delete the HPA. | `vpa.yaml`, `hpa-tile-server.yaml` | PLAUSIBLE |
| C8 | Loki keeps 24 h (`retention_period: 24h`), so the 10-07 stale-radar cause had to come from the NOAA bucket listing. Raise to 7 d or ship radar-ng logs to a longer-retention stream. | loki-stack config | VERIFIED |
| C9 | Longhorn tiles volume holds 34.9 GB for 18 GB of files; no fstrim job. Still open (09-29 #9). | Longhorn | VERIFIED |
| C10 | radar-ng runs in Temporal namespace `default` shared with deal-scout, news-digest and trend-digest: shared RPS limits, one scheduler worker, shared ID space and retention. A `radar-ng` namespace is already registered. Move needs seven config points flipped in one PR, aux restart to reseed, pause and delete the 13 schedules in `default`, and terminating running `WatchStormWorkflow` entities. A dedicated namespace does not fix single shard or single Postgres. | `worker-pools.yaml:22,121,220,319,419`, `deployment-tile-server.yaml:116`, `deployment-open-meteo.yaml:97` | PLAUSIBLE |

### Backend (medium and low)

| # | Finding | Where | Status |
|---|---|---|---|
| B1 | Rate limiter keys on the last `X-Forwarded-For` hop, which is the gateway pod, so all users share one 20 rps / 60 burst bucket. Has not fired at 8.6k requests per day; a storm day could 429 everyone. `test_request_controls.py:23` codifies the wrong model. | `server.py:125-131` | VERIFIED code |
| B2 | `/api/health` and `/api/metrics` call `_build_manifest()` directly: a 150 KB parse per Prometheus scrape and per Settings open. Use `_cached_manifest()`. | `server.py:879`, `:982` | VERIFIED |
| B3 | Manifest body and ETag can be paired from different refreshes at the TTL edge. Return body, encoded and etag from one locked refresh. | `server.py:320-321` | VERIFIED |
| B4 | Zero client 304s: server side is correct (ETag, no Vary, Cloudflare passes conditionals, 304 counted since restart). The web and the app both use bare `fetch` and rely on platform caches; Android OkHttp has none. Store the ETag and send `If-None-Match` from both clients. | `web/src/weather.ts:45-53`, `frontend/src/lib/api.ts:109-119`, `frontend/src/lib/manifestCache.ts:34-44` | VERIFIED cause |
| B5 | `OPEN_METEO_BASE` still defaults to the public API. Still open (09-29 §2). | `server.py:40`, `backend/api/Dockerfile:52` | VERIFIED |
| B6 | Tropical state write is a plain `.tmp` + `replace`; a retry overlapping a slow attempt or two worker versions in the sunset window can publish a half-written file. Lightning already uses `mkstemp`. | `backend/ingest_tropical/activities.py:176-178` | VERIFIED |
| B7 | Low: `/api/wind-field/latest` globs both grid dirs before the cache lookup (`server.py:623-627`); retention de-lists and deletes in one step, up to 30 s of 404s on the oldest frame (`tile_cleanup/activities.py:117-118`, `ingest_mrms/activities.py:524-525`, still open 09-02 B8); nowcast kick runs before `update_manifest_file` and `Client.connect` has no timeout (`ingest_mrms/activities.py:451-473`); metric counters are non-atomic across threads (`server.py:319,488,512`); `/api/lightning` re-serialises 5,000 features per request (`server.py:737-738`). | | VERIFIED |

### Temporal (medium and low)

| # | Finding | Where | Status |
|---|---|---|---|
| T1 | Watchdog logs one WARNING per schedule per pass during an outage (117 lines today) and never alerts. Emit one event per pass with a consecutive-failure count and alert on 3 consecutive all-fail passes as "control plane unreachable", separate from STALL. | `temporal/schedules/watchdog.py:142-154` | VERIFIED |
| T2 | Low: PINNED default versioning keeps the old aux version alive about 1 h per rollout because of the 50-min lightning activity. Still open (09-29 #8). Use AUTO_UPGRADE except for WatchStorm and smoke. | `temporal/worker.py:230`, `open_meteo_worker.py:42` | VERIFIED |
| T3 | Low: pausing the `nowcast` schedule is not a kill switch; `handle.trigger` runs on paused schedules. `NOWCAST_KICK_ENABLED=0` plus an mrms rollout is. Document in the runbook or skip the kick when paused. | `backend/ingest_mrms/activities.py:281-284` | PLAUSIBLE |
| T4 | Low: `fetch_nws_active_alerts` retry window is about 75 s; NWS 503 episodes run longer (12 failures per day, one run lost). Use 4 attempts, 20 s max interval, non-retryable on 4xx. | `backend/api/api/storm_watch_activities.py:279`, `temporal/workflows/poll_alerts.py:43-48` | VERIFIED |
| T5 | Low: nowcast reads all input grids before the first heartbeat and scores skill after publish without heartbeats, both against 120 s with one attempt. | `backend/nowcast/activities.py:362-442`, `:587-600` | VERIFIED code |
| T6 | Low: replay fixtures stop at v1.1.27; prod is v1.1.48. Workflow code is unchanged since then, so the gate is still valid, but the README contract is not followed. | `temporal/workflows/replay fixtures` | VERIFIED |
| T7 | Low: `IngestMrmsWorkflow` falls back to defaults that read worker env, so a manual start without input could fail replay on a pod with different env. | `temporal/workflows/ingest_mrms.py:82` | VERIFIED code |
| T8 | Info: the DEBUG "operation was canceled" on every worker every 5 to 20 min is the frontend's connection-age GOAWAY, not the 10 s timeout. The 1 s retry succeeds. Leave as DEBUG. The nowcast worker also prints the pySTEPS progress table to stdout, about 2,000 lines per hour into Loki; silence it. | `temporal/shared/health.py:43-53`, `backend/nowcast/activities.py:270-284` | PLAUSIBLE |

### App UX and performance (medium and low)

| # | Finding | Where | Status |
|---|---|---|---|
| A1 | Radar tab has no loading, stale or offline state: with no frames the timeline returns null and a dead server shows a bare basemap. The LIVE / UPDATED 12M AGO / OFFLINE chip exists only on the Home mini-map. | `frontend/src/app/(tabs)/radar.tsx:46`, `components/TimelineBar.tsx:136`, `lib/radarStatus.ts` | VERIFIED |
| A2 | Location permission is requested on first mount with no pre-prompt; after denial the notice is plain text with no `Linking.openSettings` and no "choose a city" action. | `hooks/useLocation.ts:91`, `app/(tabs)/index.tsx:260-267` | VERIFIED |
| A3 | Rain alerts exist only as a Settings toggle; the Home rain banner and Nowcast hero never offer "Notify me". | `settings.tsx:406-438`, `index.tsx:304-320` | VERIFIED |
| A4 | No CARROT-style personality: every condition gradient is five identical `#f6f2ea` stops, `expo-haptics` is not installed, copy is neutral. | `lib/cumulusTheme.ts:49-58` | VERIFIED |
| A5 | Playback timer re-arms twice a minute: frames array rebuilt every 30 s poll, `sequence` recomputed on a 60 s tick, interval effect depends on it. Still open (09-02 #10). | `hooks/useManifest.ts:129-138`, `useWeatherStore.ts:178-191`, `TimelineBar.tsx:79-111` | VERIFIED restart, hitch PLAUSIBLE |
| A6 | Low: second GL map on Home mini-map (mitigated); small text contrast 3.3:1 and 2.6:1 at 9 to 11 pt (`alerts.tsx:344`, `NowcastScreen.tsx:813,866`, `index.tsx:857`); per-instance `StyleSheet.create` (`alert/[id].tsx:188,209`, `alerts.tsx:155`, `NowcastScreen.tsx:498,529`); sliders write MMKV on every drag tick (`settings.tsx:493,508`); "Close radar" always navigates Home (`radar.tsx:117`); iOS splash is a flat colour (`app.json`). | | VERIFIED |

### Web UX and performance (medium and low)

| # | Finding | Where | Status |
|---|---|---|---|
| W1 | Forecast, NWS alerts and air quality never refresh after boot; only briefing (10 min) and next-hour (60 s) re-poll. A tab left open shows hours-old conditions. | `web/src/main.ts:689,999` | VERIFIED |
| W2 | Manifest poll snaps the playhead to now during playback every 60 s. | `main.ts:331` | VERIFIED |
| W3 | nginx `add_header` inside `location` drops the server-level headers: no `Referrer-Policy` reaches clients (confirmed by curl) and there is no CSP. Suggested CSP: `default-src 'self'; script-src 'self' https://ingest-posthog.vanillax.me; connect-src 'self' https://ingest-posthog.vanillax.me; img-src 'self' data: blob:; style-src 'self' 'unsafe-inline'; worker-src 'self' blob:; font-src 'self'; frame-ancestors 'none'; base-uri 'self'; form-action 'self'`. | `web/nginx.conf:12,19,33` | VERIFIED |
| W4 | PostHog session replay and performance capture are on unconditionally with no opt-out, and `place_chosen` events carry place names. The app has `telemetryPrivacy.ts`; web has no equivalent. | `web/src/analytics.ts:30-35` | VERIFIED |
| W5 | Mobile layout ignores safe areas and uses `100vh`; the timeline sits under the iOS home indicator and Safari's toolbar clips the sheet. | `web/src/style.css:244-251`, `index.html:5` | VERIFIED |
| W6 | Low: dark mode follows the basemap, not `prefers-color-scheme` (`style.css:19-30`); search is not a real combobox and chips have no arrow keys; single 1.09 MB bundle (299 kB gzip) with no ESLint and `main.ts` (1,005 lines) untested; gzip only at origin. | | VERIFIED |

Web and app parity gaps (AGENTS.md rule): web has no Settings (temperature unit, appearance, stack status), no nowcast detail rows (STARTS, PEAK, ENDS, TOTAL), no alert detail page, no analytics opt-out, and no precip-type or CAPE layers. Web-only: playback speed and keyboard shortcuts.

### Setup, images, CI and docs (medium and low)

| # | Finding | Where | Status |
|---|---|---|---|
| S1 | `deploy/k8s/` reference manifests are stale: 2 worker replicas on RWO PVCs, `registry.vanillax.me/...:v1.1.1`, no `WORKER_ROLE`, wrong gitops path `my-apps/development/radar-ng/`, MRMS "every 2m". | `deploy/k8s/*.yaml`, `deploy/k8s/README.md:4,138`, `GETTING_STARTED.md:93`, `docs/releasing.md:106` | VERIFIED |
| S2 | Worker image runs as root with the compiler chain in the final layer; base images are not digest-pinned; compose uses `go-pmtiles:latest`. Prod forces uid 1000 so only Compose users run as root. | `temporal/Dockerfile`, `backend/api/Dockerfile`, `deploy/docker-compose.yml` | VERIFIED |
| S3 | No Python lockfile; only numpy, Pillow and temporalio are pinned exactly. Two builds of one commit can differ. Local venv has temporalio 1.27 vs pinned 1.30 and no pygrib, so backend/temporal tests only run fully in CI. | `backend/base/requirements.txt`, `temporal/requirements.txt` | VERIFIED |
| S4 | CI: no PR check for `web/`, no ruff step, no SBOM or signing, worker image rebuilds on `backend/api/**` changes it does not ship, `docs/releasing.md:66` still cites Gitea. | `.github/workflows/*` | VERIFIED |
| S5 | Docs drift: `docs/self-hosting.md:143` says `/v1/*` has no auth (it requires a signed token and is disabled in prod); broken anchors at `docs/self-hosting.md:19` and `docs/kubernetes.md:96`; `CONTRIBUTING.md` links a missing `SECURITY.md`; `docs/configuration.md:118` documents `STORM_MIN_TRACK_RADIUS_KM` but code reads `STORM_MATCH_SLACK_KM`; `deploy/.env.example:33` default basemap date already 404s; about 50 env vars with non-trivial defaults are undocumented (`TEMPORAL_GRACEFUL_SHUTDOWN_S`, `TILE_RENDERER`, `NOWCAST_MOTION_STRIDE`, `LLM_*`, `LOG_LEVEL`, …). | | VERIFIED |

## 4. Recommended order of work

Gitops PRs (feature branch, ArgoCD deploys):

1. Photon probes `timeoutSeconds: 5` (C2). Smallest change, fixes live 502s.
2. Worker pools: `terminationGracePeriodSeconds` 180 on nowcast and hrrr, 60 on mrms, with `TEMPORAL_GRACEFUL_SHUTDOWN_S` five seconds less (H4). Remove `MRMS_RENDER_WORKERS` and the dead configmap (C5). Lower nowcast request to 6 Gi, raise hrrr to 4 Gi (C1).
3. `TILE_RENDERER=indexed` canary on mrms per the two-key procedure (C4).
4. VPA `updateMode: Initial`, delete the HPA (C7). Add the six missing alert rules plus a control-plane-unreachable rule (C6).
5. Cloudflare Cache Rule for `/basemap/tiles/*` (C3). Longhorn fstrim job (C9). Loki retention 7 d (C8).

Code PRs (each ships web and app together where user-facing):

1. H1: `PUBLIC_BASE_URL` or Caddy `trusted_proxies`, with a test that asserts `https://`. Fix the rate-limit key at the same time (B1).
2. H2: request `timeformat=unixtime` from Open-Meteo and format with the response offset; fix the app and the web formatters together.
3. H5 and T5: heartbeat downloads and nowcast setup through `run_sync_with_heartbeat`. B6: mkstemp for tropical.
4. B4: send `If-None-Match` from app and web. B2 and B3: manifest cache fixes.
5. H6, H7, W1, W2, W3, W5: web boot error path, service-worker notifications, periodic forecast refresh, playhead hold, nginx headers and CSP, safe areas.
6. A1 to A4: radar status chip, permission pre-prompt and recovery, rain-alert CTA, personality pass (gradients, haptics, copy). These are the UX items that move the app toward the CARROT direction.
7. H3: decide SDK 57 stable or SDK 58 beta; run `expo install --fix` either way. Native rebuild (`bun run ios:ipa`) needed for the phone.
8. T1, T2, T4: watchdog aggregation, AUTO_UPGRADE versioning, NWS retry ladder.

Docs PR: H8, H9, S1, S5 in one pass; regenerate `deploy/k8s` from the live pools.

## 5. What is in good shape

- Freshness and nowcast quality meet the plan: p99 radar age under 5 min, nowcast CSI above persistence at every lead, 744 scored runs.
- Serving is idle: 0 5xx in 24 h, no throttling, tile-server under 0.2 GiB.
- Backend writes for manifest, grids, storms and skill are mkstemp + replace under flock; publication is all-or-nothing; input validation is tight; geocode and alerts cannot be pointed at arbitrary hosts; httpx clients carry timeouts.
- Workflows have no non-deterministic calls; patch markers are correct; failures are isolated per frame, hour, chunk and alert; schedules have SKIP overlap, catch-up windows, jitter and ceilings; the replay gate passes against current code.
- App data layer: fetch timeouts chained to react-query, NetInfo and AppState wiring, polling gated on focus, manifest validated before MMKV, coordinates rounded before leaving the device. Accessibility labels and 44 pt targets are broad. React Compiler and hooks lint are on.
- Web: shared planner and skill code match the app, tiles are immutable, playback prefetches six frames, basemap, fonts and analytics are self-hosted, dependencies are current.
- Production manifests: per-role queues, one seeder, liveness and readiness split, PodMonitor, restricted PodSecurity, digest-pinned ghcr images, coherent release flow.

## 6. Tool and test results

| Suite | Result |
|---|---|
| `ruff check backend` and `ruff check temporal` | clean |
| Backend tests (isolated uv env) | 206 passed, 1 failed only because pysteps is not installable locally; mrms and hrrr suites need pygrib and run in CI |
| Temporal replay and discovery gate (in worker image) | 9 passed |
| `expo lint` | clean |
| `tsc --noEmit` (app) | only the three known MapChromeSurface.tsx errors |
| Jest (app, node runtime) | 50 suites, 318 tests passed; fails under the bun runtime |
| `expo install --check` | 17 outdated |
| Web build and type-check | clean; 1,093.9 kB JS (299.1 kB gzip), 96.1 kB CSS |
| Web tests | 22 passed |
| `docker compose config` | valid |
| GitHub Actions, last 8 runs | all green |

Local toolchain skew: bun 1.3.14 installed vs `packageManager` 1.4.2; local Python venv lacks pygrib and has temporalio 1.27.
