# radar-ng Temporal control plane: behavioral spec for the Rust port

Generated 2026-10-13. Source of truth: `/mnt/user-data/uploads/radar-ng` (read-only).
Python pin: `temporalio==1.30.0` (`temporal/requirements.txt`, Rust Core commit reviewed there).
Rust SDK present in tree: `/home/claude/rust-port/vendor-src/temporalio-*-1.1.0`. The crate version
and the Python Core pin are different release trains. Confirm the Rust SDK build implements the same
Core behavior (worker versioning, Continue-as-New `AUTO_UPGRADE`, patching) before depending on it.

Tags used below: `[VERIFY-SDK]` = a Python SDK default or decoding detail that could not be checked
locally (PyPI was unreachable in the sandbox). Everything else was read from source or tests.

---

## 0. Status and blocking gaps

Read in full: `temporal/worker.py`, `temporal/open_meteo_worker.py`, `temporal/task_queues.py`,
`temporal/README.md`, `temporal/Dockerfile`, `temporal/requirements.txt`, all `temporal/workflows/*.py`,
`temporal/schedules/{seed,watchdog}.py`, `temporal/shared/{push,health,otel,deployment_smoke}.py`,
all `temporal/**/test_*.py`, `docs/temporal-safe-deployments.md`, `docs/kubernetes.md`,
`deploy/docker-compose.yml`, `deploy/k8s/*.yaml`, `deploy/k8s/README.md`, `deploy/.env.example`,
`deploy/temporal-dev.yml`, `tools/debug_harness/temporal_checks.py`,
`backend/api/api/storm_watch_activities.py`.

Read partially (only what the control plane touches):
- `backend/api/api/routes_workflows.py` (watch start/unpin/state paths, queue constant, ID scheme)
- `backend/api/api/temporal_client.py` (full, 36 lines)
- `backend/ingest_*/activities.py`, `backend/nowcast/activities.py`, `backend/tile_cleanup/activities.py`,
  `backend/open_meteo_sync/activities.py`: activity names, signatures, payload dataclasses only.
  Activity bodies are out of scope.
- `backend/shared/storm_watch.py`: change-kind names and thresholds only.
- `backend/shared/push_tokens.py`: not read. Assumed `list_for_user(user_id)` returns objects with
  `user_id`, `token`, `platform`; `upsert`, `delete_by_token`, `delete_for_user`.

**Blocking gaps in the upload (the port cannot be validated against recorded histories as delivered):**
1. `temporal/workflows/replay_fixtures/` is absent. Referenced by `test_replay.py:27`,
   `test_workflow_discovery.py:25`, `temporal/README.md`, and the Dockerfile release gate
   (`temporal/Dockerfile:70`, `python -m unittest temporal.workflows.test_replay ...`). The image build
   fails as delivered, and the fixture-tree assertions cannot pass. The Rust port needs the fixtures
   from the source of record before it can claim replay compatibility. The fixture contract is in §10.
2. `temporal/open_meteo_worker.Dockerfile` is absent. Referenced by `deploy/docker-compose.yml:168`
   and `docs/kubernetes.md`.

---

## 1. Conventions

### 1.1 Wire names

| Kind | Exact string | Defined at |
|---|---|---|
| Workflow type | `RadarDeploymentSmokeWorkflow` | `workflows/deployment_smoke.py:9` (no `name=`, class name) |
| Workflow type | `IngestMrmsWorkflow` | `workflows/ingest_mrms.py:76` |
| Workflow type | `IngestHrrrWorkflow` | `workflows/ingest_hrrr.py:79` |
| Workflow type | `IngestAirQualityWorkflow` | `workflows/ingest_airquality.py:71` |
| Workflow type | `IngestLightningWorkflow` | `workflows/ingest_lightning.py:37` |
| Workflow type | `IngestTropicalWorkflow` | `workflows/ingest_tropical.py:30` |
| Workflow type | `NowcastWorkflow` | `workflows/nowcast.py:26` |
| Workflow type | `TileCleanupWorkflow` | `workflows/tile_cleanup.py:29` |
| Workflow type | `OpenMeteoSyncWorkflow` | `workflows/open_meteo_sync.py:37` |
| Workflow type | `PollAlertsWorkflow` | `workflows/poll_alerts.py:60` |
| Workflow type | `WatchStormWorkflow` | `workflows/watch_storm.py:88` |
| Workflow type (compat) | `RegisterPushTokenWorkflow` | `workflows/register_push_token.py:41` |
| Workflow type (compat) | `DeletePushTokenWorkflow` | `workflows/register_push_token.py:53` |

Registry: `workflows/__init__.py` `WORKFLOW_REGISTRY` (13 entries). Discovery test asserts the registry equals
the AST-discovered `@workflow.defn` set.

Activity names (explicit `@activity.defn(name=...)`):

| Activity | Defined at |
|---|---|
| `radar_deployment_smoke` | `shared/deployment_smoke.py:72` |
| `mrms_list_unprocessed_keys`, `mrms_process_frame`, `mrms_mark_processed`, `mrms_cleanup` | `backend/ingest_mrms/activities.py:310,329,495,504` |
| `hrrr_find_latest_run`, `hrrr_horizon_for_run`, `hrrr_process_forecast_hour`, `hrrr_publish_run`, `hrrr_mark_processed`, `hrrr_cleanup` | `backend/ingest_hrrr/activities.py:878,900,906,1115,1124,1133` |
| `aqm_find_latest_run`, `aqm_render_chunk`, `aqm_publish_run`, `aqm_mark_processed`, `aqm_cleanup` | `backend/ingest_airquality/activities.py:364,377,509,515,524` |
| `lightning_consume_stream` | `backend/ingest_lightning/activities.py:256` |
| `tropical_fetch_and_publish` | `backend/ingest_tropical/activities.py:165` |
| `nowcast_run` | `backend/nowcast/activities.py:356` |
| `tile_cleanup_sweep` | `backend/tile_cleanup/activities.py:123` |
| `open_meteo_sync` | `backend/open_meteo_sync/activities.py:58` |
| `persist_push_token`, `delete_push_token` | `backend/api/api/storm_watch_activities.py:57,62` |
| `compare_radar_frames`, `detect_storm_change` | `storm_watch_activities.py:91,130` |
| `fan_out_push_to_user` | `storm_watch_activities.py:162` |
| `fetch_nws_active_alerts`, `mark_alerts_seen`, `signal_matching_storm_watches` | `storm_watch_activities.py:266,338,435` |
| `send_push_notification` | `shared/push.py:209` (registered, never scheduled by a workflow; see §8.1) |

Signals and queries (names are the wire contract, exact case):
- `unpinSignal()`: no arguments.
- `wakeUpSignal()`: no arguments.
- `alertMatchSignal(alert_id: str)`: one positional string.
- Query `getCurrentState()`: returns `WatchStormState`.

Patch IDs (`workflow.patched`): `watch-storm-alert-queue-v1`, `watch-storm-version-boundary-v1`,
`watch-storm-handoff-v1`. Details in §4.11.

Workflow IDs:
- Watch: `"watch:" + hex(sha256(user_id + "\0" + storm_cell_id))`. Authoritative, from
  `routes_workflows._watch_id`. The docstring in `watch_storm.py:12` says `watch:{user_id}:{storm_cell_id}`.
  That docstring is stale. Use the sha256 form for in-flight compatibility.
- Schedule action prefix: `"sched-" + schedule_id`. The server appends a per-fire suffix to form each
  run's ID `[VERIFY-SDK]`.

### 1.2 Payload encoding

- No custom data converter exists anywhere (`grep DataConverter|payload_converter` finds only a debug
  harness read). Use the default converter behavior.
- Every value is one `Payload` with metadata `encoding = "json/plain"`. `None` encodes as `binary/null`
  `[VERIFY-SDK]`.
- Dataclasses encode as JSON objects with exactly the declared field names (snake_case, no aliases,
  no camelCase). Nested dataclasses encode as nested objects. `Optional[T]` is `null` when unset.
  Whether `None` fields are emitted or omitted `[VERIFY-SDK]`. The Rust decoder must accept both.
- Decoding is driven by the workflow/activity type hints.
  - Missing keys take the Python default. Rust: `#[serde(default)]` on every field that has a Python default.
  - Unknown keys: tolerate (ignore) `[VERIFY-SDK]`.
  - `float` fields accept JSON integers (the test sends `curr_max_dbz=42`).
- Non-finite floats: Python's `json` writes `NaN`/`Infinity`, which serde_json rejects. Current code
  never emits them. `compare_radar_frames` maps `-inf` to `-99.0`. The Rust encoder must never emit
  them. The Rust decoder should use a lenient parser if any retained history contains them, which the
  missing fixtures prevent checking.
- Positional arguments are separate payloads in one `Payloads` list. Example:
  `hrrr_process_forecast_hour(run_id, fhr)` is 2 payloads. Schedule `args` are one payload per entry.
  A missing args list decodes to the Python default (`IngestMrmsWorkflow.run(args=None)`).
- Schedule-supplied dict args (`{"mrms_prefix": ...}`, `{"model": ...}`) are decoded into the typed
  dataclass by the type hint. Keys must match the field names exactly.
- The API starts `WatchStormWorkflow` with a plain dict that omits `handoff`. Decode it as
  `handoff = None`.
- Failures: `ApplicationError(non_retryable=True)` is the only non-retryable marker. No
  `non_retryable_error_types` are configured on any `RetryPolicy`. Do not depend on Failure `type` strings
  (none are set explicitly).

### 1.3 Retry policy notation

`RP(init, coeff, max, attempts)`. `coeff` is always `2.0` where written.
Where a Python `RetryPolicy` sets only `maximum_attempts`, the unset fields take the Python default.
`initial=1s`, `coeff=2.0`, `max=100 x initial` `[VERIFY-SDK]`. In Rust, write every field explicitly
and use the values here.

### 1.4 Error semantics

- Workflow code calls `execute_activity` and gets an `ActivityError` after retries are exhausted
  (or on timeout). The workflow catches only `ActivityError`. Anything else fails the execution.
- Exhaustion of a non-caught activity fails the whole workflow execution. There is no automatic
  restart. This matters for WatchStorm (§4.11) and PollAlerts (§4.10).
- Timeouts are per call: `start_to_close` (per attempt), `schedule_to_close` (across all attempts
  and queue wait), `heartbeat_timeout` (activity must heartbeat). Where a call omits a timeout, the
  table says so.
- Concurrency: `asyncio.Semaphore(N)` plus `asyncio.gather` over keys. Results are returned in
  input order. Command order matters for replay: activities are scheduled in the same order the
  Python code schedules them (see §10.3).

### 1.5 Global non-goals of the control plane

- No child workflows anywhere (confirmed by sweep).
- No Updates (no `workflow.update`).
- No timers other than `wait_condition(..., timeout=...)` (WatchStorm) and `asyncio.sleep` in the
  worker/seeder, which are not workflow code.
- No Temporal-side sleeps in ingest workflows.

---

## 2. Task queues and routing

### 2.1 Queue constants (`temporal/task_queues.py`)

| Constant | Value | Served by role | Notes |
|---|---|---|---|
| `LEGACY_TASK_QUEUE` | `radar-ng` | `legacy`, `all` | Default queue of the all-in-one pool. Used by all schedules while `USE_ISOLATED_TASK_QUEUES != "1"`. |
| `MRMS_TASK_QUEUE` | `radar-ng-mrms` | `mrms` | |
| `NOWCAST_TASK_QUEUE` | `radar-ng-nowcast` | `nowcast` | |
| `HRRR_TASK_QUEUE` | `radar-ng-hrrr` | `hrrr` | |
| `AUX_TASK_QUEUE` | `radar-ng-aux` | `aux` | Default queue for schedules that do not name one. |
| `ALERTS_TASK_QUEUE` | `radar-ng-alerts` | `alerts` | Also the API's WatchStorm start queue (see below). |
| `OPEN_METEO_TASK_QUEUE` | `radar-ng-open-meteo` | open-meteo pool only | Activity `open_meteo_sync` always targets it explicitly. |

### 2.2 Routing rules (exact)

- Worker poll queue: `env TEMPORAL_TASK_QUEUE` if set, else the role's default queue (`worker.py:383`).
  An override must match what the schedules and API target.
