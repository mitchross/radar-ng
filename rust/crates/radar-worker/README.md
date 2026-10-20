# radar-worker

Temporal control plane for radar-ng. It holds the 13 workflow types, the activity implementers that wrap
the sibling ingest, aux, alerts, nowcast and smoke crates, the schedule seeder, the schedule watchdog, and the
worker process. The wire contract (names, payload shapes, retry constants, schedule definitions, queues,
and environment variable names) comes from `radar-types`. Behaviour follows
`docs/develop/specs/spec-temporal.md`. Deviations are listed below.

## Coverage

| Module | Spec | Contents |
|---|---|---|
| `config` | 3.1 | `WorkerConfig::from_env` and the `TEMPORAL_*` and `SEED_SCHEDULES` settings, with defaults and clamps |
| `activities::*` | 4.x, 5.x, 8, 9.2 | 30 activity names in `radar_types::wire` (29 registered by `legacy` and `all`, and `open_meteo_sync` on its own pool), with heartbeats, cancellation, and retryable or non-retryable mapping |
| `workflows::*` | 4.x | 13 workflow types. Activity options come from `radar_types::retry`. Concurrency is bounded deterministically |
| `seed` | 6.4, 6.5 | Create or update all 13 schedules from `radar_types::schedules::all_schedules()`. Retries only retryable RPC codes. Compares before updating |
| `watchdog` | 7 | Stall predicate, per-timer suppression, and the `TEMPORAL_SCHEDULE_STALLED` error log |
| `health` | 9.1 | Two-attempt health probe. Touches the health and alive files every 30 s |
| `temporal_io` | 5.2, 5.3 | `ScheduleKicker` (nowcast kick) and `TemporalWatchSignaler` (alert matching) over the Temporal client |
| `worker` | 3.2 to 3.8 | Role registrations, runtime and metrics, Worker Versioning, supervision, and SIGTERM or SIGINT drain |

## Public API

- `WorkerConfig`, `WorkerConfig::from_env()`: validated settings for one process.
- `run_worker(cfg: WorkerConfig) -> anyhow::Result<()>`: serves the role's queue until SIGTERM or SIGINT.
  Background tasks start after a short grace period (see deviations).
- `seed_schedules(cfg: WorkerConfig) -> anyhow::Result<BTreeMap<String, Outcome>>`: one-shot seeding for an
  operator or job. It ignores `SEED_SCHEDULES`.
