"""Idempotent Temporal Schedule seeding.

Runs as an initContainer on the worker pod. For each scheduled workflow,
attempts `client.create_schedule(...)` and falls through to `update(...)`
if the schedule already exists.

All schedules use `OverlapPolicy.SKIP` (slow run does not queue) and a
`CatchupWindow=1h` (no thundering-herd backfill on worker recovery).

Phase 0 stub. Real seeding implementation lands in Phase 1 alongside
the first workflow port.
"""

from __future__ import annotations

import asyncio
import os
from dataclasses import dataclass
from datetime import timedelta

from temporalio.client import (
    Client,
    Schedule,
    ScheduleActionStartWorkflow,
    ScheduleAlreadyRunningError,
    ScheduleIntervalSpec,
    ScheduleOverlapPolicy,
    SchedulePolicy,
    ScheduleSpec,
)


TASK_QUEUE = "radar-ng"


@dataclass
class ScheduleDef:
    schedule_id: str
    workflow_name: str
    interval: timedelta


SCHEDULES: list[ScheduleDef] = [
    ScheduleDef("ingest-mrms", "IngestMrmsWorkflow", timedelta(minutes=2)),
    ScheduleDef("ingest-hrrr", "IngestHrrrWorkflow", timedelta(minutes=15)),
    ScheduleDef("ingest-lightning", "IngestLightningWorkflow", timedelta(minutes=5)),
    ScheduleDef("ingest-tropical", "IngestTropicalWorkflow", timedelta(hours=1)),
    ScheduleDef("nowcast", "NowcastWorkflow", timedelta(minutes=2)),
    ScheduleDef("tile-cleanup", "TileCleanupWorkflow", timedelta(hours=1)),
    ScheduleDef("poll-alerts", "PollAlertsWorkflow", timedelta(minutes=5)),
]


async def seed(client: Client) -> None:
    for s in SCHEDULES:
        spec = Schedule(
            action=ScheduleActionStartWorkflow(
                s.workflow_name,
                id=f"sched-{s.schedule_id}",
                task_queue=TASK_QUEUE,
            ),
            spec=ScheduleSpec(intervals=[ScheduleIntervalSpec(every=s.interval)]),
            policy=SchedulePolicy(
                overlap=ScheduleOverlapPolicy.SKIP,
                catchup_window=timedelta(hours=1),
            ),
        )
        try:
            await client.create_schedule(s.schedule_id, spec)
            print(f"[seed] created schedule {s.schedule_id}")
        except ScheduleAlreadyRunningError:
            handle = client.get_schedule_handle(s.schedule_id)
            await handle.update(lambda _: spec)
            print(f"[seed] updated schedule {s.schedule_id}")


async def _main() -> None:
    target = os.environ.get("TEMPORAL_ADDRESS", "localhost:7233")
    namespace = os.environ.get("TEMPORAL_NAMESPACE", "default")
    client = await Client.connect(target, namespace=namespace)
    await seed(client)


if __name__ == "__main__":
    asyncio.run(_main())
