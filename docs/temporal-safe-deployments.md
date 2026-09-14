# Temporal worker release contract

Status: implemented in this branch; deployed behavior changes only after the
image and GitOps PR are released. Scope: five versioned Radar worker pools.

## Storm-watch handoff

`WatchStormWorkflow` remains pinned during each run. At a history, time, frame,
or version-change boundary, it waits for handlers and carries a versioned
`WatchStormHandoff` containing business state and all pending alert IDs into
Continue-as-New. `poll_count` and `push_count` are lifetime counters; a separate
local counter limits each run. The four original input fields still work.
`@workflow.init` restores the input before early signals can arrive.

The server's target-version notification can be present on one Workflow Task
and absent on the next. The workflow remembers it across activity completions
and signals until the safe boundary. Its normal 60-second poll already wakes
idle runs. `wakeUpSignal` also permits an explicit, no-business-work nudge.

Upgrade-on-Continue-as-New is an experimental SDK Public Preview option in
Temporal Python 1.30.0. It is not the same as permanently overriding a running
workflow to `AUTO_UPGRADE`. Unknown handoff schema versions fail explicitly.
Unpin takes precedence over a new run. Notification retries retain their
existing collapse IDs and exhaustion policy; this is not exactly-once push
delivery. An alert which exhausted delivery retries is not silently retried
forever by rollover.

## Existing pinned executions

A new image alone cannot upgrade runs pinned to old code. Keep the old workers
and their config alive. The retained historical fixtures must still replay;
the `watch-storm-*` patch markers preserve historical command paths.

For a deliberately selected existing run, export its history to a private
temporary location and replay it with the candidate before any one-time move
to the fix version. Never commit real histories: they can contain coordinates,
user IDs, and push data. Use Temporal's one-time move operation only after
reviewing that execution and candidate; verify its next Continue-as-New carries
state and removes the need for its old version. A sample fixture replay does
not authorize a bulk move. Historical completed handoffs that already discarded
state cannot be reconstructed by this fix.

## Functional promotion gate

Every role registers `RadarDeploymentSmokeWorkflow`. The controller runs it on
the candidate version before promotion. Its activity verifies the actual PNG
palette transformation, configured palette files, role-specific native modules,
and write/fsync/rename/read on each mounted tiles/grids/state volume. Scratch
files are isolated and removed; no tiles are published and no push is sent.
It has a 90-second activity schedule-to-close bound and at most two attempts.

The gate does not validate every external weather feed or the scientific
accuracy of an entire forecast. It can fail during a storage outage; investigate
the dependency before proposing a new candidate. Do not bypass the gate.

`/tmp/worker-alive` measures local health-loop progress; `/tmp/temporal-healthy`
measures Temporal reachability. The GitOps liveness and readiness probes use
different files. Neither readiness nor a successful gRPC health RPC proves
that the SDK is polling; inspect its poll metrics and controller registration
when queue work stops. Schedule recovery remains observe-only.

## Validate from the repository root

```bash
docker build -f temporal/Dockerfile -t radar-worker:test .
docker run --rm --entrypoint sh radar-worker:test -c \
  'pip install pytest==9.0.3 && python -m pytest -q temporal'
```

The handoff test starts a disposable local Temporal server. To reuse one,
provide `TEMPORAL_TEST_ADDRESS`; to pin the executable, provide
`TEMPORAL_TEST_SERVER`. The test promotes V1→V2, buffers two signals while a V1
activity runs, sends another during the new run, and checks state, queue order,
and V2 activity execution. It waits for routing propagation instead of assuming
that promotion immediately refreshes the workflow's target notification.

The [GitOps safe-deployment runbook](https://github.com/mitchross/talos-argocd-proxmox/blob/main/docs/domains/temporal/safe-deployments.md)
owns configuration retention, digest pins, rollout observation and rollback.

Sources: [Temporal upgrade on Continue-as-New](https://docs.temporal.io/production-deployment/worker-deployments/worker-versioning/upgrade-on-continue-as-new),
[pinned-workflow recovery](https://docs.temporal.io/production-deployment/worker-deployments/recover-pinned-workflows),
and [the completed tutorial](https://github.com/mikeacjones/temporal-safe-deploys-lab/tree/main).
