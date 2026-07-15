"""Role-aware radar-ng Temporal worker entrypoint.

Production runs one WorkerDeployment per workload class. A legacy role keeps
the original ``radar-ng`` queue alive while pinned executions drain; local
development defaults to the same all-in-one behavior.
"""

from __future__ import annotations

import asyncio
import os
import signal
from datetime import timedelta

from loguru import logger
from temporalio.client import Client
from temporalio.common import VersioningBehavior
from temporalio.worker import Worker, WorkerDeploymentConfig, WorkerDeploymentVersion

from backend.api.api.storm_watch_activities import (
    compare_radar_frames,
    delete_push_token,
    detect_storm_change,
    fan_out_push_to_user,
    fetch_nws_active_alerts,
    mark_alerts_seen,
    persist_push_token,
    signal_matching_storm_watches,
)
from backend.ingest_hrrr.activities import (
    hrrr_cleanup,
    hrrr_find_latest_run,
    hrrr_horizon_for_run,
    hrrr_mark_processed,
    hrrr_publish_run,
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
from backend.tile_cleanup.activities import tile_cleanup_sweep
# NOTE: open_meteo_sync activity is registered by the SEPARATE
# radar-ng-open-meteo-worker pod (temporal/open_meteo_worker.py) — its
# base image carries the Swift binary. Temporal dispatches the activity
# to whichever worker has it registered, so this worker intentionally
# does not import or register it.
from temporal.schedules.seed import seed_with_retry as seed_schedules
from temporal.shared.otel import init_tracer
from temporal.shared.push import send_push_notification
from temporal.task_queues import (
    ALERTS_TASK_QUEUE,
    AUX_TASK_QUEUE,
    HRRR_TASK_QUEUE,
    LEGACY_TASK_QUEUE,
    MRMS_TASK_QUEUE,
    NOWCAST_TASK_QUEUE,
)
from temporal.workflows.ingest_hrrr import IngestHrrrWorkflow
from temporal.workflows.ingest_lightning import IngestLightningWorkflow
from temporal.workflows.ingest_mrms import IngestMrmsWorkflow
from temporal.workflows.ingest_tropical import IngestTropicalWorkflow
from temporal.workflows.nowcast import NowcastWorkflow
from temporal.workflows.open_meteo_sync import OpenMeteoSyncWorkflow
from temporal.workflows.poll_alerts import PollAlertsWorkflow
from temporal.workflows.register_push_token import RegisterPushTokenWorkflow
from temporal.workflows.tile_cleanup import TileCleanupWorkflow
from temporal.workflows.watch_storm import WatchStormWorkflow


DEFAULT_MAX_CONCURRENT_ACTIVITIES = 4
DEFAULT_MAX_CONCURRENT_ACTIVITY_TASK_POLLS = 2


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
    hrrr_publish_run,
    hrrr_cleanup,
    # ingest-lightning
    lightning_consume_stream,
    # ingest-tropical
    tropical_fetch_and_publish,
    # nowcast
    nowcast_run,
    # tile-cleanup
    tile_cleanup_sweep,
    # storm-watch + alerts + push
    persist_push_token,
    delete_push_token,
    compare_radar_frames,
    detect_storm_change,
    fan_out_push_to_user,
    fetch_nws_active_alerts,
    mark_alerts_seen,
    signal_matching_storm_watches,
    send_push_notification,
]

ALL_WORKFLOWS = [
    IngestMrmsWorkflow,
    IngestHrrrWorkflow,
    IngestLightningWorkflow,
    IngestTropicalWorkflow,
    NowcastWorkflow,
    TileCleanupWorkflow,
    PollAlertsWorkflow,
    WatchStormWorkflow,
    RegisterPushTokenWorkflow,
    OpenMeteoSyncWorkflow,
]

ROLE_CONFIG: dict[str, tuple[str, list[type], list[object]]] = {
    "mrms": (
        MRMS_TASK_QUEUE,
        [IngestMrmsWorkflow],
        [
            mrms_list_unprocessed_keys,
            mrms_process_frame,
            mrms_mark_processed,
            mrms_cleanup,
        ],
    ),
    "nowcast": (NOWCAST_TASK_QUEUE, [NowcastWorkflow], [nowcast_run]),
    "hrrr": (
        HRRR_TASK_QUEUE,
        [IngestHrrrWorkflow],
        [
            hrrr_find_latest_run,
            hrrr_horizon_for_run,
            hrrr_process_forecast_hour,
            hrrr_mark_processed,
            hrrr_publish_run,
            hrrr_cleanup,
        ],
    ),
    "aux": (
        AUX_TASK_QUEUE,
        [
            IngestLightningWorkflow,
            IngestTropicalWorkflow,
            TileCleanupWorkflow,
            OpenMeteoSyncWorkflow,
        ],
        [lightning_consume_stream, tropical_fetch_and_publish, tile_cleanup_sweep],
    ),
    "alerts": (
        ALERTS_TASK_QUEUE,
        [PollAlertsWorkflow, WatchStormWorkflow],
        [
            compare_radar_frames,
            detect_storm_change,
            fan_out_push_to_user,
            fetch_nws_active_alerts,
            mark_alerts_seen,
            signal_matching_storm_watches,
            send_push_notification,
        ],
    ),
    "legacy": (LEGACY_TASK_QUEUE, ALL_WORKFLOWS, ALL_ACTIVITIES),
    "all": (LEGACY_TASK_QUEUE, ALL_WORKFLOWS, ALL_ACTIVITIES),
}


