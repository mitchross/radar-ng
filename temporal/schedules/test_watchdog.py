import asyncio
import unittest
from datetime import datetime, timedelta, timezone
from types import SimpleNamespace
from unittest.mock import patch

from loguru import logger as loguru_logger

from temporal.schedules import watchdog
from temporal.schedules.seed import ScheduleDef

NOW = datetime(2026, 9, 2, 12, 0, tzinfo=timezone.utc)
TWO_MIN = timedelta(minutes=2)


class IsWedgedTests(unittest.TestCase):
    def test_no_next_action_is_not_wedged(self):
        self.assertFalse(watchdog.is_wedged([], [], TWO_MIN, NOW))

    def test_future_next_action_is_not_wedged(self):
        self.assertFalse(watchdog.is_wedged([NOW + TWO_MIN], [], TWO_MIN, NOW))

    def test_threshold_is_strictly_more_than_two_intervals(self):
        self.assertFalse(watchdog.is_wedged([NOW - TWO_MIN], [], TWO_MIN, NOW))
        self.assertFalse(watchdog.is_wedged([NOW - 2 * TWO_MIN], [], TWO_MIN, NOW))
        self.assertTrue(
            watchdog.is_wedged(
                [NOW - 2 * TWO_MIN - timedelta(seconds=1)],
                [],
                TWO_MIN,
                NOW,
            )
        )

    def test_running_action_is_not_wedged(self):
        self.assertFalse(
            watchdog.is_wedged(
                [NOW - timedelta(hours=1)],
                [object()],
                TWO_MIN,
                NOW,
            )
        )


class _RecordingLogger:
    def __init__(self, records=None, fields=None):
        self.records = records if records is not None else []
        self.fields = fields or {}

    def bind(self, **fields):
        return _RecordingLogger(self.records, self.fields | fields)

    def critical(self, message, *args):
        self.records.append(("critical", self.fields, message, args))

    def warning(self, message, *args):
        self.records.append(("warning", self.fields, message, args))


class _Handle:
    def __init__(
        self,
        next_action_times,
        running_actions=(),
        *,
        paused=False,
        fail=None,
        barrier_count=0,
    ):
        self.next_action_times = list(next_action_times)
        self.running_actions = list(running_actions)
        self.paused = paused
        self.fail = fail
        self.describe_calls = 0
        self.described = asyncio.Event()
        self._barrier_count = barrier_count
        self._barrier = asyncio.Event()
        self.mutations: list[str] = []

    async def describe(self, **_kwargs):
        self.describe_calls += 1
        self.described.set()
        if self._barrier_count:
            if self.describe_calls >= self._barrier_count:
                self._barrier.set()
            await self._barrier.wait()
        if self.fail is not None:
            raise self.fail
        return SimpleNamespace(
            schedule=SimpleNamespace(state=SimpleNamespace(paused=self.paused)),
            info=SimpleNamespace(
                next_action_times=list(self.next_action_times),
                running_actions=list(self.running_actions),
            ),
        )

    async def trigger(self, **_kwargs):
        self.mutations.append("trigger")
        raise AssertionError("observer must not trigger")

    async def update(self, *_args, **_kwargs):
        self.mutations.append("update")
        raise AssertionError("observer must not update")

    async def delete(self, **_kwargs):
        self.mutations.append("delete")
        raise AssertionError("observer must not delete")

    async def terminate(self, **_kwargs):
        self.mutations.append("terminate")
        raise AssertionError("observer must not terminate")


class _BlockingHandle(_Handle):
    def __init__(self, next_action_times):
        super().__init__(next_action_times)
        self.release = asyncio.Event()

    async def describe(self, **kwargs):
        self.described.set()
        await self.release.wait()
        return await super().describe(**kwargs)


class _Client:
    def __init__(self, handles):
        self.handles = handles if isinstance(handles, dict) else {"poll-x": handles}
        self.mutations: list[str] = []

    def get_schedule_handle(self, schedule_id: str):
        return self.handles[schedule_id]

    def get_workflow_handle(self, _workflow_id: str):
        self.mutations.append("get-workflow-for-termination")
        raise AssertionError("observer must not inspect or terminate workflows")


