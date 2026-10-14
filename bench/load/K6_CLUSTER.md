# Running load/k6-radar.js before and after the Rust port (cluster)

The k6 script and its Job (`load/k8s/k6-job.yaml` in the radar-ng repository) are unchanged.
This page only fixes the parameters so the before and after runs are comparable.

## What the script drives

- 20 % of sessions are widget refreshes: `GET /api/manifest.json`, then one z7 tile.
- 80 % are app sessions: manifest, forecast, alerts, nowcast, geocode (10 % of sessions), then a
  playback of the last six radar frames as 3 x 3 tiles at z6, 500 ms apart.
- Tiles are served from `/tiles/radar/classic/...`, by Caddy in production. About 90 % of
  requests are tiles. The API is rate limited per client IP (`API_RATE_LIMIT_RPS` 20,
  burst 60, `/api/*` only), so each pod must stay below that.
- Thresholds: `http_req_failed < 1 %`, `server_errors < 0.5 %`, `rate_limited < 1 %`, manifest
  p95 < 300 ms, tile p95 < 300 ms, forecast/nowcast p95 < 1000 ms, alerts p95 < 1500 ms,
  playback frame p95 < 500 ms.

## Parameters to keep fixed

| Parameter | Value | Why |
|---|---|---|
| `BASE_URL` | the public API host (through Cloudflare, as in production) or the in-cluster service | same path for both runs |
| `completions` = `parallelism` | 10 (the default Job) | 10 client IPs |
| `VUS` | 10 per pod (default) | stays below the per-IP rate limit |
| `PROFILE` | unset for the steady profile; `stress` to find the knee | |
| Duration | default profile: 1 min ramp, 5 min at VUS, 1 min step, 3 min at 2 x VUS, 1 min down | same for both |
| Image | `grafana/k6:2.3.0` pinned by digest | same client |
| Time window | run before and after back to back, same weekday hours | the cluster and the uplink are shared |

Run the pair as two Jobs with different names (`k6-radar-before`, `k6-radar-after`) pointing
at the two deployments, or one after the other on the same deployment with the other stack
scaled to zero. Keep the pod count and VUS the same for both.

## Commands

```
# steady profile, default parameters (10 pods x 10 VUs)
kubectl -n radar apply -f load/k8s/k6-job.yaml          # BASE_URL in the Job env
kubectl -n radar logs job/k6-radar --all-containers    # per-pod JSON summaries

# stress profile: four steps up to 20 VUs per pod (60 pods x 20 VUs = 1,200 users)
# edit completions/parallelism/maxFailedIndexes to 60, VUS to 20, add PROFILE=stress
```

Local run from a laptop (the rate limit applies per laptop IP, so keep `VUS` at 10 or fewer):

```
BASE_URL=https://radar-ng-api.example.com VUS=10 k6 run load/k6-radar.js
BASE_URL=https://radar-ng-api.example.com VUS=10 PROFILE=stress k6 run load/k6-radar.js
```

## What to compare

- `http_req_duration{name:tile}` p50/p95/p99 and `http_reqs` rate: the tile path is Caddy in
  both stacks, so a difference here is the origin behind Caddy, not the tile server. Compare
  the `/tiles` numbers only when both stacks sit behind the same Caddy.
- `http_req_duration{name:manifest|forecast|alerts|nowcast}` and `server_errors`: the API path
  that changes with the Rust port.
- `rate_limited` must be close to zero in both runs; if it is not, the run measured the limiter.
- The server side: CPU and RSS of the API pods (`kubectl top pod`) during the run, for the
  same time window.

## Notes for the prepared local run (`bench/load/run_load.sh`)

- Set `RATE_LIMIT_OFF=1` locally, or the 20 rps per-IP limit makes most `/api` requests return
  429 at 200 rps and above. Production keeps the limit; the cluster run above keeps it.
- The request mix there is built from the served manifest by `mix.py`, with the same weights.
- The Python server is `py_server.py` (FastAPI plus StaticFiles at `/tiles`). Caddy is not in
  that path, so tile numbers from it are a lower bound for the production Caddy setup.
