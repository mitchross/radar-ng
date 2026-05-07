"""Secondary Temporal worker — registers ONLY the open-meteo sync activity.

Runs in its own pod (`radar-ng-open-meteo-worker`) with the open-meteo Swift
binary baked into the image. Polls the same `radar-ng` task queue as the
main worker; Temporal dispatches `open_meteo_sync` activity tasks here
because no other worker has it registered.

Workflow definitions are NOT registered here — workflow tasks for
OpenMeteoSyncWorkflow always go to the main radar-ng-temporal-worker pod.
"""

from __future__ import annotations

import asyncio
import os

from loguru import logger
from temporalio.client import Client
from temporalio.common import VersioningBehavior
from temporalio.worker import Worker, WorkerDeploymentConfig, WorkerDeploymentVersion

from backend.open_meteo_sync.activities import open_meteo_sync


TASK_QUEUE = os.environ.get("TEMPORAL_TASK_QUEUE", "radar-ng-open-meteo")


def _deployment_config_from_env() -> WorkerDeploymentConfig | None:
    # The TemporalWorkerDeployment controller injects both env vars on every
    # in-cluster pod. Without them the worker controller never marks our
    # build_id as registered and the rollout sunset gets stuck — see
    # main worker.py for the same pattern.
    name = os.environ.get("TEMPORAL_DEPLOYMENT_NAME")
    build_id = os.environ.get("TEMPORAL_WORKER_BUILD_ID")
    if not name or not build_id:
        return None
    return WorkerDeploymentConfig(
        version=WorkerDeploymentVersion(deployment_name=name, build_id=build_id),
        use_worker_versioning=True,
        default_versioning_behavior=VersioningBehavior.PINNED,
    )


async def _main() -> None:
    target = os.environ.get("TEMPORAL_ADDRESS", "localhost:7233")
    namespace = os.environ.get("TEMPORAL_NAMESPACE", "default")
    logger.info("open-meteo worker connecting to {} (ns={})", target, namespace)

    client = await Client.connect(target, namespace=namespace)
    deployment_config = _deployment_config_from_env()
    worker = Worker(
        client,
        task_queue=TASK_QUEUE,
        workflows=[],
        activities=[open_meteo_sync],
        deployment_config=deployment_config,
    )
    logger.info(
        "open-meteo worker starting on {} with 1 activity (versioning={})",
        TASK_QUEUE,
        deployment_config.version if deployment_config else "off",
    )
    await worker.run()


if __name__ == "__main__":
    asyncio.run(_main())