def _def(
    schedule_id: str = "poll-x",
    interval: timedelta = TWO_MIN,
) -> ScheduleDef:
    return ScheduleDef(
        schedule_id,
        "XWorkflow",
        max_runtime=timedelta(minutes=5),
        interval=interval,
    )


class CheckScheduleTests(unittest.IsolatedAsyncioTestCase):
    async def test_stall_emits_structured_critical_without_mutation(self):
        handle = _Handle([NOW - timedelta(hours=1)])
        client = _Client(handle)
        reports: dict[str, datetime] = {}
        recording_logger = _RecordingLogger()

        with patch.object(watchdog, "logger", recording_logger):
            emitted = await watchdog.check_schedule(
                client,
                _def(),
                reports,
                now=NOW,
            )

        self.assertTrue(emitted)
        self.assertEqual(handle.mutations, [])
        self.assertEqual(client.mutations, [])
        self.assertEqual(reports, {"poll-x": NOW - timedelta(hours=1)})
        self.assertEqual(len(recording_logger.records), 1)
        level, fields, _message, _args = recording_logger.records[0]
        self.assertEqual(level, "critical")
        self.assertEqual(fields["event"], watchdog.STALL_EVENT)
        self.assertEqual(fields["schedule_id"], "poll-x")
        self.assertEqual(fields["workflow_name"], "XWorkflow")
        self.assertEqual(fields["next_action_time"], "2026-09-02T11:00:00+00:00")
        self.assertEqual(fields["overdue_seconds"], 3600)
        self.assertFalse(fields["automatic_recovery"])
        self.assertEqual(fields["operator_action"], "inspect_temporal_timer_dlq")

    async def test_default_message_render_contains_every_actionable_field(self):
        handle = _Handle([NOW - timedelta(hours=1)])
        definition = _def()
        rendered: list[str] = []
        sink_id = loguru_logger.add(
            lambda message: rendered.append(str(message).strip()),
            level="CRITICAL",
        )

        try:
            await watchdog.check_schedule(_Client(handle), definition, {}, now=NOW)
        finally:
            loguru_logger.remove(sink_id)

        self.assertEqual(len(rendered), 1)
        for field in (
            "event=TEMPORAL_SCHEDULE_STALLED",
            "schedule_id=poll-x",
            "workflow_name=XWorkflow",
            f"task_queue={definition.task_queue}",
            "next_action_time=2026-09-02T11:00:00+00:00",
            "overdue_seconds=3600",
            "automatic_recovery=false",
            "operator_action=inspect_temporal_timer_dlq",
        ):
            with self.subTest(field=field):
                self.assertIn(field, rendered[0])

    async def test_same_stalled_timer_is_suppressed_in_one_process(self):
        handle = _Handle([NOW - timedelta(hours=1)])
        reports: dict[str, datetime] = {}
        recording_logger = _RecordingLogger()

        with patch.object(watchdog, "logger", recording_logger):
            first = await watchdog.check_schedule(
                _Client(handle), _def(), reports, now=NOW
            )
            second = await watchdog.check_schedule(
                _Client(handle), _def(), reports, now=NOW
            )

        self.assertTrue(first)
        self.assertFalse(second)
        self.assertEqual(len(recording_logger.records), 1)
        self.assertEqual(handle.mutations, [])

    async def test_concurrent_checks_emit_once_and_never_mutate(self):
        handle = _Handle([NOW - timedelta(hours=1)], barrier_count=2)
        client = _Client(handle)
        reports: dict[str, datetime] = {}
        recording_logger = _RecordingLogger()

        with patch.object(watchdog, "logger", recording_logger):
            results = await asyncio.gather(
                watchdog.check_schedule(client, _def(), reports, now=NOW),
                watchdog.check_schedule(client, _def(), reports, now=NOW),
            )

        self.assertEqual(sorted(results), [False, True])
        self.assertEqual(len(recording_logger.records), 1)
        self.assertEqual(handle.mutations, [])
        self.assertEqual(client.mutations, [])

    async def test_process_restart_intentionally_reports_same_timer_again(self):
        handle = _Handle([NOW - timedelta(hours=1)])
        recording_logger = _RecordingLogger()

        with patch.object(watchdog, "logger", recording_logger):
            first_process = await watchdog.check_schedule(
                _Client(handle), _def(), {}, now=NOW
            )
            restarted_process = await watchdog.check_schedule(
                _Client(handle), _def(), {}, now=NOW
            )

        self.assertTrue(first_process)
        self.assertTrue(restarted_process)
        self.assertEqual(len(recording_logger.records), 2)
        self.assertEqual(handle.mutations, [])

    async def test_future_timer_clears_suppression_for_a_later_stall(self):
        old_timer = NOW - timedelta(hours=1)
        new_timer = NOW + TWO_MIN
        handle = _Handle([new_timer])
        reports = {"poll-x": old_timer}
        recording_logger = _RecordingLogger()

        with patch.object(watchdog, "logger", recording_logger):
            self.assertFalse(
                await watchdog.check_schedule(_Client(handle), _def(), reports, now=NOW)
            )
            self.assertEqual(reports, {})
            handle.next_action_times = [new_timer]
            self.assertTrue(
                await watchdog.check_schedule(
                    _Client(handle),
                    _def(),
                    reports,
                    now=NOW + timedelta(hours=1),
                )
            )

        self.assertEqual(len(recording_logger.records), 1)
        self.assertEqual(handle.mutations, [])

    async def test_running_action_does_not_clear_or_emit(self):
        stalled_timer = NOW - timedelta(hours=1)
        handle = _Handle([stalled_timer], running_actions=[object()])
        reports = {"poll-x": stalled_timer}
        recording_logger = _RecordingLogger()

        with patch.object(watchdog, "logger", recording_logger):
            emitted = await watchdog.check_schedule(
                _Client(handle), _def(), reports, now=NOW
            )

        self.assertFalse(emitted)
        self.assertEqual(reports, {"poll-x": stalled_timer})
        self.assertEqual(recording_logger.records, [])
        self.assertEqual(handle.mutations, [])

    async def test_paused_schedule_clears_suppression_without_mutation(self):
        stalled_timer = NOW - timedelta(hours=1)
        handle = _Handle([stalled_timer], paused=True)
        reports = {"poll-x": stalled_timer}

        emitted = await watchdog.check_schedule(
            _Client(handle), _def(), reports, now=NOW
        )

        self.assertFalse(emitted)
        self.assertEqual(reports, {})
        self.assertEqual(handle.mutations, [])


