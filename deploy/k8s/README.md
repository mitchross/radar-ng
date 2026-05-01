# Talos / ArgoCD manifests for radar-ng Temporal

These are **templates** — the live manifests live in your
`talos-argocd-proxmox` repo. Copy these over (or kustomize-overlay them)
when wiring up the Temporal worker for the first time.

## What changes vs. the legacy CronJob world

**Delete** these from the talos repo (replaced by Temporal Schedules):

```
namespaces/radar-ng/cronjob-ingest-mrms.yaml
namespaces/radar-ng/cronjob-ingest-hrrr.yaml
namespaces/radar-ng/cronjob-ingest-lightning.yaml
namespaces/radar-ng/cronjob-ingest-tropical.yaml
namespaces/radar-ng/cronjob-nowcast.yaml
namespaces/radar-ng/cronjob-tile-cleanup.yaml
```

**Add** these (provided here):

| File | Purpose |
|---|---|
| `temporal-worker-deployment.yaml` | `TemporalWorkerDeployment` CR (worker pods) |
| `configmap-storage.yaml`          | S3 endpoint + bucket |
| `configmap-tile-config.yaml`      | Tile dirs, zoom ranges, retention windows |
| `configmap-noaa-endpoints.yaml`   | NOAA / NWS / NHC URLs |
| `secret-worker.yaml.template`     | APNS/FCM/S3/NWS creds (replace before applying) |
| `httproute-api-workflows.yaml`    | New /v1/* paths on the API gateway |

**Update** these:

- `deployment-tile-server.yaml` (rename to `deployment-api.yaml`):
  - Image stays `registry.vanillax.me/radar-ng-tile-server` for compat (or rename in coordination with Renovate config)
  - Add envs: `TEMPORAL_ADDRESS=temporal-frontend.temporal:7233`, `TEMPORAL_NAMESPACE=default`

## One-time bootstrap

1. Apply the ConfigMap + Secret + TemporalWorkerDeployment manifests
2. Wait for worker pod to be Ready
3. `kubectl exec -n radar-ng deploy/radar-ng-worker -- python -m temporal.schedules.seed`
   (or rely on the seed.py initContainer if you wire it in)
4. Verify schedules in Temporal UI: `temporal --address temporal-frontend.temporal:7233 schedule list`
5. Delete the legacy CronJobs

The first-time seed creates 7 schedules. Re-running it is idempotent (updates).

## Storm-watch & push

Push notifications require:
- `APNS_KEY` (.p8 contents) + `APNS_KEY_ID` + `APNS_TEAM_ID` + `APNS_TOPIC` in `radar-ng-worker-secrets`
- `FCM_PROJECT_ID` + service-account JSON at `/secrets/fcm.json` in the worker

Without these, `RegisterPushTokenWorkflow` still works (it just persists the
token). `WatchStormWorkflow` activities will fail at the
`send_push_notification` step. Watches still run; pushes silently fail and
are logged for triage.
