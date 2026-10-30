# radar-ng on Kubernetes

Ready-to-apply manifests for running the radar-ng backend on your own cluster. One command gets
you a working stack on a laptop cluster (kind, k3d, k3s, Docker Desktop). The same files, plus a
few optional add-ons, are how the owner runs it in production.

```
deploy/kubernetes/
  base/                 the app: tile-server (api), one worker, open-meteo (+ sync sidecar),
                        basemap, Services, PVCs, settings
  components/
    temporal-dev/       a throwaway Temporal + Postgres + Web UI, to try things out
    ingress/            publish the api with an Ingress
    gateway-api/        publish the api with a Gateway API HTTPRoute
    monitoring/         Prometheus Operator ServiceMonitor, PodMonitor and alert rules
    worker-controller/  production-style workers: five role pools via the Temporal Worker Controller
    autoscaling/        VPA
  examples/
    minimal/            base + temporal-dev: the one-command try-out
    production/         base + worker-controller + gateway-api + monitoring + autoscaling
  validate.sh           render everything and check it with kubeconform (what CI runs)
```

Want one computer and no Kubernetes? Use Docker Compose instead:
[docs/self-host/compose.md](../../docs/self-host/compose.md).

## Try it out (examples/minimal)

### You need

- A Kubernetes cluster with a **default StorageClass**. kind, k3d and k3s come with one
  (`local-path`). Check with `kubectl get storageclass`: one line should say `(default)`.