def _deployment_config_from_env() -> WorkerDeploymentConfig | None:
    # The TemporalWorkerDeployment controller injects both env vars on every
    # in-cluster pod. When they're missing (dev-compose, local runs) we
    # register without versioning so the same code works in both modes.
    name = os.environ.get("TEMPORAL_DEPLOYMENT_NAME")
    build_id = os.environ.get("TEMPORAL_WORKER_BUILD_ID")
    if not name or not build_id:
        return None
    return WorkerDeploymentConfig(
        version=WorkerDeploymentVersion(deployment_name=name, build_id=build_id),
        use_worker_versioning=True,
        default_versioning_behavior=VersioningBehavior.PINNED,
    )


def _int_env(name: str, default: int, *, minimum: int = 1) -> int:
    raw = os.environ.get(name)
    if raw is None:
        return default
    try:
        return max(minimum, int(raw))
    except ValueError:
        logger.warning("invalid integer env {}; using default {}", name, default)
        return default


async def _main() -> None:
    target = os.environ.get("TEMPORAL_ADDRESS", "localhost:7233")
    namespace = os.environ.get("TEMPORAL_NAMESPACE", "default")
    logger.info("connecting to temporal at {} (namespace={})", target, namespace)

    interceptor = init_tracer()
    client = await Client.connect(target, namespace=namespace, interceptors=[interceptor])

    role = os.environ.get("WORKER_ROLE", "legacy").strip().lower()
    try:
        default_queue, workflows, activities = ROLE_CONFIG[role]
    except KeyError as exc:
        raise ValueError(
            f"unknown WORKER_ROLE={role!r}; expected one of {sorted(ROLE_CONFIG)}"
        ) from exc
    task_queue = os.environ.get("TEMPORAL_TASK_QUEUE", default_queue)

    # One designated pool seeds Schedules. Seeding from every role creates a
    # needless startup dependency and lets an old image race a new queue map.
    seed_setting = os.environ.get("SEED_SCHEDULES")
    should_seed = seed_setting == "1" or (seed_setting is None and role in {"legacy", "all"})
    if should_seed and os.environ.get("SKIP_SCHEDULE_SEED") != "1":
        logger.info("seeding schedules…")
        await seed_schedules(client)
        logger.info("schedule seed complete")

    deployment_config = _deployment_config_from_env()
    max_concurrent_activities = _int_env(
        "TEMPORAL_MAX_CONCURRENT_ACTIVITIES",
        DEFAULT_MAX_CONCURRENT_ACTIVITIES,
    )
    max_concurrent_activity_task_polls = _int_env(
        "TEMPORAL_MAX_CONCURRENT_ACTIVITY_TASK_POLLS",
        DEFAULT_MAX_CONCURRENT_ACTIVITY_TASK_POLLS,
    )

    worker = Worker(
        client,
        task_queue=task_queue,
        workflows=workflows,
        activities=activities,
        deployment_config=deployment_config,
        max_concurrent_activities=max_concurrent_activities,
        max_concurrent_activity_task_polls=max_concurrent_activity_task_polls,
        # On shutdown, stop polling and give in-flight activities this long
        # to finish before they receive cancellation. Must stay under the
        # pod's terminationGracePeriodSeconds or k8s SIGKILLs us anyway.
        graceful_shutdown_timeout=timedelta(
            seconds=_int_env("TEMPORAL_GRACEFUL_SHUTDOWN_S", 25)
        ),
    )
    logger.info(
        (
            "worker starting on task_queue={} with {} workflows + {} activities "
            "(versioning={}, max_activities={}, activity_polls={})"
        ),
        task_queue,
        len(workflows),
        len(activities),
        deployment_config.version if deployment_config else "off",
        max_concurrent_activities,
        max_concurrent_activity_task_polls,
    )

    # k8s sends SIGTERM on pod stop; without a handler the event loop just
    # dies and in-flight activities are killed mid-write. worker.shutdown()
    # stops polling, waits graceful_shutdown_timeout, then cancels stragglers
    # — worker.run() returns once drain completes.
    loop = asyncio.get_running_loop()
    for sig in (signal.SIGTERM, signal.SIGINT):
        loop.add_signal_handler(
            sig,
            lambda s=sig: (
                logger.info("received signal {}, draining worker…", s),
                asyncio.ensure_future(worker.shutdown()),
            ),
        )

    await worker.run()
    logger.info("worker drained, exiting")


if __name__ == "__main__":
    asyncio.run(_main())
