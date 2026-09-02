# temporal/ — radar-ng worker

Temporal worker entrypoint, workflows, and Schedule definitions for radar-ng.

This directory contains the Temporal deployables that replace the backend
CronJobs. The main worker supports role-specific task queues and retains an
all-in-one `radar-ng` queue for compatibility while older executions drain.

See [`ARCHITECTURE.md`](../ARCHITECTURE.md) for the orchestration design.

## Layout

```
temporal/
├── worker.py              # entrypoint — Worker(...) registration + run
├── workflows/             # @workflow.defn classes
│   ├── __init__.py        # canonical workflow type registry
│   ├── replay_fixtures/   # sanitized synthetic histories + generator
│   ├── ingest_mrms.py
│   ├── ingest_hrrr.py
│   ├── ingest_airquality.py
│   ├── ingest_lightning.py
│   ├── ingest_tropical.py
│   ├── nowcast.py
│   ├── open_meteo_sync.py
│   ├── tile_cleanup.py
│   ├── poll_alerts.py
│   ├── register_push_token.py
│   └── watch_storm.py
├── schedules/
│   ├── seed.py            # isolated, idempotent Schedule reconciliation
│   └── watchdog.py        # read-only stalled-timer observation
├── shared/
│   ├── push.py            # APNS/FCM activity
│   └── otel.py            # OTEL span/log helpers
├── Dockerfile
└── requirements.txt
```

Activity functions live alongside the existing service code under
`backend/<service>/activities.py` and are imported into `worker.py` for
registration. This keeps the I/O code next to its existing implementation
and the orchestration code centralized here.

## Status

Phases 0–4 done. All seven legacy CronJobs replaced by Temporal Schedules
seeded automatically on worker startup. Storm-watch + push-token routes
ship in `backend/api/api/routes_workflows.py` (tile-server v1.0.6+).
Push notifications are gated behind `PUSH_DISABLED` (default `1`) so the
worker boots without APNS/FCM secrets — re-enable per
`deploy/k8s/README.md` §4.

Phase 5 (Rust hot paths for `decode_grib2` / `build_mbtiles`) deferred —
gated on OTEL data showing them as bottlenecks.

## Workflow replay safety

`temporal/workflows/__init__.py` is the canonical registry used by the worker
roles and the replay tests. It keeps compatibility-only workflow types that
the API no longer starts but retained histories may still reference.

`test_replay.py` replays every history under
`replay_fixtures/<WorkflowType>/<version>/` through `Replayer` and requires
the scenario set in `REQUIRED_SCENARIOS` (success, partial, activity error,
signal, timer, push, continue-as-new where the workflow has that path).
`test_workflow_discovery.py` finds every `@workflow.defn` class by AST and
checks it is in the registry, in a worker role (and the legacy role), and has
a fixture directory; it also checks schedules and API routes only start
registered types on queues whose role registers them.

Both run as a `RUN` step in `temporal/Dockerfile`, so every build path (GHCR
release, backend CI, Gitea build, `backend/scripts/build-push.sh`) gates the
exact image it produces. See `replay_fixtures/README.md` for the fixture
contract and `docs/releasing.md` for the release-side rules.

## Schedule safety model

The designated seeding worker reconciles the declarative Schedule definitions
with create/update RPCs. Reconciliation runs in the background, concurrently
across schedules, with bounded retries for transient Temporal failures. One
bad Schedule is reported in an aggregate failure without preventing the other
definitions from converging, and reconciliation failure never prevents the
worker from starting or continuing task-queue polling. Updates preserve the
live Schedule state, including an operator pause, note, action limit, and
remaining-action count. Existing definitions are compared in their normalized
SDK protobuf form, so a matching definition returns a no-op from the update
callback instead of issuing a redundant `UpdateSchedule` mutation. If a
concurrent delete lands between create reporting `ALREADY_EXISTS` and the
update describe, the resulting `NOT_FOUND` retries the whole create-or-update
reconciliation with the same bounded policy.

The Temporal Python SDK update callback supplies the freshest description
available to that update attempt, and Radar derives the replacement state only
from that callback input. Temporal Python SDK 1.30 does not expose a Schedule
conflict token or atomic compare-and-swap through `ScheduleHandle.update`, so
an operator change racing the final server update can still win or be
overwritten when declarative configuration genuinely differs. The no-op path
eliminates that window when configuration already matches; after an actual
definition rollout, operators should verify incident pause/action-limit state.

The stall watchdog is an observer, not a recovery controller. It only calls
`describe`. A Schedule whose next action is more than two intervals overdue,
with no action running, produces a structured `CRITICAL` event named
`TEMPORAL_SCHEDULE_STALLED`. The event includes the schedule/workflow/queue,
the overdue timer, interval and overdue duration, and directs the operator to
inspect the Temporal timer DLQ.

The observer never calls Schedule `trigger`, `update`, or `delete`, and never
terminates a workflow. Startup reconciliation likewise never deletes a
Schedule or terminates a workflow to recover a timeout. Execution timeouts
already bound stale runs; timer-DLQ repair and any exceptional destructive
operation remain deliberate operator procedures.

Repeated observations of the same schedule ID and exact overdue timer are
suppressed only in memory for the life of one worker process. A future timer
clears that suppression. A restart or another replica can emit the event
again, so downstream alert routing should deduplicate it. No marker is written
to Temporal, Postgres, a PVC, or another shared store merely to suppress logs.
