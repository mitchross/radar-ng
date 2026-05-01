# temporal/ — radar-ng worker

Temporal worker entrypoint, workflows, and Schedule definitions for radar-ng.

This directory is the single deployable that replaces all 7 K8s CronJobs in
the radar-ng backend. The worker process registers every workflow + activity
on a single task queue (`radar-ng`) and runs in the `radar-ng` namespace of
the Talos cluster as a `TemporalWorkerDeployment` CR.

See [`docs/superpowers/specs/2026-04-30-temporal-radar-ng-design.md`](../docs/superpowers/specs/2026-04-30-temporal-radar-ng-design.md)
for the full architectural spec.

## Layout

```
temporal/
├── worker.py              # entrypoint — Worker(...) registration + run
├── workflows/             # @workflow.defn classes
│   ├── ingest_mrms.py
│   ├── ingest_hrrr.py
│   ├── ingest_lightning.py
│   ├── ingest_tropical.py
│   ├── nowcast.py
│   ├── tile_cleanup.py
│   ├── poll_alerts.py
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

Phase 0 — scaffold only. Workflow bodies are placeholders. First real
workflow port (`IngestMrmsWorkflow`) lands in Phase 1.
