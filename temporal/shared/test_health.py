import asyncio
import unittest
from datetime import timedelta

from temporal.shared import health


class _ServiceClient:
    def __init__(self, outcome) -> None:
        self.outcome = outcome

    async def check_health(self, **_kwargs):
        if isinstance(self.outcome, Exception):
            raise self.outcome
        return self.outcome


class _Client:
    def __init__(self, outcome) -> None:
        self.service_client = _ServiceClient(outcome)


class HealthTests(unittest.IsolatedAsyncioTestCase):
    async def test_check_once_true_on_serving(self):
        self.assertTrue(await health.check_once(_Client(True)))

    async def test_check_once_false_on_rpc_error_without_raising(self):
        self.assertFalse(await health.check_once(_Client(RuntimeError("dns error"))))

    async def test_loop_touches_file_only_on_success(self):
        import tempfile
        from pathlib import Path

        with tempfile.TemporaryDirectory() as tmp:
            path = Path(tmp) / "temporal-healthy"

            bad = asyncio.ensure_future(
                health.health_file_loop(_Client(RuntimeError("down")), path=path, every=timedelta(0))
            )
            await asyncio.sleep(0.02)
            bad.cancel()
            with self.assertRaises(asyncio.CancelledError):
                await bad
            self.assertFalse(path.exists(), "health file must not be touched while unreachable")

            good = asyncio.ensure_future(
                health.health_file_loop(_Client(True), path=path, every=timedelta(0))
            )
            await asyncio.sleep(0.02)
            good.cancel()
            with self.assertRaises(asyncio.CancelledError):
                await good
            self.assertTrue(path.exists())


if __name__ == "__main__":
    unittest.main()
