import asyncio
import unittest
from datetime import timedelta
from types import SimpleNamespace
from unittest.mock import patch

from temporalio.client import ScheduleAlreadyRunningError, ScheduleState
from temporalio.service import RPCError, RPCStatusCode

from temporal.schedules import seed


def _definition(schedule_id: str) -> seed.ScheduleDef:
    return seed.ScheduleDef(
        schedule_id,
        "TestWorkflow",
        max_runtime=timedelta(minutes=5),
        interval=timedelta(minutes=2),
    )


def _rpc_error(
    status: RPCStatusCode = RPCStatusCode.DEADLINE_EXCEEDED,
) -> RPCError:
    return RPCError("rpc failed", status, b"")


class _ExistingScheduleHandle:
    def __init__(self, state: ScheduleState) -> None:
        self.state = state
        self.updated_schedule = None
        self.mutations: list[str] = []

    async def update(self, callback, **_kwargs):
        self.mutations.append("update")
        update = callback(
            SimpleNamespace(
                description=SimpleNamespace(schedule=SimpleNamespace(state=self.state))
            )
        )
        self.updated_schedule = update.schedule

    async def describe(self, **_kwargs):
        self.mutations.append("describe")
        raise AssertionError("seeding must not inspect running workflow actions")

    async def delete(self, **_kwargs):
        self.mutations.append("delete")
        raise AssertionError("seeding must not delete schedules")


class _ExistingScheduleClient:
    def __init__(self, state: ScheduleState) -> None:
        self.schedule_handle = _ExistingScheduleHandle(state)
        self.workflow_lookups = 0

    async def create_schedule(self, *_args, **_kwargs):
        raise ScheduleAlreadyRunningError()

    def get_schedule_handle(self, _schedule_id: str):
        return self.schedule_handle

    def get_workflow_handle(self, _workflow_id: str):
        self.workflow_lookups += 1
        raise AssertionError("seeding must not inspect or terminate workflows")


class ScheduleDefinitionTests(unittest.TestCase):
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
                self.assertEqual(
                    schedule.policy.catchup_window,
                    definition.catchup_window,
                )
                self.assertEqual(schedule.spec.jitter, definition.jitter)

    def test_two_minute_schedules_use_short_catchup_and_jitter(self):
        fast = [s for s in seed.SCHEDULES if s.interval == timedelta(minutes=2)]
        self.assertEqual(
            sorted(s.schedule_id for s in fast),
            ["ingest-mrms-base", "ingest-mrms-composite", "nowcast"],
        )
        for definition in fast:
            with self.subTest(schedule=definition.schedule_id):
                self.assertEqual(
                    definition.catchup_window,
                    timedelta(minutes=5),
                )
                self.assertEqual(definition.jitter, timedelta(seconds=20))

    def test_slow_schedules_keep_default_catchup(self):
        by_id = {s.schedule_id: s for s in seed.SCHEDULES}
        self.assertEqual(
            by_id["poll-alerts"].catchup_window,
            timedelta(minutes=10),
        )
        for schedule_id in (
            "ingest-hrrr",
            "ingest-lightning",
            "tile-cleanup",
            "open-meteo-sync-gfs",
        ):
            self.assertEqual(
                by_id[schedule_id].catchup_window,
                timedelta(hours=1),
            )
            self.assertIsNone(by_id[schedule_id].jitter)

    def test_execution_ceiling_bounds_blocked_fires(self):
        # Under SKIP, a hung run may block only a bounded number of cadences.
        for definition in seed.SCHEDULES:
            with self.subTest(schedule=definition.schedule_id):
                self.assertLessEqual(
                    definition.max_runtime,
                    6 * definition.interval,
                )


class ScheduleApplyTests(unittest.IsolatedAsyncioTestCase):
    async def test_existing_schedule_update_preserves_operator_state(self):
        live_state = ScheduleState(
            note="incident pause",
            paused=True,
            limited_actions=True,
            remaining_actions=3,
        )
        client = _ExistingScheduleClient(live_state)
        definition = _definition("existing")

        await seed._apply(client, definition, seed._spec_for(definition))

        self.assertEqual(client.schedule_handle.updated_schedule.state, live_state)
        self.assertEqual(client.schedule_handle.mutations, ["update"])
        self.assertEqual(client.workflow_lookups, 0)

    async def test_reconciliation_has_no_destructive_recovery_helpers(self):
        self.assertFalse(hasattr(seed, "_recreate_wedged"))
        self.assertFalse(hasattr(seed, "_terminate_stale_actions"))

        client = _ExistingScheduleClient(ScheduleState())
        definition = _definition("no-destructive-recovery")
        await seed._apply(client, definition, seed._spec_for(definition))

        self.assertNotIn("delete", client.schedule_handle.mutations)
        self.assertNotIn("describe", client.schedule_handle.mutations)
        self.assertEqual(client.workflow_lookups, 0)


