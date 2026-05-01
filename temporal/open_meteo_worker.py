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
from temporalio.worker import Worker

from backend.open_meteo_sync.activities import open_meteo_sync


TASK_QUEUE = os.environ.get("TEMPORAL_TASK_QUEUE", "radar-ng")


async def _main() -> None:
    target = os.environ.get("TEMPORAL_ADDRESS", "localhost:7233")
    namespace = os.environ.get("TEMPORAL_NAMESPACE", "default")
    logger.info("open-meteo worker connecting to {} (ns={})", target, namespace)

    client = await Client.connect(target, namespace=namespace)
    worker = Worker(
        client,
        task_queue=TASK_QUEUE,
        workflows=[],
        activities=[open_meteo_sync],
    )
    logger.info("open-meteo worker starting on {} with 1 activity", TASK_QUEUE)
    await worker.run()


if __name__ == "__main__":
    asyncio.run(_main())
