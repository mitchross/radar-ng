# radar-types

Temporal wire contract for radar-ng. This crate holds everything that
crosses the Temporal boundary, so the Rust workers, the API and the seeder can share one
definition. It stayed wire-compatible with the former Python workers: JSON payloads (`json/plain`),
snake_case field names, and the same workflow, activity, signal, query, patch, queue and
schedule names.

Sources of truth: `docs/develop/specs/spec-temporal.md` (sections 1, 2, 4, 6, 10 and 11) and the payload
sections of `spec-ingest-mrms-hrrr.md` (section 2), `spec-ingest-aux.md` (sections 2 and 4) and
`spec-nowcast.md` (section 2). It was cross-checked against the Python workflows, activities, seeder and task-queue
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
One struct per Python dataclass used as a workflow or activity input or result, in pipeline
submodules: `mrms`, `hrrr`, `airquality`, `lightning`, `tropical`, `nowcast`, `cleanup`,
`open_meteo`, `alerts`, `watch_storm`, `push`, `smoke`. Every struct derives `Debug`, `Clone`,
`PartialEq`, `Serialize`, `Deserialize`, and most also derive `Default`. `WatchStormHandoff`
has `check_schema_version()`, which returns `UnsupportedHandoffSchema` for any version other
than 1. The error message is the Python text, `Unsupported storm-watch handoff schema`.

Encoding and decoding rules, applied to every struct:
- Field names are the Python field names. There are no renames and no `deny_unknown_fields`.
- Encoding emits every field, with `null` for unset `Option`s.
- Fields with a Python default use `#[serde(default)]` (or a default function) with the same value.
  `IngestMrmsArgs` defaults read env `MRMS_PREFIX` and `LAYER_NAME` at call time, as Python does.
- `f64` fields accept JSON integers.
- Non-finite floats fail to encode (see deviations).

### `schedules`
- `all_schedules() -> Vec<ScheduleDef>`: the 13 schedules (`seed.py` order plus `ingest-mrms-qpe-24h`). Fields:
  `id`, `workflow_type`, `interval_s`, `jitter_s: Option<u64>`, `catchup_window_s`,
  `execution_timeout_s`, `args: Vec<serde_json::Value>`, `task_queue` (declared), `overlap`
  (`OverlapPolicy::Skip`) and `pause_on_failure` (always `false`).
- `action_workflow_id(schedule_id)` returns `sched-<id>`.
- `schedule_action_queue(def, use_isolated)` applies the routing rule.
- `should_seed_schedules(role, SEED_SCHEDULES, SKIP_SCHEDULE_SEED)` and
  `should_seed_schedules_from_env(role)` match `worker.py`. Values are compared raw and exactly.
- The `GFS013_VARIABLES`, `GFS025_SURFACE_VARIABLES` and `HRRR_VARIABLES` strings and the
  `SCHEDULE_*` ID constants are public.

### `retry`
- `RetryPolicySpec { initial_interval_s, backoff, max_interval_s, max_attempts }`, with
  `DEFAULT_RETRY` and one constant per distinct policy.
- `ActivityCallSpec { start_to_close_s, schedule_to_close_s: Option<u64>, heartbeat_s: Option<u64>, retry }`.
- One constant per activity call, named after the activity in upper case, for example
  `MRMS_PROCESS_FRAME`, `HRRR_PROCESS_FORECAST_HOUR`, `LIGHTNING_CONSUME_STREAM`, `OPEN_METEO_SYNC`.
  The values are taken from spec-temporal section 4 and checked against the Python sources.

### `ids`
- `watch_workflow_id(user_id, storm_cell_id)` returns `"watch:" + hex(sha256(user_id + "\0" + storm_cell_id))`.
  The result is 70 characters. Golden values come from python3 `hashlib`.

## Deviations from the Python code

| # | Area | Python | Rust | Why |
|---|---|---|---|---|
| 1 | Open-meteo role | `WORKER_ROLE=open-meteo` is rejected (drift D1) | Accepted as `open-meteo` or `open_meteo`, mapped to `radar-ng-open-meteo` | The docs describe this pool. The role polls only the open-meteo queue. |
| 2 | Empty queue override | `TEMPORAL_TASK_QUEUE=""` polls queue `""` | Empty value is treated as unset (also for `TEMPORAL_ALERTS_TASK_QUEUE`) | An empty queue name is never valid. |
| 3 | Non-finite floats, encode | Python writes `NaN`/`Infinity` (invalid JSON) | Encode fails with an error | Spec requires the encoder never to emit them. serde_json would write `null`, which Python cannot decode as a float. |
| 4 | Non-finite floats, decode | Python's `json` accepts `NaN`/`Infinity` | Not accepted (serde_json rejects them) | No retained history was available to check. See deferred items. |
| 5 | Alert geometry | `geometry or {}` | `null` decodes as `{}` | Same effective behaviour, for the case where the field is `null`. |
| 6 | `radar_deployment_smoke` timeouts | Spec text `sched=90s, s2c=30s` | Schedule-to-close 90 s, start-to-close 30 s | Read the `sched=` notation as schedule-to-close. This matches the other call sites. |

Everything else follows the spec as written. In particular the sha256 watch ID, the
`alertMatchSignal` positional string, the `schedule_to_close` and `start_to_close` values, and the
retry policies are preserved exactly. The `PUSH_DISABLED` gate and the collapse-ID truncation live in
the push code, not in this crate.

## Deferred

- **Replay fixtures** are not checked in this crate. Wire compatibility with the retained histories is
  proven by the replay gate in `radar-worker` (`tests/replay.rs`).
- **Heartbeat detail shapes** (spec-ingest-mrms-hrrr section 2.4, spec-nowcast section 2.1). They
  are informational and not modelled.
- **Lenient decoding** of `NaN` or `Infinity` in old payloads, which may need a custom parser if
  a fixture ever contains them.
- **Python SDK defaults** that spec-temporal lists as `[VERIFY-SDK]`: `None` encoding,
  whether `None` fields are emitted, and the missing and unknown key behaviour. The crate follows the
  spec's choices (emit all fields, ignore unknown keys).
- **`ScheduleSpec` semantic equality, jitter comparison and the seeder's RPC behaviour** belong to the
  worker crate. This crate only supplies the definitions.
- **`send_push_notification`** has a payload model (`PushToken`, `PushPayload`) but no workflow
  uses it. The name is reserved.

## Tests

`cargo test -p radar-types`: 65 passed, 0 failed.
- Unit tests (6): golden watch IDs against python3 `hashlib`, separator and length checks,
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

Also checked outside the test suite:
- The field names of all 39 Python dataclasses match the Rust struct fields.
- The 30 activity names and 12 workflow names found in the Python sources match `wire.rs`. The
  `RadarDeploymentSmokeWorkflow` has no `name=` and uses the class name, so it was checked by hand.
- The three variable lists match `seed.py` exactly.
- Every retry and timeout value in `retry.rs` matches the Python `execute_activity` and `RetryPolicy` calls.
