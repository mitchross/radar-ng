# Releasing — code to running pods

The pipeline is fully automatic; this doc exists so you know what's supposed
to happen, how long each hop takes, how to fast-track it, and how to unwedge
it when a hop silently stalls (each failure mode below has actually happened).

## The loop

```
push to radar-ng (Gitea master)
  → Gitea Actions build-*.yml         (~5–10 min)
      reads registry, bumps patch, pushes :vX.Y.Z + :latest
  → registry.vanillax.me
  → Renovate CronJob (hourly at :17)  (opens PR on talos-argocd-proxmox)
  → Renovate's NEXT hourly run        (merges the PR it opened last run)
  → ArgoCD (webhook / 3-min poll)     (syncs my-apps-radar-ng)
  → pods roll
```

**No manual tagging, ever.** CI computes the next semver from existing
registry tags (`build-api.yml` step "Compute next semver"). If you tag by
hand you will race it.

**End-to-end latency: ~1–2 h** (dominated by Renovate's open-on-one-run,
merge-on-the-next behavior). That's fine for routine changes.

### Which push builds which image

| paths touched | workflow | image |
|---|---|---|
| `backend/api/**`, `backend/shared/**`, `backend/basemap/styles/**` | build-api | `radar-ng-tile-server` |
| `backend/**`, `temporal/**` | build-temporal-worker | `radar-ng-temporal-worker` |
| `temporal/open_meteo_worker.Dockerfile` + reqs | build-open-meteo-worker | `radar-ng-open-meteo-worker` |
| `backend/basemap/**` | build-basemap | `radar-ng-basemap` |

One push to `backend/shared/` bumps both tile-server AND worker — expected.

## Fast-track (when you don't want to wait 2 h)

1. Wait for the Gitea Actions run to finish (Gitea → Actions, ~5–10 min).
   Verify: `curl -sk https://registry.vanillax.me/v2/radar-ng-tile-server/tags/list`
2. Either merge Renovate's open PR on `talos-argocd-proxmox` yourself, or —
   if Renovate hasn't run yet — edit the image tag in
   `my-apps/development/radar-ng/*.yaml` directly and push. Renovate treats a
   hand-bumped tag as up-to-date and stands down.
3. Argo picks it up within ~3 min. To force it:
   `kubectl annotate application my-apps-radar-ng -n argocd argocd.argoproj.io/refresh=normal --overwrite`

## Rollback

Set the image tag back to the last good version in the gitops repo and push.
Argo rolls back; Renovate will re-open a PR for the newer (bad) tag on its
next run — close it with a comment, or push the fixed code so a newer tag
supersedes it. Never `kubectl rollout undo`: selfHeal reverts you within
minutes.

## Failure modes (all field-tested)

| symptom | cause | fix |
|---|---|---|
| Renovate merges land in git but pods never roll; Argo says `Synced` | Argo repo-server manifest cache went stale (seen after node outages) — it compares against old manifests while labeling them with HEAD's hash | `kubectl annotate application my-apps-radar-ng -n argocd argocd.argoproj.io/refresh=hard --overwrite` |
| New tile-server tag crashloops with `caddy: Operation not permitted` | a binary copied with `COPY --from` carried file capabilities; pods run `capabilities: drop [ALL]` | copy binaries through `install` (strips xattrs) — see `backend/api/Dockerfile` |
| Forecast API crashloops with a missing shared library | something tracked upstream `:latest` and upstream broke it | pin real version tags everywhere; let Renovate propose bumps (crashes then show up in a reviewable PR, not a silent pull) |
| Dependency Dashboard shows nothing pending right after a build | images landed *after* Renovate's :17 run | wait for the next hour, or `kubectl create job -n renovate --from=cronjob/renovate renovate-manual` |
| CI flaked and a child workflow needs a `:vN.N.N` that never got pushed | — | trigger `.gitea/workflows/retag-from-latest.yml` from the Gitea Actions UI (pulls `:latest`, re-tags as next semver) |
| Worker pods don't adopt a resource-only change to the WorkerDeployment CR | the Worker Controller only rolls on new build ids | bump the worker image (any code push) or patch the build Deployment to force a new build_id |

## Verifying a release landed

```bash
# what's live vs what git wants
kubectl get deployment tile-server -n radar-ng \
  -o jsonpath='{.spec.template.spec.containers[0].image}'
curl -sk https://radar-ng-api.vanillax.me/api/health | jq .status

# worker version is encoded in the pod name (radar-ng-worker-v1-1-N-…)
kubectl get pods -n radar-ng
```