class WatchLoopTests(unittest.IsolatedAsyncioTestCase):
    async def test_one_blocked_describe_does_not_block_another_schedule(self):
        slow = _BlockingHandle([NOW + TWO_MIN])
        fast = _Handle([NOW + TWO_MIN])
        client = _Client({"slow": slow, "fast": fast})
        task = asyncio.create_task(
            watchdog.check_schedules_once(
                client,
                [_def("slow"), _def("fast")],
                {},
                now=NOW,
            )
        )

        await asyncio.wait_for(fast.described.wait(), timeout=1)
        self.assertFalse(task.done())
        slow.release.set()
        await asyncio.wait_for(task, timeout=1)

    async def test_rpc_error_is_isolated_from_other_schedules(self):
        broken = _Handle([], fail=RuntimeError("boom"))
        stalled = _Handle([NOW - timedelta(hours=1)])
        client = _Client({"broken": broken, "stalled": stalled})
        reports: dict[str, datetime] = {}
        recording_logger = _RecordingLogger()

        with patch.object(watchdog, "logger", recording_logger):
            await watchdog.check_schedules_once(
                client,
                [_def("broken"), _def("stalled")],
                reports,
                now=NOW,
            )

        self.assertIn("stalled", reports)
        self.assertEqual(
            [level for level, _fields, _message, _args in recording_logger.records],
            ["warning", "critical"],
        )
        self.assertEqual(broken.mutations, [])
        self.assertEqual(stalled.mutations, [])


if __name__ == "__main__":
    unittest.main()
