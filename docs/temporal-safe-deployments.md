# Temporal worker release contract

Scope: the five versioned worker pools (`mrms`, `nowcast`, `hrrr`, `aux`, `alerts`) and the
storm-watch workflow. Code: `rust/crates/radar-worker/`. Release steps: [releasing.md](releasing.md)
and [deploy/RUNBOOK.md](../deploy/RUNBOOK.md).

## Storm-watch handoff

`WatchStormWorkflow` stays pinned during each run. At a history, time, frame, or version-change
boundary, it waits for handlers and carries a versioned `WatchStormHandoff` with the business state
and all pending alert IDs into Continue-as-New (behind the `watch-storm-handoff-v1` patch marker).
`poll_count` and `push_count` are lifetime counters; a separate local counter limits each run. The
original input fields still work. The workflow's init step restores the handoff before early signals
can arrive.

The server's target-version notification can be present on one workflow task and absent on the next.
The workflow remembers it across activity completions and signals until the safe boundary. Its normal
60-second poll already wakes idle runs; `wakeUpSignal` permits an explicit nudge that does no business
work.

Unknown handoff schema versions fail the execution as non-retryable. Unpin takes precedence over a new
run. Notification retries keep their existing collapse IDs and exhaustion policy; this is not
exactly-once push delivery. In production the watch routes are off (`DISABLE_WORKFLOW_ROUTES=1`).

## Pinned executions

Workflows are `PINNED`: a new image does not move runs already pinned to an older build. Old
executions are disposable. They finish during the controller's `scaledownDelay` or may be terminated;
the next Schedule fire starts fresh on the current build. Because schedules use overlap policy
`SKIP`, one orphaned pinned run blocks every later fire of its schedule, so check for them after a
promotion. The talos-argocd-proxmox radar-ng `RUNBOOK.md` covers finding and clearing orphaned pinned
executions.

Never commit real workflow histories: they can contain coordinates, user IDs, and push data.

## Functional promotion gate

Every role registers `RadarDeploymentSmokeWorkflow`. The Worker Controller runs it on the candidate
build before promotion. Its activity checks the configured palettes, recolours and decodes an indexed
PNG, and probes write, fsync, rename and read-back on the volumes that role needs. Alerts requires
readable grids and writable state; it never writes grids or needs a tiles mount. The ingest and
render roles check all three tiles/grids/state volumes. Scratch files are removed; no tiles are
published and no push is sent. The activity has a 90-second schedule-to-close bound and at most two
attempts.

The gate does not validate external weather feeds or forecast accuracy. It can fail during a storage
outage; investigate the dependency before proposing a new candidate. Do not bypass the gate.

`/tmp/worker-alive` measures local health-loop progress; `/tmp/temporal-healthy` measures Temporal
reachability. The GitOps liveness and readiness probes use different files. Neither proves that the
SDK is polling; check poll metrics and controller registration when queue work stops.

## Validate

From `rust/`:

```bash
cargo test -p radar-worker --lib                          # roles, registrations, config
cargo test -p radar-worker --test replay -- --nocapture   # offline replay gate, per-fixture table
TEMPORAL_TEST_ADDRESS=localhost:7233 cargo test -p radar-worker --test integration
```

The replay gate replays every retained history in
`rust/crates/radar-worker/testdata/replay_fixtures/` with the same workflow registrations the worker
uses, and fails on any nondeterminism. The image build runs it too. The integration tests need a
Temporal server (`deploy/temporal-dev.yml`); they cover schedule seeding, the health probe, visibility
listing and signalling a missing watch. A live Continue-as-New handoff across a real version change is
not covered by an automated test.

The [GitOps safe-deployment runbook](https://github.com/mitchross/talos-argocd-proxmox/blob/main/docs/domains/temporal/safe-deployments.md)
owns configuration retention, digest pins and rollout observation.

Sources: [Temporal upgrade on Continue-as-New](https://docs.temporal.io/production-deployment/worker-deployments/worker-versioning/upgrade-on-continue-as-new),
[pinned-workflow recovery](https://docs.temporal.io/production-deployment/worker-deployments/recover-pinned-workflows).
