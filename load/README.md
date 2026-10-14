# Cluster load test (k6)

`load/k6-radar.js` generates the traffic the app, widget and Watch produce against the tile-server.
`load/k8s/k6-job.yaml` runs it in-cluster as an Indexed Job, so Cloudflare and the home uplink do not
skew results. The pass/fail target for capacity is in
[docs/capacity-acceptance.md](../docs/capacity-acceptance.md); a past run is written up in
[docs/perf/2026-09-26-load-and-bottlenecks.md](../docs/perf/2026-09-26-load-and-bottlenecks.md).

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
