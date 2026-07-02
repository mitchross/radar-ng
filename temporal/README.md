# temporal/ — radar-ng worker

Temporal worker entrypoint, workflows, and Schedule definitions for radar-ng.

This directory is the single deployable that replaces all 7 K8s CronJobs in
the radar-ng backend. The worker process registers every workflow + activity
on a single task queue (`radar-ng`) and runs in the `radar-ng` namespace of
the Talos cluster as a `TemporalWorkerDeployment` CR.

See [`ARCHITECTURE.md`](../ARCHITECTURE.md) for the orchestration design.

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

Phases 0–4 done. All seven legacy CronJobs replaced by Temporal Schedules
seeded automatically on worker startup. Storm-watch + push-token routes
ship in `backend/api/api/routes_workflows.py` (tile-server v1.0.6+).
Push notifications are gated behind `PUSH_DISABLED` (default `1`) so the
worker boots without APNS/FCM secrets — re-enable per
`deploy/k8s/README.md` §4.

Phase 5 (Rust hot paths for `decode_grib2` / `build_mbtiles`) deferred —
gated on OTEL data showing them as bottlenecks.
