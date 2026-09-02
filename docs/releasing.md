# Releasing — code to running pods

Production releases come from GitHub, GHCR, and the
`talos-argocd-proxmox` GitOps repository. The older Gitea workflows and
`registry.vanillax.me` are not the production Radar NG release path.

## Normal path

```text
merge a tested PR into mitchross/radar-ng:master
  -> the matching .github/workflows/ghcr-*.yml build runs
  -> GHCR receives :latest, :sha-<commit>, and :vX.Y.Z
  -> hosted Renovate proposes the GHCR tag in talos-argocd-proxmox
  -> Cluster CI validates the GitOps PR
  -> merge the GitOps PR
  -> Argo CD reconciles my-apps-radar-ng
  -> verify live image digests and product behavior
```

There is no release commit or git tag. Each image workflow reads its existing
GHCR semver tags and allocates the next patch version. The workflows are
serialized per image so two builds cannot choose the same version.

The workflow `on.push.paths` blocks are the source of truth for which source
change builds which artifact. A shared-code change can correctly build more
than one image. The production images are:

- `ghcr.io/mitchross/radar-ng-temporal-worker`
- `ghcr.io/mitchross/radar-ng-tile-server`
- `ghcr.io/mitchross/radar-ng-open-meteo-worker`
- `ghcr.io/mitchross/radar-ng-basemap`

Never deploy `latest`. GitOps pins immutable semver tags; record the digest and
the OCI `org.opencontainers.image.revision` label during rollout verification.

## Before merging source

1. Require the PR checks for the code being released. Backend CI builds the
   actual release images and runs their tests inside those images.
2. Keep one release for a given image in flight at a time. Workflow concurrency
   prevents a tag-allocation race, but serial source merges keep provenance and
   rollback obvious.
3. For workflow-code changes, confirm Temporal compatibility: replay a
   representative history or use a reviewed patch/version marker. A normal
   unit test is not a history-compatibility proof.
4. Confirm no unrelated backend change is being used merely to force an image
   bump. Use `workflow_dispatch` with an explicit reviewed version only when a
   rebuild is actually required.

Watch the release:

```bash
gh run list --repo mitchross/radar-ng --branch master --limit 10
gh run watch <run-id> --repo mitchross/radar-ng
gh run view <run-id> --repo mitchross/radar-ng --log
```

Read the completed workflow output for the published version. Do not predict
the tag from the current registry state; another serialized run may be ahead.

## GitOps rollout

Hosted Renovate watches the public GHCR images used by the cluster. The
in-cluster Renovate CronJob owns the private registry and is not the process to
wait for here.

Review every repeated image reference. In particular, the monolithic Temporal
worker artifact appears in the legacy WorkerDeployment and all five role
WorkerDeployments. All six must move together while they share workflow and
activity definitions.

For an urgent release, open a clean Talos worktree from current `origin/main`,
change the exact image references, render the application, and use a normal
GitOps PR:

```bash
git diff --check
kustomize build my-apps/development/radar-ng >/tmp/radar-ng.yaml
gh pr checks <pr-number> --repo mitchross/talos-argocd-proxmox --watch
```

Do not use the dirty root checkout as proof that a release branch is complete.
Do not mix a queue-routing change, storage migration, or feature flag with an
image rollout. First prove the new artifact on the old routing and storage.

## Live verification

Argo being Healthy is necessary, not sufficient. Verify the object actually
running:

```bash
kubectl get application my-apps-radar-ng -n argocd
kubectl get pods -n radar-ng -o wide
kubectl get workerdeployments.temporal.io -n radar-ng
kubectl get deployment tile-server -n radar-ng \
  -o jsonpath='{.spec.template.spec.containers[*].image}'
curl -sS https://radar-ng-api.vanillax.me/api/health | jq .
```

For workers, require every WorkerDeployment to report its target version,
every pod to be Ready, and every container `imageID` to match the reviewed
digest. Confirm workflow and activity pollers on each expected task queue.

Then verify the behavior that justified the release. Examples:

- HRRR: a natural ingest publishes a non-empty consecutive `radar-hrrr`
  manifest prefix and a representative tile returns successfully.
- MRMS/nowcast: freshness stays inside the ten-minute page budget through at
  least two natural cadences.
- Open-Meteo: both containers use the intended compatible artifact, the TCP
  endpoint is Ready, the Temporal activity poller is present, and the next
  natural sync completes.
- tile-server: health, manifest, representative immutable tile, and forecast
  endpoints work through the public route.

Watch restarts, OOMs, Temporal backlog, Schedule reconciliation errors, and the
timer-DLQ metric during the rollout. Keep the old worker version available
while pinned or already-running executions drain.

## Rollback

Revert the GitOps image change to the last known-good immutable tag in every
place it appears, then merge that revert. Do not use `kubectl rollout undo`;
Argo self-heal will restore Git and make the live state misleading.

Rollback must not delete a PVC, recreate a Temporal Schedule, terminate a
workflow, purge a DLQ, or remove last-good tiles. Those are separate operator
decisions with separate recovery plans.

Renovate may propose the newer tag again. Close or hold that PR until a fixed
version supersedes it.

## Recovery and common failures

| Symptom | Safe response |
|---|---|
| Release workflow failed before pushing the version | Fix/re-run the source workflow. Do not invent a tag by hand. |
| A version is needed from `latest` | Use `ghcr-retag-from-latest.yml` only after proving `latest` has the intended OCI source revision. Retagging the wrong `latest` creates false provenance. |
| Renovate did not open a PR yet | Confirm the GHCR tag exists, then wait for hosted Renovate or open a clean manual GitOps PR. Do not wait on the private-registry CronJob. |
| Renovate updated only one worker reference | Do not merge. Update and verify all six worker references together. |
| Git has the new image but Argo renders an old manifest | Request a normal refresh first. If the repo-server cache is proven stale, use `argocd.argoproj.io/refresh=hard`; never restart random workloads to repair Git cache state. |
| Worker resource-only change does not produce a new build ID | Publish a reviewed worker image or use the Worker Controller mechanism documented in GitOps. Do not patch the generated Deployment. |
| New image starts but the feature is still absent | Compare pod `imageID`, OCI source revision, environment flags, task-queue pollers, and the natural product output. A pod name or semver tag alone is not provenance. |

## Special cases

The basemap archive is intentionally hydrated from the existing GHCR image
because the large PMTiles file is not in git. Treat the current image as an
artifact input and verify its digest before rebuilding.

Open-Meteo is a filesystem-coupled, single-pod `Recreate` workload. Keep its
serve and sync binaries on an explicitly compatible image, merge its GitOps
rollout between sync runs, and budget for one cold pull plus Longhorn attach.
Changing it to `RollingUpdate`, splitting the two containers across pods, or
moving its live files to generic NFS is not a routine release optimization.
