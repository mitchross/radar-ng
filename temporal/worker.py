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

from backend.ingest_mrms.activities import (
    mrms_cleanup,
    mrms_list_unprocessed_keys,
    mrms_mark_processed,
    mrms_process_frame,
)
from temporal.shared.otel import init_tracer
from temporal.shared.push import send_push_notification
from temporal.workflows import ALL_WORKFLOWS


TASK_QUEUE = "radar-ng"


# Activities are registered here as each workflow is ported. Phase 1 lights
# up the full ingest-mrms pipeline; remaining services follow in Phase 2.
ALL_ACTIVITIES = [
    # ingest-mrms
    mrms_list_unprocessed_keys,
    mrms_process_frame,
    mrms_mark_processed,
    mrms_cleanup,
    # storm-watch (Phase 3 stub)
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
