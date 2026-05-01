# Talos / ArgoCD reference manifests for radar-ng Temporal

These are **reference templates**. The live manifests live in your
`talos-argocd-proxmox` repo at `my-apps/development/radar-ng/`. ArgoCD
auto-discovers via the ApplicationSet at `my-apps/*/*`, so adding files
to that directory and updating its `kustomization.yaml` is the entire
deploy.

This directory contains zero application code — only k8s manifests.
Source lives in this monorepo under `backend/`, `temporal/`, `frontend/`.

## What's here

| File | Purpose |
|---|---|
| `temporal-connection.yaml`             | `TemporalConnection` CR pointing at `temporal-frontend.temporal:7233` |
| `temporal-worker-deployment.yaml`      | `TemporalWorkerDeployment` CR — the worker pod (2 replicas) |
| `configmap-temporal-config.yaml`       | All non-secret tunables (palettes, retentions, NWS URLs, etc.) |
| `rbac-temporal-worker.yaml`            | ServiceAccount + Role + RoleBinding (worker creates k8s Jobs for open-meteo sync) |
| `secret-temporal-worker.yaml.template` | APNS p8 + FCM JSON placeholders (apply out-of-band) |

## Step-by-step talos repo deploy

### 1. Add four files

Copy these four from `radar-ng/deploy/k8s/` into
`talos-argocd-proxmox/my-apps/development/radar-ng/`:

```
temporal-connection.yaml
temporal-worker-deployment.yaml
configmap-temporal-config.yaml
rbac-temporal-worker.yaml
```

### 2. Update kustomization.yaml

Append the four new files to `resources:` AND remove **all** legacy
ingest deployments + cleanup CronJob + open-meteo sync CronJobs. Also
remove the `configMapGenerator` block (the cleanup script is now an
in-Python activity):

```diff
 resources:
   - namespace.yaml
   - pvcs.yaml
   - services.yaml
   - httproute.yaml
   - deployment-tile-server.yaml
   - hpa-tile-server.yaml
   - pdb-tile-server.yaml
   - deployment-basemap.yaml
   - deployment-open-meteo.yaml
-  - deployment-ingest-mrms.yaml
-  - deployment-ingest-radar-composite.yaml
-  - deployment-ingest-hrrr.yaml
-  - deployment-ingest-lightning.yaml
-  - deployment-ingest-tropical.yaml
-  - deployment-nowcast.yaml
-  - cronjob-tile-cleanup.yaml
-  - cronjob-open-meteo-sync.yaml
   - servicemonitor.yaml
+  - configmap-temporal-config.yaml
+  - rbac-temporal-worker.yaml
+  - temporal-connection.yaml
+  - temporal-worker-deployment.yaml

 namespace: radar-ng

-configMapGenerator:
-  - name: tile-cleanup-script
-    files:
-      - cleanup.sh=tile-cleanup.sh
```

Also `rm` the corresponding `.yaml` files + `tile-cleanup.sh` from the
talos repo directory once you're confident:

```
deployment-ingest-mrms.yaml
deployment-ingest-radar-composite.yaml
deployment-ingest-hrrr.yaml
deployment-ingest-lightning.yaml
deployment-ingest-tropical.yaml
deployment-nowcast.yaml
cronjob-tile-cleanup.yaml
cronjob-open-meteo-sync.yaml
tile-cleanup.sh
```

(I'd remove from the resources list first, wait one ArgoCD sync to
confirm the worker is happy, then `git rm` the actual files.)

### 3. Bump tile-server image (only when you're ready to expose `/v1/*`)

Mobile-facing API gains `/v1/push-tokens`, `/v1/watches`, `/v1/workflows`.
Your existing `httproute.yaml` already routes `/` to `tile-server`, so
no HTTPRoute changes needed. But the tile-server image must be the new
one with the workflow routes compiled in. Edit `deployment-tile-server.yaml`:

