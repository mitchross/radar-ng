# Releasing — code to running pods

Production releases come from GitHub Actions, GHCR, and the `talos-argocd-proxmox` GitOps repository.
Releases are **forward-only**: a bad release is fixed by merging a fix and shipping the next version.
The step-by-step runbook is [deploy/RUNBOOK.md](../deploy/RUNBOOK.md).

## Normal path

```text
merge a tested PR into mitchross/radar-ng:master
  -> the matching .github/workflows/ghcr-*.yml build runs
  -> GHCR receives :vX.Y.Z, :sha-<commit> and :latest; backend runs print tag@digest in the summary
  -> a PR in talos-argocd-proxmox bumps the image tag and digest (Renovate may open it)
  -> Cluster CI validates the GitOps PR
  -> merge the GitOps PR
  -> Argo CD reconciles my-apps-radar-ng
  -> verify live image digests and product behaviour
```

There is no release commit or git tag. Each image workflow reads its existing GHCR semver tags and
allocates the next patch version; the three backend images never go below `v2.0.0`, where the Rust
backend starts. The workflows are serialized per image so two builds cannot choose the same version. `workflow_dispatch` accepts an
explicit version for a deliberate rebuild.

The workflow `on.push.paths` blocks decide which change builds which image. A change under `rust/`
rebuilds all three backend images:

| Image | Workflow | `deploy/Dockerfile` target |
|---|---|---|
| `ghcr.io/mitchross/radar-ng-temporal-worker` | `ghcr-temporal-worker.yml` | `worker` |
| `ghcr.io/mitchross/radar-ng-tile-server` | `ghcr-tile-server.yml` | `api` |
| `ghcr.io/mitchross/radar-ng-open-meteo-worker` | `ghcr-open-meteo-worker.yml` | `open-meteo` |
| `ghcr.io/mitchross/radar-ng-web` | `ghcr-web.yml` | `web/Dockerfile` |

Never deploy `latest`. GitOps pins `vX.Y.Z@sha256:…`.

## Before merging source

1. `ci-rust.yml` must be green: fmt, clippy, tests, the Temporal integration tests, the live-NOAA check,
   and the image builds with their smoke runs. App changes run `ci-frontend.yml`; the web image is
   built by `ghcr-web.yml`.
2. Keep one release per image in flight at a time.
3. For workflow-code changes, the replay gate must pass (below). If it fails, use a reviewed patch
   marker rather than editing fixtures. A unit test is not a history-compatibility proof.

Watch the release:

```bash
gh run list --repo mitchross/radar-ng --branch master --limit 10
gh run watch <run-id> --repo mitchross/radar-ng
```

Read the published version (and, for the backend images, `tag@digest`) from the completed run; do not
predict the tag.

## Temporal worker replay gate

`deploy/Dockerfile` runs `cargo test -p radar-worker --test replay` before building the binary
(`REPLAY_GATE=1`), so no worker image exists unless its workflow code replays every retained history
under `rust/crates/radar-worker/testdata/replay_fixtures/`. CI's `lint-test` job runs the same test. The
images carry the label `io.radar-ng.replay-gate=rust-offline-replay`, which CI checks.

## GitOps rollout

Bump every repeated image reference together: the temporal-worker image appears in all five
WorkerDeployments, and the open-meteo-worker image in both containers of the open-meteo Deployment.
A PR that moves only some of them must not merge.

Do not mix a queue-routing change, storage migration or feature flag with an image bump. Render the
app and check the diff before merging:

```bash
git diff --check
kustomize build my-apps/personal-projects/radar-ng >/tmp/radar-ng.yaml
gh pr checks <pr-number> --repo mitchross/talos-argocd-proxmox --watch
```

Each worker pool runs `RadarDeploymentSmokeWorkflow` on the new build before the Worker Controller
promotes it ([temporal-safe-deployments.md](temporal-safe-deployments.md)). A pool whose smoke fails
stays on its current build until a fixed release replaces the candidate.

## Live verification

Argo being Healthy is necessary, not sufficient:

```bash
radar debug doctor --server https://radar.vanillax.me --temporal-address <temporal-frontend>:7233 --namespace radar-ng
```

Also check that every WorkerDeployment reports its target version, every pod is Ready, and every
container `imageID` matches the published digest, with pollers on each task queue. Then verify the
behaviour that justified the release, for example:

- MRMS/nowcast: freshness stays inside the ten-minute budget through at least two natural cadences.
- HRRR: a natural ingest publishes a non-empty consecutive `radar-hrrr` manifest prefix and a
  representative tile returns.
- Open-Meteo: both containers run the new image, the activity poller is present, and the next natural
  sync completes.
- tile-server: health, manifest, an immutable tile and the forecast endpoint work through the public
  route.

Watch restarts, OOMs, Temporal backlog and Schedule errors during the rollout. Pinned executions
started on the previous build may be terminated; the GitOps repo's radar-ng `RUNBOOK.md` covers
orphaned pinned executions.

## When something goes wrong

| Symptom | Response |
|---|---|
| Release workflow failed before pushing | Fix and re-run the source workflow. Do not invent a tag by hand. |
| A release is bad in production | Fix forward: merge the fix, let the next `vX.Y.Z` publish, bump GitOps. |
| Renovate updated only some references | Do not merge. Update all references together. |
| Git has the new image but Argo renders an old manifest | Request a normal refresh; use a hard refresh only when the repo-server cache is proven stale. |
| Worker resource-only change does not produce a new build ID | Publish a new worker image or use the Worker Controller mechanism documented in GitOps. Do not patch the generated Deployment. |
| New image runs but the feature is absent | Compare pod `imageID`, OCI source revision, env flags, task-queue pollers and the product output. |

## Special cases

The basemap archive is fetched by the `basemap` Deployment's init container from the newest Protomaps
build; it is not an image. Bump its `REFRESH` env to re-extract.

Open-Meteo is a filesystem-coupled, single-pod `Recreate` workload. Keep its serve and sync containers
on the same image, merge its rollout between sync runs, and budget for one cold pull plus volume attach.