class SeedIsolationTests(unittest.IsolatedAsyncioTestCase):
    async def test_one_failure_does_not_block_other_schedules(self):
        bad = _definition("bad")
        good = _definition("good")
        applied: list[str] = []

        async def apply(_client, definition, _spec):
            applied.append(definition.schedule_id)
            if definition is bad:
                raise ValueError("invalid bad definition")

        with (
            patch.object(seed, "SCHEDULES", [bad, good]),
            patch.object(seed, "_apply", side_effect=apply),
        ):
            with self.assertRaises(seed.ScheduleSeedError) as raised:
                await seed.seed(object())

        self.assertCountEqual(applied, ["bad", "good"])
        self.assertEqual(set(raised.exception.failures), {"bad"})

    async def test_schedule_passes_start_concurrently(self):
        first = _definition("first")
        second = _definition("second")
        started: set[str] = set()
        both_started = asyncio.Event()
        release = asyncio.Event()

        async def apply(_client, definition, _spec):
            started.add(definition.schedule_id)
            if len(started) == 2:
                both_started.set()
            await release.wait()

        with (
            patch.object(seed, "SCHEDULES", [first, second]),
            patch.object(seed, "_apply", side_effect=apply),
        ):
            task = asyncio.create_task(seed.seed(object()))
            await asyncio.wait_for(both_started.wait(), timeout=1)
            self.assertFalse(task.done())
            release.set()
            await asyncio.wait_for(task, timeout=1)

        self.assertEqual(started, {"first", "second"})

    async def test_retries_only_transient_failures_and_keeps_permanent_error(self):
        transient = _definition("transient")
        permanent = _definition("permanent")
        healthy = _definition("healthy")
        attempts = {
            definition.schedule_id: 0 for definition in (transient, permanent, healthy)
        }

        async def apply(_client, definition, _spec):
            attempts[definition.schedule_id] += 1
            if definition is transient and attempts[definition.schedule_id] == 1:
                raise _rpc_error()
            if definition is permanent:
                raise ValueError("bad configuration")

        with (
            patch.object(seed, "SCHEDULES", [transient, permanent, healthy]),
            patch.object(seed, "_apply", side_effect=apply),
        ):
            with self.assertRaises(seed.ScheduleSeedError) as raised:
                await seed.seed_with_retry(
                    object(),
                    max_attempts=2,
                    base_delay=0,
                    max_delay=0,
                )

        self.assertEqual(
            attempts,
            {"transient": 2, "permanent": 1, "healthy": 1},
        )
        self.assertEqual(set(raised.exception.failures), {"permanent"})

    async def test_timeout_exhaustion_never_deletes_or_terminates(self):
        definition = _definition("timed-out")
        calls: list[str] = []

        class Client:
            async def create_schedule(self, *_args, **_kwargs):
                calls.append("create")
                raise _rpc_error()

            def get_schedule_handle(self, _schedule_id):
                calls.append("get-schedule-handle")
                raise AssertionError("timeout recovery must not mutate a Schedule")

            def get_workflow_handle(self, _workflow_id):
                calls.append("get-workflow-handle")
                raise AssertionError("timeout recovery must not terminate a workflow")

        with patch.object(seed, "SCHEDULES", [definition]):
            with self.assertRaises(seed.ScheduleSeedError) as raised:
                await seed.seed_with_retry(
                    Client(),
                    max_attempts=3,
                    base_delay=0,
                    max_delay=0,
                )

        self.assertEqual(calls, ["create", "create", "create"])
        self.assertEqual(set(raised.exception.failures), {"timed-out"})

    async def test_invalid_attempt_count_is_rejected(self):
        with self.assertRaisesRegex(ValueError, "max_attempts"):
            await seed.seed_with_retry(object(), max_attempts=0)


if __name__ == "__main__":
    unittest.main()