```diff
       containers:
         - name: tile-server
-          image: registry.vanillax.me/radar-ng-tile-server:v1.0.5
+          image: registry.vanillax.me/radar-ng-tile-server:v1.1.0   # whichever new tag
           env:
             - name: TILE_DIR
               value: /data/tiles
+            - name: TEMPORAL_ADDRESS
+              value: temporal-frontend.temporal.svc.cluster.local:7233
+            - name: TEMPORAL_NAMESPACE
+              value: default
```

Renovate opens the image-bump PR automatically once `build-api.yml` ships
a new tag — just merge it.

### 4. Apply the secret out-of-band (only if you want push notifications)

```bash
kubectl apply -f /path/to/your-edited-secret-temporal-worker.yaml
```

The TemporalWorkerDeployment references the secret as `optional: true`,
so the worker boots fine without it. Without it, ingest workflows still
run; only `send_push_notification` raises non-retryable.

### 5. Seed the Schedules (one-time)

After the worker pod is Ready:

```bash
kubectl exec -n radar-ng deploy/radar-ng-worker -- \
  python -m temporal.schedules.seed
```

Re-running is idempotent. Creates ten schedules:

| Schedule ID | Cadence | Replaces |
|---|---|---|
| `ingest-mrms-base`       | every 2m    | `deployment-ingest-mrms.yaml` |
| `ingest-mrms-composite`  | every 2m    | `deployment-ingest-radar-composite.yaml` |
| `ingest-hrrr`            | every 15m   | `deployment-ingest-hrrr.yaml` |
| `ingest-lightning`       | every 60m   | `deployment-ingest-lightning.yaml` |
| `ingest-tropical`        | every 1h    | `deployment-ingest-tropical.yaml` |
| `nowcast`                | every 2m    | `deployment-nowcast.yaml` |
| `tile-cleanup`           | every 1h    | `cronjob-tile-cleanup.yaml` |
| `poll-alerts`            | every 5m    | _new — server-side NWS alert poll_ |
| `open-meteo-sync-gfs`    | `30 */6 * * *` cron | `cronjob-open-meteo-sync.yaml` (gfs) |
| `open-meteo-sync-hrrr`   | `45 * * * *` cron   | `cronjob-open-meteo-sync.yaml` (hrrr) |

### 6. Verify

```bash
# pods
kubectl -n radar-ng get pods -l app=radar-ng-worker -w

# schedules visible in Temporal UI
temporal --address temporal-frontend.temporal.svc.cluster.local:7233 \
  schedule list

# logs
kubectl -n radar-ng logs deploy/radar-ng-worker --tail=50 -f

# spot-check open-meteo k8s Job creation
kubectl -n radar-ng get jobs --watch
```

## Things to confirm before pasting

1. **TemporalConnection apiVersion = `temporal.io/v1alpha1`** (matches
   news-reader-temporal-worker pattern).
2. **PVC names** are `tiles`, `grids`, `state` — already exist (legacy
   ingestors use them). Worker mounts each separately at `/data/{tiles,grids,state}`.
3. **Worker namespace = `radar-ng`** (NOT `radar-ng-temporal-worker`).
   Required because PVCs are namespace-scoped and the worker shares
   them with the API + the legacy services during the cutover.
4. **No mobile-side Temporal access.** All reachability is `mobile →
   HTTPS → tile-server → Temporal`. Existing `httproute.yaml` (catch-all
   `/`) is sufficient — no new HTTPRoute needed.
5. **k8s RBAC required** — the worker creates Jobs (open-meteo sync).
   The Role grants `jobs` + `pods` + `pods/log` in the `radar-ng`
   namespace **only**. ServiceAccount is `radar-ng-worker`.

## Rollback

`kubectl scale -n radar-ng deploy/radar-ng-worker --replicas=0` stops the
worker; schedules keep firing in Temporal but no worker picks them up. To
fully revert: restore the deleted deployment YAML entries in
`kustomization.yaml`; ArgoCD recreates them on next sync.
