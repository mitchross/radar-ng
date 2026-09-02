# Radar NG flagship reliability and scale plan

Updated 2026-09-02 after reviewing the Radar NG backend, Temporal workflows,
Expo/React Native app, Kubernetes manifests, Longhorn volumes, shared
VersaTiles service, production metrics, and the 2026-09-01/02 outage.

This is the canonical execution plan. It separates what is live, what is
merged but not live, and what is only proposed. Detailed review notes remain
in `docs/tasks/2026-09-02-*.md`; this document owns the decisions and order.

## 1. The outcome

Radar NG should keep showing the last good map when an ingest worker, Temporal,
a Kubernetes node, or an upstream weather source is unhealthy. New data should
resume without manual repair, and public traffic should mostly hit a cache
rather than the home-lab origin.

The basemap decision is portability first:

- **Radar owns a provider contract, not a basemap implementation.** The app
  consumes a MapLibre-compatible style URL. Rendering, ingest, weather tiles,
  and health never depend on which basemap provider sits behind that URL.
- **Default self-hosting stays batteries-included.** A new operator gets the
  bundled Protomaps/PMTiles path without needing another map service or account.
- **This homelab may share maps.** Production can point the same style contract
  at `maps.vanillax.me` to reuse VersaTiles, styles, glyphs, and sprites across
  Radar NG, Project NOMAD, Home Assistant, and future apps. That is a deployment
  optimization, not a Radar product dependency. If it becomes painful, switch
  this deployment back without redesigning Radar.
- **One independent Radar data plane:** Radar observations, nowcasts,
  forecasts, alerts, and their manifest keep their own lifecycle and SLO. They
  do not get folded into the VersaTiles process.
- **Platform maps stay platform maps.** Expo/MapLibre uses the shared vector
  styles. Native Apple surfaces may use the operating-system basemap instead
  of adding a rasterizing service only for visual uniformity. Their weather
  overlay still comes from Radar NG, not a second radar provider.
- **One front door is fine; one blast radius is not.** Gateway routes can make
  the services look unified later. Internally, static maps and fast-changing
  weather stay independently deployable and scalable.

This lets the homelab share a map foundation when it is useful while keeping a
portable, one-stack Radar install for everyone else. Shared-map work never
blocks Radar reliability work.

## 2. Where production really is today

Five related PRs merged on 2026-09-02:

