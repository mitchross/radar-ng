# radar-ng Kubernetes manifests (Rust build)

Manifests for the production topology, rebuilt on the Rust image. File names mirror the Python
`deploy/k8s/` tree, so the diff against it is the image and the args. Namespace `radar-ng`
throughout. The cutover procedure is in [../RUNBOOK.md](../RUNBOOK.md). Read its section 2 (open
items) before applying anything.

## Layout

| File | Kind | Notes |
|---|---|---|
| `configmap-temporal-config.yaml` | ConfigMap | Shared tunables (worker and api). Same name as Python |
| `temporal-connection.yaml` | TemporalConnection | `cluster-temporal`, frontend `temporal-frontend.temporal.svc.cluster.local:7233` |
| `temporal-worker-deployment.yaml` | TemporalWorkerDeployment | `radar-ng-worker`, role `legacy`, queue `radar-ng`. Same TWD as Python, Rust image |
| `open-meteo-worker-deployment.yaml` | TemporalWorkerDeployment | `radar-ng-open-meteo-worker`, role `open-meteo`, uid 999. Same TWD as Python, Rust image |
| `tile-server-deployment.yaml` | Deployment | The api. Single container, one replica, read-only PVCs, probes on `/api/livez` |
| `tile-server-service.yaml` | Service | ClusterIP `tile-server:8080`, same name as before |
| `basemap-bootstrap-job.yaml` | Job | One-shot basemap PMTiles extract + atomic rename onto the `basemap` PVC. Rerun to refresh |
| `secret-temporal-worker.yaml.template` | Secret template | Push secret. Apply out of band. Never commit real values |

Apply order: connection, configmap, then the TWDs and the api. The Temporal Worker Controller must
already be running. It injects `TEMPORAL_DEPLOYMENT_NAME` and `TEMPORAL_WORKER_BUILD_ID`.

## Placeholders to replace

- `image: registry.vanillax.me/radar-ng-temporal-worker:REPLACE_ME` (worker TWD): the Rust build tag.
- `image: registry.vanillax.me/radar-ng-open-meteo-worker:REPLACE_ME` (open-meteo TWD): the Rust build tag.
- `image: registry.vanillax.me/radar-ng-tile-server:REPLACE_ME` (tile-server): the Rust build tag for the api.
- `OPEN_METEO_BASE` in `tile-server-deployment.yaml` (RUNBOOK open item 1).
- `secret-temporal-worker.yaml.template`: APNS and FCM values, and the key files.

## Rollout

Both TWDs ship with the first-cutover ramp: 5% for 30 min, 25% for 1 h, 50% for 2 h. The Python
steady-state values are in a comment in each file. Restore them after the cutover (RUNBOOK
section 5.5). To hold at a step, switch `strategy` to `Manual`.

## Probes and shutdown

- **Worker:** liveness is `busybox test -f /tmp/worker-alive`. The worker loop touches that file
  every 30 s (`radar-worker` `health.rs`). `/tmp/temporal-healthy` is written only after a
  successful Temporal health check, so it is a readiness signal. It is not used as a probe yet.
  `TEMPORAL_GRACEFUL_SHUTDOWN_S` is 25 and `terminationGracePeriodSeconds` is 35. No preStop hook,
  because workers have no endpoints.
- **open-meteo worker:** liveness is `busybox test -f /tmp/worker-alive`, with `initialDelaySeconds`
  120, `periodSeconds` 60, and `failureThreshold` 5 (F2). busybox is copied into the image.
- **api:** readiness and liveness on `/api/livez`. `/api/health` is not a probe, because it returns
  503 when radar data is stale. `preStop` sleeps 5 s so endpoints drop first.
  `terminationGracePeriodSeconds: 40`.

## Storage

- `tiles`, `grids`, `state`: written by the worker, read by the api (read-only mount).
- `basemap`: read-only in the api. It must be filled before the first api rollout by a one-shot
  Job equivalent to compose's `basemap-bootstrap` and `basemap-publish` (RUNBOOK open item 4). Write
  `basemap.pmtiles.partial`, then rename it. The api reopens the archive after a rename. No Job is
  included yet.
- `state` is read-only in the api. `/v1` push-token registration returns 503 until a writable state
  mount is added (F9).
- `openmeteo-data`: the open-meteo worker (read-write). The open-meteo API that reads it has no
  manifest yet (RUNBOOK open item 1).
- During a ramp, old and new pods mount the same PVCs. Confirm RWX, or same-node placement, before
  step 5.2 (RUNBOOK open item 2).

## Pod security

All pods run as non-root (1000 for radar, 999 for open-meteo) with `fsGroup` set,
`seccompProfile: RuntimeDefault`, `allowPrivilegeEscalation: false`, and all capabilities
dropped. Images are distroless (radar) or the upstream Open-Meteo image. `readOnlyRootFilesystem`
is not set (RUNBOOK section 12).
