# Premium no-key radar north star

## Product truth

The app has four distinct time horizons and must name them honestly:

| Horizon | Product | Source | User-facing label |
| --- | --- | --- | --- |
| Past to now | measured reflectivity | NOAA MRMS, with selected NEXRAD Level II close-range detail | Observed radar |
| 0–60 minutes, stretch to 90 | motion extrapolation from recent observations | MRMS/NEXRAD + pySTEPS | Radar nowcast |
| 1–48 hours | simulated reflectivity | NOAA HRRR | Model radar guidance |
| 2–10 days | precipitation probability/rate, not reflectivity | RRFS when operational, then GFS/GEFS | Precipitation outlook |

No public API key or paid weather API is required. NOAA, NWS, NHC, and model
object-store feeds are public, but compute, storage, bandwidth, observability,
and operations are still real infrastructure costs.

## SLOs

- Observed MRMS age: p95 at or below 6 minutes; page at 10 minutes.
- Manifest-to-tile consistency: 100%; a frame is never advertised before all
  required palettes and zooms exist.
- Nowcast: every published lead uses the measured source cadence; degraded
  motion never silently becomes a fading stationary image.
- API: 99.9% monthly availability, p95 manifest below 500 ms, p95 cached tile
  below 800 ms at the public edge.
- Capacity: pass the 100-concurrent-user scenario without radar age exceeding
  10 minutes or Temporal schedule-to-start latency exceeding one cadence.
- Recovery: prior complete observations and forecast run remain readable while
  a producer retries or a new run is incomplete.

## Architecture

```text
NOAA/NWS feeds
  -> isolated Temporal queues and worker pools
  -> numeric immutable products (COG/Zarr) in RustFS
  -> on-demand color/tile workers + bounded object cache
  -> CDN/public gateway
  -> stateless manifest/forecast API (3 replicas)
  -> mobile last-known-good manifest + provenance-aware timeline
```

Temporal owns retries, schedules, and coherent publication—not bulk imagery.
Workers publish an immutable run/frame path and commit one small manifest row
only after object verification. PostgreSQL owns manifests, subscriptions, and
push-token metadata; object storage owns radar artifacts; Redis may own only
rebuildable hot cache/rate-limit state. No stateless API replica mounts the
producer's RWO filesystem.

### Workload isolation

- `radar-ng-mrms`: newest observation first, one heavy activity per pod.
- `radar-ng-nowcast`: one CPU-heavy motion run, never shares a slot with ingest.
- `radar-ng-hrrr`: reflectivity-first forecast fanout with bounded concurrency.
- `radar-ng-aux`: lightning, tropical, cleanup, and schedule control.
- `radar-ng-alerts`: watches and alert fanout, sized for at least 250 watches.
- `radar-ng-open-meteo`: model sync binary in its volume-co-located pool.

Each pool gets independent queue-backlog alerts, CPU/memory budgets, graceful
shutdown, and WorkerDeployment version routing. The legacy queue remains only
while pinned histories drain.

## Hyper-local observed radar

1. Build a NEXRAD site catalog and select the nearest healthy radars whose
   coverage contains the viewport. Never describe Level II range-gate spacing
   as uniform ground resolution.
2. Decode base reflectivity plus dual-pol quality fields. Apply clutter/QC,
   beam-height awareness, and range masking before mosaicking.
3. Publish station/run metadata, scan time, elevation angle, range, beam
   height estimate, and source sites with every frame.
4. Cross-fade MRMS national coverage into Level II only at zooms where Level II
   has real additional information. Fall back to MRMS when a station is late.
5. Add storm-relative velocity only as a separately labeled expert product;
   never color it as reflectivity.

This is the path to street-scale visual detail. It does not make the atmosphere
predictable at street scale days in advance.

## Forecast roadmap

### Now through 60 minutes

- Use 4–6 cadence-validated observed frames.
- Run optical flow and deterministic/probabilistic pySTEPS at explicit
  fractional input timesteps.
- Verify displacement, mass growth/decay, edge effects, and stale-input gates.
- Add ensemble perturbations and calibrated probability bands only after
  backtesting; freshness alone is not confidence.

### 60 minutes through 48 hours

- Publish one immutable complete HRRR run at a time.
- Render simulated reflectivity first; unrelated layers have separate budgets.
- Blend nowcast toward model guidance over an overlap window only after a
  repeatable verification score beats either input alone.

### Beyond 48 hours

- Feature-gate RRFS until the official operational feed is available and its
  latency/completeness is verified in production-like shadow ingest.
- Use deterministic/ensemble precipitation guidance through day 10.
- Show probability, accumulation, and timing windows. Never draw synthetic
  reflectivity loops and call them future radar days out.

## Delivery and scale milestones

1. **Correctness baseline (implemented in this branch):** immutable tile
   staging, generation-pointed grids, manifest v2 frame metadata, fail-closed
   nowcast, measured-cadence timesteps, coherent HRRR publication, honest zoom
   ceilings, bounded API caches, workflow-route auth contract, opt-in telemetry,
   provenance-correct mobile copy, and a 100-user k6 acceptance test.
2. **Queue rollout:** publish the role-aware worker image, create isolated
   WorkerDeployments, verify pollers, switch schedules, then drain legacy.
3. **Object delivery:** dual-write numeric products and cached tiles to RustFS;
   verify byte/checksum parity; switch reads; remove tile/grid mounts from API;
   raise API replicas to three and restore HPA to 3–12.
4. **Hyper-local pilot:** shadow-ingest 3–5 NEXRAD sites around target metros,
   run 30 days of quality/latency scoring, then enable close zooms by region.
5. **Verified forecast:** retain forecast/observation pairs, compute lead-time
   skill by season/intensity, introduce blend and probabilities only where
   validation supports them.
6. **250-user headroom:** test 100, 250, and failure modes (NOAA delay, worker
   restart, object-store latency, Temporal failover), then publish the measured
   capacity envelope and cost per active user.

## Explicit gates

- Do not claim multi-node scale while the API mounts Longhorn RWO artifacts.
- Do not enable push/watch endpoints without an identity issuer, per-user
  authorization, encrypted token storage, quotas, and deletion/audit flows.
- Do not increase Temporal history shards in place; create and migrate to a
  correctly sized cluster if shard capacity becomes the limiter.
- Do not ship RRFS, NEXRAD mosaics, probabilistic confidence, or model blending
  based only on code completion. Each requires feed readiness and measured
  verification data.
