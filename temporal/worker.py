"""radar-ng Temporal worker entrypoint.

Registers every workflow + every activity on the `radar-ng` task queue.
Single-pod, single-task-queue topology — split task queues later only if
perf demands it.
"""

from __future__ import annotations

import asyncio
import os

from loguru import logger
from temporalio.client import Client
from temporalio.worker import Worker

from backend.api.api.storm_watch_activities import (
    compare_radar_frames,
    detect_storm_change,
    fan_out_push_to_user,
    fetch_nws_active_alerts,
    persist_push_token,
    signal_matching_storm_watches,
)
from backend.ingest_hrrr.activities import (
    hrrr_cleanup,
    hrrr_find_latest_run,
    hrrr_horizon_for_run,
    hrrr_mark_processed,
    hrrr_process_forecast_hour,
)
from backend.ingest_lightning.activities import lightning_consume_stream
from backend.ingest_mrms.activities import (
    mrms_cleanup,
    mrms_list_unprocessed_keys,
    mrms_mark_processed,
    mrms_process_frame,
)
from backend.ingest_tropical.activities import tropical_fetch_and_publish
from backend.nowcast.activities import nowcast_run
from backend.open_meteo_sync.activities import open_meteo_sync_via_k8s_job
from backend.tile_cleanup.activities import tile_cleanup_sweep
from temporal.shared.otel import init_tracer
from temporal.shared.push import send_push_notification
from temporal.workflows import ALL_WORKFLOWS


TASK_QUEUE = "radar-ng"


ALL_ACTIVITIES = [
    # ingest-mrms
    mrms_list_unprocessed_keys,
    mrms_process_frame,
    mrms_mark_processed,
    mrms_cleanup,
    # ingest-hrrr
    hrrr_find_latest_run,
    hrrr_horizon_for_run,
    hrrr_process_forecast_hour,
    hrrr_mark_processed,
    hrrr_cleanup,
    # ingest-lightning
    lightning_consume_stream,
    # ingest-tropical
    tropical_fetch_and_publish,
    # nowcast
    nowcast_run,
    # tile-cleanup
    tile_cleanup_sweep,
    # open-meteo sync (creates k8s Jobs)
    open_meteo_sync_via_k8s_job,
    # storm-watch + alerts + push
    persist_push_token,
    compare_radar_frames,
    detect_storm_change,
    fan_out_push_to_user,
    fetch_nws_active_alerts,
    signal_matching_storm_watches,
    send_push_notification,
]


async def _main() -> None:
    target = os.environ.get("TEMPORAL_ADDRESS", "localhost:7233")
    namespace = os.environ.get("TEMPORAL_NAMESPACE", "default")
    logger.info("connecting to temporal at {} (namespace={})", target, namespace)

    interceptor = init_tracer()
    client = await Client.connect(target, namespace=namespace, interceptors=[interceptor])

    worker = Worker(
        client,
        task_queue=TASK_QUEUE,
        workflows=ALL_WORKFLOWS,
        activities=ALL_ACTIVITIES,
    )
    logger.info(
        "worker starting on task_queue={} with {} workflows + {} activities",
        TASK_QUEUE,
        len(ALL_WORKFLOWS),
        len(ALL_ACTIVITIES),
    )
    await worker.run()


if __name__ == "__main__":
    asyncio.run(_main())
