# Cluster load test (k6)

`load/k6-radar.js` generates the traffic the app, widget and Watch produce against the tile-server.
`load/k8s/k6-job.yaml` runs it in-cluster as an Indexed Job, so Cloudflare and the home uplink do not
skew results. The pass/fail target for capacity is in
[docs/reference/capacity-acceptance.md](../docs/reference/capacity-acceptance.md); a past run is written up in
[docs/archive/perf/2026-09-26-load-and-bottlenecks.md](../docs/archive/perf/2026-09-26-load-and-bottlenecks.md).

Level II close-up has its own script, Job and failure drills: [Level II close-up](#level-ii-close-up-k6-level2js).

## What the script drives

- 20 % of sessions are widget refreshes: `GET /api/manifest.json`, then one z7 tile.
- 80 % are app sessions: manifest, forecast, alerts, nowcast, geocode (10 % of sessions), then a
  playback of the last six radar frames as 3 x 3 tiles at z6, 500 ms apart.
- Tiles come from `/tiles/radar/classic/...` on the api's static lane, which is not rate limited.
  About 90 % of requests are tiles. `/api/*` is rate limited per client IP (`API_RATE_LIMIT_RPS` 20,
  burst 60), so each k6 pod must stay below that.
- Thresholds: `http_req_failed < 1 %`, `server_errors < 0.5 %`, `rate_limited < 1 %`, manifest
  p95 < 300 ms, tile p95 < 300 ms, forecast/nowcast p95 < 1000 ms, alerts p95 < 1500 ms,
  playback frame p95 < 500 ms.

## Parameters

| Parameter | Default | Notes |
|---|---|---|
| `BASE_URL` | the in-cluster `tile-server` Service in the Job; the public host for laptop runs | |
| `completions` = `parallelism` | 10 | 10 pods = 10 client IPs |
| `VUS` | 10 per pod | stays below the per-IP rate limit |
| `PROFILE` | unset (steady) | `stress` steps up to find the knee |
| Duration | steady: 1 min ramp, 5 min at VUS, 1 min step, 3 min at 2 x VUS, 1 min down | |
| Image | `grafana/k6:2.3.0` pinned by digest | |

To compare two builds, run the same parameters back to back in the same weekday hours; the cluster
and the uplink are shared.

## Running it in the cluster (GitOps only)

The cluster changes only through `talos-argocd-proxmox`, so the Job is deployed as a temporary GitOps
change, never with `kubectl apply`:

1. In a PR, add `load/k8s/k6-job.yaml` under the radar-ng app (or its own small app directory) with a
   Kustomize `configMapGenerator` named `k6-script` built from `k6-radar.js`, and list both in the
   `kustomization.yaml`. Set `BASE_URL` in the Job env if it is not the in-cluster Service.
2. Merge; Argo CD syncs the Job. Read the per-pod JSON summaries from the pod logs in the Argo UI or
   Loki.
3. Remove the Job in a follow-up PR.

Stress profile: set `completions`, `parallelism` and `maxFailedIndexes` to 60, `VUS` to 20, and add
`PROFILE=stress` (four 3-minute steps up to 1,200 users).

## Running it from a laptop

The rate limit applies per laptop IP, so keep `VUS` at 10 or fewer:

```
BASE_URL=https://radar-ng-api.example.com VUS=10 k6 run load/k6-radar.js
BASE_URL=https://radar-ng-api.example.com VUS=10 PROFILE=stress k6 run load/k6-radar.js
```

## What to look at

- `http_req_duration{name:tile}` p50/p95/p99 and the `http_reqs` rate: the static tile path.
- `http_req_duration{name:manifest|forecast|alerts|nowcast}` and `server_errors`: the API path.
- `rate_limited` must be close to zero; if it is not, the run measured the limiter.
- Server side, for the same window: tile-server CPU and RSS, MRMS age (`radar_ng_mrms_age_seconds`),
  and Temporal backlog. A passing API run is invalid if publication fell behind.

## Level II close-up (`k6-level2.js`)

`load/k6-level2.js` is the #293 load test. Each user does what the web and app close-up does
(`Level2Session` in `web/src/shared/level2.ts`): read `/api/level2/sites`, submit the exact visible
tile set to `POST /api/level2/views`, poll the status while it warms (at most 30 s), read the
viewport manifest, play its sweeps (up to six) at 2 frames a second, then re-submit the same view every 15 s.
A user stays on one view for `SESSION_TICKS` refreshes (8, two minutes), then pans to a new one.
On a fallback or a pending timeout it reads the MRMS manifest and one MRMS tile instead, as the map
does.

Run it against the shadow deployment only: the in-cluster `level2-api` Service, then the shadow
host for the edge clock. On the public hosts Level II is off and `setup()` stops the run on the
404.

| Parameter | Default | Notes |
|---|---|---|
| `BASE_URL` | `http://level2-api.radar-ng.svc.cluster.local:8080` | the Level II API; the shadow host for the edge run |
| `MRMS_URL` | `http://tile-server.radar-ng.svc.cluster.local:8080` | where MRMS is served; the shadow host when it fronts both |
| `VUS` | 5 per pod | a user re-submits every 15 s and `POST /api/level2/views` allows 0.5 rps per IP, so 7 is the ceiling |
| pods | 20 (`k8s/k6-level2-job.yaml`) | 20 x 5 = 100 users |
| `PROFILE` | `steady` | `steady`: 1 min ramp, 10 min at `VUS`, 1 min step, 5 min at 2 x `VUS`. `overflow`: users spread over `OVERFLOW_STATIONS` (12) distinct stations, enabled first, then the nearest catalogue sites. `smoke`: 1 user, 30 s |
| `SESSION_TICKS` | 8 | refreshes per view |
| `REFRESH_S` | 15 | the client refresh; lower it only for dry runs against a stub |

Thresholds, from #293 and the design:

| Metric | Rule |
|---|---|
| `http_req_duration{name:l2_manifest}` | p95 < 500 ms |
| `http_req_duration{name:l2_tile}` | p95 < 800 ms (a user fetches each frame once, like a browser cache) |
| `http_req_duration{name:l2_submit}`, `{name:l2_status}`, `{name:l2_sites}` | p95 < 500, 300, 300 ms |
| `level2_observed_age_s` | max <= 600 s: the newest frame of every ready manifest a user reads or holds |
| `mrms_observed_age_s` | max <= 600 s while users are on MRMS |
| `level2_missing_advertised_tile` | 0: a ready manifest promised that tile |
| `http_req_failed`, `server_errors`, `rate_limited` | < 1 %, < 0.5 %, < 1 % |

Reported, not pass rules: `level2_cold_ready_ms` (submit to ready, the cold view),
`level2_pending_timeout`, `level2_fallback` and `level2_fallback_reasons{reason}`,
`level2_view_submits_by_station{station}`, `level2_playback_frame_ms`.

The overflow profile can only exceed the pool's 8-station cap if the shadow API enables more
than 8 stations. With 3–5 pilot sites the extra centres get `out_of_range` (or the nearest
enabled station). That run still proves that fallbacks are fast and that no queue grows; the
8-station and 2-decode caps themselves are proven by the pool harness
([../docs/develop/level2-pool.md](../docs/develop/level2-pool.md#reproduce-the-harness)).
Record which case the run covered.

In the cluster, ship `k8s/k6-level2-job.yaml` in a temporary talos PR, as for the MRMS test, with a
`configMapGenerator` named `k6-level2-script` built from `k6-level2.js`. Run steady against the
Service, steady against the shadow host (`BASE_URL` and `MRMS_URL` both set to it), then overflow.
Save each pod's summary line and score the run:

```sh
python3 load/drills/gate.py --prom <prometheus> --mrms <tile-server> \
  --start <UTC> --end <UTC> --level2-cadence-s <recorder median> --k6 k6-level2-*.json
```

`gate.py` also checks MRMS age and Temporal schedule-to-start for the same window. If
`rate_limited` is not near zero on the shadow-host run, the edge is folding the pods into one
client IP; the run measured the limiter, so do not record it as a capacity result.

Failure drills, run alongside the steady profile: [drills/README.md](drills/README.md).

### Dry run without a cluster

`k6 inspect load/k6-level2.js` checks syntax and options. For the request flow, run it against
the local stand-in in `dev/level2-stub.py` (fresh frames; `STALE_S=900` makes them 15 minutes old,
and the age threshold must then fail with k6 exit 99):

```sh
python3 load/dev/level2-stub.py 38611 &
k6 run -e BASE_URL=http://127.0.0.1:38611 -e MRMS_URL=http://127.0.0.1:38611 \
  -e PROFILE=smoke -e SESSION_TICKS=2 -e REFRESH_S=3 load/k6-level2.js
```

A stub tells you the script works, not how the service performs.