- Schedule action queue (`seed.py:297-301`):
  `s.task_queue if env USE_ISOLATED_TASK_QUEUES == "1" else LEGACY_TASK_QUEUE`.
  The `ScheduleDef` default `task_queue` is `AUX_TASK_QUEUE`.
- Activity queue: inherits the calling workflow's queue, except where a call names one explicitly.
  Only `open_meteo_sync` (`open_meteo_sync.py:43`) names one. `radar_deployment_smoke` inherits.
- API WatchStorm start (`routes_workflows.py:33,177`): queue = `env TEMPORAL_ALERTS_TASK_QUEUE`,
  default `radar-ng-alerts`. This ignores `USE_ISOLATED_TASK_QUEUES`. See drift D3 in §11.

### 2.3 Role-to-queue summary

The five versioned pools are `mrms`, `nowcast`, `hrrr`, `aux`, `alerts`. The open-meteo pool is a
separate sixth pool with its own image. The all-in-one `legacy` pool serves `radar-ng`.

---

## 3. Worker process (`temporal/worker.py`, `temporal/open_meteo_worker.py`)

### 3.1 Environment variables (names, defaults, effects)

| Variable | Default | Effect |
|---|---|---|
| `TEMPORAL_ADDRESS` | `localhost:7233` | gRPC target. No TLS, no API key. |
| `TEMPORAL_NAMESPACE` | `default` | |
| `WORKER_ROLE` | `legacy` | Lowercased and stripped. Must be a key of `ROLE_CONFIG`: `legacy`, `all`, `mrms`, `nowcast`, `hrrr`, `aux`, `alerts`. **`open-meteo` is documented in `docs/configuration.md:62` but is not a key. The worker raises `ValueError` and exits (drift D1).** |
| `TEMPORAL_TASK_QUEUE` | role default | Poll queue override. |
| `SEED_SCHEDULES` | unset | `"1"` forces seeding. Unset means seed only for `legacy`/`all`. Any other set value (including `"0"`, `"true"`) disables seeding. |
| `SKIP_SCHEDULE_SEED` | unset | `"1"` disables seeding even if otherwise enabled. Any other value has no effect. |
| `TEMPORAL_DEPLOYMENT_NAME`, `TEMPORAL_WORKER_BUILD_ID` | unset | Both must be non-empty to enable Worker Versioning. Otherwise versioning is off. |
| `TEMPORAL_MAX_CONCURRENT_ACTIVITIES` | `4` (min 1) | Passed to `Worker`. Invalid int: warning and default. |
| `TEMPORAL_MAX_CONCURRENT_ACTIVITY_TASK_POLLS` | `2` (min 1) | Passed to `Worker`. |
| `TEMPORAL_GRACEFUL_SHUTDOWN_S` | `25` (min 1) | `graceful_shutdown_timeout`. Must stay below pod `terminationGracePeriodSeconds`. |
| `TEMPORAL_METRICS_BIND` | `0.0.0.0:9464` | Prometheus bind for the SDK runtime. `off` (case-insensitive) disables it. |
| `OTEL_EXPORTER_OTLP_ENDPOINT` | unset | If set, OTLP gRPC BatchSpanProcessor. The tracer provider is always installed. |
| `PUSH_DISABLED` | `"1"` | Enables the no-op push path only when the value is exactly `"1"`. Any other value, including `""`, `"0"`, `"true"`, enables real APNS/FCM sends (drift D5). |
| `TEMPORAL_HEALTH_FILE` | `/tmp/temporal-healthy` | Readiness-style touch file (§9.1). |
| `TEMPORAL_ALIVE_FILE` | `/tmp/worker-alive` | Liveness-style touch file. |
| `USE_ISOLATED_TASK_QUEUES` | unset | `"1"` routes schedules to role queues. See §2.2. |
| `TEMPORAL_ALERTS_TASK_QUEUE` | `radar-ng-alerts` | API only. WatchStorm start queue. |
| `WORKER_ROLE`-dependent smoke vars | see §9.2 | Read by the deployment smoke activity. |

Client construction (`worker.py:369-374`): `Client.connect(target, namespace=..., interceptors=[TracingInterceptor()], runtime=<PrometheusRuntime or None>)`. No TLS argument, no `api_key`, no data converter.

### 3.2 Role table (`ROLE_CONFIG`, `worker.py:140-216`)

Every role except `legacy`/`all` also gets `RadarDeploymentSmokeWorkflow` and `radar_deployment_smoke`
appended (`worker.py:210-216`). `legacy` and `all` already include them through `ALL_WORKFLOWS` and
`ALL_ACTIVITIES`.

| Role | Queue | Workflows | Activities |
|---|---|---|---|
| `legacy`, `all` | `radar-ng` | all 13 registry types | `ALL_ACTIVITIES` (29 names). Excludes `open_meteo_sync`. |
| `mrms` | `radar-ng-mrms` | `IngestMrmsWorkflow` | `mrms_list_unprocessed_keys`, `mrms_process_frame`, `mrms_mark_processed`, `mrms_cleanup` |
| `nowcast` | `radar-ng-nowcast` | `NowcastWorkflow` | `nowcast_run` |
| `hrrr` | `radar-ng-hrrr` | `IngestHrrrWorkflow` | `hrrr_find_latest_run`, `hrrr_horizon_for_run`, `hrrr_process_forecast_hour`, `hrrr_mark_processed`, `hrrr_publish_run`, `hrrr_cleanup` |
| `aux` | `radar-ng-aux` | `IngestLightningWorkflow`, `IngestTropicalWorkflow`, `IngestAirQualityWorkflow`, `TileCleanupWorkflow`, `OpenMeteoSyncWorkflow` | `lightning_consume_stream`, `tropical_fetch_and_publish`, `aqm_find_latest_run`, `aqm_render_chunk`, `aqm_publish_run`, `aqm_mark_processed`, `aqm_cleanup`, `tile_cleanup_sweep` |
| `alerts` | `radar-ng-alerts` | `PollAlertsWorkflow`, `WatchStormWorkflow`, `RegisterPushTokenWorkflow`, `DeletePushTokenWorkflow` | `persist_push_token`, `delete_push_token`, `compare_radar_frames`, `detect_storm_change`, `fan_out_push_to_user`, `fetch_nws_active_alerts`, `mark_alerts_seen`, `signal_matching_storm_watches`, `send_push_notification` |
| open-meteo pool (separate module) | `radar-ng-open-meteo` (env `TEMPORAL_TASK_QUEUE` overrides) | none | `open_meteo_sync` only |

Notes:
- `OpenMeteoSyncWorkflow` runs on the aux or legacy queue. Its activity runs on the open-meteo queue.
  The open-meteo pool must run in both legacy and isolated modes.
- The open-meteo worker (`open_meteo_worker.py`) passes `workflows=[]`, uses the same versioning
  block, `graceful_shutdown_timeout=25s`, and does not install the tracing interceptor, runtime, or
  concurrency overrides (SDK defaults apply).

### 3.3 Startup sequence (`worker.py:363-449`)

```
main():
  target = env TEMPORAL_ADDRESS or "localhost:7233"
  ns     = env TEMPORAL_NAMESPACE or "default"
  interceptor = init_tracer()          # installs global tracer provider; OTLP only if endpoint set
  client = Client.connect(target, namespace=ns, interceptors=[interceptor], runtime=runtime_from_env())
  role = lower(strip(env WORKER_ROLE or "legacy"))
  if role not in ROLE_CONFIG: raise ValueError  # process exits before polling
  (default_queue, workflows, activities) = ROLE_CONFIG[role]
  queue = env TEMPORAL_TASK_QUEUE or default_queue
  flag = env SEED_SCHEDULES                    # None | "1" | other
  should_seed = (flag == "1") or (flag is None and role in {"legacy","all"})
  should_seed = should_seed and (env SKIP_SCHEDULE_SEED != "1")
  deployment  = versioning_from_env()          # None unless both vars non-empty
  max_act     = int_env("TEMPORAL_MAX_CONCURRENT_ACTIVITIES", 4, min=1)
  max_polls   = int_env("TEMPORAL_MAX_CONCURRENT_ACTIVITY_TASK_POLLS", 2, min=1)
  worker = Worker(client, task_queue=queue, workflows=..., activities=...,
                  deployment_config=deployment, max_concurrent_activities=max_act,
                  max_concurrent_activity_task_polls=max_polls,
                  graceful_shutdown_timeout=int_env("TEMPORAL_GRACEFUL_SHUTDOWN_S", 25, min=1) seconds)
  on SIGTERM or SIGINT: schedule worker.shutdown()        # see §3.5
  run_supervised(worker, client, should_seed)
  log "worker drained, exiting"
```

`int_env` returns the default for an absent or unparsable value, logs a warning for unparsable, and
applies `max(minimum, value)`.

### 3.4 Supervision (`_run_worker`, `worker.py:298-360`)

```
run_supervised(worker, client, should_seed):
  worker_task = spawn(worker.run())
  loop:                                       # _wait_for_worker_pollers
    if worker_task done: await it (propagates); return WITHOUT starting background work
    if worker.is_running: yield once (sleep 0); if worker_task done -> propagate; break
    sleep 10 ms
  background = [spawn(health_file_loop(client))]
  if should_seed:
    background += [spawn(reconcile_schedules_once(client)), spawn(watch_schedules(client, SCHEDULES))]
  await worker_task
  finally: cancel all background; cancel worker_task if not done; gather(..., return_exceptions=True)
```

Invariants:
- Validation or startup failure starts no background work and propagates the exception. The
  reconciler and the watchdog never run on a failed startup.
- Reconciliation failure is never fatal. The worker keeps polling. It is a one-shot per process start:
  the seeder does not reconcile periodically (drift vs definitions is corrected only on restart or
  rollout).
- The health loop and the seeder both start only after `is_running` and one yield (test
  `test_reconciliation_waits_for_validation_and_poller_start`).

### 3.5 Graceful shutdown

- SIGTERM and SIGINT both call `worker.shutdown()` through `asyncio.ensure_future`.
- `shutdown()` stops polling, waits up to `graceful_shutdown_timeout` (25s default) for in-flight
  activities, then cancels stragglers. `worker.run()` then returns, and the process logs and exits 0.
- The reference Kubernetes manifest sets no `terminationGracePeriodSeconds`, so Kubernetes uses its
  default of 30s. The 25s drain budget stays below that.

### 3.6 Worker Versioning

- Built only when both `TEMPORAL_DEPLOYMENT_NAME` and `TEMPORAL_WORKER_BUILD_ID` are non-empty:
  `WorkerDeploymentConfig(version=WorkerDeploymentVersion(deployment_name=N, build_id=B),
  use_worker_versioning=True, default_versioning_behavior=VersioningBehavior.PINNED)`.
- The TemporalWorkerDeployment controller injects both variables on every pod. The compose stack and
  the dev stack do not, so versioning is off there.
- Workflow-level versioning: only `RadarDeploymentSmokeWorkflow` sets
  `@workflow.defn(versioning_behavior=PINNED)` explicitly. Every other workflow inherits the worker
  default `PINNED`.
- Deployment name and build ID are provided by the controller. The Rust worker must read the same
  variables and set the same `PINNED` default.

### 3.7 Tracing, metrics, data converter, TLS

- Tracing: `init_tracer()` sets `service.name = "radar-ng-temporal-worker"` and installs
  `TracingInterceptor()` from `temporalio.contrib.opentelemetry`. OTLP export only if
  `OTEL_EXPORTER_OTLP_ENDPOINT` is set. Only the main worker traces; the open-meteo worker does not.
- Metrics: `Runtime(telemetry=TelemetryConfig(metrics=PrometheusConfig(bind_address=...)))`. Main
  worker only, default `0.0.0.0:9464`.
- Data converter: default everywhere (worker, API, seeder, watchdog, debug harness).
- TLS and auth: none. `temporal-connection.yaml` documents that mTLS is not configured. Plaintext gRPC
  to `temporal-frontend.temporal.svc.cluster.local:7233` in the reference manifest.

### 3.8 Container

- `temporal/Dockerfile`: `python:3.12-slim`, `PYTHONPATH=/app`, `CMD ["python","-m","temporal.worker"]`.
- Build-time gate: `python -m unittest -q temporal.workflows.test_replay temporal.workflows.test_workflow_discovery`.
  Unavailable as delivered (§0).
- Build-time gate: `python -c "import cv2; from pysteps import motion, nowcasts; ..."` fails the build
  if OpenCV or pysteps is missing.

