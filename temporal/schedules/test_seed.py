from datetime import datetime, timedelta, timezone
from types import SimpleNamespace
import unittest

from temporalio.client import (
    ScheduleActionExecutionStartWorkflow,
    ScheduleAlreadyRunningError,
)

from temporal.schedules import seed


class _WorkflowHandle:
    def __init__(self, start_time: datetime) -> None:
        self.start_time = start_time
        self.terminated = False

    async def describe(self, **_kwargs):
        return SimpleNamespace(start_time=self.start_time)

    async def terminate(self, **_kwargs):
        self.terminated = True


class _ScheduleHandle:
    def __init__(self, workflow_id: str) -> None:
        self.workflow_id = workflow_id

    async def update(self, _callback, **_kwargs):
        return None

    async def describe(self, **_kwargs):
        action = ScheduleActionExecutionStartWorkflow(
            workflow_id=self.workflow_id,
            first_execution_run_id="first-run",
        )
        return SimpleNamespace(info=SimpleNamespace(running_actions=[action]))


class _ExistingScheduleClient:
    def __init__(self, workflow_id: str, start_time: datetime) -> None:
        self.schedule_handle = _ScheduleHandle(workflow_id)
        self.workflow_handle = _WorkflowHandle(start_time)

    async def create_schedule(self, *_args, **_kwargs):
        raise ScheduleAlreadyRunningError()

    def get_schedule_handle(self, _schedule_id: str):
        return self.schedule_handle

    def get_workflow_handle(self, _workflow_id: str):
        return self.workflow_handle


class ScheduleSeedTests(unittest.IsolatedAsyncioTestCase):
    def test_every_schedule_sets_its_execution_ceiling(self):
        for definition in seed.SCHEDULES:
            with self.subTest(schedule=definition.schedule_id):
                schedule = seed._spec_for(definition)
                self.assertEqual(
                    schedule.action.execution_timeout,
                    definition.max_runtime,
                )

    async def test_existing_action_older_than_ceiling_is_terminated(self):
        definition = seed.SCHEDULES[0]
        client = _ExistingScheduleClient(
            "stale-mrms-run",
            datetime.now(timezone.utc) - definition.max_runtime - timedelta(minutes=1),
        )

        await seed._apply(client, definition, seed._spec_for(definition))

        self.assertTrue(client.workflow_handle.terminated)

    async def test_existing_action_inside_ceiling_is_preserved(self):
        definition = seed.SCHEDULES[0]
        client = _ExistingScheduleClient(
            "current-mrms-run",
            datetime.now(timezone.utc) - definition.max_runtime + timedelta(minutes=1),
        )

        await seed._apply(client, definition, seed._spec_for(definition))

        self.assertFalse(client.workflow_handle.terminated)


if __name__ == "__main__":
    unittest.main()
