import unittest
from datetime import datetime, timedelta, timezone
from types import SimpleNamespace

from temporalio.client import ScheduleOverlapPolicy

from temporal.schedules import watchdog
from temporal.schedules.seed import ScheduleDef

NOW = datetime(2026, 9, 2, 12, 0, tzinfo=timezone.utc)
TWO_MIN = timedelta(minutes=2)


class IsWedgedTests(unittest.TestCase):
    def test_no_next_action_is_not_wedged(self):
        self.assertFalse(watchdog.is_wedged([], [], TWO_MIN, NOW))

    def test_future_next_action_is_not_wedged(self):
        self.assertFalse(watchdog.is_wedged([NOW + TWO_MIN], [], TWO_MIN, NOW))

    def test_slightly_late_is_not_wedged(self):
        # One interval late is normal jitter / slow start; two is the threshold.
        self.assertFalse(watchdog.is_wedged([NOW - TWO_MIN], [], TWO_MIN, NOW))
        self.assertFalse(watchdog.is_wedged([NOW - 2 * TWO_MIN], [], TWO_MIN, NOW))

    def test_overdue_with_nothing_running_is_wedged(self):
        self.assertTrue(
            watchdog.is_wedged([NOW - 2 * TWO_MIN - timedelta(seconds=1)], [], TWO_MIN, NOW)
        )

    def test_overdue_but_running_is_not_wedged(self):
        self.assertFalse(
            watchdog.is_wedged([NOW - timedelta(hours=1)], [object()], TWO_MIN, NOW)
        )


class _Handle:
    def __init__(self, next_action_times, running_actions=(), paused=False, fail=None):
        self.next_action_times = list(next_action_times)
        self.running_actions = list(running_actions)
        self.paused = paused
        self.fail = fail
        self.triggers: list[ScheduleOverlapPolicy | None] = []

    async def describe(self, **_kwargs):
        if self.fail is not None:
            raise self.fail
        return SimpleNamespace(
            schedule=SimpleNamespace(state=SimpleNamespace(paused=self.paused)),
            info=SimpleNamespace(
                next_action_times=self.next_action_times,
                running_actions=self.running_actions,
            ),
        )

    async def trigger(self, *, overlap=None, **_kwargs):
        self.triggers.append(overlap)


class _Client:
    def __init__(self, handle: _Handle) -> None:
        self.handle = handle

    def get_schedule_handle(self, _schedule_id: str) -> _Handle:
        return self.handle


def _def(interval: timedelta = TWO_MIN) -> ScheduleDef:
    return ScheduleDef("poll-x", "XWorkflow", max_runtime=timedelta(minutes=5), interval=interval)


class CheckScheduleTests(unittest.IsolatedAsyncioTestCase):
    async def test_wedged_schedule_is_triggered_with_skip(self):
        handle = _Handle([NOW - timedelta(hours=1)])
        last: dict[str, datetime] = {}

        triggered = await watchdog.check_schedule(_Client(handle), _def(), last, now=NOW)

        self.assertTrue(triggered)
        self.assertEqual(handle.triggers, [ScheduleOverlapPolicy.SKIP])
        self.assertEqual(last["poll-x"], NOW)

    async def test_trigger_is_rate_limited_per_schedule(self):
        handle = _Handle([NOW - timedelta(hours=1)])
        last = {"poll-x": NOW - timedelta(minutes=1)}

        triggered = await watchdog.check_schedule(_Client(handle), _def(), last, now=NOW)

        self.assertFalse(triggered)
        self.assertEqual(handle.triggers, [])

    async def test_trigger_allowed_again_after_gap(self):
        handle = _Handle([NOW - timedelta(hours=1)])
        last = {"poll-x": NOW - watchdog.MIN_TRIGGER_GAP}

        self.assertTrue(await watchdog.check_schedule(_Client(handle), _def(), last, now=NOW))

    async def test_paused_schedule_is_left_alone(self):
        # trigger() runs even on a paused schedule — an operator's incident
        # pause must not be undone by the watchdog.
        handle = _Handle([NOW - timedelta(hours=1)], paused=True)

        triggered = await watchdog.check_schedule(_Client(handle), _def(), {}, now=NOW)

        self.assertFalse(triggered)
        self.assertEqual(handle.triggers, [])

    async def test_healthy_schedule_is_not_triggered(self):
        handle = _Handle([NOW + TWO_MIN])

        self.assertFalse(await watchdog.check_schedule(_Client(handle), _def(), {}, now=NOW))
        self.assertEqual(handle.triggers, [])

    async def test_running_action_is_not_triggered(self):
        handle = _Handle([NOW - timedelta(hours=1)], running_actions=[object()])

        self.assertFalse(await watchdog.check_schedule(_Client(handle), _def(), {}, now=NOW))


class WatchLoopTests(unittest.IsolatedAsyncioTestCase):
    async def test_rpc_error_does_not_escape_loop(self):
        handle = _Handle([], fail=RuntimeError("boom"))
        loop_task = watchdog.watch_schedules(
            _Client(handle), [_def()], check_every=timedelta(0), clock=lambda: NOW
        )
        import asyncio

        task = asyncio.ensure_future(loop_task)
        await asyncio.sleep(0.05)
        self.assertFalse(task.done(), "watchdog loop died on an RPC error")
        task.cancel()
        with self.assertRaises(asyncio.CancelledError):
            await task


if __name__ == "__main__":
    unittest.main()
