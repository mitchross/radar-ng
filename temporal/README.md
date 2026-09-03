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
│   └── seed.py            # idempotent Schedule create/update on startup
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