| Area | PR | State on 2026-09-02 |
|---|---|---|
| Phase 0 backend/app fixes | [radar-ng #33](https://github.com/mitchross/radar-ng/pull/33) | Merged |
| Phase 0 GitOps/alerts/Temporal fixes | [talos #2160](https://github.com/mitchross/talos-argocd-proxmox/pull/2160) | Merged |
| Five role worker pools | [talos #2161](https://github.com/mitchross/talos-argocd-proxmox/pull/2161) | Merged and live at `v1.1.25`; routing remains on legacy |
| Shared VersaTiles production styles | [radar-ng #34](https://github.com/mitchross/radar-ng/pull/34) | Merged; old Radar basemap kept for device rollback |
| Constant-child raster carousel | [radar-ng #35](https://github.com/mitchross/radar-ng/pull/35) | Merged behind `CAROUSEL_WINDOW=1` |

The stale Argo render cleared on a fresh repo-server render without an operator
restart. Production now has the legacy worker plus all five role
WorkerDeployments at `v1.1.25`, and the PodMonitor scrapes all six. Both
workflow and activity pollers are present on every role queue.

Argo is still `OutOfSync/Healthy`: admission adds WorkerDeployment defaults
(`minReadySeconds`, `progressDeadlineSeconds`, and port protocol), while Git
omits them. Argo server-side-applies the same six resources every five minutes.
Declare those stable defaults in Git; do not hide arbitrary WorkerDeployment
differences.

**Do not switch schedules to role queues yet.** The new pools are healthy but
idle, HRRR is absent, alert polling still needs watchdog recovery, and all
shared RWO mounts keep every pool on one node.

Other live facts:

| Signal | Current evidence | Meaning |
|---|---|---|
| MRMS and nowcast | Fresh during the audit | Main observation path is working now |
| HRRR radar | Missing from the public manifest | Phase 0 has not passed |
| HRRR failure | Future 404s are now pending, but NOAA reports REFC as `shortName=refc`; the display-name selector silently renders zero layers | Match the stable short name and fail loudly when mandatory REFC is absent |
| Schedules | Live scheduler `USER_TIMER` tasks for alerts, cleanup, and Open-Meteo were found in Temporal's timer DLQ; a scoped replay restored their future timers | Watchdog preserves continuity but cannot repair a dead-lettered durable timer |
| Temporal | One history shard, one replica per service, single Postgres | A known control-plane failure domain |
| Push/workflow API | `DISABLE_WORKFLOW_ROUTES=1`; all workers use `PUSH_DISABLED=1` | Deliberately off; not a production-ready feature |
| Public manifest | ~119 KB raw, ~4.8 KB through Cloudflare compression, ~0.3 s sample | Bandwidth is not the first problem |
| Radar tile | ~0.18–0.25 s cold public sample | Origin is fast; tunnel/edge dominates the sample |
| Basemap vector tile | ~0.50 s public sample | Measure/cache before changing storage |

These are point-in-time observations, not capacity results.

### Storage and compute measurements

| Volume | Used / requested | Live observation |
|---|---:|---|
| Tiles | 5.08 / 100 GiB | peak ~85 read IOPS and ~0.42 MB/s reads |
| Grids | 8.52 / 20 GiB | peak ~21 write IOPS and ~1.63 MB/s writes |
| State | 1.6 MiB / 5 GiB | peak write latency ~4.1 ms |
| Open-Meteo | 5.44 / 30 GiB | quiet during the sample; requires an ext4-style RWO mount |
| Retiring PMTiles | 17.8 / 50 GiB | quiet |

The worker peaked near 1.63 CPU and 6.68 GiB in the initial observed hour. The tile
server used about 4 millicores and 77 MiB. A representative radar frame was
207 PNGs per palette and ~9.5 MB across three palettes; composite was 396 PNGs
per palette and ~18.9 MB.

After the pools went live, the five idle pods used only about 85–89 MiB each,
but reserved 6.75 CPU and 11 GiB. The shared GPU node was at 88% requested
memory with about 11 GiB scheduler headroom. Actual memory was comfortable;
requested capacity is the cutover and rollout constraint.

**Disk speed is not the current hot path.** Moving the same workload to a local
PVC would trade away failover without fixing the CPU, memory, scheduler, or
single-node coupling problems.

## 3. What failed and what it teaches us

| Failure | Verified cause | Permanent rule |
|---|---|---|
| Four-hour radar gap | CoreDNS was down; worker polling failed; a 90-minute workflow under `SKIP` blocked later fires | Workflow timeout is at most 2–3 cadences; poll health drives liveness |
| Three schedules slept for hours | Their exact scheduler `USER_TIMER` tasks were dead-lettered during Temporal persistence/control-plane faults; manual triggers never restored them | Alert on the timer DLQ, recover it deliberately, and migrate to a new sharded Temporal database |
| Worker OOM | Nowcast, HRRR, AQ, two MRMS renders, alerts, and lightning shared one process | One bounded worker pool per role |
| HRRR absent | Future 404 handling was fixed, then a mutable GRIB display-name selector stopped matching NOAA's current REFC metadata | Use stable GRIB identifiers and fail a downloaded hour when a mandatory field is absent |
| One node owns everything | Tiles, grids, and state are Longhorn RWO volumes | Durable shared artifacts leave POSIX volumes; scratch stays local |
| Argo said Synced but was stale | Repo-server returned a poisoned cached render | A rollout verifies live inventory and image digests, not only Argo health text |

## 4. Target architecture

```text
NOAA / NWS / NHC / Blitzortung
              |
              v
     Temporal control plane
  schedules, retries, timeouts, routing
              |
              v
  bounded role worker pools
  MRMS | nowcast | HRRR | auxiliary | alerts
              |
      node-local render scratch
              |
      validate one complete frame
              |
       +------+-------------------+
       |                          |
       v                          v
RustFS S3-compatible store     Postgres
immutable tiles/grids          catalog, state, alerts, tokens
       |                          |
       +------ versioned manifest snapshot
                         |
                         v
           stateless Radar API/tile pods x2+
                         |
           Cloudflare cache + Envoy Gateway
                         |
                         v
            Expo app: observed + future data

Basemap provider selected by deployment:
  default self-host -> bundled Protomaps/PMTiles
  this homelab      -> optional VersaTiles -> maps.vanillax.me
  native surface    -> operating-system map where appropriate
```

Temporal starts work; it does not own the only copy of useful data. Postgres is
the write-side catalog; a versioned last-good manifest snapshot is the serving
contract. If Temporal or Postgres is unavailable, clients can still read the
last published manifest and immutable tiles.

### Publish contract

Every producer follows the same sequence:

1. Download and decode upstream data with a bounded timeout.
2. Render into node-local scratch. Nothing is public yet.
3. Validate expected palettes, tile paths, counts, metadata, and timestamps.
4. Upload immutable objects under a run/frame ID.
5. In one Postgres transaction, validate the active publication epoch, update
   the catalog with compare-and-set, allocate a monotonic revision, and append
   an outbox row.
6. An idempotent reconciler builds the manifest from that committed revision,
   writes `manifest/{revision}.json`, then advances the current pointer only
   when its revision is newer.
7. Catalog-driven garbage collection deletes unreferenced objects only after a
   grace period longer than every client/CDN cache window. Bucket lifecycle
   handles abandoned staging keys, not live catalog data.

The catalog revision is the group commit and the manifest is its read model. A
frame is never advertised because one palette directory happened to rename
first. The outbox/reconciler repairs a database commit followed by a failed
snapshot write; revision compare-and-set prevents concurrent publishers from
moving the pointer backward.

Every frame records source, source run/issued time, forecast valid time,
ingest time, checksum, renderer version, and whether a forecast run is
complete. Observed, nowcast, and forecast times must never be overloaded into
one ambiguous timestamp.

Each source adapter owns provider identification, attribution, rate limits,
availability timing, retries, checksums, and a recorded fixture. Product
attribution travels through the manifest to every client surface.

## 5. Data placement

| Data | Correct home | Why |
|---|---|---|
| Published weather tiles and retained grids | Dedicated `radar-ng` RustFS bucket | Immutable/shared data lets workers and servers run on any node |
| Render staging and hot disposable cache | `emptyDir` on node NVMe/local disk | Fast, cheap, safe to lose |
| Catalog, processed-source state, alerts, push tokens | Postgres on a small Longhorn volume with kopiur backup | Transactions and compare-and-set replace `flock` and SQLite sharing |
| Open-Meteo model files | Existing Longhorn RWO volume | Its writer needs filesystem behavior that SMB/RWX did not provide |
| Static basemap | Bundled PMTiles in the default self-host profile; optional TrueNAS/VersaTiles in this homelab | Same style-URL contract, deployment chooses the provider |
| Client delivery cache | Cloudflare plus bounded per-pod cache if measurements justify it | Public immutable tiles should not repeatedly cross the home uplink |

RustFS currently shares a TrueNAS failure domain with other important data.
Radar must use its own bucket, credentials, quota, and lifecycle. Load-test it
against backup traffic. Do not reuse the kopiur bucket. A NAS outage may stop
new publication and uncached reads; the honest home-lab target is graceful
stale serving and quick recovery, not imaginary zero downtime.

Longhorn RWX is allowed only as a short bridge. It adds an NFS share-manager
and metadata latency while preserving a storage bottleneck. It is not the
destination. Longhorn RWO remains appropriate for the small database and
Open-Meteo; local PVC is appropriate only for rebuildable scratch/cache.

## 6. Reliability and release budgets

These are release gates. Recalibrate them after the first representative load
test; do not silently weaken them.

| User-visible signal | Initial target |
|---|---|
| MRMS freshness | 99% of samples have latest issued data under 6 minutes old; page at 10 minutes |
| Publish correctness | 100% of advertised frame/palette combinations have a validated non-empty pyramid |
| Radar serving plane | 99.9% monthly while Gateway, WAN, and object origin are reachable; each serving pod keeps a last-good manifest copy |
| End-to-end home-lab service | Start at 99.5%; claim 99.9% only after independent NAS/object and WAN/power failure domains exist |
| MRMS worker/node loss | A second spread poller continues work; latest data remains under the 10-minute stale page threshold |
| Point forecast | Track separately while the single RWO Open-Meteo data service remains; publish its measured RTO |
| Manifest response | p95 under 500 ms through the public route |
| Immutable tile response | warm origin p95 under 250 ms; public cold p95 under 800 ms |
| Playback after warm-up | no blank frame; at least 95% of transitions need no tile request at display time |
| Mobile rendering | p95 frame under 18 ms, p99 under 50 ms, and no upward memory slope or >100 MiB growth after 20 loops |
| Background behavior | polling, prefetch, playback, and wind callbacks stop within 5 seconds of AppState becoming inactive |

Measure freshness from the timestamp a client can actually see, not only from
whether a workflow says Completed.

Public-release security is part of the same gate: internal S3 and Postgres are
not public, workloads use dedicated service accounts and least-privilege
ExternalSecret-managed credentials with a rotation drill, NetworkPolicies
limit east-west access, containers run non-root, and the Gateway is the only
origin entry. Rate limits are route-specific;
immutable tiles lean on the CDN while forecast, geocoding, and alert queries
have bounded caches and abuse limits. Log request IDs and coarse regions, never
tokens, object credentials, exact coordinates, or unbounded URL paths.
Metrics and liveness stay cluster-only; a small public readiness endpoint may
report only user-actionable freshness. Trust forwarded client IPs only from
known proxies, and enforce shared request-body, concurrency, and rate limits at
the edge rather than one unrelated in-memory limiter per replica.

## 7. Execution plan

### Gate 0 — make the merged safety work real

**Status: merged and live, but not passed.**

1. Stop the WorkerDeployment default-field self-heal loop and prove Argo stays
   `Synced/Healthy` for at least 15 minutes.
2. Keep proving all five role WorkerDeployments, their workflow/activity
   pollers, the SDK-metrics PodMonitor, and the live image versions match Git.
3. Ship the REFC selector fix, then verify future HRRR 404s remain pending and
   a consecutive prefix publishes with `complete: false`.
4. Confirm worker health/liveness, Temporal SDK metrics, Radar/Temporal alerts,
   and the watchdog are present and scraped.
5. Run 24 hours with MRMS action count at least 95% of expected fires, HRRR in
   the manifest, no overdue schedule, and worker memory below 70% of limit.
6. Route critical stale-data alerts to a human and perform one test page.

The 2026-09-02 recovery merged only timer-DLQ messages 0–6, the smallest prefix
covering Radar's three live stuck schedulers. Five messages remain because the
next prefix includes two live News schedules. Coordinate that cleanup
separately; never purge a timer DLQ without reading and matching every entry to
its current workflow. A healthy gate requires natural schedule fires, not only
watchdog-triggered runs.

Rollback through a Git revert to the last safe image, but preserve the bounded
timeouts, catch-up windows, watchdog, and freshness alerts. Never restore the
known-bad 90-minute schedule behavior, disable alerts, or delete last-good
tiles as part of rollback.

### Gate 1 — cut over the five worker pools

**Status: pools live and polling; schedules still route to legacy.**

The pools are `mrms`, `nowcast`, `hrrr`, `aux`, and `alerts`. They isolate CPU,
memory, and task slots, but their shared RWO mounts still force them onto the
same node. This phase is process isolation, not high availability.

1. Replay representative production histories against the new worker image and
   use Temporal patch/version markers for Workflow-code changes.
2. Bring up every pool with `SKIP_SCHEDULE_SEED=1`.
3. Inventory every queue producer and consumer: the five role queues, legacy
   `radar-ng`, the `radar-ng-open-meteo` activity queue, tile-server Workflow
   routes, and any long-lived watch/lightning execution.
4. Check both workflow and activity pollers on every role queue and the
   Open-Meteo activity poller.
5. Check node headroom. During overlap, requested memory can reach roughly
   94–96% on the only eligible node; reduce overlap or right-size from observed
   use before creating eviction pressure.
6. Set legacy `SKIP_SCHEDULE_SEED=1` first so a restart cannot route schedules
   backward.
7. On `aux` only, set `SKIP_SCHEDULE_SEED=0`, `SEED_SCHEDULES=1`, and
   `USE_ISOLATED_TASK_QUEUES=1`; that one seeder rewrites every schedule.
8. Before workflow routes are ever enabled, set tile-server
   `TEMPORAL_ALERTS_TASK_QUEUE=radar-ng-alerts` and verify the namespace.
9. Verify every schedule's actual task queue and run one schedule per role.
10. Keep both pool sets until pinned/running legacy executions, including
    long-lived work, are inventoried and drained.
11. Prove a three-minute nowcast stall does not delay MRMS, then watch OOM,
   schedule-to-start, queue backlog, and publish lag for seven days.

Rollback: first prove the legacy workflow/activity pollers are live, keep the
bounded schedule definitions and alerts, then re-seed every schedule onto
`radar-ng`. Wait for role-queue executions to drain. Never point a schedule at
a queue before its poller exists.

### Gate 2 — finish render-once correctly

**Status: promising dirty worktree; not merge-ready.**

The current candidate changes a synthetic three-palette run from about 7.00 s
and 6.9 MiB to 0.39 s and 0.9 MiB on tmpfs. That is directional, not a
production benchmark. The full isolated suite has 114 passing tests, 36
passing subtests, and five failures in the new indexed-tiler tests.

The safe design is:

- `GridSpec`: immutable axes/grid ID, shape, CRS, bounds, orientation, and
  nodata policy. Use it as the geometry-cache identity.
- `TileSamplingPlan`: bounded single-flight cache of fractional coordinates
  for continuous fields and nearest indices for categorical fields. The
  resampling method is part of the key.
- `PaletteModel`: validated bin edges and indexed-PNG palette/alpha chunks.
- `FramePublisher`: staging, completion metadata, winner validation, cleanup,
  cancellation safety, and per-stage timings.

Never interpolate palette indices. Define and review a resampling policy per
product and zoom: bilinear physical-value sampling is the default candidate
for continuous fields, nearest-neighbor for categorical fields, while radar
reflectivity may need a reference-tested max/nearest/bilinear policy to retain
small intense cells. Validate each choice against GDAL/Rasterio reference
fixtures and product meaning. A synthetic MRMS-sized test found physical-first
bilinear only ~0.37 s slower than nearest-class sampling and changed about 1%
of visible pixels, proving that the choice matters but not which policy is
scientifically right for every product. Nodata interpolation must be explicit
and validity-aware.

Also fix the grid-pruning race: pruning can remove a just-fsynced generation
before its metadata pointer commits. Serialize write/prune per layer or protect
recent orphans with a grace period. Treat an existing final directory as valid
only when its completion marker and tile count pass validation.

Release behind `TILE_RENDERER=legacy|indexed`. Required tests:

- Real MRMS, nowcast, projected HRRR/AQ, and categorical golden fixtures.
- Analytical ramps for threshold placement and nodata edges.
- Palette path-set equality and at least one tile per advertised palette.
- Cache collision, eviction, concurrent fill, cancellation, failed rename,
  duplicate renderer, and write/prune race injection.
- Physical MapLibre decode check for indexed PNG transparency.

Exit: production canary keeps MRMS under 10 s and nowcast under 90 s, uses
bounded cache/RSS, and advertises zero empty or partial pyramids. Rollback is a
single flag change; keep the legacy renderer until the canary passes.

### Gate 3 — separate compute, storage, and serving

**Status: design approved here; implementation not started.**

1. Add `TileStore`, `GridStore`, `CatalogStore`, and `StateStore` interfaces.
   Keep today's filesystem behavior as the first implementation.
2. Make activities thin Temporal wrappers around source, transform, render,
   validate, and publish services. A renderer returns bytes/metadata; it does
   not know a PVC path.
3. Create a dedicated Radar bucket and database schema. Add lifecycle, quotas,
   backups, restore test, and least-privilege credentials before data arrives.
4. Shadow-write immutable tiles/grids to object storage. Compare object count,
   checksum, manifest references, and representative pixels with local output.
5. Move manifest/state updates to transactional Postgres and emit a versioned
   last-good JSON snapshot through the revisioned outbox. This Radar catalog
   is separate from Temporal's persistence database.
6. Switch reads behind a flag. Stock Caddy does not provide a durable upstream
   object cache: choose and test an explicit cache layer or serve from a small
   Radar tile service, plus a Cloudflare Cache Rule for immutable paths.
7. Run two stateless Radar serving pods with RollingUpdate, PDB, and topology
   spread plus required cross-node anti-affinity. Remove their PVC mounts and
   required node affinity. Persist/warm a bounded last-good manifest/tile cache
   per replica.
8. Stop local writes only after seven days of parity. Keep the old PVC intact
   through the rollback window, then retire it through GitOps.
9. Give `emptyDir` scratch/cache a `sizeLimit`, ephemeral-storage requests and
   limits, startup cleanup, and eviction alerts. Local disk is bounded even
   though it is disposable.
10. Run at least two spread MRMS and alerts pollers after publication is
    idempotent and fenced. A PDB protects voluntary disruption only; replicas
    and anti-affinity handle a failed node.

Scale serving on request concurrency/latency and scale each worker role on
Temporal backlog plus its measured memory per activity. CPU-only autoscaling
can start too many render activities and create another OOM. All activities
must be idempotent before a role runs more than one replica.

Open-Meteo remains an explicit exception. Give point forecast its own SLO and
measured Longhorn reattach/rebuild RTO. Before scaling its read path, publish
immutable model snapshots and benchmark a bounded whole-file/chunk cache per
reader; do not turn each point or wind request into many small object-store
range GETs.

Exit: killing one serving/worker node does not blank existing maps or make MRMS
stale beyond 10 minutes. New publishes may pause during a NAS/database
incident. Cached clients and already-warm serving replicas keep last-good data,
but cold clients are not called available until an independent object copy and
tested failover exist. Record NAS and WAN recovery times explicitly.

### Optional track — shared homelab basemaps

**Status: useful resource sharing, not a Radar release gate.**

The production profile currently points at `maps.vanillax.me`. Keep it if it is
cheap and reliable enough; switch this deployment back to bundled PMTiles if it
starts consuming time that belongs to Radar.

The minimum Radar work is small:

1. Keep `bundled`, bring-your-own PMTiles, and `external style URL` as supported
   provider modes. Test the same app behavior against bundled and external.
2. Show Radar overlays even when an external style is stale or unavailable;
   keep last-good style/assets where MapLibre permits it.
3. Never remove bundled support from the Radar repository. This homelab may
   remove its duplicate PMTiles Deployment/PVC after the deferred device gate,
   because that is a local deployment choice.

Only if the shared service proves valuable, harden it independently in the
GitOps repository: replicate VersaTiles and `map-styles`, use safe rolling
updates and cross-node placement, measure the TrueNAS path, add external
checks, and prove edge cache HIT/purge behavior. None of that blocks worker,
render, weather-storage, API, Temporal, or mobile correctness milestones.

### Gate 5 — make the app truthful and smooth

**Status: correctness work can start now; carousel enablement waits for real
phones, as agreed.**

Before the phone gate:

1. Stop telemetry from serializing query keys containing exact latitude and
   longitude. Record only the query family. This is a release blocker.
2. Validate network and MMKV manifest data at runtime. Update frame data and
   selected index atomically when old frames are pruned.
3. Show `Live`, `Updated N min ago`, `Offline`, or `Unavailable` from real
   freshness. Never stamp a cached mini-map `LIVE`.
4. Wire NetInfo into React Query's online manager. Define bounded last-good
   behavior for manifest, forecast, alerts, styles, and offline packs.
5. Serve alert queries through a cached Radar backend endpoint. Do not make
   every public phone poll NWS directly with its exact coordinates.
6. Put search/reverse-geocoding behind a policy-compliant cached endpoint, or
   explicitly budget and identify direct provider traffic. Public upstreams
   must not scale linearly with active phones.
7. Make every client use the same Radar contract. CarPlay currently bypasses
   it for Iowa Mesonet radar; Watch and CarPlay call NWS directly. Move weather
   overlays and alerts to Radar NG. A native system basemap remains acceptable.
8. Wire playback speed, units, and map mode end to end, or remove the settings.
9. Add frontend CI for typecheck, lint, tests, and manifest contract fixtures.

Push and Workflow routes remain disabled until there is an identity/session
issuer, quotas, key rotation, encrypted token storage, token deletion/audit,
and abuse tests. Replace the single pending-alert signal with a durable,
deduplicated queue so alert bursts cannot overwrite each other. An alert is
marked handled only after an at-least-once delivery attempt is durably
recorded. Test expiry, retry exhaustion, duplicate delivery, revoked tokens,
and a burst before enabling `/v1` routes or `PUSH_DISABLED=0`.

Performance work can proceed behind flags: throttle continuous camera bridge
updates, persist only the final camera, project all wind paths in one pass,
stop wind work when the tab/app is inactive, and lazy-mount the Home mini-map.

The five-slot carousel is structurally sensible and reuses four of five native
sources on sequential playback, but `CAROUSEL_WINDOW` stays `1` until a release
build runs 20 loops on one supported iPhone and one mid-range Android with:

- zero crashes, child-index errors, or blank frames;
- at most one raster-source remount per sequential tick;
- at least 95% no-request transitions after the first loop;
- p95 frame under 18 ms, p99 under 50 ms, and no sampled stall over 100 ms;
- no upward memory slope or more than 100 MiB growth after 20 loops;
- correct rapid scrub, palette/style/server changes, and background/foreground;
- background work stops within 5 seconds and stays stopped for the test; and
- safe recovery from a corrupt cache and malformed manifest.

Split the large Settings, Home, FAB, API, and activity files by domain after
behavior is covered. Do not mix an architectural rewrite into native crash
debugging.

### Gate 6 — replace the one-shard Temporal platform

**Status: mitigated, not solved.**

Move Radar schedules into the existing `radar-ng` Temporal namespace first;
that gives retention, quota, and ownership isolation, but does not isolate a
shared server failure.

History shard count is fixed when Temporal's database is created. Build a new
Temporal v2 database with 32 shards rather than trying to mutate the one-shard
database. Run the Temporal services separately, spread replicas where the
home-lab budget permits, back up Postgres, and test an actual restore. A
single-writer Longhorn Postgres is recoverable, not magically highly
available; serving old maps must not depend on it.

Before every worker rollout—not only this migration—replay representative
production Workflow histories and use Temporal versioning for incompatible
Workflow changes. Inventory and drain or deliberately recreate long-lived
watch/lightning executions. Migrate every producer, including tile-server and
Open-Meteo queue clients, not just schedule definitions.

Pausing schedules does not fence workflows that are already running. Allocate
a publication epoch in the Radar catalog and require its fencing token on
every state/manifest commit. Pause old schedules, revoke the old epoch, drain
or terminate old writers, seed the new namespace/cluster once, verify pollers,
then migrate one role at a time. A stale cluster can upload an orphan object,
but it cannot publish it.

CoreDNS needs three spread replicas and a PDB through the Talos/Omni-owned
configuration path. Alert on schedule-to-start, zero pollers, slot exhaustion,
persistence errors, missed catch-up, and client-visible publish lag.

Exit: restart Temporal and its database during a test; serving stays healthy,
no duplicate publish occurs, and schedules resume inside their catch-up
window.

### Gate 7 — prove the public capacity envelope

Replace the current single-location k6 loop with scenarios that cover:

- cold and warm cache, 100 then 250 concurrent users;
- several regions, zooms, palettes, layers, and full playback loops;
- manifest/style/glyph/sprite/forecast/alert traffic;
- load during MRMS and nowcast publication;
- upstream 404/delay, worker OOM, pod and node loss;
- Temporal/Postgres restart, NAS pause, and CoreDNS disruption; and
- cache-hit ratio, home-uplink bandwidth, origin RPS, publish lag, and battery.

The test passes only if user SLOs hold and ingest freshness does not degrade.
Publish results in `docs/capacity-acceptance.md`, including hardware, dataset,
cache state, exact image digests, and the first saturated resource.

## 8. Code shape after the migration

This is a gradual extraction, not a rewrite.

Backend target:

```text
backend/
  core/        settings, logging, models, clocks
  sources/     NOAA/NWS/NHC/Blitzortung clients
  render/      grid specs, sampling plans, palettes, encoders
  stores/      local, S3, Postgres implementations
  pipelines/   ingest -> transform -> validate -> publish
  activities/  thin Temporal wrappers
  api/         read-only client contracts and health
```

Use one typed settings object, one structured logger, typed manifest/catalog
models, one reproducible `pyproject.toml`/lock, multi-stage non-root images, and
shared ingest primitives. Metrics need histograms for source latency, render
stages, tiles/bytes, publish lag, queue wait, cache results, and HTTP duration.
Avoid high-cardinality labels such as frame IDs or coordinates.

Frontend target: one validated API boundary, one source of manifest/frame
state, explicit freshness/offline state, domain-sized components, and behavior
tests. The native map owns visual state; React state changes only at useful
boundaries, not on every camera frame or particle.

## 9. Things we will not do

- Do not move durable Radar data to local PVC and call it scale.
- Do not make Longhorn RWX the permanent tile architecture.
- Do not merge dynamic Radar rendering/serving into VersaTiles.
- Do not require `maps.vanillax.me` or any external basemap to self-host Radar.
- Do not spend Radar's critical path hardening a shared map service unless
  measurements show that service is the user-visible blocker.
- Do not switch queues without confirmed workflow and activity pollers.
- Do not advertise a frame before every required palette validates.
- Do not enable the five-slot carousel before the deferred device gate.
- Do not delete the old basemap/PVC before the same gate.
- Do not claim HA while TrueNAS, Temporal Postgres, or the home uplink is a
  single failure domain. Name the limitation and make failure graceful.
- Do not add Kafka, Redis, or another platform until a measured need exists.

## 10. Immediate order of work

1. Land the WorkerDeployment-default fix and prove Argo stops self-healing.
2. Land the HRRR REFC selector fix and prove a real forecast prefix publishes.
3. Start the 24-hour Phase 0 watch, especially HRRR and overdue schedules.
4. During that watch, fix telemetry location leakage and add frontend manifest
   validation/CI; these do not depend on phone builds.
5. Pass Phase 0, then cut the worker pools over with poller proof and
   node-headroom checks.
6. Repair and canary render-once behind a rollback flag.
7. Implement storage interfaces and shadow-write the Radar object bucket.
8. Replicate the Radar serving plane. Harden shared maps separately only if the
   optional homelab profile remains worth operating.
9. Move Radar to the new sharded Temporal database with publication fencing.
10. Run capacity/failure/security tests, then publish the honest support
   envelope and recovery times.
11. Perform the deferred iPhone/Android gate and only then enable five slots
    and retire the duplicate basemap.

## 11. Engineering references

- Temporal server roles and fixed history shards:
  <https://github.com/temporalio/documentation/blob/main/docs/encyclopedia/temporal-service/temporal-server.mdx>
- Temporal production history replay:
  <https://github.com/temporalio/documentation/blob/main/docs/develop/safe-deployments.mdx>
- Kubernetes volume access modes:
  <https://kubernetes.io/docs/concepts/storage/persistent-volumes/>
- Kubernetes topology spread:
  <https://kubernetes.io/docs/concepts/scheduling-eviction/topology-spread-constraints/>
- Longhorn production practices:
  <https://longhorn.io/docs/1.12.1/best-practices/>
- MapLibre React Native raster sources and offline manager:
  <https://maplibre.org/maplibre-react-native/docs/components/sources/raster-source/>
  and <https://maplibre.org/maplibre-react-native/docs/modules/offline-manager/>
- VersaTiles server/cache guidance:
  <https://docs.versatiles.org/guides/deploy_using_docker>

## 12. Definition of done

Radar NG is ready to share when a new operator can deploy it from Git with the
bundled basemap or choose an external style without code changes, a user can
tell live from stale data, one failed pod/node does not blank existing weather,
upstream delays do not wedge later schedules, render and serving capacity are
measured, alerts reach a person, restore/rollback drills pass, and the app stays
smooth on supported real devices.
