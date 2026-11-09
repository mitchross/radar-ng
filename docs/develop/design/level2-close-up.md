# Level II close-up reflectivity

**Proposed, not approved.** Build spec for [#263](https://github.com/mitchross/radar-ng/issues/263)
and [map #272](https://github.com/mitchross/radar-ng/issues/272). This note implements nothing.
It covers [north-star steps 1–4](../../reference/north-star.md#hyper-local-observed-radar).
Velocity, higher tilts, composites and multi-station mosaics remain out of scope.
Approve this design before building; merging it is not permission to enable the feature.

## Recommendation

Keep MRMS as the public Observed radar layer. Add one server-selected station's lowest
reflectivity sweep as close-up detail, not a separate expert product in the layer picker.
Show the same detail and provenance on the web map and the app's Radar tab.
Decode only viewed stations. Store immutable numeric sweeps in RustFS, then render only requested
tiles. Publish a bounded viewport manifest **after** its tiles exist. Never call an unrendered
source a ready frame. Clients talk only to our server.

## 1. Decisions already settled

These are the closed tickets' resolutions, not new choices. Reopen the source ticket to change one.
The four research notes remain on their research branches, not on master.

| Resolution | Settled answer | Evidence and limits |
|---|---|---|
| [#273](https://github.com/mitchross/radar-ng/issues/273): access | Poll anonymous `unidata-nexrad-level2-chunks` for viewed stations; use `unidata-nexrad-level2` for bootstrap/history/gaps. Parse sweep completion, never a fixed chunk count. | [Access research](https://github.com/mitchross/radar-ng/blob/research/level2-access/docs/research/level2-access.md): one KGRR lowest sweep used 1.43 MB and preceded its full volume by 439 s. Retrospective sample, not an SLA. Old `noaa-nexrad-level2` returned 403; SNS needs an account. |
| [#274](https://github.com/mitchross/radar-ng/issues/274): decoder | Use MIT `nexrad-data` / `nexrad-decode` 1.0.0 behind our bounded adapter; no new Archive II parser. | [Decoder research](https://github.com/mitchross/radar-ng/blob/research/level2-decoder/docs/research/level2-decoder.md): real KGRR full-volume warm median 1.108 s on Rust 1.97.0; excludes HTTP/QC/tiles. Stock expansion is unbounded. |
| [#275](https://github.com/mitchross/radar-ng/issues/275): rendering | Cache QC'd polar sweeps; inverse-map requested XYZ tiles, not a whole-station pyramid. Use actual gate geometry and 4/3-Earth propagation. | [Render research](https://github.com/mitchross/radar-ng/blob/research/level2-render/docs/research/level2-render.md): synthetic timings only; require real geometry, seams and aliasing validation. |
| [#276](https://github.com/mitchross/radar-ng/issues/276): QC | Mask invalid/sentinel, blanked and unobserved gates once per sweep. Keep valid REF without dual-pol. No default CC/ZDR/texture deletion. | [QC research](https://github.com/mitchross/radar-ng/blob/research/level2-qc/docs/research/level2-qc.md): blanket CC removal can erase hail. Residual clutter remains possible. |
| [#277](https://github.com/mitchross/radar-ng/issues/277): crossover | Linear z8→9 fade; full detail at z≥9. Nearest healthy station within 150 km ground range of view centre; newest complete low sweep ≤10 min old. One station per view; MRMS always underneath. | Fall back within one manifest refresh for late/down/out-of-range data. Show station, scan time, tilt and beam height. Zoom alone is insufficient. |
| [#278](https://github.com/mitchross/radar-ng/issues/278): sweep/fields | Lowest-elevation REF only, nominal 0.5°; repeated SAILS/MESO-SAILS low scans are separate frames. Decode RHO/ZDR only for QC/provenance. | Stop fetching each selected cut once complete; continue bounded discovery for later supplemental cuts. Existing reflectivity palette and colour-vision modes must match MRMS. |
| [#279](https://github.com/mitchross/radar-ng/issues/279): capacity | New `radar-ng-level2` pool/queue; 10 s polls; 8 active stations; 2 simultaneous decodes. Tile demand activates a station; idle after 10 min without demand. Last 6 low sweeps; evict 30 min after idle. | 512 MiB rendered-tile and 1 GiB decoded-sweep caches, byte-capped and rebuildable. Overflow → MRMS, not a growing queue. Require 3–5-site shadow pilot and QC p95 ≤25 ms/sweep. |

## 2. Site catalogue and health

Use the public [NCEI HOMR station file](https://www.ncei.noaa.gov/access/homr/file/nexrad-stations.txt)
as the versioned catalogue input. A real unsigned GET on 2026-11-09 returned column headers
`NCDCID ICAO WBAN NAME COUNTRY ST COUNTY LAT LON ELEV UTC STNTYPE` and this record:

```text
30001856 KGRR 94860 GRAND RAPIDS ... MI KENT 42.893889 -85.544889 875 -5 NEXRAD
```

Filter `STNTYPE=NEXRAD`; the same file contains TDWR, which is not this decoder's input.
Keep ICAO, name, coordinates, source URL, fetch time and checksum. Validate coordinate bounds,
unique station IDs and sample locations before replacing the last-good catalogue.
Ship a checked snapshot; refresh server-side daily with conditional requests and operator-visible
diffs. A failed refresh never empties the map. Do not assume every catalogue site has a live chunk feed.
Allow only verified feed-capable sites in the enabled-region list.

The [ROC network list](https://www.roc.noaa.gov/branches/program-branch/site-id-database/site-id-network-sites.php)
confirms KGRR / Grand Rapids but dates itself January 2018; it is a cross-check, not a current
health API. The [ROC site database](https://www.roc.noaa.gov/branches/program-branch/site-id-database.php)
links network status and site metadata. Do not scrape status pages to decide frame readiness.
The HOMR text header does not establish the elevation datum or antenna reference. Read antenna
altitude from decoded volume metadata and cross-check site metadata before publishing MSL heights;
never silently treat catalogue `ELEV=875` as metres or antenna height.

Track three separate clocks per active station:

| Signal | Recommended rule | Effect |
|---|---|---|
| Feed progress | Record newest chunk LastModified and last observed advancing key. Warn `feed_late` after 120 s without progress. | Diagnostic, not an early replacement for the agreed 10-minute eligibility rule. A recent chunk may belong to a higher tilt, not a ready low sweep. |
| Usable observation | Age = server UTC now minus newest completed lowest REF sweep's **end time**, not download time or volume-name time. Eligible through exactly 600 s. Reject impossible future times. | Older/missing complete sweep → `late`/`warming`, MRMS. Current chunks cannot make an old sweep fresh. |
| Our access path | Record listing/GET failures separately from successful but stalled listings. | `source_unreachable` means our access failed, not proof the physical radar is down. Keep last-good only while age permits. |

Call the user-facing state **Level II unavailable**, not “radar down,” unless an authoritative
outage is independently confirmed. An inactive station is `unknown`, not stale by definition.
Probe the nearest candidate when demand arrives, then try the next nearest eligible station within
150 km if the first cannot provide a fresh sweep. Bound attempts to three candidates per request;
never activate more than eight stations globally. Break equal-distance ties by station ID.
Freshness is still enforced locally on cached client metadata during network failure.

## 3. Artifacts, caches and retention

This follows the north star's object-delivery target, not the current filesystem baseline.
[Architecture](../../reference/architecture.md) still describes PVC pyramids; this design does
not claim that object migration or three stateless API replicas are already deployed.

```text
viewport tile demand → station lease → live chunks / archive recovery
  → bounded decode + conservative QC → immutable numeric polar artifact in RustFS
  → requested XYZ + palette tiles in RustFS → verified viewport manifest in PostgreSQL
  → stateless API / public gateway → web + app (MRMS underlay)
```

| Data | Owner and location | Rules |
|---|---|---|
| Numeric sweep | Level II worker; RustFS `level2/sweeps/<station>/<sweep-id>/<revision>/` | Versioned chunked Zarr representation: REF, QC reason bits, per-ray time/azimuth/elevation, per-moment geometry and available RHO/ZDR. Preserve provenance/quality availability; bulk arrays never enter Temporal payloads or PostgreSQL. Write metadata/commit object last, verify checksum and read-back before use. |
| Hot sweep | Worker-local memory or bounded disk, 1 GiB across the pool | Rebuild from verified RustFS artifacts. Include all retained moments/masks/geometry in byte accounting; stop admission before allocation exceeds budget. |
| Rendered tiles | RustFS immutable tile objects; worker hot cache 512 MiB across the pool | Include sampled intermediates and geometry maps in bounded accounting or a separately measured bounded scratch allowance. Coalesce misses; store valid transparent PNGs explicitly. No producer tiles PVC. |
| Index and readiness | PostgreSQL | Station leases, sweep index, viewport revision and exact ready XYZ/palette inventory. API reads metadata and objects, never worker-local paths. |
| Coordination | PostgreSQL leases; optional Redis hot state | Global 8-station/2-decode admission survives extra replicas and rolling overlap. Local semaphores alone cannot enforce pool-wide caps. |

Identity includes station, volume start/key, sweep ordinal and actual start/end time, field,
geometry digest, decoder/QC/propagation/sampler versions and palette version where relevant.
Never group repeated cuts only by angle. Frame timestamp is the sweep end in canonical UTC;
reject/quarantine a same-station timestamp collision with different identity rather than overwrite it.
Two stations with identical timestamps remain separate.

Keep at most six indexed sweeps per active station. Pin objects referenced by ready viewports
until their advertised expiry, then garbage-collect unreferenced objects. Retire views before
removing their sources. Idle at 10 min without tile demand; evict retained station data 30 min
later, subject to existing pins. Proposed viewport lifetime is 60 s with a 120 s object-read grace;
renew only while every referenced object still exists. Cache byte pressure can remove unpinned
hot entries immediately, not promised RustFS objects. Reserve object bytes before rendering;
cap the Level II RustFS namespace at a proposed 10 GiB including staging and pins. Reject new
work on quota exhaustion instead of deleting pinned data. Pilot measures may lower admission,
not silently raise the agreed cache limits. A missing retained frame produces an explicit gap.

## 4. Manifest and readiness contract

Use **station-scoped layers in a separate viewport manifest**, not a global `level2` frame list.
The checked [manifest model](../../../rust/crates/radar-core/src/manifest.rs) makes timestamps
unique within each layer and preserves extra metadata. Name a layer `level2-ref-KGRR` with
`product=level2-ref`, `station_id=KGRR`, `kind=observed`, `units=dBZ` and `delivery=viewport`.
Keep `/api/manifest.json` and its complete-pyramid semantics unchanged. Old clients ignore Level II.
Do not feed station frames into nowcast or forecast merging.

These are proposed endpoints and fields, not routes that exist today:

| Route | Contract |
|---|---|
| `GET /api/level2/sites` | Checked catalogue, enabled regions, per-station health clocks/reasons, `schema_version=1`. Cacheable snapshot; no per-request feed polling. |
| `POST /api/level2/views` | Submit centre and exact visible XYZ set at current integer source zoom (z8–13), plus requested palette. This is tile demand and renews station activity. Validate station candidates server-side. Limit to 64 XYZs, six frames and one palette per view. Return `202 pending` plus opaque view ID/status URL, or explicit fallback reason. No pending frames in a manifest. |
| `GET /api/level2/views/{id}` | Small metadata read: `pending`, `ready`, `fallback` or `expired`, with reason, expiry and polling delay. Only `ready` includes `manifest_url`. Poll at most once per 15 s; bound pending to 30 s, then MRMS. |
| `GET /api/level2/views/{id}/{revision}/manifest.json` | Versioned envelope `schema_version=1`, `manifest` in existing v2 shape, exact `required_tiles`, palettes, source zoom, station, expiry and immutable tile template. Six or fewer complete frames. Missing history is explicit, never duplicated. |
| `GET /level2/tiles/{station}/{sweep}/{revision}/{palette}/{z}/{x}/{y}.png` | Immutable verified PNG. Missing means 404/410, transient failure 503, never fake transparent 200. Only a rendered known-no-data tile is transparent success. |

**Resolve lazy readiness without weakening the SLO:** demand chooses a finite tile set and one
palette; workers render that set for the retained loop, write/verify **every** required tile,
then atomically publish its viewport revision. Required zoom is the current source zoom, not an
unbounded station pyramid. Other zooms, pans or palettes start another pending view; the old
revision is not a promise about them. The z8 set supports the fractional z8→9 fade; source zooms
stop at 13, with truthful overzoom thereafter. No decode or render runs in either manifest handler.

Renderable numeric sweeps are internal readiness, not advertised frames. Do not reuse the
[filesystem tile handler](../../../rust/crates/radar-api/src/basemap.rs)'s missing-as-transparent
rule for Level II. Tests must attempt every advertised URL across every frame, XYZ and palette,
including real transparent tiles, before accepting publication. Object-write, DB-commit and
retention failures leave the previous complete viewport readable within its lease.

Share identical view work across visitors by station/sweep-set/XYZ-set/palette identity.
Use bounded admission, per-visitor/IP quotas and short TTLs for view requests; reject excessive
area, arbitrary station IDs, path traversal and rate-limit evasion. No unlimited public render queue.
Expose these POST/GET/tile paths through the external gateway allowlist in the later GitOps PR.
API failures or changed centre/station/palette/zoom revert uncovered areas to MRMS immediately.
Do not retain old Level II over newly exposed tiles while a new view warms.

Every frame carries these typed, unit-named fields (validate them, not untyped UI guesses):

| Group | Fields / user meaning |
|---|---|
| Acquisition | `station_id`, station name/lat/lon, `source_sites`, `volume_id`, `sweep_id`, `scan_start`, `scan_end`, `timestamp=scan_end`, actual `elevation_deg` and per-ray geometry digest |
| Coverage | `max_ground_range_m=150000`, coverage/missing-sector metadata; clamp per pixel as well as at view centre |
| Height | `antenna_altitude_m_msl` when verified, `beam_model=effective-earth-4/3`, elevation; compute `ground_range_m` and `beam_height_m_above_radar` at the current centre; MSL only if altitude is known |
| Processing | Source bucket/keys/checksums, decoder version, `qc_profile=level2-conservative-v1`, dual-pol availability/reason summaries, geometry/sampler/palette versions, `issued_at` |

For ground distance `s`, elevation `e`, Earth radius `Re=6371000 m`, set `K=4Re/3`,
`r=K sin(s/K)/cos(e+s/K)` and `h=sqrt(r²+K²+2rK sin(e))-K`.
This is the [Py-ART propagation model](https://github.com/ARM-DOE/pyart/blob/main/pyart/core/transforms.py),
not height above terrain. Use actual ray elevations for rendering. Mark centre height as an
estimate; never label range-gate spacing as uniform ground resolution.

## 5. Web and app experience

Put selection, eligibility, age checks, cross-fade, height formatting and loop alignment in
`web/src/shared/level2.ts`; add app re-exports using the existing
[shared storm logic pattern](../../../frontend/src/lib/stormTracks.ts).
Keep native per-frame worklets self-contained; test parity against the shared pure functions.
Both clients ship together in every UI slice, behind the same disabled-by-default region gate.

1. Keep **Observed radar** selected. At z≤8 show MRMS only. Over 8<z<9 use
   `clamp(zoom-8,0,1)` opacity for eligible, ready Level II; use full opacity from z9.
   Keep MRMS at full opacity underneath. Transparent QC holes and pixels beyond 150 km reveal MRMS.
2. Request a bounded view only when close detail could show. Cache only until its explicit expiry;
   refresh health/readiness every 15 s while visible. Cancel obsolete requests and ignore late
   responses using the view identity. Station changes reset the Level II loop and prefetch cache.
3. Keep the last six completed low sweeps in acquisition order, including supplemental cuts.
   Do not run the existing long-loop thinning on these frames. For each Level II scan time,
   select the newest MRMS observation at or before it, no more than 10 min older; otherwise show
   an explicit MRMS gap, not a future observation. On returning to MRMS-only mode restore its
   normal timeline. Never relabel an old loop frame “Now.”
4. Display **Single-site observed · KGRR · scan [local time + zone] · [tilt]° · beam ~[height]
   above radar at centre** beside the web timeline and app Radar legend. Also show ground range;
   offer UTC/full date and source/QC details in the provenance panel. Show the selected frame's
   real scan time, not the newest frame's time. Do not imply surface rain or terrain-relative height.
5. Use the existing MRMS reflectivity class boundaries, palettes and colour-vision modes on both
   products. A palette change creates a new required tile set; keep MRMS visible until ready.
   Test all offered modes, including muted, not just classic. No client-only palette guesses.
6. For warming, saturation, late, out-of-range, store error, expired view or missing tile, show
   **MRMS · Level II unavailable** with a short reason and fade Level II out within one refresh
   (immediately on known expiry/range change). A cached manifest cannot bypass the 600 s clock.
   Do not promise a blank-free map if MRMS itself is unavailable; retain its honest gap/stale state.

## 6. Decode, rendering and deploy shape

Implement a dedicated adapter with pinned decoder crates, networking outside the decoder,
`default-features=false`, and a capped bzip2 reader. Proposed initial hard limits: 32 MiB compressed
per recovery volume, 8 MiB expanded per record, 256 MiB expanded per volume, 2048 radials per sweep,
4096 gates per moment and 32 sweeps per volume. These are admission limits, not verified maxima;
validate a multi-VCP corpus before launch. Exceeding a cap preserves MRMS and records a reason.
Two decode permits include timed-out jobs still running; a Temporal timeout does not free memory.
Preserve continuity, actual end-of-elevation status, missing sectors and split/supplemental cuts.
Never publish a truncated sweep as complete. Archive recovery is bounded and cannot beat live chunks.

Apply conservative QC once before sampling; retain masks and optional dual-pol provenance.
Use nearest gate-footprint ownership for magnification, bounded 2×2 subpixel sampling for
minification and strict missing-support masking. Average linear Z, not colours/dBZ, if aggregating.
Derive sample positions from global XYZ pixels so seams match. Compare against Py-ART/reference
geometry with ≤0.5 output-pixel placement error through z13 on the validation corpus; if the
geodetic approximation misses that budget, improve it before launch. Do not hide errors by smoothing.

The later deployment is one new WorkerDeployment with worker role `level2`, dedicated queue
`radar-ng-level2`, independent version routing, metrics, backlog alerts and graceful shutdown.
Start at one replica; enforce global limits even during rolling overlap. No all-station schedules;
station workflows run only while demand leases are active and continue-as-new to bound history.
Use a separate bounded render executor (initially two jobs) so tile bursts cannot consume decode
permits or starve MRMS. Carry only IDs/object references through Temporal.

The checked [MRMS pool template](../../../deploy/kubernetes/components/worker-controller/worker-pools.yaml)
requests 500m CPU / 1 GiB and limits 6 CPU / 6 GiB. Use that as the initial **resource envelope**
for Level II, not a capacity claim; the proposed 1.5 GiB combined hot-cache cap requires raising
its memory request to **3 GiB** before running the pool. Keep the 6 GiB limit until measured RSS
justifies another reviewed value. Use ephemeral bounded scratch plus RustFS, no tiles/grids/state
RWO mounts or tile-server node affinity. API replicas stay stateless and perform no heavy CPU work.
Credentials arrive through existing secret management, never committed env values.

Prerequisites: verified RustFS read/write and checksum behavior, PostgreSQL metadata access,
private worker dispatch, role-aware image and external-route allowlists. Implement app-repo
configuration/tests first; deploy through a separate later `talos-argocd-proxmox` branch/PR and
Argo CD. Do not change the cluster in this design or the build tickets' product-code PRs.
Require live pollers, queue/version routing, no-RWO mounts and resource headroom before the pilot.

## 7. Gates and ticket-sized build split

Use a branch/PR per slice, not separate horizontal web/app tickets. No slice enables public
close-up before the last gate. Every implementation PR runs its touched-area gates from
[AGENTS.md](../../../AGENTS.md): Rust fmt/clippy/workspace tests and Temporal replay/integration
where relevant; web tests/build; app lint/types/tests; docs links. App acceptance includes native
rebuilds and real iOS/Android exercises, not just JS tests. Record unrun gates honestly.

| Slice | User-visible outcome / server piece | Web + app piece | Blockers and named acceptance gates |
|---|---|---|---|
| A. One-station numeric tracer | Behind disabled gate, demand for KGRR produces one verified lowest-sweep RustFS artifact and health result. Catalogue, leases, bounded chunk/archive adapter and conservative QC work end to end. | None yet; server serves both later. | Design approval; RustFS/DB test services. **Numeric gate:** independent decoder gate/value/geometry parity; real KGRR plus storm/VCP/supplemental/missing-dual-pol fixtures; corrupt/expansion-limit cases; checksum/read-back; no unbounded allocation or incomplete publication. |
| B. One-station tile tracer | A requested tile from that artifact becomes a verified palette PNG without a station pyramid. Add byte caps, single-flight and pool-wide limits. | Shared contract fixtures only; no UI launch. | A. **Tile gate:** geometry ≤0.5 pixel, seams/wrap/gaps, 150 km mask, thin-echo AA reference, hail/weak-weather retention and all palette parity; real CPU/RSS/PNG measurements; timeout/stampede tests. |
| C. Ready viewport loop API | A bounded request returns pending then a ready six-sweep station manifest only after all required tiles exist. Add leases, ETags, expiry, retention and rate limits. | Matching consumer contract fixtures on both clients. | B. **Publication gate:** two sites with same timestamp; repeated low cuts; enumerate every ready tile; interrupted write/DB commit/GC/store delay; palette/zoom/pan changes; no fake transparent misses; API never decodes/renders. |
| D. Close-up experience, both clients in one PR | Viewers get the shared z8→9 fade, accurate provenance, six-sweep loop and MRMS fallback, still region-gated off. | Web map + app Radar tab together; shared functions with app re-exports; cache/cancellation/loop/colour-vision parity. | C. **Parity gate:** browser and native iOS/Android load/scrub/loop/pan/zoom/palette/offline/stale/overflow tests; exact 600 s and 150 km boundaries; MRMS time alignment and holes; no NOAA client requests. |
| E. Isolated pool and failure harness | Operators can run a bounded Level II pool without slowing existing observed radar. Add app-repo role/queue/config/metrics, deployment examples and reproducible load/failure harness. | No new UI; shared fallback fixtures cover failure results. | C. **Operations gate:** global 8 stations / 2 decodes during rollout, idle/eviction, quotas, worker restart, object-store/Temporal failure; no RWO mounts; independent MRMS cadence; required later talos PR + live checks explicitly recorded, not performed by this slice. |
| F. Shadow pilot and regional launch gate | Enable only reviewed regions after 3–5 sites complete 30 days of shadow quality/latency scoring. Include rare-hazard archive cases and meteorological review. | Exercise the same real web/iOS/Android flows; enable both only after approval. | D + E, plus separately merged/deployed talos pool/route/storage PR and verified live pollers. **Launch gate:** QC p95 ≤25 ms/sweep; p95 manifest <500 ms and cached edge tile <800 ms; 100 concurrent users without observed age >10 min or queue start delay >one measured cadence; report cold view readiness/timeouts, gaps, masked valid weather, clutter, source lag, RSS and cost. Require no valid REF lost solely to missing dual-pol, no sentinel conversion, and retained severe cores. Keep MRMS and feature gate on failure; publish evidence and request Mitch's enablement approval. |