- `kubectl` 1.27 or newer (it has Kustomize built in).
- A machine with room to spare: about **8 CPU cores, 16 GB RAM and 50 GB free disk** is
  comfortable. The worker alone asks for 1 CPU and 3 GB. On a smaller machine, see
  [If it didn't work](#if-it-didnt-work).
- Internet access from the cluster: it pulls public images from `ghcr.io` and Docker Hub, and the
  worker downloads radar data from NOAA.

### Steps

1. **Get the repo.**

   ```bash
   git clone https://github.com/mitchross/radar-ng.git
   cd radar-ng
   ```

2. **Put your email in the NWS User-Agent.** The National Weather Service asks every client to
   say who it is. Open `deploy/kubernetes/examples/minimal/kustomization.yaml` and change
   `you@example.com` on the `NWS_USER_AGENT` line to your own address.

3. **(Optional) Make a laptop cluster.** Skip this if you already have one.

   ```bash
   kind create cluster --name radar-ng
   ```

4. **Apply it.**

   ```bash
   kubectl apply -k deploy/kubernetes/examples/minimal
   ```

5. **Open the api on your machine.**

   ```bash
   kubectl -n radar-ng port-forward svc/tile-server 8080:8080
   ```

   Leave that running and use `http://localhost:8080`. To point the phone app at it, see
   [docs/app/running-the-app.md](../../docs/app/running-the-app.md).

### How to know it worked

```bash
kubectl -n radar-ng get pods
```

After a few minutes every pod should be `Running`. The `basemap` pod stays in `Init:0/1` while it
downloads the map (10-40 minutes for the whole US; see [Basemap](#basemap) for a quicker one). The
radar works without it; the map is just gray until it's done.

```bash
curl -s localhost:8080/api/livez      # the api process is up
curl -s localhost:8080/api/health     # "degraded" (HTTP 503) at first, then "ok"
```

`/api/health` says `degraded` until the first radar frame lands, which takes **about 2 minutes**
after the worker starts. Then it returns `"status":"ok"`. Other layers fill in over the next half
hour (HRRR forecast radar, nowcast); the first forecast appears after the first Open-Meteo sync.

To watch the workflows run, open the Temporal Web UI:

```bash
kubectl -n radar-ng port-forward svc/temporal-ui 8233:8080   # then http://localhost:8233
```

You should see Schedules (`ingest-mrms-base` and friends) and workflows completing.

### If it didn't work

| What you see | What to do |
|---|---|
| Pods stuck `Pending`, events say `unbound PersistentVolumeClaim` | No default StorageClass. Install one (e.g. [local-path-provisioner](https://github.com/rancher/local-path-provisioner)) or set `storageClassName` in an overlay; see [Storage](#storage). |
| Pods stuck `Pending`, events say `Insufficient cpu` / `Insufficient memory` | The machine is too small for the default requests. Lower `resources.requests` in `base/worker.yaml` (and `base/open-meteo.yaml`) in your copy. The radar still needs the MRMS render to finish within 2 minutes. |
| `radar-ng-worker` stays `Pending` with `didn't match pod affinity rules` | The worker must run on the same node as `tile-server` (they share volumes). Wait for `tile-server` to be scheduled first; on a multi-node cluster read [Storage](#storage). |
| `radar-ng-worker` is `Running` but `0/1` ready | It can't reach Temporal yet. Check `kubectl -n radar-ng get pods -l app=temporal`; the first start takes a minute while it creates its database. |
| `/api/health` stays `degraded` for more than 5 minutes | Look at the worker: `kubectl -n radar-ng logs deploy/radar-ng-worker`. Errors about NOAA usually mean the cluster has no outbound internet. |
| `basemap` init fails | `kubectl -n radar-ng logs deploy/basemap -c fetch-basemap`. It needs to reach `build.protomaps.com` and `github.com`. |
| Something else | Run the doctor: `radar debug doctor --server http://localhost:8080` ([debug-harness.md](../../docs/reference/debug-harness.md)). |

To start over: `kubectl delete -k deploy/kubernetes/examples/minimal` (this deletes the volumes
too), or `kind delete cluster --name radar-ng`.

## Make it your own

Don't edit these files in place if you can help it. Make your own kustomization (in your own
repo, or next to this one) that points at the base and adds the components you want:

```yaml
# my-radar/kustomization.yaml
apiVersion: kustomize.config.k8s.io/v1beta1
kind: Kustomization
resources:
  # A local path, or a remote base (pin ref to a release tag or commit, not a branch):
  # - https://github.com/mitchross/radar-ng//deploy/kubernetes/base?ref=<tag-or-commit>
  - ../radar-ng/deploy/kubernetes/base
components:
  - ../radar-ng/deploy/kubernetes/components/ingress
configMapGenerator:
  - name: radar-ng-settings
    behavior: merge
    literals:
      - NWS_USER_AGENT=(radar-ng, me@my-domain.org)
      - TEMPORAL_ADDRESS=temporal-frontend.temporal.svc.cluster.local:7233
patches:
  - target: { kind: Ingress, name: radar-ng-api }
    patch: |
      - op: replace
        path: /spec/rules/0/host
        value: radar.my-domain.org
```

### Settings

Two ConfigMaps hold the settings, generated from files in `base/`:

- `settings.env` → `radar-ng-settings`: the things you're expected to change.
  **`NWS_USER_AGENT` must be changed** (look for `>>> CHANGE THIS <<<`). `TEMPORAL_ADDRESS` and
  `TEMPORAL_NAMESPACE` point at your Temporal.
- `worker.env` → `radar-ng-worker-config`: worker tunables. The defaults work.

Every variable is described in [docs/self-host/configuration.md](../../docs/self-host/configuration.md).
Changing a value restarts the pods that read it.

**AI briefing card (optional).** The api shows it only when `LLM_BASE_URL` is set. Add
`LLM_BASE_URL` (and `LLM_MODEL`) to the tile-server's env, and put the key in a Secret:

```bash
kubectl -n radar-ng create secret generic radar-ng-llm --from-literal=LLM_API_KEY=sk-...
```

**City search** (`/api/geocode`) returns 503 until you set `GEOCODER_URL` or `PHOTON_URL` on the
tile-server.

### Temporal

The workers need a Temporal server. Pick one:

- **`components/temporal-dev`**: one Temporal pod (`temporalio/auto-setup:1.25.2`), Postgres 16
  and the Web UI, the same versions as `deploy/docker-compose.yml`. Fine for trying things out.
  Not for production: no HA, no backups, a fixed password.
- **Your own Temporal** (the [Helm chart](https://github.com/temporalio/helm-charts), or one you
  already run) or **Temporal Cloud**. Leave out `temporal-dev` and set `TEMPORAL_ADDRESS` /
  `TEMPORAL_NAMESPACE`. Create the namespace in Temporal first.

### Storage

The PVCs in `base/pvcs.yaml` have **no `storageClassName`**, so they use your cluster's default.
To pick another class, patch it in your overlay:

```yaml
patches:
  - target: { kind: PersistentVolumeClaim }
    patch: |
      - op: add
        path: /spec/storageClassName
        value: my-fast-ssd
```

| PVC | size | used by |
|---|---|---|
| `tiles` | 100 Gi | worker (writes) · tile-server (reads) |
| `grids` | 20 Gi | worker (writes) · tile-server (reads) |
| `state` | 5 Gi | worker (writes) · tile-server (reads) |
| `openmeteo-data` | 60 Gi | open-meteo serve + sync sidecar |
| `pmtiles` | 50 Gi | basemap |
| `temporal-postgres` | 5 Gi | temporal-dev only |

The sizes are generous ceilings; real use is much lower (see the disk notes in
[compose.md](../../docs/self-host/compose.md)). On `local-path` the size isn't enforced anyway.

**ReadWriteOnce (RWO) is the default, and it has rules.** Most storage (local-path, cloud block
disks, Longhorn's default) is RWO: a volume can be attached to **one node** at a time. `tiles`,
`grids` and `state` are shared by the tile-server and the worker, so with RWO:

- the worker has a `podAffinity` that puts it on the tile-server's node (already set up);
- every Deployment uses `strategy: Recreate`, so the old pod lets go of the volume before the new
  one starts (otherwise the rollout hangs on a Multi-Attach error);
- the tile-server stays at **one replica**. That's why `components/autoscaling` has no HPA.

On a single-node cluster none of this matters. If your class is **ReadWriteMany** (NFS, CephFS,
Longhorn RWX), change `accessModes` to `ReadWriteMany` for those three PVCs and you can drop the
`podAffinity` from the workers.

`openmeteo-data` is different: it **must be a block or local filesystem** (ext4, xfs). Open-Meteo's
writer fails on NFS. RWO is fine there because the sync worker is a sidecar in the same pod.

### Basemap

The `basemap` pod's init container downloads a [Protomaps](https://protomaps.com) extract once
into the `pmtiles` volume; later restarts skip it. The default area is the continental US. To try
things out faster, set `BBOX` in `base/basemap.yaml` to a small area (the comment has Michigan as
an example). Bump `REFRESH` to force a fresh download.

### Publishing the api

The tile-server Service (`tile-server:8080`) is the only thing that needs to be public. Add one of:

- `components/ingress`: a standard Ingress. Uses your default IngressClass (Traefik on k3s).
- `components/gateway-api`: an HTTPRoute. Needs the Gateway API CRDs and a Gateway.

Both use `radar-ng-api.example.com`. Change it to your hostname.

## Going to production

`examples/production` is the owner's production layout with everything personal removed. It
adds:

| Component | What it does | Needs |
|---|---|---|
| `worker-controller` | Replaces the single `legacy` worker with five role pools (`mrms`, `nowcast`, `hrrr`, `aux`, `alerts`) on their own task queues. Each release is smoke-tested (`RadarDeploymentSmokeWorkflow`) before it takes work. The `aux` pool seeds the Schedules. | [Temporal Worker Controller](https://github.com/temporalio/temporal-worker-controller) and its CRDs; a Temporal server with Worker Deployment (versioning) support. Set the address in `connection.yaml` and `TEMPORAL_ADDRESS`. |
| `gateway-api` / `ingress` | Publishes the api | see above |
| `monitoring` | Scrapes the api (`/api/metrics`) and every worker (`:9464/metrics`), plus alert rules for stale radar, failed syncs, missing pollers, full volumes | Prometheus Operator CRDs (kube-prometheus-stack). If your Prometheus selects by label, add it in your overlay. |
| `autoscaling` | Memory-only VPAs (no HPA or PDB: the tile-server stays at one replica, see [Storage](#storage)) | VPA CRDs and controller for `vpa.yaml` |

More on sizing, probes and Schedules: [docs/self-host/kubernetes.md](../../docs/self-host/kubernetes.md).
The owner's live manifests, for reference:
[talos-argocd-proxmox `my-apps/personal-projects/radar-ng/`](https://github.com/mitchross/talos-argocd-proxmox/tree/main/my-apps/personal-projects/radar-ng).

## Things that must stay true

If you change the manifests, keep these, or pods restart for no reason or can't write their data:

- **tile-server liveness is `/api/livez` on 8080, never `/api/health`.** Health returns 503
  whenever NOAA data is late, which would restart every replica at once.
- **Worker liveness** is `sh -c "test $(find /tmp/worker-alive -mmin -3)"`. The worker touches
  that file every 30 s.
- **Users:** the images run as uid 1000 (open-meteo: 999). Keep `fsGroup` set to match, or the
  pods can't write to fresh volumes.
- **Paths:** data lives at `/data/tiles`, `/data/grids`, `/data/state` (or change `TILE_DIR`,
  `GRID_DIR`, `STATE_DIR`).
- **Exactly one worker seeds the Schedules** (`SEED_SCHEDULES=1`): the `legacy` worker in base,
  the `aux` pool with `worker-controller`.

## Checking your changes

```bash
deploy/kubernetes/validate.sh
```

It builds the base, every example and every component, and validates the output with
[kubeconform](https://github.com/yannh/kubeconform) when it's installed. CI runs the same script
on every pull request that touches `deploy/kubernetes/`.
