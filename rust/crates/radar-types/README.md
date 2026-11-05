# radar-types

The Temporal wire contract for radar-ng. This crate holds everything that
crosses the Temporal boundary, so the workers, the API and the seeder share one
definition. The wire format is JSON payloads (`json/plain`) with snake_case field names. Workflow,
activity, signal, query, patch, queue and schedule names are unchanged from the Python workers, so histories
recorded by those workers still replay.

Sources of truth: `docs/archive/rust-migration/specs/spec-temporal.md` (sections 1, 2, 4, 6, 10 and 11) and the payload
sections of `spec-ingest-mrms-hrrr.md` (section 2), `spec-ingest-aux.md` (sections 2 and 4) and
`spec-nowcast.md` (section 2). These are design history. The crate was cross-checked against the Python workflows, activities, seeder and task-queue
module before they were removed.

## Public API

### `wire`
- Workflow types: `WORKFLOW_*` (13 in total) and `ALL_WORKFLOW_TYPES`.
- Activity names: `ACTIVITY_*` (30 in total, including the reserved `send_push_notification`)
  and `ALL_ACTIVITY_NAMES`.
- Signals and query: `SIGNAL_UNPIN` (`unpinSignal`), `SIGNAL_WAKE_UP` (`wakeUpSignal`),
  `SIGNAL_ALERT_MATCH` (`alertMatchSignal`), `QUERY_GET_CURRENT_STATE` (`getCurrentState`).
- Patch IDs: `PATCH_WATCH_STORM_ALERT_QUEUE_V1`, `PATCH_WATCH_STORM_VERSION_BOUNDARY_V1`,
  `PATCH_WATCH_STORM_HANDOFF_V1`.
- Task queues: `LEGACY_TASK_QUEUE`, `MRMS_TASK_QUEUE`, `NOWCAST_TASK_QUEUE`, `HRRR_TASK_QUEUE`,
  `AUX_TASK_QUEUE`, `ALERTS_TASK_QUEUE`, `OPEN_METEO_TASK_QUEUE`.
- Env var names: `ENV_*` constants (`WORKER_ROLE`, `TEMPORAL_TASK_QUEUE`,
  `USE_ISOLATED_TASK_QUEUES`, `SEED_SCHEDULES`, `SKIP_SCHEDULE_SEED`,
  `TEMPORAL_ALERTS_TASK_QUEUE`, `NOWCAST_SCHEDULE_ID`).
- `WorkerRole` enum (`Legacy`, `All`, `Mrms`, `Nowcast`, `Hrrr`, `Aux`, `Alerts`, `OpenMeteo`)
  with `default_task_queue()`, `as_str()`, `Display`, `FromStr` (trims and lowercases) and
  `WorkerRole::ALL`. Parse errors are `UnknownWorkerRole`.
- Routing: `schedule_task_queue(declared, use_isolated)`, `use_isolated_task_queues(env)`,
  `poll_task_queue(role, override)`, `watch_start_task_queue(env)`.

### `payloads`
One struct per workflow or activity input or result, in pipeline
submodules: `mrms`, `hrrr`, `airquality`, `lightning`, `tropical`, `nowcast`, `cleanup`,
`open_meteo`, `alerts`, `watch_storm`, `push`, `smoke`. Every struct derives `Debug`, `Clone`,
`PartialEq`, `Serialize`, `Deserialize`, and most also derive `Default`. `WatchStormHandoff`
has `check_schema_version()`, which returns `UnsupportedHandoffSchema` for any version other
than 1. The error message is `Unsupported storm-watch handoff schema`.

Encoding and decoding rules, applied to every struct:
- Field names are the wire field names. There are no renames and no `deny_unknown_fields`.
- Encoding emits every field, with `null` for unset `Option`s.
- Fields with a wire default use `#[serde(default)]` (or a default function) with the same value.
  `IngestMrmsArgs` defaults read env `MRMS_PREFIX` and `LAYER_NAME` at call time.
- `f64` fields accept JSON integers.
- Non-finite floats fail to encode (see Behavior notes).

### `schedules`
- `all_schedules() -> Vec<ScheduleDef>`: the 13 schedules (the original seeding order plus `ingest-mrms-qpe-24h`). Fields:
  `id`, `workflow_type`, `interval_s`, `jitter_s: Option<u64>`, `catchup_window_s`,
  `execution_timeout_s`, `args: Vec<serde_json::Value>`, `task_queue` (declared), `overlap`
  (`OverlapPolicy::Skip`) and `pause_on_failure` (always `false`).
- `action_workflow_id(schedule_id)` returns `sched-<id>`.
- `schedule_action_queue(def, use_isolated)` applies the routing rule.
- `should_seed_schedules(role, SEED_SCHEDULES, SKIP_SCHEDULE_SEED)` and
  `should_seed_schedules_from_env(role)` match the worker's seeding rule. Values are compared raw and exactly.
- The `GFS013_VARIABLES`, `GFS025_SURFACE_VARIABLES` and `HRRR_VARIABLES` strings and the
  `SCHEDULE_*` ID constants are public.

