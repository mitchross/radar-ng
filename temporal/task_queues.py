"""Task queue ownership for radar-ng workloads.

Queue names are an API contract shared by schedule seeding, workers, and the
FastAPI Temporal client. Keep them centralized so an ingest workload cannot
silently fall back onto the latency-sensitive alerts queue.
"""

LEGACY_TASK_QUEUE = "radar-ng"
MRMS_TASK_QUEUE = "radar-ng-mrms"
NOWCAST_TASK_QUEUE = "radar-ng-nowcast"
HRRR_TASK_QUEUE = "radar-ng-hrrr"
AUX_TASK_QUEUE = "radar-ng-aux"
ALERTS_TASK_QUEUE = "radar-ng-alerts"
OPEN_METEO_TASK_QUEUE = "radar-ng-open-meteo"
