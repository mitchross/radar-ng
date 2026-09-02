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


class _RecreateClient:
    """delete() succeeds, create() reports a peer already recreated it."""

    def __init__(self) -> None:
        self.created = 0

    def get_schedule_handle(self, _schedule_id: str):
        async def _delete(**_kwargs):
            return None

        return SimpleNamespace(delete=_delete)

    async def create_schedule(self, *_args, **_kwargs):
        self.created += 1
        raise ScheduleAlreadyRunningError()


class ScheduleSeedTests(unittest.IsolatedAsyncioTestCase):
    def test_every_schedule_sets_its_execution_ceiling(self):
        for definition in seed.SCHEDULES:
            with self.subTest(schedule=definition.schedule_id):
                schedule = seed._spec_for(definition)
                self.assertEqual(
                    schedule.action.execution_timeout,
                    definition.max_runtime,
                )

    def test_spec_carries_catchup_window_and_jitter(self):
        for definition in seed.SCHEDULES:
            with self.subTest(schedule=definition.schedule_id):
                schedule = seed._spec_for(definition)
                self.assertEqual(schedule.policy.catchup_window, definition.catchup_window)
                self.assertEqual(schedule.spec.jitter, definition.jitter)

    def test_two_minute_schedules_use_short_catchup_and_jitter(self):
        fast = [s for s in seed.SCHEDULES if s.interval == timedelta(minutes=2)]
        self.assertEqual(
            sorted(s.schedule_id for s in fast),
            ["ingest-mrms-base", "ingest-mrms-composite", "nowcast"],
        )
        for definition in fast:
            with self.subTest(schedule=definition.schedule_id):
                self.assertEqual(definition.catchup_window, timedelta(minutes=5))
                self.assertEqual(definition.jitter, timedelta(seconds=20))

    def test_slow_schedules_keep_default_catchup(self):
        by_id = {s.schedule_id: s for s in seed.SCHEDULES}
        self.assertEqual(by_id["poll-alerts"].catchup_window, timedelta(minutes=10))
        for schedule_id in ("ingest-hrrr", "ingest-lightning", "tile-cleanup", "open-meteo-sync-gfs"):
            self.assertEqual(by_id[schedule_id].catchup_window, timedelta(hours=1))
            self.assertIsNone(by_id[schedule_id].jitter)

    def test_execution_ceiling_bounds_blocked_fires(self):
        # A stuck run blocks max_runtime / interval fires under SKIP; the
        # 45x ratio on the 2-min schedules is what made one stuck run a
        # 90-minute radar gap.
        for definition in seed.SCHEDULES:
            with self.subTest(schedule=definition.schedule_id):
                self.assertLessEqual(definition.max_runtime, 6 * definition.interval)

    async def test_recreate_wedged_tolerates_peer_recreate(self):
        definition = seed.SCHEDULES[0]
        client = _RecreateClient()

        await seed._recreate_wedged(client, definition, seed._spec_for(definition))

        self.assertEqual(client.created, 1)

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