- `run_watchdog(cfg: WorkerConfig) -> anyhow::Result<()>`: runs the read-only schedule watchdog forever.
- `registration(role: WorkerRole) -> Registration`: the workflow and activity names a role registers.
- `worker::register_workflows(target, role)`: registers the role's workflow implementations on any
  `worker::WorkflowTarget` (`WorkerOptions` or the SDK's `WorkflowReplayerOptions`). The worker and the replay
  gate share it.
- `workflows::OptionalArg<T>`: an optional single workflow argument that also accepts a start with no payloads,
  as Python's `args: T | None = None` does.
- `SeedError`, `Outcome`: seeding failure and per-schedule result.

## Role to registrations

Every role except `legacy` and `all` also registers `RadarDeploymentSmokeWorkflow` and
`radar_deployment_smoke`. The open-meteo pool is not a `WORKER_ROLE` key in Python. It is selected here
with `WORKER_ROLE=open-meteo`, and it serves only `open_meteo_sync`.

| Role | Queue (default) | Workflows | Activities |
|---|---|---|---|
| `legacy`, `all` | `radar-ng` | all 13 types | 29 activities. Excludes `open_meteo_sync` |
| `mrms` | `radar-ng-mrms` | `IngestMrmsWorkflow`, smoke | `mrms_list_unprocessed_keys`, `mrms_process_frame`, `mrms_mark_processed`, `mrms_cleanup`, smoke |
| `nowcast` | `radar-ng-nowcast` | `NowcastWorkflow`, smoke | `nowcast_run`, smoke |
| `hrrr` | `radar-ng-hrrr` | `IngestHrrrWorkflow`, smoke | `hrrr_find_latest_run`, `hrrr_horizon_for_run`, `hrrr_process_forecast_hour`, `hrrr_publish_run`, `hrrr_mark_processed`, `hrrr_cleanup`, smoke |
| `aux` | `radar-ng-aux` | `IngestLightningWorkflow`, `IngestTropicalWorkflow`, `IngestAirQualityWorkflow`, `TileCleanupWorkflow`, `OpenMeteoSyncWorkflow`, smoke | `lightning_consume_stream`, `tropical_fetch_and_publish`, `aqm_find_latest_run`, `aqm_render_chunk`, `aqm_publish_run`, `aqm_mark_processed`, `aqm_cleanup`, `tile_cleanup_sweep`, smoke |
| `alerts` | `radar-ng-alerts` | `PollAlertsWorkflow`, `WatchStormWorkflow`, `RegisterPushTokenWorkflow`, `DeletePushTokenWorkflow`, smoke | `persist_push_token`, `delete_push_token`, `compare_radar_frames`, `detect_storm_change`, `fan_out_push_to_user`, `fetch_nws_active_alerts`, `mark_alerts_seen`, `signal_matching_storm_watches`, `send_push_notification`, smoke |
| `open-meteo` | `radar-ng-open-meteo` (`TEMPORAL_TASK_QUEUE` overrides) | none | `open_meteo_sync` |

The table is enforced by unit tests (`worker::tests`). Those tests check exact pool contents, that each
workflow and activity has one owner (smoke has five), and that every name matches a `radar_types::wire`
constant.

## Running the tests

Unit tests and the replay gate need no server:

```text
cargo test -p radar-worker --lib
cargo test -p radar-worker --test replay -- --nocapture   # prints the per-fixture table
```

Integration tests need a Temporal server. Each test returns early and passes, with a skip message, when
`TEMPORAL_TEST_ADDRESS` is unset. The seeding test creates and updates real schedules, so use a disposable
server:

```text
temporal server start-dev
TEMPORAL_TEST_ADDRESS=localhost:7233 cargo test -p radar-worker --test integration
```

Set `TEMPORAL_TEST_NAMESPACE` to use a namespace other than `default`.

## Worker Versioning

Versioning turns on only when both `TEMPORAL_DEPLOYMENT_NAME` and `TEMPORAL_WORKER_BUILD_ID` are non-empty;
in production the Temporal Worker Controller sets them. Workflows default to `PINNED` (spec 3.6): a run stays
on the build that started it. Only new work, and Continue-as-New with `AUTO_UPGRADE` (the WatchStorm handoff),
moves to a new build. The controller promotes a build after `RadarDeploymentSmokeWorkflow` passes on it.
Operator notes: `docs/develop/temporal-safe-deployments.md` and `deploy/RUNBOOK.md`.

## Deviations from the spec

1. **Supervision.** The SDK's `Worker::run` future is not `Send`, so it runs on the supervising task. The SDK
   exposes no running-state check, so background work starts after `POLLER_START_GRACE` (250 ms) instead of
   the Python `is_running` poll. A worker that exits within that window starts no background work. Background
   work is aborted when the worker returns.
2. **Tropical activity thread.** `radar_aux::tropical::fetch_gis` has a closure whose elided `&str` tuple
   element is not `Send` for every lifetime. The activity macro rejects it. The activity therefore runs its
   future on a blocking thread with `Handle::block_on`. Cancelling the activity stops waiting on it, but the
   thread finishes on its own, bounded by the HTTP timeouts. The fix belongs in radar-aux, with the closure
   annotated. This is marked `TODO(integration)` in `activities/aux.rs`.
3. **Schedules use raw WorkflowService RPCs.** The high-level schedule builder cannot set
   `workflow_execution_timeout` or `catchup_window`. Compare-before-update projects the live schedule into a
   comparable form. It normalises timezone `""` and `UTC`, maps unspecified overlap to `SKIP`, and ignores
   zero phase and jitter. Paused state, notes, and limited actions come from the live schedule, so
   operator-set values are kept.
4. **WatchStorm schema check.** The `WatchStormHandoff` schema check runs at the start of `run`, not in
   `__init__` (spec 4.11.1). An unknown schema fails the execution as non-retryable. The check is the same,
   but its failure point differs.
5. **`seed_schedules` returns `anyhow::Result`.** A connection failure surfaces as its real cause, and seeding
   failures carry the `SeedError`.
6. **Local-activity and Nexus slots.** The Python worker does not set them. This port uses 100, which is an
   assumed SDK default that has not been checked against the Python SDK.
7. **Open-meteo pool.** It runs through `run_worker` with role `open-meteo`, not as a separate binary (spec D1
   allows this). It registers no metrics or tracing and uses a 25 s drain. It runs the same health loop as the
   main pool, from the start, so `/tmp/worker-alive` is touched every 30 s during a sync (F2).
8. **Deployment smoke.** The palette, PNG, grid and volume checks are native. The Python gate also imported
   pysteps and ecCodes; the Rust build has no such runtime modules, so those checks have no counterpart.
9. **Metrics.** Prometheus is served at `TEMPORAL_METRICS_BIND` (default `0.0.0.0:9464`, `off` disables it).
   OTLP tracing is deferred.

## Review fixes (docs/archive/rust-migration/REVIEW.md)

- **F2 open-meteo liveness.** `run_open_meteo_pool` starts `health_file_loop` before the poller. The loop is its
  own task, so it keeps touching the alive file during a sync of up to about 35 minutes. The
  open-meteo image target copies in busybox, and the probe timings are looser (`deploy`). Not covered by a
  test: the loop needs a live Temporal client.
- **F3 bounded fan-out.** `workflows::bounded` is a hand-written sliding window in input order, not `buffered`.
  A slot frees when any in-flight item finishes, as `Semaphore` plus `gather` does in Python. Results come back in
  input order. The window uses no futures-util combinators, so the SDK's deterministic-polling rules are the only
  ones that apply. Command order (spec 10.2): the first `limit` items are scheduled in input order, and each later
  item is scheduled when a slot frees, in the same workflow task that processed the completion.
  - **Replay.** Completion order comes from history, so replay of this code is deterministic. The window is
    also what Python's `Semaphore(N)` plus `gather` does (FIFO waiters, the next item is scheduled in the task
    that processed the completion), and every recorded Python history of MRMS, HRRR and AQM replays on it,
    v1.1.49 included (see "Replay" below). Only runs started on an earlier Rust build that used `buffered`
    could diverge. That matters only if such a build ever served a real namespace. No new patch ID was
    added, because spec 10.2 forbids that without a matching existing one.
  - Tests: a freed slot starts the next item while a slow head is still pending, the limit is never exceeded,
    results stay in input order, and a zero limit is treated as one.
