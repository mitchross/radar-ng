# Level II failure drills

Five drills for the #293 gate: "Verify fallback under NOAA delay, worker restart, store
latency/outage and Temporal failover". Run them against the shadow deployment only, in the first
week after it is healthy, with the steady k6 run from [../README.md](../README.md#level-ii-close-up-k6-level2js)
going at the same time. Write each result into the
[launch-gate report](../../docs/reference/level2-launch-gate.md#4-failure-drills).

Every cluster change goes through a talos-argocd-proxmox PR that Mitch merges. Agents may delete
the Level II shadow pods. Mitch runs the Temporal restart, because it touches every app on the
cluster.

| Drill | Who runs it | Cluster change |
|---|---|---|
| [1. NOAA delay](#1-noaa-delay) | Agents watch; Mitch merges two small talos PRs | `LEVEL2_TEST_FEED_DELAY_S` on the Level II pool, then removed |
| [2. Worker restart](#2-worker-restart) | Agents | none: `kubectl delete pod` on a Level II shadow pod |
| [3. Store latency](#3-store-latency) | Agents watch and set the delay; Mitch merges the proxy PR | a temporary TCP proxy in front of RustFS |
| [4. Store outage](#4-store-outage) | Agents, on the same proxy | none beyond drill 3 |
| [5. Temporal failover](#5-temporal-failover) | Mitch | `kubectl rollout restart` of the Temporal services |

## Pass rules (all drills)

- Observed age never over 10 minutes. No ready Level II manifest has a newest frame older than
  600 s (k6 `level2_observed_age_s` max), and MRMS age stays at or below 600 s
  (`radar_ng_mrms_age_seconds`).
- Temporal schedule-to-start never exceeds one measured source cadence, on any radar-ng queue.
- While Level II is impaired, every client gets an explicit fallback (`state: fallback` with a
  reason, a 503, or a 404/410 tile), never a ready manifest it cannot load and never a blank 200
  tile. Clients then show **MRMS · Level II unavailable**.
- MRMS is not slowed: its age and its Temporal queue are the same as before the drill.
- After the fault is removed, Level II is ready again within one low-sweep cadence plus the 30 s
  pending limit.

`gate.py` checks the first two rules and the k6 thresholds over the drill window. The rest come
from the `observe.py` timeline.

## Tools

Run from the agent pod. Both scripts are stdlib Python.

```sh
PROM=http://kube-prometheus-stack-prometheus.prometheus-stack.svc.cluster.local:9090
SHADOW=https://<shadow host>     # D1; until it exists use the in-cluster level2-api Service
MRMS=http://tile-server.radar-ng.svc.cluster.local:8080

# Timeline: one row every 15 s with sites, a view submit, MRMS age and the pool metrics.
python3 load/drills/observe.py --base "$SHADOW" --mrms "$MRMS" --prom "$PROM" --out drill-<name>.csv

# Pass rules over the window, after recovery.
python3 load/drills/gate.py --prom "$PROM" --mrms "$MRMS" --start <UTC> --end <UTC> \
  --level2-cadence-s <median low-sweep gap> --k6 k6-<run>-*.json
```

`gate.py` exits 0 on pass, 1 on fail, 2 when the Temporal histogram cannot decide. The SDK
histogram is in milliseconds with buckets at 0.1, 0.5, 1, 5, 10, 100 and 1000 s, so for a 120 s
cadence the check is "no task over 100 s". A task between 100 s and 1000 s needs its own workflow
history (Temporal UI, the task's scheduled and started events) before the drill passes.

The Level II cadence is the median gap between low sweeps per station from the shadow recorder.
<!-- TODO(C3): confirm the table, schema and report command against docs/develop/level2-shadow.md
once C3 lands. The query below follows the C3 work in progress (`sweep_clocks`). -->

```sql
SELECT station, percentile_cont(0.5) WITHIN GROUP (ORDER BY gap_s) AS median_gap_s
FROM (
  SELECT station, EXTRACT(EPOCH FROM scan_end - lag(scan_end) OVER (PARTITION BY station ORDER BY scan_end)) AS gap_s
  FROM level2_numeric.sweep_clocks
  WHERE scan_end BETWEEN '<start>' AND '<end>'
) g WHERE gap_s IS NOT NULL GROUP BY station;
```

Use the smallest station median. The MRMS cadence comes from the manifest (`gate.py` measures it;
about 120 s on 2026-11-16).

For each drill record: start and end UTC, who ran it, the exact change or command, the
`observe.py` CSV, the `gate.py` output and exit code, and the k6 summaries.

## 1. NOAA delay

**Who:** an agent writes both talos PRs and watches; Mitch merges them.

The feed URLs are constants, so the delay comes from a test-only switch on the Level II pool:
`LEVEL2_TEST_FEED_DELAY_S` hides NOAA objects newer than that many seconds (0 or unset is off,
maximum 3600). The pool exports `radar_level2_test_feed_delay_seconds` so the timeline shows when
it took effect. <!-- TODO(C3): confirm the variable and gauge names in docs/develop/level2-shadow.md. -->

Steps:

1. Start `observe.py` and the steady k6 run. Wait 10 minutes for a baseline.
2. talos PR: set `LEVEL2_TEST_FEED_DELAY_S=900` on the shadow Level II pool only. Mitch merges;
   wait for the new pod to be Ready and for `radar_level2_test_feed_delay_seconds` to read 900.
3. Hold for 20 minutes, two full fallback windows.
4. talos PR: remove the variable (or set `0`). Mitch merges. Hold until every pilot station is
   ready again, then 10 more minutes.

Expected:

- `radar_level2_feed_age_seconds` and `radar_level2_sweep_age_seconds` climb past 900 s.
- Once the newest complete low sweep is older than 600 s, `/api/level2/sites` reports the
  station `late`, `POST /api/level2/views` returns `state: fallback`, `reason: late`, and ready
  views expire. Clients drop to MRMS.
- `RadarNgLevel2FeedStale` fires after its 10-minute `for`.
- MRMS age and the `radar-ng-mrms` queue are unchanged.

Proving queries:

```promql
max(radar_level2_test_feed_delay_seconds)
max by (station) (radar_level2_sweep_age_seconds)
sum by (reason, stage) (increase(radar_level2_fallback_total[5m]))
max_over_time(radar_ng_mrms_age_seconds[30m])
```

Fail if a ready manifest is served whose newest frame is older than 600 s, or if the pool keeps
reporting `healthy` for a station whose low sweep is over 600 s old.

## 2. Worker restart

**Who:** agents.

Steps:

1. Baseline 10 minutes with `observe.py` and k6.
2. Delete the Level II pool pod: `kubectl -n radar-ng delete pod -l <level2 pool label>`
   (<!-- TODO(C3): exact label from the C3 deploy examples -->the WorkerDeployment's current
   version). Note the UTC time.
3. Hold 15 minutes. Then delete the `level2-api` pod the same way and hold 10 minutes.

Expected:

- Pool pod: SIGTERM drains; the new pod polls `radar-ng-level2` again and the worker restarts
  `Level2PoolWorkflow` (ID `level2-pool`, reuse if running). Ready views stay readable through their
  lease because the API serves them from Postgres and RustFS, not from the pool. New demand may sit
  `pending`; past 30 s the client gives up with `pending_timeout` and shows MRMS. Stale decode
  reservations are reaped within 2 minutes.
- API pod: the shadow host returns 5xx while it restarts; clients treat that as unreachable and
  show MRMS. MRMS on the tile-server is not touched.
- No MRMS change.

Proving queries:

```promql
temporal_worker_task_slots_available{task_queue="radar-ng-level2"}
increase(radar_level2_reaped_total[15m])
sum by (le) (increase(temporal_activity_schedule_to_start_latency_bucket{task_queue="radar-ng-level2"}[30m]))
increase(kube_pod_container_status_restarts_total{namespace="radar-ng"}[30m])
```

Fail if the queue has no poller for longer than one cadence, if a reservation stays held after
15 minutes, or if a task waited longer than one cadence to start.

## 3. Store latency

**Who:** an agent writes the proxy PR and sets the delay; Mitch merges the PR.

RustFS is outside the cluster (`LEVEL2_S3_ENDPOINT`), and there is no chaos tooling in the
cluster. The drill uses a temporary TCP proxy with adjustable latency (for example Toxiproxy) as a
Deployment and Service in `radar-ng`. The talos PR adds the proxy and points the shadow pool's and
`level2-api`'s `LEVEL2_S3_ENDPOINT` at it, with no latency configured. Agents change the latency
through the proxy's own HTTP API from inside the cluster; that changes no Kubernetes object. A
follow-up PR restores the endpoint and removes the proxy after drill 4.

Steps:

1. Merge the proxy PR, check that the shadow host still serves ready views, then baseline
   10 minutes.
2. Add 500 ms latency to every request for 10 minutes, then 5 s for 10 minutes.
3. Remove the latency; hold 10 minutes.

Expected:

- At 500 ms: tile p95 rises, views still turn ready, no fallback storm.
- At 5 s: renders and object reads slow; views that cannot finish inside 30 s fall back with
  `pending_timeout`; tile reads that fail return 503 with `Retry-After`, never a transparent 200.
  `radar_level2_decodes_running` stays at or below 2 and permit wait stays bounded; work does
  not queue without limit.
- MRMS unaffected (it does not use RustFS).

Proving queries:

```promql
histogram_quantile(0.95, sum by (le) (rate(radar_level2_render_seconds_bucket[5m])))
max(radar_level2_decodes_running)
histogram_quantile(0.95, sum by (le) (rate(radar_level2_permit_wait_seconds_bucket[5m])))
sum by (reason, stage) (increase(radar_level2_fallback_total[5m]))
```

k6 during this drill: `level2_missing_advertised_tile` must stay 0 (503 is allowed, 404 on a
promised tile is not).

## 4. Store outage

**Who:** agents, on the drill 3 proxy.

Steps:

1. Baseline 5 minutes after drill 3 has recovered.
2. Disable the proxy (all connections refused) for 15 minutes.
3. Enable it; hold until every pilot station is ready again, then 10 more minutes.
4. Open the follow-up talos PR that removes the proxy and restores the endpoint.

Expected:

- Object writes fail, so no new sweep is published; the previous index stays. Evictions fail and
  retry next tick without deleting data.
- Tile reads return 503; new and renewed views fall back with a reason; clients show MRMS.
- After recovery the pool publishes the next low sweep and views turn ready again, with no manual
  cleanup.
- MRMS unaffected.

Proving queries:

```promql
sum by (result) (increase(radar_level2_evict_total[15m]))
sum by (reason, stage) (increase(radar_level2_fallback_total[5m]))
max(radar_level2_object_bytes)
```

Fail on any 200 tile while the store is down that is not a byte-identical copy of a verified tile,
or if recovery needs a manual step.

The Level II Postgres has the same failure mode for the index and leases. Its outage drill
(deleting the CNPG primary) touches a database pod and is Mitch's call; list it as not run if he
skips it.

## 5. Temporal failover

**Who:** Mitch. It restarts the Temporal services every app on the cluster uses.

Steps:

1. Agents start `observe.py` and k6 and confirm a 10-minute baseline.
2. Mitch: `kubectl -n temporal rollout restart deploy/temporal-frontend deploy/temporal-history deploy/temporal-matching`
   and notes the UTC time. Temporal runs one replica of each, so this is a full restart, not a
   leader handover.
3. Hold until all radar-ng queues have pollers again, then 15 more minutes.

Expected:

- During the restart workers lose their polls and retry; MRMS schedules buffer and catch up.
- `level2-api` keeps serving ready views (it does not call Temporal). New sweeps stop until the
  pool workflow resumes, so if the restart takes longer than about 10 minutes, stations go `late`
  and clients show MRMS. That is the right fallback, not a failure.
- After recovery `Level2PoolWorkflow` continues and the pool picks up demand again.

Proving queries:

```promql
temporal_worker_task_slots_available{task_queue=~"radar-ng.*"}
sum by (task_queue, le) (increase(temporal_workflow_task_schedule_to_start_latency_bucket{task_queue=~"radar-ng.*"}[30m]))
increase(temporal_workflow_completed{workflow_type="IngestMrmsWorkflow"}[30m])
max_over_time(radar_ng_mrms_age_seconds[30m])
```

This is the drill most likely to break the schedule-to-start rule: a task scheduled just before
the restart waits for it to finish. If the restart takes longer than one cadence (about 2 minutes
for MRMS), the rule fails and the report says so; it is not waived.
