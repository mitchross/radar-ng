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
| `temporal-worker-deployment.yaml`      | Main worker pool (2 replicas, all radar-ng-owned activities) |
| `open-meteo-worker-deployment.yaml`    | Secondary worker pool (1 replica, FROM ghcr.io/open-meteo/open-meteo) |
| `configmap-temporal-config.yaml`       | All non-secret tunables (palettes, retentions, NWS URLs, etc.) |
| `secret-temporal-worker.yaml.template` | APNS p8 + FCM JSON placeholders (apply out-of-band) |

The two workers poll the same task queue. The main worker registers every
activity except `open_meteo_sync`; the open-meteo worker registers only
that one. Temporal dispatches activity tasks based on registration —
nothing else is needed to wire them.

## Step-by-step talos repo deploy

### 1. Add four files

Copy these four from `radar-ng/deploy/k8s/` into
`talos-argocd-proxmox/my-apps/development/radar-ng/`:

```
temporal-connection.yaml
temporal-worker-deployment.yaml
open-meteo-worker-deployment.yaml
configmap-temporal-config.yaml
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
+  - temporal-connection.yaml
+  - temporal-worker-deployment.yaml
+  - open-meteo-worker-deployment.yaml

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

### 3. Tile-server `/v1/*` routes

Mobile-facing API gains `/v1/push-tokens`, `/v1/watches`, `/v1/workflows`
in `radar-ng-tile-server:v1.0.6+`. The talos `deployment-tile-server.yaml`
already pins v1.0.6 and sets `TEMPORAL_ADDRESS` +
`TEMPORAL_NAMESPACE=default`, so nothing to do here on the cluster side.
Renovate opens image-bump PRs as new `build-api.yml` tags ship.

### 4. Push notifications — disabled by default

The worker pod sets `PUSH_DISABLED=1` in `temporal-worker-deployment.yaml`.
The `send_push_notification` activity logs+returns successfully without
touching APNS/FCM, so `WatchStormWorkflow` still runs end-to-end (poll
frames, detect change) but no notification is sent.

To re-enable later:
1. Apply `radar-ng-temporal-secrets` (template at
   `secret-temporal-worker.yaml.template` in this dir) with `APNS_KEY`
   and/or `FCM_CREDENTIALS_JSON`.
2. In `temporal-worker-deployment.yaml`: set `PUSH_DISABLED=0`, restore
   the `secretRef` envFrom block, the `push-keys` volume mount, and the
   `push-keys` volume entry. (Check git history of that file for the
   exact blocks that were removed.)

### 5. Schedule seeding — automatic

The worker calls `temporal/schedules/seed.py` on startup before
`worker.run()`. `seed()` is idempotent (create-or-update per
`schedule_id`) so the two HA replicas racing on first boot is fine —
both converge on the same desired state.

Set `SKIP_SCHEDULE_SEED=1` on the worker if you ever need to inhibit
seeding (e.g. while fixing a bad schedule definition by hand in the
Temporal UI). No `kubectl exec` ever required.

The seeded schedules:

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
| `open-meteo-sync-gfs`    | every 6h    | `cronjob-open-meteo-sync.yaml` (gfs) |
| `open-meteo-sync-hrrr`   | every 1h    | `cronjob-open-meteo-sync.yaml` (hrrr) |

### 6. Verify

```bash
# pods
kubectl -n radar-ng get pods -l app=radar-ng-worker -w

# schedules visible in Temporal UI
temporal --address temporal-frontend.temporal.svc.cluster.local:7233 \
  schedule list

# worker logs — look for "schedule seed complete" then "worker starting"
kubectl -n radar-ng logs deploy/radar-ng-worker --tail=50 -f

# open-meteo runs as a separate worker pool now (no k8s Jobs)
kubectl -n radar-ng logs deploy/radar-ng-open-meteo-worker --tail=50 -f
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
5. **No k8s RBAC needed.** Earlier drafts had the worker create k8s
   Jobs to run the open-meteo Swift binary; that violated "no
   CronJobs/Jobs at all" so the work moved into a dedicated worker
   pool. The open-meteo worker's image is `FROM
   ghcr.io/open-meteo/open-meteo`, the activity subprocess-execs
   `/app/openmeteo-api`, and zero Job/CronJob resources are ever
   created in the cluster.

## Rollback

`kubectl scale -n radar-ng deploy/radar-ng-worker --replicas=0` stops the
worker; schedules keep firing in Temporal but no worker picks them up. To
fully revert: restore the deleted deployment YAML entries in
`kustomization.yaml`; ArgoCD recreates them on next sync.