- **F8 schedule arguments.** `json_payload` writes canonical JSON with sorted keys at every depth, as Python's
  `json.dumps(sort_keys=True)` does. `comparable()` compares the arguments as parsed JSON values, so key order
  does not count as a change, and re-seeding does not rewrite unchanged schedules.

## Replay

`tests/replay.rs` is the offline determinism gate. It needs no Temporal server. It replays the histories in
`testdata/replay_fixtures/<WorkflowType>/<version>/<scenario>.json[.gz]`, which were recorded from the Python
worker releases (see `testdata/replay_fixtures/README.md`). There are 143 histories: versions
v1.1.26, v1.1.27 and v1.1.49 for every workflow, plus `src-e8c8659` for WatchStorm and the deployment smoke
workflow.

What it checks:

- `fixture_tree_matches_registry`: the fixture tree, `REQUIRED_SCENARIOS` and the `all` registration name the
  same 13 workflow types. It also checks the version regex, that labels are unique, and that the first event
  starts the directory's workflow type.
- `required_scenarios_are_retained_and_prove_their_path`: `REQUIRED_SCENARIOS` is carried over from the Python gate.
  Every release keeps every required scenario, and each history contains the event types or scheduled
  activity names that prove its path.
- `every_fixture_replays`: replays every history with the SDK's `WorkflowReplayer`. The workflows are
  registered by `worker::register_workflows`, which is the same function `radar worker` uses. Each history is
  replayed once for every role that registers its type (`legacy`, `all`, and the dedicated pool). A
  nondeterminism error or a workflow-task failure fails the fixture. The test prints a per-fixture and a
  per-version table.
- `a_tampered_history_is_reported_as_nondeterministic`: a history with a renamed activity must fail. This
  shows the gate is not vacuous.

`ALLOWLIST` in the test holds fixtures that are known not to replay, each with its reason. It is empty. An
allowlisted fixture that starts replaying fails the test, so the list cannot go stale.

JSON decoding: the histories are proto3 JSON (camelCase fields, enum names, base64 payload `data`, int64 as
strings). In temporalio-protos 0.10 the history messages derive plain serde (snake_case fields, numeric enums),
so `WorkflowHistory::from_json` cannot read them. The test decodes them with `prost-reflect` against the
SDK's own descriptor set, then re-decodes the binary encoding into `History`. Unknown fields are rejected. The
descriptor set reaches the test through `build.rs`, which reads `DEP_TEMPORALIO_PROTOS_DESCRIPTOR_PATH`. That
is why `temporalio-protos` is a direct dependency; it is the same version the SDK already uses.