### `retry`
- `RetryPolicySpec { initial_interval_s, backoff, max_interval_s, max_attempts }`, with
  `DEFAULT_RETRY` and one constant per distinct policy.
- `ActivityCallSpec { start_to_close_s, schedule_to_close_s: Option<u64>, heartbeat_s: Option<u64>, retry }`.
- One constant per activity call, named after the activity in upper case, for example
  `MRMS_PROCESS_FRAME`, `HRRR_PROCESS_FORECAST_HOUR`, `LIGHTNING_CONSUME_STREAM`, `OPEN_METEO_SYNC`.
  The values are taken from spec-temporal section 4 and checked against the original Python `execute_activity` and `RetryPolicy` calls.

### `ids`
- `watch_workflow_id(user_id, storm_cell_id)` returns `"watch:" + hex(sha256(user_id + "\0" + storm_cell_id))`.
  The result is 70 characters. Golden values come from Python `hashlib`.

## Behavior notes

Each row differs from the old Python workers or from the spec's literal text.

| # | Area | Rust behavior | Why |
|---|---|---|---|
| 1 | Open-meteo role | `open-meteo` or `open_meteo` is accepted and mapped to `radar-ng-open-meteo`. Python rejected `WORKER_ROLE=open-meteo` (drift D1). | The docs describe this pool. The role polls only the open-meteo queue. |
| 2 | Empty queue override | An empty value counts as unset, for `TEMPORAL_TASK_QUEUE=""` and for `TEMPORAL_ALERTS_TASK_QUEUE`. Python polled queue `""`. | An empty queue name is never valid. |
| 3 | Non-finite floats, encode | Encode fails with an error. Python wrote `NaN`/`Infinity`, which is invalid JSON. | The spec requires the encoder never to emit them. serde_json would write `null`, which Python cannot decode as a float. |
| 4 | Non-finite floats, decode | `NaN`/`Infinity` are rejected, because serde_json rejects them. Python's `json` accepts them. | No retained history was available to check. See deferred items. |
| 5 | Alert geometry | `null` decodes as `{}`. Python wrote `geometry or {}`. | Same effective behavior when the field is `null`. |
| 6 | `radar_deployment_smoke` timeouts | Schedule-to-close is 90 s and start-to-close is 30 s. The spec text says `sched=90s, s2c=30s`. | Read the `sched=` notation as schedule-to-close. This matches the other call sites. |

Everything else follows the spec as written. The sha256 watch ID, the
`alertMatchSignal` positional string, the `schedule_to_close` and `start_to_close` values, and the
retry policies match the Python workers exactly. The `PUSH_DISABLED` gate and the collapse-ID truncation live in
the push code, not in this crate.

## Deferred

- Replay fixtures are not checked in this crate. Wire compatibility with the retained histories is
  proven by the replay gate in `radar-worker` (`tests/replay.rs`).
- Heartbeat detail shapes (spec-ingest-mrms-hrrr section 2.4, spec-nowcast section 2.1). They
  are informational and the crate does not model them.
- Lenient decoding of `NaN` or `Infinity` in old payloads, which may need a custom parser if
  a fixture ever contains them.
- Python SDK defaults that spec-temporal lists as `[VERIFY-SDK]`: `None` encoding,
  whether `None` fields are emitted, and the missing and unknown key behaviour. The crate follows the
  spec's choices (emit all fields, ignore unknown keys).
- `ScheduleSpec` semantic equality, jitter comparison and the seeder's RPC behaviour belong to the
  worker crate. This crate only supplies the definitions.
- `send_push_notification` has a payload model (`PushToken`, `PushPayload`), but no workflow
  uses it. The crate reserves the name.

## Tests

`cargo test -p radar-types`: 65 passed, 0 failed.
- Unit tests (6): golden watch IDs from Python `hashlib`, separator and length checks,
  retry-table spot checks against spec-temporal section 4.
- `tests/payload_roundtrip.rs` (24): every payload round-trips using example documents from the
  specs. Decoding, re-encoding and re-decoding give the same value, and the encoded document has the
  same keys and JSON kinds at every level as the example.
- `tests/payload_defaults.rs` (15): missing defaulted keys, required keys still required, unknown
  keys ignored, integers accepted for floats, `null` geometry, handoff schema check, and non-finite
  floats rejected on encode.
- `tests/schedules.rs` (8): all 13 schedules against the spec-temporal section 6.2 table (ID order,
  interval, jitter, catch-up, execution timeout, queue, args), plus the section 11 invariants and the
  seed gate matrix.
- `tests/wire.rs` (12): names, unique counts, queue routing, role parsing and defaults.

Also checked by hand against the Python sources before their removal:
- The field names of all 39 Python dataclasses match the Rust struct fields.
- The 30 activity names and 12 workflow names found in the Python sources match `wire.rs`. The
  `RadarDeploymentSmokeWorkflow` has no `name=` and uses the class name, so it was checked by hand.
- The three variable lists match the old `seed.py` exactly.
- Every retry and timeout value in `retry.rs` matches the Python `execute_activity` and `RetryPolicy` calls.
