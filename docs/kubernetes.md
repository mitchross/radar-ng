# Running radar-ng on Kubernetes

The advanced path. Compose ([self-hosting.md](self-hosting.md)) gives you everything on one box; Kubernetes buys you HA tile-serving, HPA under load, PodDisruptionBudgets, and a Prometheus scrape target. This doc is **bring-your-own-cluster** — any conformant Kubernetes with a decent storage class works. Nothing here requires the author's Talos/ArgoCD setup.

## What you need first

| prerequisite | notes |
|---|---|
| Kubernetes 1.28+ | author runs 1.35; nothing exotic is used |
| A **Temporal server** | self-host [temporal](https://github.com/temporalio/temporal) in-cluster or use Temporal Cloud. The worker needs `TEMPORAL_ADDRESS` + `TEMPORAL_NAMESPACE` |
| Storage class | see PVC section below |
| Image registry | build the `backend/` and `temporal/` images and push somewhere your cluster can pull from |
| (optional) Prometheus operator | for the `ServiceMonitor` + Grafana dashboard |

## Reference manifests

`deploy/k8s/` contains **reference templates**, not a ready-to-apply kustomization — copy them into your own manifest/gitops repo and adapt image names, storage classes, and hostnames. See [`deploy/k8s/README.md`](../deploy/k8s/README.md) for the full walkthrough (written against the author's cluster, but the mechanics are generic).

| file | purpose |
|---|---|
| `temporal-connection.yaml` | `TemporalConnection` CR pointing at your Temporal frontend (`temporal.io/v1alpha1`, requires the Temporal k8s operator — swap for a plain env-var Deployment if you don't run it) |
| `temporal-worker-deployment.yaml` | main worker pool — 2 replicas, registers every ingest activity |
| `open-meteo-worker-deployment.yaml` | secondary worker pool (1 replica, `FROM ghcr.io/open-meteo/open-meteo`) that registers only the `open_meteo_sync` activity |
| `configmap-temporal-config.yaml` | all non-secret tunables (palettes, cadences, NWS URLs — see [tuning.md](tuning.md)) |
| `secret-temporal-worker.yaml.template` | APNS/FCM placeholders; push is off by default (`PUSH_DISABLED=1`) |

You additionally need the serving-side manifests (tile-server Deployment + Service + HPA + PDB, basemap Deployment, open-meteo Deployment, PVCs, ingress/HTTPRoute). The author keeps those in a separate gitops repo — the pattern:

- **App repo** (this one): source code, Dockerfiles, CI that builds + pushes images
- **GitOps repo**: the k8s manifests, watched by ArgoCD/Flux; deploying = bumping an image tag and pushing

You can just as well `kubectl apply -k` a local kustomization. The split is a workflow choice, not a requirement.

## Storage

Four volumes, shared across pods:

| PVC | size | mounted by |
|---|---|---|
| `tiles` | 50 Gi | worker (rw) · tile-server replicas (ro) |
| `grids` | 20 Gi | worker (rw) · tile-server (ro) |
| `state` | 5 Gi | worker (rw) · tile-server (ro) |
| `openmeteo` | 30 Gi | open-meteo-worker (rw) · open-meteo (rw) |

Because the tile-server is HPA'd (2–6 replicas) **and** the worker writes the same volumes, you need either an RWX-capable storage class (Longhorn RWX, NFS, CephFS) or node-affinity pinning everything that shares a volume onto one node. The author runs Longhorn.

The worker mounts each PVC separately at `/data/tiles`, `/data/grids`, `/data/state` — keep the same paths or override `TILE_DIR`/`GRID_DIR`/`STATE_DIR`.

The basemap PMTiles archive needs the same one-time download as compose (see [self-hosting.md](self-hosting.md)) — an initContainer with `curl` into the basemap volume is the clean way.

## Resource shapes

Production-tested requests/limits:

| service | requests | limits |
|---|---|---|
| worker (runs MRMS/HRRR/nowcast activities) | 1 cpu · 1 Gi | **6 cpu · 6 Gi** — parallel palette render is the hot path |
| open-meteo-worker | 0.5 cpu · 512 Mi | 2 cpu · 2 Gi |
| tile-server | 0.2 cpu · 256 Mi | 2 cpu · 1 Gi |
| open-meteo | 0.2 cpu · 512 Mi | 2 cpu · 2 Gi |
| basemap | 50 m · 64 Mi | 500 m · 256 Mi |

Undersizing the worker's CPU limit directly shows up as stale radar — the 2-minute MRMS cadence needs the render to finish in under 2 minutes. See [tuning.md](tuning.md).

## Probes

Point liveness **and** readiness at **`/api/livez`** on the tile-server's port 8080 (through Caddy, deliberately — the probe then proves both the Caddy and uvicorn processes in the pod are alive).

Do **not** probe `/api/health`. It returns 503 when radar *data* is stale — a condition shared by every replica at once. Wiring it to liveness would restart the whole fleet the moment NOAA has a slow day; wiring it to readiness would drain all endpoints simultaneously. `/api/health` is for humans, dashboards, and the app's "data delayed" banner.

## HPA

The tile-server scales 2 → 6 on CPU. Tile serving is static-file + gzip, so it scales near-linearly; 2 replicas comfortably handle a handful of clients, and the ceiling of 6 exists mostly to cap noisy-neighbor damage. Add a `PodDisruptionBudget` (`minAvailable: 1`) so node drains don't kill the map.

The worker should **not** be HPA'd: schedule seeding is idempotent (two replicas racing on boot converge fine — that's how the stock 2-replica deployment works), but activity throughput is bounded by `TEMPORAL_MAX_CONCURRENT_ACTIVITIES`, not replica count, and each replica wants the full 6-cpu limit during renders.

## Schedules

The worker seeds all Temporal Schedules on boot (`temporal/schedules/seed.py`, create-or-update per schedule ID) — there are no CronJobs to install. Set `SKIP_SCHEDULE_SEED=1` on the worker if you need to hand-edit a schedule in the Temporal UI without the worker stomping it on next restart.

Verify after rollout:

```bash
kubectl -n radar-ng logs deploy/radar-ng-worker --tail=50 -f
# look for "schedule seed complete" then "worker starting"

temporal --address <temporal-frontend>:7233 schedule list
```

Rollback lever: `kubectl scale deploy/radar-ng-worker --replicas=0` stops all ingest; schedules keep firing in Temporal but nothing picks them up until you scale back.

## Observability

- tile-server exports Prometheus metrics at `/api/metrics` — add a `ServiceMonitor` if you run the Prometheus operator
- all services log one-line JSON to stdout (`backend/shared/logger.py`) — any log pipeline that scrapes container stdout works
- the Grafana dashboard the author uses (panel list in the [README](../README.md#observability--full-pipeline)) is a good starting template