Result: all 143 histories replay, with no allowlist.

| Workflow | v1.1.26 | v1.1.27 | v1.1.49 | src-e8c8659 |
|---|---|---|---|---|
| IngestMrmsWorkflow | 5/5 | 5/5 | 5/5 | |
| IngestHrrrWorkflow | 7/7 | 7/7 | 7/7 | |
| IngestAirQualityWorkflow | 6/6 | 6/6 | 6/6 | |
| PollAlertsWorkflow | 5/5 | 5/5 | 5/5 | |
| WatchStormWorkflow | 7/7 | 7/7 | 7/7 | 7/7 |
| Lightning, Tropical, Nowcast, TileCleanup, RegisterPushToken, DeletePushToken, OpenMeteoSync | 2/2 each | 2/2 each | 2/2 each | |
| RadarDeploymentSmokeWorkflow | | | 2/2 | 2/2 |

Two workflow fixes were needed. Before them, 23 histories failed: 3 MRMS and 20 WatchStorm, all versions.

1. **WatchStorm poll wait.** `wait_for_poll` used `select!` over `wait_condition` and `ctx.timer`. The SDK
   pushes the `StartTimer` command when `ctx.timer` is called, and dropping the future does not cancel it.
   When an unpin or an alert arrived during an iteration, Rust recorded a 60 s timer that Python never sends.
   That failed `signal-unpin`, `timer-poll`, `push-change`, `push-activity-error` and `alert-signal-push`
   with `Timer machine does not handle this event`. The condition branch now cancels the timer. This matches
   Python's `wait_condition(fn, timeout=...)`, which creates the timer first and so uses up a timer sequence
   number, then cancels it. A timer cancelled in the same activation is never sent: Core drops it, for both
   SDKs. A timer cancelled in a later task records `TimerCanceled` before the next iteration's commands. This
   was also a live defect: an in-flight Python execution signalled mid-iteration would have hit a
   nondeterminism error after cutover.
2. **MRMS optional input.** Python's `run(self, args: IngestMrmsArgs | None = None)` accepts a start with no
   arguments. The SDK decodes `Option<T>` only from exactly one payload. `empty-backlog` (no input) therefore
   failed with `Failed to create workflow execution`. The input is now `workflows::OptionalArg<IngestMrmsArgs>`,
   which accepts zero payloads, `binary/null`, or one `json/plain` payload. It has unit tests in
   `workflows::optional_arg_tests`.

Not covered by fixtures (reasoned, not proven):

- Out-of-order completions in the bounded fan-outs. The synthetic activities complete in schedule order, so
  the fixtures cannot tell the F3 sliding window from the old `buffered` window. The sliding window follows
  Python's semaphore semantics, so it is the version expected to match.
- A WatchStorm timer cancelled in a later task (`TimerCanceled`), and a timer-ID gap after an iteration whose
  wait was satisfied at once. No fixture records either case.
- Continue-as-new with the handoff path under a real version change. The Python handoff test needed a
  server, and it has no Rust port.

## Known gaps and TODOs

- **Live workflow scenario tests are not implemented.** Replay compatibility with the recorded Python
  histories is verified offline (see "Replay"). The integration suite covers schedule seeding (idempotent
  re-seed), the health probe, visibility listing, and signalling a missing watch. Running the spec 10.1
  scenarios against a server with fake activities, and porting the WatchStorm version-handoff test, are still
  open.
- **Integration tests** run in CI (`ci-rust.yml`, job `temporal`) against `deploy/temporal-dev.yml`. The job
  fails if they take the skip path.
- **`ScheduleKicker` ignores the `timeout` argument.** `radar-ingest` wraps the call in its own timeout.
- **Spec section 13 items marked `[VERIFY-SDK]` are unverified.** These are `RetryPolicy` defaults, schedule run
  ID format, `AUTO_UPGRADE` behaviour, and heartbeat payload shapes.
- **Spec D3 and D19.** The API routes watches to the alerts queue regardless of `USE_ISOLATED_TASK_QUEUES`.
  The worker does not detect a missing alerts pool. That check belongs in the API or the deployment.
- **Spec D15.** APNS collapse IDs are truncated as in Python (kept for compatibility).
- **Deferred:** OTLP tracing.
