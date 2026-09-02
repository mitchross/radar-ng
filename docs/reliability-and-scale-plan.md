# radar-ng: reliability and scale plan

Written 2026-09-02 after a full review of the backend, the Temporal layer, the
Expo app, and the Kubernetes deployment, plus a night of watching production.
This is the map from "one pod on one node, breaks when the cluster sneezes" to
"many pods on many nodes, shrugs off a bad night". Detailed findings with
file:line references live in `docs/tasks/2026-09-02-*.md`.

The plan is written so a junior can follow the order and a senior can check the
reasoning. Each phase has an exit test. Nothing in a later phase is needed to
finish an earlier one.

---

## 1. The short version

radar-ng renders a frame in 25–40 seconds, uses almost no disk IO, and serves
almost no traffic yet. None of that is the problem. The problem is coupling:

- **One node.** Tiles, grids, and state are ReadWriteOnce Longhorn volumes, so
  the worker and the tile-server must share a node. The tile-server cannot
  scale past one pod and dies with that node.
- **One process.** Every ingest activity (MRMS render, pysteps nowcast, HRRR
  fan-out, air quality, a 50-minute lightning websocket) runs in one Python
  process with 4 activity slots. It was OOM-killed at 12 GiB on 2026-08-31.
- **One shard.** The shared Temporal server runs `numHistoryShards: 1` on a
  single-replica Postgres. When that DB moves nodes, every schedule in the
  cluster stalls. It also loses scheduler timers, which is why three radar-ng
  schedules slept for 36 hours.
- **No self-defense.** A 2-minute radar schedule carried a 90-minute workflow
  timeout, so one stuck run blocked 45 fires. The worker has no liveness probe,
  so a 4-hour cluster DNS outage never restarted it.
- **Silent breakage.** HRRR forecast radar has not published in days; nothing
  alerted.

The fix order is: stop bleeding (timeouts, watchdog, liveness, alerts), then
split the worker into pools, then move tiles off the RWO volume so the
tile-server can run anywhere, then make the app's playback smooth.

---

## 2. What is true today (measured, not assumed)

