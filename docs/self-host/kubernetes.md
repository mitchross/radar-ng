# Running radar-ng on Kubernetes

The advanced path. Compose ([self-hosting.md](compose.md)) gives you everything on one box; Kubernetes adds controlled rollouts, workload isolation, disruption policy, and Prometheus integration. HA tile-serving requires an object/RWX data plane—it does not come from Kubernetes while producer and API pods share ReadWriteOnce volumes. This doc is **bring-your-own-cluster**; nothing here requires the author's Talos/Argo CD setup.

## What you need first

| prerequisite | notes |
|---|---|
| Kubernetes 1.28+ | author runs 1.35; nothing exotic is used |
| A **Temporal server** | self-host [temporal](https://github.com/temporalio/temporal) in-cluster or use Temporal Cloud. Workers and the api need `TEMPORAL_ADDRESS` + `TEMPORAL_NAMESPACE` |
| Storage class | see the storage section below |
| Images | the three `ghcr.io/mitchross/radar-ng-*` images (public), or build them from `deploy/Dockerfile` |
| (optional) Temporal Worker Controller | versioned worker rollouts gated by `RadarDeploymentSmokeWorkflow`; plain Deployments also work |
| (optional) Prometheus operator | for `ServiceMonitor`/`PodMonitor` and the Grafana dashboard |

## Reference manifests

Start with **[`deploy/kubernetes/`](../../deploy/kubernetes/README.md)** in this repo: a Kustomize
base (tile-server, one `legacy` worker, open-meteo with its sync sidecar, basemap, Services, PVCs
with no storage class, settings) plus optional components (`temporal-dev`, `ingress`,
`gateway-api`, `monitoring`, `worker-controller`, `autoscaling`). To try it on kind, k3d or k3s:

```bash
kubectl apply -k deploy/kubernetes/examples/minimal
```

`deploy/kubernetes/examples/production` is the production layout (role pools under the Worker
Controller, Gateway API, monitoring, autoscaling) with the owner's hostnames, secrets and storage
classes removed. Its README covers settings, Temporal options, storage and publishing the api.

The owner's live manifests are a production example built on the same layout, in
[talos-argocd-proxmox `my-apps/personal-projects/radar-ng/`](https://github.com/mitchross/talos-argocd-proxmox/tree/main/my-apps/personal-projects/radar-ng).
They add cluster-specific pieces (ExternalSecret, PriorityClass, named storage classes, the web
frontend):

| file | purpose |
|---|---|
| `temporal-workers/worker-pools.yaml` | five `WorkerDeployment`s (`mrms`, `nowcast`, `hrrr`, `aux`, `alerts`) running `radar-ng-temporal-worker`, each with a smoke-workflow rollout gate |
| `temporal-workers/configmap-temporal-config.yaml` | worker tunables (palettes, cadences, NWS URLs — see [configuration.md](configuration.md)) |
| `temporal-workers/temporal-connection.yaml` | `TemporalConnection` for the Worker Controller |
| `deployment-tile-server.yaml` | `radar api`, plus `hpa-tile-server.yaml` and `pdb-tile-server.yaml` |
| `deployment-open-meteo.yaml` | Open-Meteo serve container with the `sync-worker` sidecar (`radar worker --role open-meteo`) |
| `deployment-basemap.yaml` | go-pmtiles serving the CONUS basemap archive; a `fetch-basemap` init container fills it |
| `pvcs.yaml`, `services.yaml`, `httproute.yaml` | storage, Services, Gateway API route |

The pattern is app repo (this one: code, Dockerfile, CI that publishes images) plus GitOps repo
(manifests watched by Argo CD or Flux; deploying is bumping a pinned image tag in a PR). A local
kustomization works too; the split is a workflow choice, not a requirement.

## Deploy steps

`deploy/kubernetes/` already does all of this. The steps below are what it does, for when you
write your own manifests or need to know why a piece is there.

1. **Get the images.** Use the three `ghcr.io/mitchross/radar-ng-*` images, pinned by `vX.Y.Z@sha256:…`, or mirror them into your own registry.
2. **Temporal.** Run Temporal plus its datastore. Pick one:
   - the [Temporal Helm chart](https://github.com/temporalio/helm-charts)
   - `temporalio/auto-setup` for a quick single-pod Temporal
   - a Temporal you already run, or Temporal Cloud

   Temporal needs **PostgreSQL** (a `postgres:16` StatefulSet, CloudNativePG or managed PG) or Cassandra. The Helm chart and auto-setup can create it for you. Create the namespace the workers use (default `default`; production uses `radar-ng`). The reference uses the Temporal Worker Controller (`TemporalConnection` + `WorkerDeployment` CRs) so each release is smoke-tested before promotion; plain Deployments running `radar worker` also work.
3. **Config.** The worker tunables go in a ConfigMap. **Set `NWS_USER_AGENT` to your email.** Everything else has working defaults; see [configuration.md](configuration.md) and [tuning.md](tuning.md).
4. **Workers.** Run either one worker with `WORKER_ROLE=legacy` (every workflow on queue `radar-ng`), or five role pools (`mrms`, `nowcast`, `hrrr`, `aux`, `alerts`) with `USE_ISOLATED_TASK_QUEUES=1`, as production does. Add the open-meteo worker (`--role open-meteo`). Temporal dispatches by registration; there's nothing else to wire.
5. **Tile-server, open-meteo and basemap.** Add the serve Deployments, Services, PVCs (see [Storage](#storage)) and your Ingress or Gateway (for example `radar-ng-api.example.com` → tile-server `:8080`).
6. **Basemap.** Fetch the PMTiles extract once into the basemap volume so the map isn't gray (same step as [Compose](compose.md)).
7. **Schedules.** Exactly one worker seeds the Temporal Schedules on boot; see [Schedules](#schedules).

Verify against your ingress host: `/api/livez`, `/api/health`, `/api/manifest.json`, or run
`radar debug doctor --server https://<host>` ([debug-harness.md](../reference/debug-harness.md)).

## Storage

| PVC | size (reference) | mounted by |
|---|---|---|
| `tiles` | 100 Gi | ingest workers (rw) · tile-server (ro) |
| `grids` | 20 Gi | ingest workers (rw) · tile-server (ro) |
| `state` | 5 Gi | workers (rw) · tile-server (ro) |
| `openmeteo-data` | 60 Gi | open-meteo serve + sync sidecar (rw) |
| `pmtiles` | 50 Gi | basemap (rw) |

If these PVCs are ReadWriteOnce, use `strategy: Recreate`, keep the tile-server at one replica, and co-locate every pod that mounts them (`podAffinity` or a `nodeSelector`). With an RWX class (NFS, CephFS, Longhorn RWX) the tile-server and workers can run on different nodes. `openmeteo-data` is fine on RWO: the reference runs the sync worker as a sidecar in the open-meteo serve pod for exactly this reason. Do not combine an RWO data plane with a multi-replica HPA. For horizontal scale, publish immutable tiles to S3-compatible object storage and keep the serving tier stateless; RWX storage is a smaller migration but still leaves the shared filesystem on the request path.

Pods mount the PVCs at `/data/tiles`, `/data/grids`, `/data/state` — keep those paths or override `TILE_DIR`/`GRID_DIR`/`STATE_DIR`. The images run as uid 1000 (open-meteo: 999); set `fsGroup` to match.

The basemap PMTiles archive needs the same one-time extract as compose (see [self-hosting.md](compose.md)). Either mount it in the tile-server and set `BASEMAP_PMTILES`, or run go-pmtiles as a `basemap` Service and let the api proxy `/basemap/tiles/*` to `BASEMAP_TILES_UPSTREAM` (default `http://basemap:8081`), as the reference does.

## Resource shapes

Starting points; validate them against your source cadence and a load test:

| service | requests | limits |
|---|---|---|
| single `legacy` worker (all activities) | 1 cpu · 3 Gi | 12 cpu · 12 Gi |
| open-meteo sync | 0.5 cpu · 512 Mi | 2 cpu · 2 Gi |
| tile-server | 0.2 cpu · 256 Mi | 2 cpu · 1 Gi |
| open-meteo serve | 0.2 cpu · 512 Mi | 2 cpu · 2 Gi |
| basemap | 50 m · 64 Mi | 500 m · 256 Mi |

With role pools, size each pool from its own queue latency and publication freshness; the reference manifests carry the current values. Undersizing the MRMS worker's CPU shows up as stale radar — the 2-minute MRMS cadence needs the render to finish in under 2 minutes. See [tuning.md](tuning.md).

## Probes

Tile-server: point liveness at **`/api/livez`** on port 8080. Do **not** probe `/api/health`. It returns 503 when radar *data* is stale — a condition shared by every replica at once. Wiring it to liveness would restart the fleet the moment NOAA has a slow day; wiring it to readiness would drain all endpoints simultaneously. `/api/health` is for humans, dashboards, and the app's "data delayed" banner. The reference uses a TCP readiness check so a slow API never empties the single-replica Service.

Workers: liveness execs `sh -c "test $(find /tmp/worker-alive -mmin -3)"` (the health loop touches it every 30 s); `/tmp/temporal-healthy` tracks Temporal reachability. The images carry busybox `sh`, `find` and `test` for these probes.

## HPA

With ReadWriteOnce tiles/grids/state, set HPA `minReplicas: 1` and `maxReplicas: 1`; the HPA is an explicit guardrail, not scaling. After tiles move to object storage and the API becomes stateless, establish the replica range with the capacity acceptance test ([capacity-acceptance.md](../reference/capacity-acceptance.md)) and add a `PodDisruptionBudget` that matches that tested floor.

For worker scale, run the five role pools with `USE_ISOLATED_TASK_QUEUES=1` instead of scaling one all-in-one worker. Deploy every role worker before switching queues, and confirm each queue has a poller.

## Schedules

Exactly one worker seeds the Temporal Schedules on boot (`SEED_SCHEDULES=1`; the reference sets it on the `aux` pool, the others set `SKIP_SCHEDULE_SEED=1`). Seeding is create-or-update per schedule ID — there are no CronJobs. Set `SKIP_SCHEDULE_SEED=1` on the seeder too if you need to hand-edit a schedule in the Temporal UI without it being rewritten on the next restart.

Verify after a rollout:

```bash
radar debug temporal --temporal-address <temporal-frontend>:7233 --namespace <ns>
temporal --address <temporal-frontend>:7233 --namespace <ns> schedule list
```

## Observability

- the tile-server exports Prometheus metrics at `/api/metrics`; workers at `:9464/metrics` (`TEMPORAL_METRICS_BIND`)
- every process logs one-line JSON to stdout — any log pipeline that scrapes container stdout works
- `radar debug doctor` ([debug-harness.md](../reference/debug-harness.md)) checks freshness, serving, Temporal and disk in one sweep