---

## 4. Workflows

Each subsection gives: type name, queue, input and output JSON, activity calls (with timeouts and
retry), concurrency, continue-as-new and patches, and pseudo-code. Pseudo-code notation:
`ACT(name, arg, q=, s2c=, sched=, hb=, RP=)` is `execute_activity` with those options.

### 4.1 RadarDeploymentSmokeWorkflow (`workflows/deployment_smoke.py`)

- Type: `RadarDeploymentSmokeWorkflow`. Versioning `PINNED` (explicit).
- Queue: the queue the controller addresses (the candidate's own queue). Not a schedule target.
- Input: none. Output: JSON object `{"ok": true, "role": "<role>"}` (the activity's dict return).
- Signals, queries, timers: none.

```
run():
  return ACT("radar_deployment_smoke", no args,
             sched=90s, s2c=30s, RP(maximum_attempts=2))
```

Non-retryable errors: none in this workflow. The activity raises plain `ValueError`/`OSError`, which
are retryable. See §9.2.

### 4.2 IngestMrmsWorkflow (`workflows/ingest_mrms.py`)

- Type: `IngestMrmsWorkflow`. Queue: `radar-ng-mrms` (isolated) or `radar-ng` (legacy).
- Schedules: `ingest-mrms-base`, `ingest-mrms-composite` (same type, different args; §6).
- Input: `IngestMrmsArgs | None`, JSON `{"mrms_prefix": str, "layer_name": str}`. Absent or
  `null` means `IngestMrmsArgs()`. Defaults come from env `MRMS_PREFIX` and `LAYER_NAME`
  (default `"radar"`), `ingest_mrms/activities.py:59-62`.
- Output: `IngestMrmsResult` = `{"layer": str, "backlog_total": int, "rendered_count": int,
  "cleanup": {"tile_dirs_removed": int, "grid_files_removed": int} | null}`.
- Constants: `RETENTION_HOURS = 4`, `FRAME_CONCURRENCY = 2`.

Activity calls:

| Call | Input JSON | Queue | s2c | sched2c | hb | RP |
|---|---|---|---|---|---|---|
| `mrms_list_unprocessed_keys` | `IngestMrmsArgs` | inherited | 30s | none | none | RP(1s,2.0,10s,3) |
| `mrms_process_frame` | `{"key": str, "layer_name": str}` | inherited | 4m | 5m | 60s | RP(2s,2.0,30s,2) |
| `mrms_mark_processed` | `{"key": str, "layer_name": str}` | inherited | 30s | none | none | RP(1s,2.0,100s,5) |
| `mrms_cleanup` | `{"layer_name": str, "retention_hours": 4}` | inherited | 5m | none | none | RP(1s,2.0,100s,5) |

Pseudo-code:

```
run(args):
  args = args or IngestMrmsArgs()
  listing = ACT(mrms_list_unprocessed_keys, args, ...)        # failure: workflow fails
  if listing.keys is empty:
     cleanup = ACT(mrms_cleanup, {layer, 4}, ...)            # failure: workflow fails
     return {layer, backlog_total: listing.backlog_total, rendered_count: 0, cleanup}
  sem = Semaphore(2)
  async process_key(key):
     acquire sem
     try:   return ACT(mrms_process_frame, {key, layer}, ...)
     catch ActivityError:  return {key, rendered: false}     # swallowed; frame stays unmarked
  results = gather(process_key(k) for k in listing.keys)    # results in key order
  rendered = 0
  for r in results:                                          # sequential, key order
     if r.rendered:
        ACT(mrms_mark_processed, {key: r.key, layer}, ...)   # failure: workflow fails
        rendered += 1
  cleanup = ACT(mrms_cleanup, {layer, 4}, ...)
  return {layer, backlog_total: listing.backlog_total, rendered_count: rendered, cleanup}
```

- Ordering contract: all `mrms_process_frame` commands are scheduled in key order, then all
  `mrms_mark_processed` commands in key order, then `mrms_cleanup`. Keep the same command sequence
  in Rust.
- Keys come from the activity already reversed and capped at `BACKLOG_PER_CYCLE` (newest first).
- Only `ActivityError` from frame processing is swallowed. A list, mark, or cleanup failure ends the
  execution. Schedules recover on the next fire.
- No continue-as-new, no signals, no timers, no patches.

### 4.3 IngestHrrrWorkflow (`workflows/ingest_hrrr.py`)

- Type: `IngestHrrrWorkflow`. Queue: `radar-ng-hrrr` (isolated) or `radar-ng` (legacy).
- Schedule: `ingest-hrrr` (§6). Input: none. The schedule passes no args.
- Constants: `RETENTION_HOURS = 12`, `FORECAST_CONCURRENCY = 4`,
  `HOUR_START_TO_CLOSE = 8m`, `HOUR_SCHEDULE_TO_CLOSE = 12m`, `HOUR_HEARTBEAT_TIMEOUT = 90s`.
  Forecast retry `RP(2s,2.0,120s,2)`. Default retry `RP(1s,2.0,100s,5)`.
- Horizon comes from the activity (`FORECAST_HOURS` 18, or `EXTENDED_FORECAST_HOURS` 48 for runs
  00/06/12/18). The workflow processes hours `1..horizon` inclusive.

Output `IngestHrrrResult`:
`{"run_id": str|null, "skipped_already_processed": bool, "forecast_hours_processed": int,
"cleanup": {"tile_dirs_removed": int, "grid_files_removed": int}|null,
"layers_per_hour": [[str]], "published_layers": [str], "complete": bool}`.
Field defaults: `layers_per_hour=[]`, `published_layers=[]`, `complete=false`.

Activity calls:

| Call | Arguments | s2c | sched2c | hb | RP |
|---|---|---|---|---|---|
| `hrrr_find_latest_run` | none | 120s | none | none | RP(1s,2.0,100s,5) |
| `hrrr_horizon_for_run` | `run_id` | 10s | none | none | RP(1s,2.0,100s,5) |
| `hrrr_process_forecast_hour` | `run_id, fhr` (2 positional payloads) | 8m | 12m | 90s | RP(2s,2.0,120s,2) |
| `hrrr_publish_run` | `run_id, results` (full list of all hour results, not only the prefix) | 2m | none | none | RP(1s,2.0,100s,5) |
| `hrrr_mark_processed` | `run_id` | 30s | none | none | RP(1s,2.0,100s,5) |
| `hrrr_cleanup` | `retention_hours: 12` | 5m | none | none | RP(1s,2.0,100s,5) |

`ForecastHourResult` JSON: `{"fhr": int, "rendered_layers": [str], "duration_s": float, "valid_timestamp": str}`.
A failed hour is replaced by `ForecastHourResult(fhr=fhr)` (empty `rendered_layers`).

Pseudo-code:

```
run():
  find = ACT(hrrr_find_latest_run, ...)                          # failure: workflow fails
  if find.run_id is null:
     cleanup = ACT(hrrr_cleanup, 12, ...)
     return {run_id: null, skipped: false, hours: 0, cleanup}
  if find.already_processed:
     cleanup = ACT(hrrr_cleanup, 12, ...)
     return {run_id, skipped: true, hours: 0, cleanup}
  horizon = ACT(hrrr_horizon_for_run, run_id, ...)
  sem = Semaphore(4)
  async process_hour(fhr):
     acquire sem
     try:   return ACT(hrrr_process_forecast_hour, [run_id, fhr], ...)
     catch ActivityError:  return ForecastHourResult(fhr)       # swallowed
  results = sorted(gather(process_hour(f) for f in 1..horizon), by fhr)
  layers_per_hour = [r.rendered_layers for r in results]
  prefix = len(publishable_prefix(results))                    # longest f01.. run with "radar-hrrr"
  complete = (prefix == horizon)
  if prefix == 0:
     cleanup = ACT(hrrr_cleanup, 12, ...)
     return {run_id, skipped: false, hours: 0, cleanup, layers_per_hour}
  published = ACT(hrrr_publish_run, [run_id, results], ...)
  if "radar-hrrr" not in published:
     cleanup = ACT(hrrr_cleanup, 12, ...)
     return {run_id, skipped: false, hours: 0, cleanup, layers_per_hour}   # published_layers empty
  if complete:
     ACT(hrrr_mark_processed, run_id, ...)
  cleanup = ACT(hrrr_cleanup, 12, ...)
  return {run_id, skipped: false, hours: prefix, cleanup, layers_per_hour,
          published_layers: published, complete}
```

- Partial publication is intentional. A run is marked processed only when `complete`. The next 15-minute
  fire re-polls the same run (the finder prefers it while under 3h old) and renders only missing hours.
- `forecast_hours_processed` reports the prefix on the success path and `0` on the three early-return
  paths, even when hours rendered. Preserve this for history compatibility.
- Command order: `find`, `horizon`, then hour calls in ascending `fhr`, then `publish`, optionally
  `mark_processed`, then `cleanup`.
- No continue-as-new, signals, timers, or patches.

### 4.4 IngestAirQualityWorkflow (`workflows/ingest_airquality.py`)

- Type: `IngestAirQualityWorkflow`. Queue: `radar-ng-aux` (isolated) or `radar-ng` (legacy).
- Schedule: `ingest-airquality` (§6). Input: none.
- Constants: `RETENTION_HOURS = 36`, `CHUNK_CONCURRENCY = 2`, `FORECAST_MESSAGES = 72`,
  `CHUNK_MESSAGES = 24`, so chunk starts are `[0, 24, 48]`. `AQM_LAYERS` (activities module):
  `air-quality` (pm25), `ozone`. Total chunk calls: 3 starts x 2 layers = 6, in layer-major order.
- Default retry `RP(1s,2.0,100s,5)`. Chunk retry `RP(5s,2.0,60s,2)`.

Output `IngestAirQualityResult`:
`{"run_id": str|null, "skipped_already_processed": bool, "published_layers": [str],
"frames_rendered": int, "cleanup": {"tile_dirs_removed": int}|null}`.

Activity calls:

| Call | Arguments | s2c | sched2c | hb | RP |
|---|---|---|---|---|---|
| `aqm_find_latest_run` | none | 120s | none | none | default |
| `aqm_render_chunk` | `run_id, layer, start_msg` (3 positional payloads) | 20m | 25m | 120s | RP(5s,2.0,60s,2) |
| `aqm_publish_run` | `run_id, results` (all 6 chunk results, including failed empty ones) | 2m | none | none | default |
| `aqm_mark_processed` | `run_id` | 30s | none | none | default |
| `aqm_cleanup` | `retention_hours: 36` | 5m | none | none | default |

`AqmChunkResult` JSON: `{"layer": str, "start_msg": int, "rendered_timestamps": [str]}`.
A failed chunk becomes `AqmChunkResult(layer, start_msg)` with empty `rendered_timestamps`.

Pseudo-code:

```
run():
  find = ACT(aqm_find_latest_run, ...)
  if find.run_id is null:
     cleanup = ACT(aqm_cleanup, 36, ...)
     return {run_id: null, skipped: false, cleanup}
  if find.already_processed:
     cleanup = ACT(aqm_cleanup, 36, ...)
     return {run_id, skipped: true, cleanup}
  sem = Semaphore(2)
  async render_chunk(layer, start):
     acquire sem
     try:   return ACT(aqm_render_chunk, [run_id, layer, start], ...)
     catch ActivityError:  return AqmChunkResult(layer, start)  # swallowed
  results = gather(render_chunk(l, s) for l in [air-quality, ozone] for s in [0,24,48])
  frames = sum(len(r.rendered_timestamps) for r in results)
  published = ACT(aqm_publish_run, [run_id, results], ...)
  if "air-quality" in published:
     ACT(aqm_mark_processed, run_id, ...)
  # else: log error, not processed; next tick retries (tiles already on disk make retries cheap)
  cleanup = ACT(aqm_cleanup, 36, ...)
  return {run_id, skipped: false, published_layers: published, frames_rendered: frames, cleanup}
```

- Only the `air-quality` layer gates processed status. An `ozone`-only publish re-runs next tick.
- Command order: find, six chunk calls in the layer-major order above, publish, optional mark, cleanup.
- No continue-as-new, signals, timers, or patches.

---

### 4.5 IngestLightningWorkflow (`workflows/ingest_lightning.py`)

- Type: `IngestLightningWorkflow`. Queue: `radar-ng-aux` (isolated) or `radar-ng` (legacy).
- Schedule: `ingest-lightning` (every 60 min). Input: none.
- Constant: `RUN_DURATION_S = 3000` (50 min).
- Single call, activity argument is the JSON integer `3000`:
  `ACT(lightning_consume_stream, 3000, s2c=3060s, sched2c=55m, hb=90s, RP(5s,2.0,120s,3))`.
- Output `LightningRunResult`: `{"duration_s": float, "msgs": int, "parsed": int, "in_bbox": int,
  "final_buffer": int, "seeded": int (default 0), "connect_failures": int (default 0)}`.
- Exhaustion fails the execution. The next hourly fire starts a fresh stream. Buffer state is lost
  across runs by design (rolling 15-minute retention).
- No continue-as-new, signals, timers, or patches.

### 4.6 IngestTropicalWorkflow (`workflows/ingest_tropical.py`)

- Type: `IngestTropicalWorkflow`. Queue: `radar-ng-aux` (isolated) or `radar-ng` (legacy).
- Schedule: `ingest-tropical` (every 60 min). Input: none. Output `TropicalResult`:
  `{"storm_count": int, "feature_count": int}`.
- Single call: `ACT(tropical_fetch_and_publish, no args, s2c=60s, RP(2s,2.0,100s,5))`.
  No heartbeat, no schedule-to-close.
- No continue-as-new, signals, timers, or patches.

### 4.7 NowcastWorkflow (`workflows/nowcast.py`)

- Type: `NowcastWorkflow`. Queue: `radar-ng-nowcast` (isolated) or `radar-ng` (legacy).
- Schedule: `nowcast` (every 2 min, backstop). MRMS also triggers the schedule directly (§5.3).
  Input: none. Output `NowcastResult`:
  `{"ran": bool, "anchor_ts": str|null, "leadtimes": int, "palettes": [str], "duration_s": float,
  "skill_scored": int}`.
- Single call: `ACT(nowcast_run, no args, s2c=10m, sched2c=12m, hb=120s, RP(max_attempts=1))`.
  A single attempt is deliberate (the run is deterministic and a retry would pin the schedule).
  The Python SDK default backoff is irrelevant with one attempt.
- No continue-as-new, signals, timers, or patches.

### 4.8 TileCleanupWorkflow (`workflows/tile_cleanup.py`)

- Type: `TileCleanupWorkflow`. Queue: `radar-ng-aux` (isolated) or `radar-ng` (legacy).
- Schedule: `tile-cleanup` (every 60 min). Input: none. Output `TileCleanupResult`:
  `{"layers_swept": int, "tile_dirs_removed": int, "grid_files_removed": int}`.
- Single call: `ACT(tile_cleanup_sweep, no args, s2c=5m, RP(2s,2.0,60s,3))`.
- No continue-as-new, signals, timers, or patches.

### 4.9 OpenMeteoSyncWorkflow (`workflows/open_meteo_sync.py`)

- Type: `OpenMeteoSyncWorkflow`. Workflow queue: `radar-ng-aux` (isolated) or `radar-ng` (legacy).
  Activity queue: always `radar-ng-open-meteo` (explicit, `open_meteo_sync.py:43`).
- Schedules: `open-meteo-sync-gfs`, `open-meteo-sync-gfs025`, `open-meteo-sync-hrrr` (§6).
- Input `OpenMeteoSyncArgs`: `{"model": str, "variables": str, "past_days": int}`, where
  `variables` is a comma-separated list and `past_days` defaults to 1.
- Output `OpenMeteoSyncResult`: `{"model": str, "succeeded": bool, "duration_s": float,
  "return_code": int (default 0), "stderr_tail": str (default "")}`.
- Single call:
  `ACT(open_meteo_sync, args, queue=radar-ng-open-meteo, s2c=35m, sched2c=50m, hb=90s,
  RP(10s,2.0,5m,3))`.
- The activity runs `OPENMETEO_BIN sync <model> <variables> --past-days <n>` as a subprocess
  (the binary is in the open-meteo pool image), heartbeating about every 30s.
- No continue-as-new, signals, timers, or patches.

### 4.10 PollAlertsWorkflow (`workflows/poll_alerts.py`)

- Type: `PollAlertsWorkflow`. Queue: `radar-ng-alerts` (isolated) or `radar-ng` (legacy).
- Schedule: `poll-alerts` (every 5 min). Input: none.
- Output `PollAlertsResult`: `{"total": int, "new": int, "signaled_watches": int,
  "failed_alerts": int, "new_ids": [str]}`.
  `total` = `FetchAlertsResult.alert_count`. `new` = `len(new_alert_ids)` (counts every new alert,
  including zoneless and failed ones). `new_ids` = `new_alert_ids`.

Activity calls:

| Call | Input JSON | s2c | sched2c | hb | RP |
|---|---|---|---|---|---|
| `fetch_nws_active_alerts` | none | 60s | none | none | RP(2s,2.0,15s,3) |
| `signal_matching_storm_watches` | `{"alert_id": str, "geometry": object}` | 60s | none | none | RP(2s,2.0,15s,3) |
| `mark_alerts_seen` | `{"handled_ids": [str]}` | 30s | none | none | RP(2s,2.0,15s,3) |

`FetchAlertsResult` JSON: `{"alert_count": int, "new_alert_ids": [str], "new_alerts": [{"alert_id": str, "geometry": object}]}`.
`geometry` is `{}` when absent.

Pseudo-code:

```
run():
  fetched = ACT(fetch_nws_active_alerts, ...)                  # fetch failure: workflow fails
  normalized = [(a.alert_id, a.geometry or {}) for a in fetched.new_alerts]
  handled = [aid for (aid, geo) in normalized if geo is empty]   # zoneless: no activity call
  to_signal = [(aid, geo) for (aid, geo) in normalized if geo non-empty]
  async signal_one(aid, geo):
     try:   res = ACT(signal_matching_storm_watches, {alert_id: aid, geometry: geo}, ...)
            return (aid, res.matched)
     catch ActivityError: log; return None               # stays unseen; next poll retries
  results = gather(signal_one(aid, geo) for to_signal)   # concurrent, input order
  signaled = 0; failed = 0
  for r in results:
     if r is None: failed += 1; continue
     handled.append(r.aid); signaled += r.matched
  ACT(mark_alerts_seen, {handled_ids: handled}, ...)      # failure: workflow fails
  return {total: fetched.alert_count, new: len(fetched.new_alert_ids),
          signaled_watches: signaled, failed_alerts: failed, new_ids: fetched.new_alert_ids}
```

Semantics the port must reproduce:
- `failed_alerts` counts only `ActivityError` from the signal call. A per-watch signal failure inside
  the activity is logged and swallowed there, so the alert counts as handled (drift D7).
- `mark_alerts_seen` is always called, even with an empty `handled` list.
- Delivery is at-least-once across polls. `fetch` is read-only with respect to the seen set. The
  active-set snapshot is written to the state volume by `fetch`. Alerts that exhausted the signal
  retries are not committed, so the next poll re-diffs them as new.
- Command order: fetch, signal calls in `to_signal` order, mark. No child workflows.
- No continue-as-new, signals, timers, or patches.

### 4.11 WatchStormWorkflow (`workflows/watch_storm.py`)

Entity workflow, one run per (user, storm cell), lifetime hours to days.

- Type: `WatchStormWorkflow`. Queue: `radar-ng-alerts` (the API always starts it there; see §2.2).
- Workflow ID: `watch:<sha256-hex>` as in §1.1.
- Input `WatchStormInput`: `{"user_id": str, "storm_cell_id": str, "lat": float, "lng": float,
  "handoff": WatchStormHandoff|null}`. The API omits `handoff`.
- `WatchStormHandoff`: `{"schema_version": int (must be 1), "state": WatchStormState,
  "pending_alert_ids": [str]}`.
- `WatchStormState`: `{"user_id": str, "storm_cell_id": str, "lat": float, "lng": float,
  "last_frame_ts": str|null, "last_max_dbz": float|null, "last_change_kind": str|null,
  "last_notified_at": float|null (epoch seconds), "poll_count": int, "push_count": int}`.
  Defaults: strings `""`, floats `0.0`, optionals `null`, counts `0`.
- Output: `WatchStormState` (returned when unpinned).

Constants:
- `POLL_S = 60`, `MAX_FRAMES_PER_RUN = 1000` (this counts loop iterations, not frames; each iteration is
  one poll attempt, so the name is misleading), `MAX_RUN_DURATION = 24h`.
- Retry `_DEFAULT_RETRY = RP(2s,2.0,60s,5)` for compare and detect. Retry `_PUSH_RETRY = RP(1s,2.0,30s,3)`
  for fan-out.

### 4.11.1 Init and schema check (`__init__`, `watch_storm.py:91-97`)

```
__init__(inp):                                  # @workflow.init, runs before any signal is delivered
  if inp.handoff and inp.handoff.schema_version != 1:
     fail non-retryable ApplicationError("Unsupported storm-watch handoff schema")
  state = copy(inp.handoff.state) if handoff else WatchStormState()
  unpinned = false
  continue_requested = false
  pending = copy(inp.handoff.pending_alert_ids) if handoff else []
```

The `__init__` is the restore point. Signals may arrive before `run` starts, so the restored state must
exist here.

### 4.11.2 Signals and query

| Name | Argument | Effect |
|---|---|---|
| `unpinSignal` | none | `unpinned = true` |
| `wakeUpSignal` | none | `observe_continue_request()` only (no business work) |
| `alertMatchSignal` | `alert_id: str` | see below |
| Query `getCurrentState` | none | returns `state` (includes `lat`/`lng`, used by alert matching) |

`alertMatchSignal`:
```
if not patched("watch-storm-alert-queue-v1"):   # historical behavior, replay only
    pending.clear()                             # keeps only the most recent alert
pending.append(alert_id)
observe_continue_request()
```
New executions always take the "patched" branch and queue every alert. Old histories without the marker
keep last-wins behavior. The `patched()` call is evaluated on every signal, in signal order.

`observe_continue_request()` (sticky, never cleared):
```
continue_requested = continue_requested or info.continue_as_new_suggested()
                                         or info.target_worker_deployment_version_changed()
```
Rust names (vendored `temporalio-workflow-1.1.0`): `continue_as_new_suggested()`,
`target_worker_deployment_version_changed()`, `all_handlers_finished()`, `patched(id)`,
`deprecate_patch(id)`.

### 4.11.3 Run loop (exact order)

```
run(inp):
  deadline = now() + 24h
  polls_this_run = 0
  state.user_id, state.storm_cell_id, state.lat, state.lng = inp.user_id, inp.storm_cell_id, inp.lat, inp.lng
  log "watch start"

  while not unpinned:
    observe_continue_request()

    # CAN bound. Short-circuit order is load-bearing: patched() is evaluated
    # ONLY when continue_requested is already true.
    if polls_this_run >= 1000
       or now() >= deadline
       or (continue_requested and patched("watch-storm-version-boundary-v1")):
        if patched("watch-storm-handoff-v1"):
            wait_condition(all_handlers_finished)
            if unpinned: break                  # unpin wins over a new run; return state
            continue_as_new(
                inp with handoff = {schema_version: 1,
                                    state: copy(state),
                                    pending_alert_ids: copy(pending)},
                initial_versioning_behavior = AUTO_UPGRADE)
        continue_as_new(inp)                    # legacy path: ORIGINAL input, state is lost

    # Drain one pending alert per iteration (FIFO), before the frame comparison.
    if pending non-empty:
       alert_id = pending.pop_front()
       push_alert(inp, alert_id)
       observe_continue_request()

    cmp = ACT(compare_radar_frames,
              {lat: inp.lat, lng: inp.lng, prev_timestamp: state.last_frame_ts,
               prev_max_dbz: state.last_max_dbz}, s2c=30s, RP=_DEFAULT_RETRY)   # failure fails the run
    observe_continue_request()

    state.poll_count += 1
    polls_this_run += 1

    if cmp.sampled:
       state.last_frame_ts = cmp.curr_timestamp
       state.last_max_dbz  = cmp.curr_max_dbz
       det = ACT(detect_storm_change,
                 {has_prev: cmp.has_prev, curr_max_dbz: cmp.curr_max_dbz,
                  prev_max_dbz: cmp.prev_max_dbz, max_dbz_delta: cmp.max_dbz_delta},
                 s2c=10s, RP=_DEFAULT_RETRY)                                    # failure fails the run
       observe_continue_request()
       if det.kind non-empty:
          push_change(inp, det)
          observe_continue_request()

    try:
       wait_condition(unpinned or pending non-empty, timeout=60s)
    except TimeoutError: pass

  log "watch unpinned"
  return state
```

Notes on the loop:
- `compare_radar_frames` and `detect_storm_change` are NOT wrapped in `try/except`. After their
  retries are exhausted, the whole WatchStorm execution fails and ends. The user must re-watch
  (the API start allows restart after failure, see §5.1).
- Within one wakeup, the signal handler appends and the loop drains one alert per iteration. A
  batch of N alerts takes N iterations, each followed by a compare call. Alerts do not all push at once.
- The 60s wait can wake early on an unpin or an alert signal.
- Once `continue_requested` latches, it stays true for the rest of the run. The version-boundary
  check then fires at the next loop top, and the handoff branch runs if its patch marker is present.

### 4.11.4 Push helpers

```
push_change(inp, det):
  collapse = f"{workflow_id}:{state.last_frame_ts}:{det.kind}"   # last_frame_ts already updated
  result = fan_out(FanOutPushInput{user_id, title=title_for(det.kind), body=det.summary or "",
                                    collapse_id=collapse,
                                    extra={storm_cell_id, kind: det.kind}})
  state.last_change_kind = det.kind
  state.last_notified_at = now().timestamp()     # epoch float, even if push failed
  state.push_count += result.sent if result else 0

push_alert(inp, alert_id):
  collapse = f"{workflow_id}:alert:{alert_id}"
  result = fan_out(FanOutPushInput{user_id, title="Severe weather alert near your storm",
                                    body="NWS issued an alert in your watched area",
                                    collapse_id=collapse,
                                    extra={storm_cell_id, alert_id, kind: "alert"}})
  state.last_change_kind = "alert"
  state.last_notified_at = now().timestamp()
  state.push_count += result.sent if result else 0

fan_out(push):
  try:   return ACT(fan_out_push_to_user, push, s2c=60s, RP(1s,2.0,30s,3))
  catch ActivityError: log "push fan-out failed after retries; watch continues"; return null
```

`title_for(kind)`: `intensifying` -> "Storm intensifying", `dissipating` -> "Storm weakening",
`severe` -> "Severe storm cell", `alert` -> "Severe weather alert", anything else (including `""`) ->
"Storm update". Kinds from `detect_change`: `intensifying`, `dissipating`, `severe`. The `ChangeKind`
comment also lists `appeared`/`disappeared`, which fall to "Storm update".

Push failure never fails the watch. `push_count` increments by the fan-out `sent` count. With
`PUSH_DISABLED=1`, sends are no-ops that report success, so `push_count` counts attempted-and-
"sent" notifications, not deliveries (drift D6).

### 4.11.5 Patch markers

| Patch ID | Where | New executions | Historical executions (no marker) |
|---|---|---|---|
| `watch-storm-alert-queue-v1` | `alertMatchSignal` handler | queue every alert (append) | `pending.clear()` then append (last alert wins) |
| `watch-storm-version-boundary-v1` | CAN condition, only evaluated when `continue_requested` | version change triggers CAN | the version-change term is false; no version-based CAN |
| `watch-storm-handoff-v1` | CAN branch | handoff CAN with `AUTO_UPGRADE` | plain `continue_as_new(inp)`, state lost |

Rust must call the same patch IDs at the same decision points, in the same order, to replay histories
with these markers. Use `patched(id)` (not `deprecate_patch`) while histories containing the old branch
can still exist.

### 4.11.6 Handoff contract (from `docs/temporal-safe-deployments.md`)

- Handoff carries business state and every pending alert ID. `poll_count` and `push_count` are lifetime
  counters. `polls_this_run` resets to 0 per run.
- A handoff with an unknown `schema_version` fails explicitly (non-retryable).
- Unpin takes precedence over a new run: if `unpinned` is set at the boundary, the run returns instead of
  continuing.
- Notification retries keep their existing collapse IDs and exhaustion policy. Delivery is not exactly-once.
- An alert that exhausted delivery retries is not retried forever by a rollover.

### 4.12 RegisterPushTokenWorkflow and DeletePushTokenWorkflow (compatibility only)

Both stay registered (on `alerts` and `legacy`/`all`) so retained executions can replay. The API no
longer starts them (`routes_workflows.register_push_token` upserts directly, so tokens never enter new
histories). Retained histories may contain plaintext tokens. Treat them as sensitive.

- `RegisterPushTokenWorkflow`. Input `RegisterPushTokenInput`: `{"user_id": str, "token": str, "platform": str}`.
  Calls `ACT(persist_push_token, {"user_id", "token", "platform"}, s2c=10s, RP(1s,2.0,10s,3))`.
  Output: none (null).
- `DeletePushTokenWorkflow`. Input: `token: str`. Calls `ACT(delete_push_token, token, s2c=10s, RP(1s,2.0,10s,3))`.
  Output: `int` (rows deleted).
- No signals, queries, timers, continue-as-new, or patches.

---

## 5. External clients of the control plane

These are the places outside workflow code that start, signal, query, or trigger Temporal objects.
The Rust port must keep them compatible if it also replaces the API.

### 5.1 API watch lifecycle (`backend/api/api/routes_workflows.py`)

- Client: singleton `Client.connect(TEMPORAL_ADDRESS or "localhost:7233", namespace=TEMPORAL_NAMESPACE or "default")`.
  Reset on failure (`reset_client`). No TLS, no converter override.
- Start (`POST /v1/watches`): `client.start_workflow("WatchStormWorkflow", {"user_id", "storm_cell_id",
  "lat", "lng"}, id=_watch_id(user_id, storm_cell_id), task_queue=TASK_QUEUE,
  id_reuse_policy=ALLOW_DUPLICATE_FAILED_ONLY)`.
  Returns 201 with `{workflow_id, run_id}`. `WorkflowAlreadyStartedError` -> 409 "already watching".
  `RPCError` -> Temporal-unreachable mapping.
  Consequence: a failed watch can be re-created. A running one cannot.
- Unpin (`DELETE /v1/watches/{user_id}/{storm_cell_id}`): `handle.signal("unpinSignal")` with no args.
  `RPCError` whose message contains "not found" is treated as success (already gone).
- State (`GET /v1/watches/...`): `handle.query("getCurrentState")`.
- Ownership: the caller's `user_id` must equal the path/body `user_id` (constant-time compare), or 404.
- `TASK_QUEUE = env TEMPORAL_ALERTS_TASK_QUEUE or "radar-ng-alerts"`.

### 5.2 Alert-to-watch matching (`storm_watch_activities.signal_matching_storm_watches`)

Runs as the activity inside `PollAlertsWorkflow`:
```
if geometry is empty: return matched=0
client = get_client()
for w in client.list_workflows("WorkflowType='WatchStormWorkflow' AND ExecutionStatus='Running'"):
   try:
     handle = client.get_workflow_handle(w.id)
     state  = handle.query("getCurrentState")
     if state.lat is None or state.lng is None: continue
     if not point_in_geojson(lng=float(state.lng), lat=float(state.lat), geometry): continue
     handle.signal("alertMatchSignal", alert_id)     # positional str argument
     matched += 1
   except Exception: log warning; continue            # per-watch failures swallowed
return {matched}
```
Note the visibility query needs advanced visibility (a list-with-query call), not just basic listing.
Per-watch exceptions do not fail the activity. Only a whole-activity failure (for example
`list_workflows` or `get_client` raising) reaches `PollAlertsWorkflow` as `ActivityError`.

Point-in-polygon (exact algorithm, ray casting over GeoJSON coordinates `[lng, lat]`):
```
point_in_geojson(lng, lat, g):
  if g empty: false
  if g.type == "Polygon":      return point_in_polygon(lng, lat, g.coordinates)
  if g.type == "MultiPolygon": return any(point_in_polygon(lng, lat, p) for p in g.coordinates)
  return false

point_in_polygon(lng, lat, rings):          # rings[0] outer, rings[1..] holes
  if rings not a non-empty list: false
  if not point_in_ring(lng, lat, rings[0]): false
  return not any(point_in_ring(lng, lat, h) for h in rings[1..])

point_in_ring(lng, lat, ring):
  if ring not a list or len(ring) < 4: false
  prev = coord(ring[-1]); if prev is None: false
  inside = false
  for pt in ring:
     curr = coord(pt); if curr is None: return false     # any malformed vertex -> whole ring false
     crosses = (curr.lat > lat) != (prev.lat > lat)
     if crosses:
        x = (prev.lng - curr.lng) * (lat - curr.lat) / (prev.lat - curr.lat) + curr.lng
        if lng < x: inside = not inside
     prev = curr
  return inside

coord(p): p is list/tuple with len >= 2 and both convertible to float, else None
```
The watch point is `(state.lng, state.lat)`. The alert geometry is the NWS GeoJSON.

### 5.3 MRMS-triggered nowcast kick (`ingest_mrms/activities.py:kick_nowcast`)

After a science grid is published, `mrms_process_frame` calls, best-effort:
```
handle = client.get_schedule_handle(NOWCAST_SCHEDULE_ID)     # default "nowcast"
handle.trigger(overlap=ScheduleOverlapPolicy.BUFFER_ONE, rpc_timeout=5s)   # NOWCAST_KICK_TIMEOUT_S
```
Gated by `NOWCAST_KICK_ENABLED` (default `"1"`). Any exception is logged and returns `false`. The
frame is never failed for a kick. `BUFFER_ONE` applies to this trigger only. The schedule's own policy
stays `SKIP`. An idle nowcast starts immediately, and one follow-up run queues while busy.

### 5.4 Push-token endpoints

`POST /v1/push-tokens` and `DELETE /v1/push-tokens/{token}` call `upsert` and `delete_for_user` in a
thread directly. No Temporal call is made.

---

## 6. Schedules (`temporal/schedules/seed.py`)

### 6.1 Definition object

`ScheduleDef(schedule_id, workflow_name, max_runtime, workflow_input=[], interval, task_queue=AUX, catchup_window=1h, jitter=None)`.

Common to all 12 schedules:
- Spec: `intervals=[every=interval]`, `jitter=<jitter or none>`. No cron, no calendars, no timezone,
  no start/end time.
- Action: `StartWorkflow(workflow_name, *workflow_input, id="sched-"+schedule_id,
  task_queue=<queue rule, §2.2>, execution_timeout=max_runtime)`. No run timeout, task timeout,
  retry policy, or memo are set. Server defaults apply.
- Policy: `overlap=SKIP`, `catchup_window` as listed. No pause-on-failure, no limited actions.
- Initial state: unpaused (seeding does not set paused).

### 6.2 The 12 schedules

Interval, jitter, catchup, and execution timeout are literal. Queue shown is the isolated-mode queue.

| schedule_id | workflow | interval | jitter | catchup | execution_timeout | queue (isolated) | workflow_input (args, JSON) |
|---|---|---|---|---|---|---|---|
| `ingest-mrms-base` | IngestMrmsWorkflow | 1 min | 20 s | 5 min | 6 min | radar-ng-mrms | `[{"mrms_prefix":"CONUS/MergedBaseReflectivityQC_00.50","layer_name":"radar"}]` |
| `ingest-mrms-composite` | IngestMrmsWorkflow | 1 min | 20 s | 5 min | 6 min | radar-ng-mrms | `[{"mrms_prefix":"CONUS/MergedReflectivityComposite_00.50","layer_name":"radar-composite"}]` |
| `ingest-hrrr` | IngestHrrrWorkflow | 15 min | none | 1 h | 45 min | radar-ng-hrrr | `[]` |
| `ingest-airquality` | IngestAirQualityWorkflow | 30 min | none | 1 h | 75 min | radar-ng-aux | `[]` |
| `ingest-lightning` | IngestLightningWorkflow | 60 min | none | 1 h | 56 min | radar-ng-aux | `[]` |
| `ingest-tropical` | IngestTropicalWorkflow | 60 min | none | 1 h | 5 min | radar-ng-aux | `[]` |
| `nowcast` | NowcastWorkflow | 2 min | 20 s | 5 min | 12 min | radar-ng-nowcast | `[]` |
| `tile-cleanup` | TileCleanupWorkflow | 60 min | none | 1 h | 15 min | radar-ng-aux | `[]` |
| `poll-alerts` | PollAlertsWorkflow | 5 min | none | 10 min | 4 min 30 s | radar-ng-alerts | `[]` |
| `open-meteo-sync-gfs` | OpenMeteoSyncWorkflow | 6 h | none | 1 h | 60 min | radar-ng-aux | `[{"model":"ncep_gfs013","variables":GFS013,"past_days":2}]` |
| `open-meteo-sync-gfs025` | OpenMeteoSyncWorkflow | 6 h | none | 1 h | 60 min | radar-ng-aux | `[{"model":"ncep_gfs025","variables":GFS025,"past_days":2}]` |
| `open-meteo-sync-hrrr` | OpenMeteoSyncWorkflow | 1 h | none | 1 h | 55 min | radar-ng-aux | `[{"model":"ncep_hrrr_conus","variables":HRRR,"past_days":1}]` |

Variable lists (`GFS013`, `GFS025`, `HRRR`, `seed.py:107-121`), exact comma-joined strings, no spaces:

- `GFS013` = `temperature_2m,relative_humidity_2m,precipitation,showers,snowfall_water_equivalent,frozen_precipitation_percent,cloud_cover,wind_u_component_10m,wind_v_component_10m,shortwave_radiation,uv_index`
- `GFS025` = `wind_gusts_10m,pressure_msl,cape,lifted_index,visibility,categorical_freezing_rain`
- `HRRR` = `temperature_2m,relative_humidity_2m,precipitation,snowfall_water_equivalent,frozen_precipitation_percent,categorical_freezing_rain,cloud_cover,wind_u_component_10m,wind_v_component_10m,wind_gusts_10m,pressure_msl,cape,lifted_index,visibility,shortwave_radiation`

Constraints from tests:
- `execution_timeout <= 6 x interval` for every schedule.
- Fast schedules (interval <= 2 min) are exactly `ingest-mrms-base`, `ingest-mrms-composite`, `nowcast`.
  Each has catchup 5 min and jitter 20 s, and jitter < interval.
- `poll-alerts` catchup 10 min. Listed slow schedules have catchup 1 h and no jitter.
- Open-meteo schedules request only variables the domain actually stores (test `OpenMeteoSyncVariablesTest`),
  and never the query-time derived names (`weather_code`, `wind_speed_10m`, `wind_direction_10m`,
  `apparent_temperature`, `dew_point_2m`, `surface_pressure`, `precipitation_probability`).

Known drift in documentation (not in code): `deploy/k8s/README.md` lists MRMS as "every 2m" (code:
1 min) and omits `ingest-airquality` and `open-meteo-sync-gfs025`.

### 6.3 Shared reconciliation knobs

- `RPC timeout` for every schedule RPC: 15s (`_RPC_TIMEOUT`).
- Seeding runs in the designated pool only (see §3.3). It is one pass per process start.
- The seeder never describes, deletes, triggers, or terminates a Schedule or workflow. Tests assert
  this (`test_reconciliation_has_no_destructive_recovery_helpers`).

### 6.4 Create-or-update algorithm

Retryable RPC status codes (`_RETRYABLE_RPC_CODES`, `seed.py:60-77`):
`CANCELLED`, `UNAVAILABLE`, `DEADLINE_EXCEEDED`, `RESOURCE_EXHAUSTED`, `ABORTED`, `INTERNAL`,
`UNKNOWN`, `NOT_FOUND`. Classification is by RPC status code, not by message text. The comment
attributes the dominant case to "shard status unknown" (UNAVAILABLE) after a server restart.
`NOT_FOUND` is retryable because a create can race a concurrent delete. Every other exception is
permanent, including non-RPC errors such as `ValueError`.

```
seed_with_retry(client, max_attempts=10, base_delay=1.0s, max_delay=20.0s):
  if max_attempts < 1: raise ValueError("max_attempts must be at least 1")
  pending = SCHEDULES
  permanent = {}
  for attempt in 1..max_attempts:
     failures = gather(apply_isolated(client, s) for s in pending)  # all run concurrently; errors returned as values
     retry = []
     for s in pending with failure e:
        if e is RPCError and e.status in RETRYABLE:
           retry.append(s); print event=TEMPORAL_SCHEDULE_RECONCILIATION_RETRY ...
        else:
           permanent[s.id] = e; print event=TEMPORAL_SCHEDULE_RECONCILIATION_ERROR ... retryable=false
     if retry empty: break
     if attempt == max_attempts:
        permanent.update(retry failures); break
     sleep(min(max_delay, base_delay * 2^(attempt-1)))             # 1,2,4,8,16,20,20,20,20 s
     pending = retry
  if permanent non-empty: raise ScheduleSeedError(permanent)

apply_isolated(client, s):                  # CancelledError propagates; other exceptions returned
  try: apply(client, s, spec_for(s))
  except CancelledError: raise
  except Exception as e: return e
  return null

apply(client, s, desired):
  try:
     client.create_schedule(id, desired, rpc_timeout=15s)
     print "[seed] created schedule <id>"
  except ScheduleAlreadyRunningError:          # i.e. ALREADY_EXISTS
     handle = client.get_schedule_handle(id)
     update_required = false
     handle.update(callback, rpc_timeout=15s)
     print "[seed] updated|unchanged schedule <id>" based on the LAST callback decision

callback(inp):                               # may be invoked more than once; last decision wins
  current = inp.description.schedule         # fresh description for this attempt
  desired2 = desired with state = current.state   # keeps paused, note, limited_actions, remaining_actions
  if normalize(current) == normalize(desired2): update_required = false; return None   # no-op
  update_required = true; return ScheduleUpdate(schedule=desired2)
  # normalize = SDK protobuf form (Schedule._to_proto), not Python dataclass equality
```

Consequences:
- An `update` that finds the schedule deleted in the race returns `NOT_FOUND`. The whole
  create-or-update retries, so the next create succeeds (test `test_already_exists_then_update_not_found_retries_as_create`).
- Attempt budget worst case: 10 passes, about 111s of backoff, plus up to 15s per RPC.
- Only the still-retryable definitions are retried on later passes. Permanent failures are reported
  once. Healthy definitions converge in the first pass.
- Stdout formats are fixed and never include exception text. Labels are `ClassName` or, for an
  `RPCError` with a known status enum, `ClassName[STATUS]` (`safe_error_label`).

### 6.5 Worker-level reconciliation (`worker._reconcile_schedules`)

```
reconcile_schedules_once(client):
  try:  seed_with_retry(client)
  except ScheduleSeedError as e:
     log CRITICAL event=TEMPORAL_SCHEDULE_RECONCILIATION_FAILED
         schedule_ids=sorted(keys), errors="id:Label,id:Label", operator_action=inspect_schedule_reconciliation
  except Exception as e:  log CRITICAL same event with schedule_ids=[] errors="unexpected:<Label>"
  else: log "schedule reconciliation complete"
```
Never raises out. Never restarts the worker.

---

## 7. Schedule watchdog (`temporal/schedules/watchdog.py`)

Read-only observer. Runs only in the seeding pool, after polling has started.

Constants: `CHECK_EVERY = 60s`, `RPC_TIMEOUT = 15s`, `OVERDUE_INTERVALS = 2`, event
`TEMPORAL_SCHEDULE_STALLED`.

### 7.1 Stall predicate

```
is_wedged(next_action_times, running_actions, interval, now):
  if next_action_times empty: false
  if running_actions non-empty: false
  return next_action_times[0] < now - 2 * interval          # strict
```

### 7.2 Per-schedule check (`check_schedule`)

```
check(client, s, reported: map<id, datetime>, now):
  if s.interval is null: return false
  desc = client.get_schedule_handle(id).describe(rpc_timeout=15s)
  if desc.schedule.state.paused:
     reported.pop(id); return false                        # operator pause is not a stall
  info = desc.info
  if not is_wedged(info.next_action_times, info.running_actions, s.interval, now):
     if info.next_action_times non-empty and info.next_action_times[0] > now:
        reported.pop(id)                                  # timer advanced: allow a future event
     return false                                         # running action does NOT clear suppression
  stalled = info.next_action_times[0]
  if reported.get(id) == stalled: return false            # same timer already reported in this process
  reported[id] = stalled                                   # record BEFORE logging
  emit CRITICAL event=TEMPORAL_SCHEDULE_STALLED
       schedule_id, workflow_name (live action or s.workflow_name),
       task_queue (live action or s.task_queue),
       next_action_time = stalled.isoformat(), overdue_seconds = int(now - stalled),
       interval_seconds, automatic_recovery=false, operator_action=inspect_temporal_timer_dlq
  return true
```

Per-schedule failures are isolated: a `describe` error logs `WARNING
TEMPORAL_SCHEDULE_OBSERVATION_FAILED` with `schedule_id`, `error=<safe label>`,
`operator_action=inspect_schedule_observation`. The watchdog never raises.

### 7.3 Loop

```
watch_schedules(client, schedules):
  reported = {}                            # in-memory, lost on restart, not shared between replicas
  forever:
     sleep 60s
     gather(check_one(client, s, reported, now) for s in schedules)    # one slow describe cannot block others
```

Never calls `trigger`, `update`, `delete`, or `terminate`. Suppression is per process and per exact
timer. A restart or another replica can re-emit, so alert routing should deduplicate.

---

## 8. Push notifications (`temporal/shared/push.py`, `storm_watch_activities.fan_out_push_to_user`)

### 8.1 Dispatch path

`fan_out_push_to_user(FanOutPushInput)` (an activity, `s2c=60s` from WatchStorm, `RP(1s,2.0,30s,3)`):

```
tokens = list_for_user(user_id)
sent = 0; last_err = null
for t in tokens:
   try:  send_push_notification(PushToken{user_id, token, platform},
                                PushPayload{title, body, collapse_id, extra})   # direct Python call, NOT a scheduled activity
         sent += 1
   except Exception as e: last_err = e; log warning
if tokens non-empty and sent == 0 and last_err is not null:
   raise RuntimeError("all N push sends failed ...")       # retryable by the activity policy
return {sent}
```

`send_push_notification` is registered as an activity (on `alerts` and `legacy`/`all`), but no workflow
schedules it. The fan-out calls it inline. The docstring that says "scheduled as a child activity" is
stale (drift D4). The Rust port should keep the inline call and may drop the registration, but the
registration name must stay reserved for compatibility.

Convergence after a dead token: the dead token is deleted inside the failing send, so the retry
sees fewer tokens. When no tokens remain, the retry returns `sent=0` without raising.

### 8.2 Gate

`PUSH_DISABLED = (env PUSH_DISABLED default "1") == "1"` (module-level, read once).
- Enabled-disabled only for the exact string `"1"`. Anything else (including `""`, `"0"`, `"true"`) enables real sends.
- When disabled: `send_push_notification` logs `push_skipped` and returns normally. The fan-out counts
  that as `sent`, so `push_count` increments with no delivery.

### 8.3 Platform dispatch

`platform == "ios"` -> APNS. `platform == "android"` -> FCM. Otherwise non-retryable
`ApplicationError("unknown platform: ...")`.

### 8.4 APNS

Config (env): `APNS_HOST` (default `https://api.push.apple.com`), `APNS_TOPIC` (bundle ID),
`APNS_KEY_ID`, `APNS_TEAM_ID`, `APNS_KEY_PATH` (default `/secrets/apns.p8`, file read). Missing
`APNS_TOPIC`, `APNS_KEY_ID`, or `APNS_TEAM_ID` -> non-retryable `ApplicationError`.

Request:
- Auth: ES256 JWT, header `kid = APNS_KEY_ID`, claims `iss = APNS_TEAM_ID`, `iat = now`. Minted fresh per send.
- `POST {APNS_HOST}/3/device/{token}`, HTTP/2, timeout 10s.
- Headers: `authorization: bearer <jwt>`, `apns-topic: <APNS_TOPIC>`, `apns-push-type: alert`,
  `apns-priority: 10`, `apns-collapse-id: collapse_id[:64]` (Python character slice, not bytes).
- Body (JSON): `{"aps": {"alert": {"title": ..., "body": ...}, "sound": "default", "mutable-content": 1}, ...extra}`.
  `extra` keys are merged at the top level beside `aps`.

Response handling:
- `200`: success.
- `410`, or the body contains `BadDeviceToken` or `Unregistered`: `delete_by_token(token)`, then
  non-retryable `ApplicationError("apns dead token: ...")`.
- Other `4xx`: non-retryable `ApplicationError("apns 4xx: ...")`.
- Anything else (`5xx`, transport error): retryable (`RuntimeError` or the transport exception).

### 8.5 FCM

Config: `FCM_PROJECT_ID` (required, else non-retryable), `FCM_CREDENTIALS_PATH` (default
`/secrets/fcm.json`, file read).

Access token (process-cached):
- JWT RS256 signed with the service account `private_key`. Claims: `iss = client_email`,
  `scope = https://www.googleapis.com/auth/firebase.messaging`, `aud = https://oauth2.googleapis.com/token`,
  `iat = now`, `exp = now + 3600`.
- `POST https://oauth2.googleapis.com/token` with form `grant_type=urn:ietf:params:oauth:grant-type:jwt-bearer`,
  `assertion=<jwt>`, timeout 10s. `raise_for_status()`, read `access_token`.
- Cache stored as `(token, now + 3000)` and reused while `expiry > now + 60`.

Send:
- `POST https://fcm.googleapis.com/v1/projects/{FCM_PROJECT_ID}/messages:send`, timeout 10s,
  `authorization: Bearer <token>`, `content-type: application/json`.
- Body: `{"message": {"token": t, "notification": {"title", "body"},
  "data": extra, "android": {"collapse_key": collapse_id, "priority": "high",
  "notification": {"sound": "default"}}}}`. `data` values must be strings.

Response handling:
- `200`: success.
- Body contains `NOT_REGISTERED` or `INVALID_ARGUMENT`, or status `404`: `delete_by_token`, then
  non-retryable `ApplicationError("fcm dead token: ...")`.
- Other `4xx`: non-retryable.
- Else: retryable `RuntimeError`.

### 8.6 Collapse IDs and dedupe

- Collapse IDs: `"<workflow_id>:<last_frame_ts>:<kind>"` for changes, `"<workflow_id>:alert:<alert_id>"`
  for alerts.
- APNS truncates to 64 characters. With the sha256 watch-ID scheme, the workflow ID is 70 characters
  (`watch:` plus 64 hex), so every APNS collapse ID is truncated to the watch prefix. Distinct
  notifications for the same watch share one collapse ID, and APNS replaces undelivered older
  ones (drift D15). FCM receives the full ID.
- Dedupe is at-least-once. Retries reuse the same collapse IDs. Partial fan-out failure is not retried
  for the tokens that already succeeded.

### 8.7 Secrets wiring (reference manifests)

`deploy/k8s/secret-temporal-worker.yaml.template` supplies `APNS_KEY` and `FCM_CREDENTIALS_JSON` as
environment values. The code reads only `APNS_KEY_PATH` and `FCM_CREDENTIALS_PATH` (files). The worker
manifest no longer mounts the secret, and the README describes restoring a `push-keys` volume (drift D12).

---

## 9. Health, liveness, and the deployment smoke gate

### 9.1 Health loop (`temporal/shared/health.py`)

Started only after pollers are running (§3.4). Every 30 s:
```
loop:
  touch(ALIVE_FILE)                              # OSError logged; never fatal
  if check_once(client):                         # Temporal health RPC
      touch(HEALTH_FILE)                         # readiness-style file touched ONLY on success
  sleep 30s
```
`check_once`: `client.service_client.check_health(timeout=10s)` with one retry after 1s. Any exception
other than cancellation returns `false` (warning logged after the final failure). Never raises.

Files:
- `TEMPORAL_ALIVE_FILE` (default `/tmp/worker-alive`): touched every iteration (local event-loop liveness).
- `TEMPORAL_HEALTH_FILE` (default `/tmp/temporal-healthy`): touched only when Temporal is reachable.
  Never deleted. Consumers must check mtime to detect staleness.
The probes that consume these files are not in the repo. The reference manifests set no
liveness or readiness probes (drift D14). A successful health RPC does not prove the SDK is polling.
Check poll metrics and worker-deployment registration for that.

### 9.2 Deployment smoke (`temporal/shared/deployment_smoke.py`)

Activity `radar_deployment_smoke()` (no arguments) runs `check_release()` via `asyncio.to_thread`.

```
check_release():
  role = env WORKER_ROLE or "legacy"
  if role not in {mrms, nowcast, hrrr, aux, alerts, legacy, all}: ValueError("Unknown worker role")
  palettes = (env PALETTES or "classic,muted,vivid").split(",")
  for p in palettes:
     if not exists(<repo>/backend/shared/palettes/<p.strip()>.json): ValueError("Configured palette is absent from the image")
  # Exercise real imaging code: 2x2 paletted PNG, black/white palette, replace_palette(png, black->red)
  if decoded pixel (1,0) != (255,0,0): ValueError("Palette fixture produced incorrect pixels")
  if role in {nowcast, legacy, all}:  pysteps motion.get_method("LK"); nowcasts.get_method("sprog")
  if role in {hrrr, aux, legacy, all}: pygrib.grib_api_version must be truthy, else ValueError("ecCodes is unavailable")
  volumes = [TILE_DIR=/data/tiles, GRID_DIR=/data/grids, STATE_DIR=/data/state]   (env overrides)
  if role == alerts:
     grid_dir = env GRID_DIR or /data/grids
     os.scandir(grid_dir) (read one entry)
     require os.access(grid_dir, R_OK | X_OK) else PermissionError
     volumes = [STATE_DIR]          # alerts never writes grids and has no tiles mount
  for (var, default) in volumes:
     TemporaryDirectory(prefix=".deployment-smoke-", dir=env[var] or default)   # dir must exist; never created
        write transformed PNG to pending; fsync; rename to published; read back; must match
  return {"ok": true, "role": role}
```

Failure classes: a missing mount raises `FileNotFoundError`. An unreadable grid dir raises
`PermissionError` (only enforced as non-root). Scratch files are removed. Nothing is published and no
push is sent.

Workflow contract: `RadarDeploymentSmokeWorkflow` is registered on every role. The controller runs it on
the candidate version before promotion (`docs/temporal-safe-deployments.md`). A Rust worker must
register the same workflow type name and the activity name `radar_deployment_smoke`, and must
return `{"ok": true, "role": ...}`. The Python-specific checks (pysteps, pygrib) would have to be
replaced by equivalent native checks in a Rust build. The gate's semantic purpose is kept. The
interface stays the same.

---

## 10. Payload and in-flight compatibility rules

### 10.1 Replay fixture contract (`test_replay.py`, fixtures absent from upload)

- Layout: `replay_fixtures/<WorkflowType>/<version>/<scenario>.json[.gz]`. Version directories match
  `^(v\d+\.\d+\.\d+|src-[0-9a-f]{7,40})$` and are immutable. Every retained version must replay.
- Every registry type needs a fixture directory. Every version needs every required scenario, and each
  scenario must prove its path by containing the listed event types or activity names.
- Required scenarios (from `REQUIRED_SCENARIOS`; add, never remove):

| Workflow | Scenario -> required markers |
|---|---|
| RadarDeploymentSmoke | `success` -> COMPLETED; `activity-error` -> ACTIVITY_FAILED or FAILED |
| IngestMrmsWorkflow | `success` -> COMPLETED + `mrms_mark_processed`; `partial` -> COMPLETED; `frame-activity-error` -> ACTIVITY_FAILED + COMPLETED; `list-activity-error` -> ACTIVITY_FAILED + FAILED |
| IngestHrrrWorkflow | `success` -> COMPLETED + `hrrr_mark_processed`; `partial` -> COMPLETED + `hrrr_publish_run`; `hour-activity-error` -> ACTIVITY_FAILED + COMPLETED; `find-activity-error` -> ACTIVITY_FAILED + FAILED |
| IngestAirQualityWorkflow | `success` -> COMPLETED + `aqm_mark_processed`; `partial` -> COMPLETED + `aqm_publish_run`; `chunk-activity-error` -> ACTIVITY_FAILED + COMPLETED; `find-activity-error` -> ACTIVITY_FAILED + FAILED |
| PollAlertsWorkflow | `geometry-fanout` -> COMPLETED + `signal_matching_storm_watches`; `signal-activity-error` -> ACTIVITY_FAILED + COMPLETED; `fetch-activity-error` -> ACTIVITY_FAILED + FAILED |
| WatchStormWorkflow | `signal-unpin` -> SIGNALED + COMPLETED; `timer-poll` -> TIMER_FIRED + COMPLETED; `push-change` -> `detect_storm_change` + `fan_out_push_to_user` + COMPLETED; `alert-signal-push` -> SIGNALED + `fan_out_push_to_user` + COMPLETED; `push-activity-error` -> ACTIVITY_FAILED + COMPLETED; `compare-activity-error` -> ACTIVITY_FAILED + FAILED; `continue-as-new` -> CONTINUED_AS_NEW |
| Lightning, Tropical, Nowcast, TileCleanup, RegisterPushToken, DeletePushToken, OpenMeteoSync | `success` -> COMPLETED; `activity-error` -> ACTIVITY_FAILED or FAILED |

- Fixtures must be synthetic: identities start with `synthetic-`; no `stackTrace`; messages start with
  `synthetic` or equal `Activity task failed`; `user_id`, `storm_cell_id`, `alert_id`, `run_id`
  start with `synthetic-`; `token` equals `synthetic-placeholder`; `lat`/`lng` equal `0.0`; geometry
  coordinates are 0 or 1.
- The discovery test (`test_workflow_discovery.py`) fails without the fixture tree (`FIXTURE_DIR.iterdir()`).

### 10.2 Patch and command-order rules

- Replay matches commands by sequence. Rust must issue the same activity, timer, and signal commands
  in the same order as Python, for every workflow, including inside `gather`. Order rules:
  - MRMS: list; frame calls in key order; mark calls in key order; cleanup.
  - HRRR: find; horizon; hour calls in ascending `fhr`; publish; optional mark; cleanup.
  - AQM: find; six chunk calls (layer-major, starts 0/24/48); publish; optional mark; cleanup.
  - PollAlerts: fetch; signal calls in the order of `new_alerts`; mark.
  - WatchStorm: per iteration, optional alert push (fan-out), compare, optional detect and change push,
    then the 60s wait.
- The WatchStorm CAN check reads `patched("watch-storm-version-boundary-v1")` only when
  `continue_requested` is true. Preserve this short-circuit exactly.
- Patch IDs and their decision points are listed in §4.11.5. Do not add a new patch ID without
  matching the existing ones in the same code path.

### 10.3 Names and shapes that must not change

- Workflow type strings (13), activity strings (§1.1), signal names, query name, patch IDs, queue
  constants (§2.1), and schedule IDs (§6.2).
- Every dataclass field name listed in §4 (JSON keys). Field additions require defaults. Renames
  break in-flight decoding.
- `WatchStormHandoff.schema_version == 1`. A new version must be a new schema with its own migration.
- The watch workflow ID scheme (`watch:` + sha256) and the schedule action prefix (`sched-`).
- Schedule args JSON (§6.2 exact strings). The seeder overwrites live definitions with the declared
  ones on every worker start, so any drift in args silently changes running schedules.

### 10.4 Rollout and pinned executions (from `docs/temporal-safe-deployments.md`)

- A new image does not upgrade runs pinned to older code. Keep the older Python worker pools and their
  configuration running until their executions drain.
- Pinned runs stay on their build. Only new work, and Continue-as-New with `AUTO_UPGRADE` (the handoff
  path), moves to the new build.
- A CLI pinned-version override is sticky. Apply it only with approval of the specific execution and
  candidate, clear it after the repaired worker processes the run, and verify the next Continue-as-New.
- Histories with discarded state (legacy CAN without a handoff marker) cannot be reconstructed.
- Never commit real histories. They contain coordinates, user IDs, and push data.

### 10.5 Schedule parity

Because the seeder rewrites every schedule on worker start, a Rust seeder must generate definitions
equal to §6.2 in: schedule ID, workflow type, interval, jitter, catchup window, execution timeout,
overlap `SKIP`, action ID prefix `sched-`, args JSON, and the queue rule in §2.2. Keep
`SKIP_SCHEDULE_SEED` and `SEED_SCHEDULES` semantics identical.

---

## 11. Invariants checklist (asserted by existing tests)

Each line is a property the Rust port must satisfy. The test file is in brackets.

Schedules [`schedules/test_seed.py`]:
1. Every schedule's action `execution_timeout` equals `max_runtime`.
2. Every schedule's policy `catchup_window` and spec `jitter` equal the definition.
3. Fast schedules are exactly `ingest-mrms-base`, `ingest-mrms-composite`, `nowcast`; catchup 5 min; jitter 20 s; jitter < interval.
4. MRMS interval 1 min; nowcast interval 2 min.
5. `poll-alerts` catchup 10 min; `ingest-hrrr`, `ingest-lightning`, `tile-cleanup`, `open-meteo-sync-gfs` catchup 1 h and no jitter.
6. `max_runtime <= 6 x interval`.
7. Semantic equality compares the SDK protobuf form. A matching live schedule returns a `None` callback (no `UpdateSchedule`).
8. Updates preserve live `paused`, `note`, `limited_actions`, `remaining_actions`.
9. The callback reads the current description on each invocation; the last decision wins.
10. The seeding path never calls `describe`, `delete`, `terminate`, or `get_workflow_handle`.
11. Stdout never includes exception message or payload text. Only the class name and, for RPC errors, the status name.
12. `safe_error_label` ignores untrusted `status` attributes on non-RPC exceptions.
13. One failing schedule does not block others; all definitions start concurrently.
14. Only retryable RPC failures are retried. Permanent errors are kept. Healthy definitions are not re-applied.
15. `ALREADY_EXISTS` then update `NOT_FOUND` retries the full create-or-update, with the next create succeeding.
16. Retry exhaustion never deletes or terminates.
17. `max_attempts < 1` raises `ValueError`.
18. Open-meteo syncs request only stored variables for their model (3 models covered); no derived names; GFS and HRRR both include the weather-code inputs (`cloud_cover`, `precipitation`, `snowfall_water_equivalent`, `cape`, `visibility`); `ncep_gfs013` and `ncep_hrrr_conus` both include the wind components.

Watchdog [`schedules/test_watchdog.py`]:
19. `is_wedged` is false for empty next times, for a running action, and for a future next time; true only when strictly older than 2 intervals.
20. The stall event has exactly the listed fields (including `automatic_recovery=false`, `operator_action=inspect_temporal_timer_dlq`) and performs no mutation.
21. The default rendered message contains every actionable field.
22. Each stalled timer is reported once per process, even with concurrent checks.
23. A restarted process re-reports the same timer (intentional).
24. A future timer clears suppression, so a later stall is reported again.
25. A running action neither clears suppression nor emits.
26. A paused schedule clears suppression and emits nothing.
27. An observation failure logs a WARNING with a safe label and no payload text.
28. One blocked `describe` does not block another schedule's check.
29. An RPC error on one schedule does not affect the others.

Worker supervision [`test_worker.py`]:
30. A reconciliation failure is non-fatal and logs one CRITICAL event with sorted `schedule_ids` and `errors` in `id:Label` form.
31. Rendered messages contain IDs and safe labels only.
32. The poller starts before reconciliation, and reconciliation starts only after `is_running` and one yield.
33. Validation failure starts no background work and propagates.
34. The non-seeding pool starts the health loop only.

Health [`shared/test_health.py`]:
35. `check_once` returns true on success, false (no raise) on failure, and retries a transient failure once.
36. The readiness file is touched only on success. The alive file is touched on every iteration, including during outage. A stopped loop does not refresh the alive file.

Deployment smoke [`shared/test_deployment_smoke.py`]:
37. All five non-legacy roles pass against valid volumes. Existing files are preserved and no residue remains.
38. A missing mount raises `FileNotFoundError` and does not create the mount root.
39. `alerts` accepts read-only grids and never creates a tiles directory.
40. `alerts` requires readable grids and writable state (enforced as non-root).
41. A missing configured palette raises `ValueError` mentioning "palette".

Registry and roles [`workflows/test_workflow_discovery.py`]:
42. The registry equals the AST-discovered set of `@workflow.defn` classes, with matching qualified names and no duplicate type names.
43. Every workflow has a fixture directory (fixtures absent in this upload).
44. Every schedule's workflow and every API-started type is registered; each schedule's queue is served by a role that registers the type; the legacy pool registers it too.
45. `legacy` and `all` register every workflow. The non-legacy roles together cover every workflow. No role registers a workflow twice.

Replay [`workflows/test_replay.py`]:
46. Every version of every workflow carries every required scenario, and each scenario shows its listed markers (§10.1).
47. Fixtures are synthetic only (§10.1 constraints).
48. Every fixture replays with `ALL_WORKFLOWS` without a determinism error.

Handoff [`workflows/test_watch_storm_handoff.py`] (real-server test, needs a Temporal dev server):
49. A WatchStorm started with a handoff restores state. After a version promotion, a V1 activity completion does not push. V2 processes all three alerts in order. `compare_radar_frames` on V2 receives `prev_timestamp` and `prev_max_dbz` from the handoff state. The final state shows `last_max_dbz=42`, `push_count=10`, and `poll_count >= 4001`. The CAN input decodes with `pending_alert_ids == [a, b]`, `last_notified_at == 123`, `last_change_kind == "intensifying"`, and `push_count == 7` at handoff.

HRRR constants [`workflows/test_ingest_hrrr.py`]:
50. `FORECAST_CONCURRENCY == 4`, hour start-to-close 8 min, hour schedule-to-close 12 min, heartbeat 90 s, forecast retry `maximum_attempts == 2`, and start-to-close < schedule-to-close.
51. `IngestHrrrResult.complete` defaults to false.

Known test gaps (not invariants, but do not treat these tests as proof):
- Discovery checks that API-started types are registered on the alerts role. It does not check that the
  alerts queue has a poller in a legacy-mode deployment (see D3).
- Replay coverage depends on fixture content that is absent from this upload.

---

## 12. Drift and defects found in the source

These are behaviors or documents that disagree. For each, the Rust port must choose: keep the observable
behavior for in-flight compatibility, or document a deliberate change.

| ID | Where | Finding | Port guidance |
|---|---|---|---|
| D1 | `docs/configuration.md:62` vs `worker.py:140-216` | Docs list `open-meteo` as a valid `WORKER_ROLE`. It is not a `ROLE_CONFIG` key, so the worker raises `ValueError` at boot. | Do not add an `open-meteo` role that polls the wrong queue. Document the open-meteo pool as its own binary. |
| D2 | `watch_storm.py:12` vs `routes_workflows._watch_id` | Docstring says `watch:{user_id}:{storm_cell_id}`. Production uses `watch:<sha256>`. | Use the sha256 scheme (§1.1). |
| D3 | `routes_workflows.py:33,177` | The API starts watches on `radar-ng-alerts` regardless of `USE_ISOLATED_TASK_QUEUES`. Compose and reference manifests set neither `TEMPORAL_ALERTS_TASK_QUEUE` nor an alerts pool. With a legacy-only deployment, watch executions may never be polled. Only a live-cluster check can confirm. | Verify the alerts poller exists before enabling watches. The Rust port should fail loudly or route to the legacy queue when no alerts pool is present. |
| D4 | `storm_watch_activities.py:163-170` | Docstring says the push is a scheduled child activity. The code calls `send_push_notification` directly. `send_push_notification` is registered but never scheduled. | Keep the inline call. |
| D5 | `push.py:47` | `PUSH_DISABLED` disables only for the exact string `"1"`. `""`, `"0"`, `"true"` enable real sends. | Use the same exact-match rule (for compatibility) and document it. |
| D6 | `storm_watch_activities.py` fan-out + `push.py:211` | With push disabled, sends report success, so `push_count` counts non-deliveries. | Keep the counting; document that `push_count` is not a delivery count. |
| D7 | `poll_alerts.py` docstring vs `storm_watch_activities.signal_matching_storm_watches` | The docstring says failed alerts stay unseen. Per-watch signal failures are swallowed inside the activity, so an alert with partial failures is marked seen. Only whole-activity failures stay unseen. | Keep as-is. Document that per-watch signal failures are not retried and that only whole-activity failures count in `failed_alerts`. |
| D8 | `watch_storm.py:163` | The legacy CAN branch (no handoff marker) restarts with the original input, losing `state`, `pending_alert_ids`, and counters. Preserved only for old histories. | Keep the branch for replay. New runs always take the handoff branch. |
| D9 | `deploy/k8s/README.md` schedule table | MRMS listed as "every 2m" (code: 1 min). `ingest-airquality` and `open-meteo-sync-gfs025` omitted. | Follow code (§6.2). |
| D10 | `docs/kubernetes.md`, `deploy/k8s/README.md` verify steps | Docs say to look for "schedule seed complete". Code logs "schedule reconciliation complete" (worker) and `[seed] ...` (seeder). | Log the code's messages. Update the docs. |
| D11 | `tools/debug_harness/temporal_checks.py:18-22` | `FALLBACK_SCHEDULE_IDS` omits `ingest-airquality` and `open-meteo-sync-gfs025`. | Use the full set from §6.2. |
| D12 | `deploy/k8s/secret-temporal-worker.yaml.template` | Supplies `APNS_KEY` and `FCM_CREDENTIALS_JSON` as env values. The code reads file paths, and the worker manifest mounts no secret. | Mount key files and set `APNS_KEY_PATH` / `FCM_CREDENTIALS_PATH`. |
| D13 | Upload tree | `replay_fixtures/` and `temporal/open_meteo_worker.Dockerfile` are missing. The image build gate and the tests fail as delivered. | Obtain both from the source of record before the port is validated. |
| D14 | `deploy/k8s/temporal-worker-deployment.yaml` | No liveness or readiness probes, although docs describe GitOps probes on the `/tmp` files. | Define probes explicitly in the Rust deployment manifests. |
| D15 | `push.py:109` with the sha256 watch ID | APNS collapse ID is cut to 64 characters, leaving only the watch prefix. Notifications for one watch collapse into each other. | Decide whether to keep this (preserves existing behavior) or hash the collapse ID. Document the choice. |
| D16 | `deploy/docker-compose.yml:54` and `temporal-dev.yml:30` | The local server is `temporalio/auto-setup:1.25.2`, while the worker SDK is 1.30.0. Versioning is off in those stacks, so this is untested there. | Production server version is not in the repo. Confirm it supports Worker Deployments and `AUTO_UPGRADE`. |
| D17 | `watch_storm.py:208-209` | Exhaustion of `compare_radar_frames` or `detect_storm_change` ends the WatchStorm execution. Only push failures are tolerated, which the docs do not state for compare/detect. | Keep the behavior. Document that a failed compare requires the user to re-watch. |
| D18 | `watch_storm.py:40` | `MAX_FRAMES_PER_RUN` counts loop iterations, not frames. | Keep the name in docs; the behavior is per iteration. |
| D19 | `seed.py:297` | Schedules always use the legacy queue unless `USE_ISOLATED_TASK_QUEUES == "1"`, but the API ignores that flag (D3). The two routing rules can disagree in the same deployment. | Keep both rules as documented; make them one function in Rust. |

---

## 13. Open questions and items to verify before implementation

1. `[VERIFY-SDK]` Python 1.30.0 defaults: `RetryPolicy` fields left unset (initial 1s, coefficient 2.0,
   max 100x initial); `None` encoding (`binary/null`); whether `None` dataclass fields are emitted; decoding
   of missing and unknown keys; the exception type raised by `wait_condition(timeout=...)`.
2. `[VERIFY-SDK]` Schedule run ID suffix format for `sched-` actions, from an observed run.
3. Production Temporal server version and whether it supports Worker Deployments and `AUTO_UPGRADE` on
   Continue-as-New (D16). The repo does not set it.
4. Whether any retained history contains non-finite floats in payloads. Requires the missing fixtures.
5. The Rust SDK version that matches Python Core 1.30.0. The vendored crates are 1.1.0. Confirm that worker
   versioning, `AUTO_UPGRADE` on Continue-as-New, and patching behave the same way before relying on them.
6. Activity heartbeat payload shapes. Only timeouts are specified here. Heartbeat details matter for
   observability (`docs` debug harness decodes them) but not for control-plane correctness.
7. Unread files that affect payloads: `backend/shared/storm_watch.py` (`detect_change` full logic),
   `backend/shared/push_tokens.py` (`list_for_user` return type and token storage semantics), and the
   activity bodies in `backend/ingest_*/activities.py`.
8. Whether the production cluster runs an alerts pool (D3). Check the live deployment, not the repo.
