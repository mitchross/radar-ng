# Level II pool runbook

The `radar-ng-level2` pool polls NOAA Level II for stations with live tile demand. It is isolated
from MRMS and every other pool, mounts no volumes and stays behind `LEVEL2_NUMERIC_ENABLED`
(default `false`). Design: [level2-close-up.md](design/level2-close-up.md).

## What runs

| Part | Behavior |
| --- | --- |
| Role and queue | `WORKER_ROLE=level2`, task queue `radar-ng-level2`. No other role registers its workflow or activities. |
| Workflow | `Level2PoolWorkflow`, one per pool, ID `level2-pool`. The worker starts it (reuse if running); no Schedule exists. |
| Loop | A tick (reap dead leases, evict idle stations, list demanded stations), then one poll per demanded station, then a 10 s wait. A failing station backs off 10, 20, 40, 80, 120 s. |
| History | Continues as new after 60 passes or when Temporal suggests it, and moves to the newest deployment version. Only the failure counts roll over; all other state is in Postgres. |
| Limits | 8 active stations and 2 running decodes pool-wide, enforced by Postgres leases, so they hold across replicas and during rollout overlap. |
| Idle and eviction | A tile request holds a station for 10 minutes. Eviction runs 30 minutes after that expires and skips stations with a live pin. A storage failure leaves the data and retries next tick. |
| Retention | Six sweeps per station stay indexed. Older sweeps are deleted unless a view pins them. |
| Dead leases | A decode or render lease older than 15 minutes, or a reservation whose process stopped beating for 2 minutes, is reaped. |

## Configuration

Gate and storage variables are in the [crate README](../../rust/crates/radar-level2/README.md#storage-and-admission).
With the gate off the pool still polls its queue; ticks report disabled and no work runs.
`LEVEL2_METRICS_BIND` (default `0.0.0.0:9466`) serves `/metrics`.

The example envelope in `deploy/kubernetes/components/worker-controller/worker-pools.yaml`:
request 500m CPU and 3 GiB memory, limit 6 CPU and 6 GiB, 4 GiB `emptyDir` scratch, byte-capped
sweep, render and object caches (10 GiB object quota). Measure peak RSS under load before changing
these. A Tokio timeout does not stop blocking decode work; permits are held until that work ends.

## Metrics

Served on port 9466 and scraped by the `level2` endpoint of the `radar-ng-workers` PodMonitor.

| Metric | Meaning |
| --- | --- |
| `radar_level2_active_stations`, `_busy_stations`, `_decodes_running` | Pool occupancy against 8 and 2. |
| `radar_level2_permit_wait_seconds` | Decode permit wait (queue start latency). |
| `radar_level2_poll_seconds`, `_decode_seconds`, `_qc_seconds_total`, `_render_seconds` | Stage timings. |
| `radar_level2_sweep_cache_bytes`, `_render_cache_bytes`, `_object_bytes` | Cache and object bytes in use. |
| `radar_level2_feed_age_seconds`, `_sweep_age_seconds` | Newest chunk age versus newest complete low sweep age, per station. |
| `radar_level2_fallback_total{reason,stage}` | Why a poll or tile fell back to MRMS. |
| `radar_level2_poll_total{status}`, `_poll_skipped_total{reason}` | Poll outcomes. |
| `radar_level2_evict_total{result}`, `_pruned_sweeps_total`, `_reaped_total{kind}` | Housekeeping. |
| `radar_level2_cold_ready_seconds` | Time from first demand to a ready sweep. |

Alerts live in `components/monitoring/prometheusrule.yaml` under `radar-ng.level2`:
feed stale, sweep stale, fallback rate, permit backlog, object quota, pool workflow missing.

## Failure checks

| Symptom | Look at |
| --- | --- |
| Feed age grows, sweep age grows | NOAA lag. Fallbacks show `low_sweep_ineligible`; clients keep MRMS. |
| Feed fine, sweep age grows | `radar_level2_fallback_total` by reason, then the decode timings. |
| Permit wait grows | `radar_level2_decodes_running` stuck at 2: a lease the reaper has not freed yet (15 minutes). |
| Evictions failing | RustFS reachability. Data stays; the claim is released and retried each tick. |
| No pool workflow | Gate off, or the worker cannot reach Temporal. The worker retries the start every minute. |

## Reproduce the harness

```sh
source /opt/data/tools/radar-env.sh
LEVEL2_HARNESS_REQUIRE_PG=1 LEVEL2_TEST_PG=postgres://user@host/db \
  cargo test -p radar-level2 --test pool_harness
```

It covers 8 stations and 2 decodes across replicas and rollout overlap, 10-minute idle,
30-minute eviction with pins, six-sweep retention, reaping, NOAA outage, RustFS outage and decode
saturation. Without `LEVEL2_TEST_PG` it prints `SKIP`; with `LEVEL2_HARNESS_REQUIRE_PG=1` it fails.

## Deployment prerequisites (separate talos PR)

Slice F is blocked until a deployment PR in the cluster repo supplies all of:

1. An immutable worker image digest that includes `WORKER_ROLE=level2`.
2. ExternalSecrets for `LEVEL2_PG_URL`, `LEVEL2_S3_*` and the endpoint and bucket.
3. Public POST, GET and tile routes for the Level II API, allowed at the gateway.
4. Live storage checksum checks against RustFS and Postgres.
5. Pollers, Worker Controller version routing and node resource headroom for 6 CPU / 6 GiB.
6. Proof that no Level II or API replica mounts an RWO volume: `kubectl get pod -o yaml` shows no
   `persistentVolumeClaim` on them.

Do not enable `LEVEL2_NUMERIC_ENABLED` publicly before all six hold.