| Fact | Value | Source |
|---|---|---|
| MRMS frame render (z4–z7, 3 palettes) | radar 22–28 s, composite 31–40 s | worker `frame_done` logs |
| Worker steady use | ~3 CPU, 5–8 GiB of a 12 GiB limit | `kubectl top`, Prometheus |
| Tiles volume IO | ~5 read IOPS, ~3 write IOPS, 350 KB/s write, 0.6 ms write latency | Longhorn metrics |
| Tiles on disk | 3.8 GiB of 100 GiB | `df` in tile-server |
| Grids on disk | 5.8 GiB of 20 GiB (nowcast input grids kept 12 h, only 4 needed) | `du` |
| tile-server traffic | ~0.2 requests/s | `radar_ng_http_requests_total` |
| tile-server CPU | ~4 m (idle) | `kubectl top` |
| Schedules that ran vs. expected | `ingest-mrms-base` 29 %, `poll-alerts` 31 % since creation | `schedule describe ActionCounts` |
| `radar-hrrr` layer in manifest | absent | `/api/manifest.json` |
| Temporal history shards | 1 | `values.yaml` |
| Temporal server metrics scraped | none (until PR #2160) | Prometheus |

Take-away: disk speed, storage class, and PVC sizing are not on the critical
path. CPU in the worker and the Temporal control plane are.

---

## 3. What broke on 2026-09-01/02 and the rule each one teaches

| Incident | Cause (verified) | Rule going forward |
|---|---|---|
| Radar dark 23:22Z → 03:22Z (4 h) | Both CoreDNS pods down; worker retried polls forever; each MRMS run had a 90-min timeout under `SKIP` | Workflow timeout ≤ 2–3 cadences. Every worker has a liveness probe tied to Temporal reachability. Alerts page a human. |
| `poll-alerts`, `tile-cleanup`, `open-meteo-sync-hrrr` asleep 36 h | Temporal server lost the scheduler workflow's timer (1-shard history). Any signal wakes it, then it wedges again | The seeding worker runs a watchdog that re-triggers overdue schedules. Move to a dedicated namespace, then to a sharded server. |
| Worker OOMKilled at 12 GiB | Nowcast + HRRR + air quality + 2 MRMS renders + lightning in one process | One pool per role, each with its own slot count and memory budget. |
| Control plane down 05:11Z–05:21Z | Node reboot moved the single-replica Temporal Postgres; Longhorn re-attach took 7 min | No `Recreate`-capable VPA on a single-replica DB. Co-locate DB with the server. |
| HRRR never publishes | NOAA hasn't uploaded the last hours yet → 404 → retry → whole run discarded every 15 min | 404 = "not yet". Publish the consecutive prefix, resume from existing tiles. Alert when a layer is absent. |

---

## 4. Target architecture

```
NOAA / NWS / NHC / Blitzortung
        │
        ▼
Temporal (dedicated `radar-ng` namespace, later a sharded server)
  schedules: SKIP, jitter, catchup ≤ 5 min, timeout ≤ 2–3 cadences, watchdog
        │
        ▼
Worker pools (one WorkerDeployment per role, no shared process)
  mrms │ nowcast │ hrrr │ aux (lightning, tropical, AQ, cleanup, seed+watchdog) │ alerts
        │  render once (indexed color), palette = 256-byte PLTE swap
        ▼
Object storage: RustFS S3 (tiles, grids)          Postgres: manifest, state, alerts, push tokens
  immutable keys, lifecycle rules                  small rows, CAS updates, kopiur-backed
        │                                                   │
        ▼                                                   ▼
tile-server × N (any node, no PVC)  ── Caddy: /tiles → S3 (local cache) · /api → FastAPI
  HPA 2–6 on CPU, PDB minAvailable 1, topology spread across nodes
        │
        ▼
Envoy gateway (Cloudflare tunnel) → Expo app (5-slot prefetch carousel, offline manifest)
```

Why each box:

- **Object storage for tiles.** A frame is ~250 non-empty PNGs per palette
  (the "12.6k tiles" figure is from the retired z9 pyramid). That is ~6 PUTs/s
  at the 2-minute cadence. Object keys equal today's URL paths, so the client
  and the manifest contract don't change. The tile-server stops needing a
  volume and can run on every node. RustFS already exists for backups.
- **Postgres for the small shared files.** `manifest.json`, `ProcessedSet`,
  `storms.json`, `alerts_active_snapshot.json`, and the push-token SQLite all
  assume one POSIX filesystem with `flock`. A few small tables with
  compare-and-set replace all of them, and the existing plain-Postgres +
  kopiur pattern in the gitops repo covers backup.
- **Pools per role.** Memory and CPU budgets stop fighting. MRMS is never
  starved by a 9-minute nowcast run. Once tiles are in S3, `alerts` and
  `aux` need no volume and can run 2 replicas on other nodes.
- **Render once, color later.** All three palettes share the same dBZ bin
  edges, so classify the grid once to a uint8 index and write indexed PNGs.
  Two palettes are then a 256-byte `PLTE` chunk rewrite. Estimated frame time
  22–28 s → 5–8 s, tiles ~4× smaller, worker RSS −1.2 GiB.
- **Dedicated Temporal namespace, then sharded server.** The namespace is
  already seeded and unused. It gives radar-ng its own internal scheduler
  worker and rate limits. The shard count cannot change in place; a new
  release with 32–64 shards and a fresh co-located Postgres is a cheap
  migration because all durable state lives outside Temporal history.

### Where the bytes live (and what a node needs)

Radar tiles are small. The big files are the static basemaps, and they belong
on the NAS, not in Longhorn on one node.

| Data | Size | Home after Phase 3 | Per-node disk |
|---|---|---|---|
| Radar/nowcast/HRRR/AQ tiles | ~7 GiB, 4–12 h retention, self-cleaning | RustFS bucket on the NAS | tile-server cache only, ≤ 10 GiB, disposable |
| Grids | ~7 GiB today, ~1 GiB after keep-last-4 | RustFS bucket | none |
| Manifest, state, alerts, push tokens | < 100 MiB | Postgres (kopiur-backed) | Longhorn, 10 Gi |
| Open-Meteo model data | ~6 GiB rolling | Longhorn RWO (the writer needs ext4; NFS fails with errno 95) | 30 Gi on one node, as today |
| CONUS basemap PMTiles | up to 50 GiB, static | **Retired.** The app uses the shared VersaTiles server (`maps.vanillax.me/styles/{light,dark}.json`) | none |
| VersaTiles planet | 62 GiB, static | already on the NAS over SMB | none |

No node needs more than ~40 Gi for radar-ng (Postgres + Open-Meteo + a tile
cache), and only one node holds the Open-Meteo volume. The NAS holds
everything large. Failure mode: NAS down → new frames cannot publish and the
tile-servers serve what they have cached; that is the same blast radius as
today's single node, minus the part where serving also dies.

Bridge option, not the destination: switch the `tiles` PVC to Longhorn RWX so
several tile-servers can read it from other nodes with zero code change. It
adds an NFS share-manager as a new single point of failure and puts NFS
metadata latency on every request. Use it only if multi-node serving is needed
before Phase 3 lands.

---

## 5. Phases

### Phase 0 — stop the bleeding (this week; PRs open)

Gitops (`talos-argocd-proxmox` PR #2160): worker requests/limits and
`AllAtOnce` rollout, `MRMS_RENDER_WORKERS=3`, worker/layer alerts, Temporal
Postgres VPA off and sized, Temporal server metrics scraped, runbook entry for
sleeping schedules.

radar-ng (`feat/reliability-and-scale`):

| Change | Why |
|---|---|
| Schedule `execution_timeout` ≤ 2–3 cadences; jitter 20 s on 2-min schedules; catch-up 5–10 min | A stuck run blocks 3 fires, not 45. No three-way start burst every 2 min. |
| Schedule watchdog in the seeding worker | Re-triggers a schedule whose timer the server lost. The only fix that works on the current server. |
| Worker health file + SDK Prometheus metrics on :9464 | Liveness probe (`test $(find /tmp/temporal-healthy -mmin -3)`) turns a silent 4-hour poll failure into a restart. Metrics give schedule-to-start latency and slot usage. |
| Activity timeouts tightened (MRMS 4/5 min, nowcast 10/12 min, 1 attempt) | Retrying a deterministic 9-minute nowcast doubles the pin. |
| HRRR: 404 = pending, resume from existing tiles, publish the consecutive prefix with `complete: false` | Forecast radar publishes within one fire instead of never. |
| Manifest read errors re-raise instead of wiping the manifest | One transient EIO no longer deletes every layer's history. |
| MRMS `finally` cleanup, pygrib close, manifest update off the event loop | Cancelled renders stop leaking staging dirs and stalling heartbeats. |
| Lightning heartbeat guard + writes off the loop | A disk error no longer kills a healthy activity. |
| API metrics label cardinality, XFF last-hop, uvicorn on 127.0.0.1 | Bounded memory, honest rate limiting, no Caddy bypass. |
| App: pause polling and playback in background, fresh "now", one manifest query, health JSON, fetch timeouts, MMKV validation | Battery, correctness, and the Settings screen no longer says ERROR when data is merely delayed. |

Exit test: 24 h with `ingest-mrms-base` action count ≥ 95 % of expected fires,
`radar-hrrr` in the manifest, no schedule with `NextRunTime` in the past, worker
memory < 70 % of limit.

After the new worker image is live, add to the WorkerDeployment:

```yaml
livenessProbe:
  exec: { command: [sh, -c, "test $(find /tmp/temporal-healthy -mmin -3)"] }
  periodSeconds: 30
  failureThreshold: 3
```

and a PodMonitor for port 9464.

### Phase 1 — one pool per role (1–2 weeks)

The code already has roles (`temporal/task_queues.py`, `WORKER_ROLE`). This is
a rollout, not a rewrite.

| Role / queue | Schedules | Slots | Requests → limits | Volumes today |
|---|---|---|---|---|
| `mrms` | mrms-base, mrms-composite | 6 | 2 CPU / 3 Gi → 6 / 6 Gi | tiles, grids, state |
| `nowcast` | nowcast | 1 | 3 CPU / 4 Gi → 8 / 8 Gi | grids, tiles, state |
| `hrrr` | ingest-hrrr | 4 | 1 CPU / 2 Gi → 4 / 6 Gi | tiles, grids, state |
| `aux` | lightning, tropical, air quality, tile-cleanup; seeds + watchdog | 4 | 0.5 CPU / 1.5 Gi → 2 / 4 Gi | tiles, state |
| `alerts` | poll-alerts, storm watches | 8 | 0.25 CPU / 512 Mi → 1 / 1 Gi | state (ro grids) |

Cutover order (the order matters; `_spec_for` reads env at runtime):

1. Ship the Phase 0 image.
2. Apply the five WorkerDeployments with `SKIP_SCHEDULE_SEED=1`, `AllAtOnce`,
   `podAffinity` to tile-server (verified: the controller keeps it).
3. `task-queue describe --select-all-active` shows a poller on every role queue.
4. Set `SKIP_SCHEDULE_SEED=1` on legacy **first**, then `USE_ISOLATED_TASK_QUEUES=1`
   + `SEED_SCHEDULES=1` on `aux` only.
5. tile-server `TEMPORAL_ALERTS_TASK_QUEUE=radar-ng-alerts`.
6. Scale legacy to 0 when `workflow list --query 'TaskQueue="radar-ng" AND ExecutionStatus="Running"'` is empty.

Exit test: an induced 3-minute nowcast stall does not delay an MRMS frame;
worker OOM alert silent for 7 days.

### Phase 2 — render once (1–2 weeks, can overlap Phase 1)

1. `np.searchsorted` classification to a uint8 index once per frame.
2. Per-tile bilinear sample of the index grid once, encode a mode-`P` PNG.
3. Other palettes: rewrite `PLTE`/`tRNS` chunks, reuse `IDAT`.
4. Cache per-tile projected coordinates per `(grid signature, z, x, y)` for the
   process lifetime — removes the per-tile pyproj transform on HRRR/AQ (~10× there).
5. Persistent `spawn` pool + shared memory instead of pickling 98 MB per palette.
6. Keep only the last N nowcast input grids (frees ~8 GiB on the grids PVC).

Golden-tile tests first (pixel-exact for palette equivalence). Exit test: MRMS
frame < 10 s, nowcast run < 90 s, worker CPU halves.

### Phase 3 — tiles to object storage, stateless tile-server (2–4 weeks)

1. Introduce three seams: `TileStore.put_pyramid/delete/list`,
   `ManifestStore.read/update(mutator)` with compare-and-set,
   `StateStore.processed/put_json/get_json`. The render function returns
   `(z, x, y, bytes)`; today's directory rename becomes `LocalFsTileStore`.
2. Dual-write: `FanoutTileStore(LocalFs, S3)` + `DualManifestStore(file, Postgres)`.
   Nightly object-count parity check against the PVC.
3. Read switch: Caddy `/tiles/*` → `reverse_proxy` to the RustFS bucket
   (public-read, `immutable` headers preserved, local disk cache); FastAPI reads
   the manifest from Postgres.
4. Remove the tile/grid/state mounts from the tile-server. HPA 2–6, PDB
   `minAvailable: 1`, `topologySpreadConstraints` across nodes, `RollingUpdate`
   is now safe.
5. Stop local writes. Bucket lifecycle rules replace `tile-cleanup`. Worker
   roles that only write to S3/Postgres lose their `podAffinity` and schedule
   anywhere.
6. Push tokens and alert state move to Postgres; the `alerts` role runs 2 replicas.
7. Basemap: one map server for everything. The app's `frontend/.env.production` points
   the light/dark styles at the shared VersaTiles instance (`maps.vanillax.me`),
   which already serves Shortbread tiles, glyphs, sprites and two styles for any
   consumer (radar-ng, Project NOMAD, Home Assistant). After one on-device
   check, delete `deployment-basemap.yaml`, `job-basemap-bootstrap.yaml`, the
   `pmtiles` PVC and the `basemap` Service from gitops; the bundled Protomaps
   styles stay in the image as the fallback for other self-hosters.

Storage model after this phase: Longhorn holds Postgres (10 Gi, kopiur-backed)
and the Open-Meteo data volume; RustFS holds every tile and grid; nothing in
radar-ng is pinned to a node.

Exit test: kill the node running the worker; the app keeps serving from the
other tile-server replicas; radar resumes within 2 cadences of the worker
rescheduling. k6 100-user scenario passes (`docs/capacity-acceptance.md`).

### Phase 4 — the app (parallel track, needs a physical iPhone)

1. Restore the 5-slot opacity-swap carousel from commit `b12012f` behind
   `WINDOW` (kill switch `WINDOW = 1`). Every overlay always mounts its source
   (empty GeoJSON when there is nothing to show) so the native child count is
   constant — that is what avoids the iOS `insertReactSubview` crash.
2. Throttle camera bridging off the 60 Hz `onRegionIsChanging`; stop
   recentering on every GPS fix.
3. One projection pass for the four Skia wind paths.
4. Lazy-load the OTel SDK and the Home mini-map after first paint.
5. Break up `settings.tsx` (1366 lines), `index.tsx` (1096), `NowcastScreen.tsx`
   (868) into the `components/home/*` split that was also lost in the merge.
6. Re-enable the four disabled `react-hooks` lint rules, then the React Compiler.

Exit test: playback at 750 ms with zero tile refetches per loop on device;
no frame in the Xcode/Perfetto trace > 16 ms during scrubbing.

### Phase 5 — Temporal platform (gitops repo)

1. Move radar-ng to its existing `radar-ng` namespace (worker
   `temporalNamespace`, tile-server and open-meteo env). Pause and delete the
   11 `default` schedules after the first seed. Retention 72 h.
2. New Temporal release with `numHistoryShards: 32–64`, fresh Postgres
   co-located with the server pods, Longhorn `numberOfReplicas: 2` for that one
   volume. Repoint `Connection/cluster-temporal`; the seeder recreates schedules.
   Pause old schedules first so two clusters never write the same PVC.
3. Temporal alert rules once metrics are confirmed: missed catch-up windows,
   schedule-to-start latency p95 > 60 s, zero available activity slots for
   10 min, poll failures, persistence errors, `no_poller_tasks`.
4. CoreDNS: 3 replicas, topology spread, PDB. Talos manages the CoreDNS
   manifest, so this goes through the Omni cluster template patch, not ArgoCD.
5. Route `severity: critical` to a pager. `RadarNgObservedRadarStale` fired for
   3 hours on 2026-09-02 and nobody was told.

### Phase 6 — prove the capacity envelope

Run the k6 scenario at 100 and 250 users against N tile-servers while killing
a node, delaying NOAA (pause the schedule), and restarting Temporal. Publish
the measured numbers in `docs/capacity-acceptance.md` and set the SLOs from
`docs/radar-north-star.md` as Prometheus recording rules.

---

## 6. Code health (backend)

Do these inside the phases above, not as a separate rewrite:

- **One settings object.** `TILE_DIR/STATE_DIR/GRID_DIR` are read at import in
  nine modules and ~45 other `os.environ.get` sites exist. A pydantic-settings
  `Settings` with `get_settings()` replaces them.
- **One logger.** Activities use the JSON logger, the worker uses loguru, the
  API uses bare stdlib. Pick the JSON logger everywhere.
- **Typed manifest.** Frames are untyped dicts written by four modules by
  convention. `Frame`/`Layer`/`Manifest` pydantic models.
- **Shared ingest kit.** `_safe_path_part`, `_load_palette_tables`,
  `_normalize_lons`, `_render_per_palette`, and the cleanup walkers are copied
  across MRMS/HRRR/AQ. Target layout: `core/`, `stores/`, `render/`,
  `sources/`, `pipelines/`, `activities/` (thin Temporal wrappers).
- **Reproducible builds.** One `pyproject.toml` + `uv.lock`, multi-stage
  Dockerfile (no `gcc` in the runtime image, non-root `USER`), delete the
  unused `backend/base/Dockerfile`, CI runs pytest the same way a laptop does.
- **Tests that bite.** Pixel-level tiler test, HRRR pending/resume/publish
  tests, manifest concurrency test, "every manifest frame × palette has ≥ 1
  tile" invariant test, middleware tests.
- **Observability.** Worker: render duration histogram, tiles written,
  publish lag (the real freshness SLI), backlog depth, HRRR hours pending.
  API: `prometheus_client` histograms, OTel FastAPI/httpx instrumentation,
  Caddy JSON logs with request IDs so tile 404 rates per layer are graphable.

## 7. Code health (app)

- Fabricated placeholder values in Settings and Alerts are removed in Phase 0.
- Dead settings (Playback FPS, °F/°C, Flat/Globe) either work or disappear.
- ~1.5 k lines of dead components and two duplicate manifest→frames mappers go.
- Persisted MMKV values are validated; a bad stored value can no longer crash
  the map on launch.
- ARCHITECTURE.md describes the code that exists, and says what is planned.

---

## 8. Operating rules (keep these true)

1. A schedule's workflow timeout is at most 2–3 cadences.
2. Every schedule has a poller on its queue before it is switched there.
3. Every worker has a liveness probe that fails when Temporal is unreachable.
4. No VPA may `Recreate` a single-replica database.
5. Publish is atomic and the manifest is the only publish boundary: a frame is
   never advertised unless every palette exists.
6. A 404 from NOAA means "not yet", never "failed".
7. Alerts fire on the client-visible symptom (layer absent, publish lag), and
   `critical` reaches a human.
8. Observed data is immutable and cached for a day; forecast paths include the
   run id so they can be immutable too.
9. Comments say what will bite the next editor. Forensics go in commits and
   Mink notes, procedures in `docs/`.
