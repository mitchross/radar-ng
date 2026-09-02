import asyncio
import sys
import types
import unittest
from unittest.mock import patch

from loguru import logger as loguru_logger
from temporalio.service import RPCError, RPCStatusCode

from temporal.schedules.seed import ScheduleSeedError


def _placeholder(*_args, **_kwargs):
    return None


def _stub_module(name: str, members: list[str]) -> types.ModuleType:
    module = types.ModuleType(name)
    for member in members:
        if member.endswith("Workflow"):
            value = type(member, (), {})
        else:
            value = _placeholder
        setattr(module, member, value)
    return module


# The production worker image contains GRIB, SciPy, and OpenTelemetry wheels.
# Unit-test environments intentionally do not. Stub only registration imports
# so this test can exercise the worker supervisor without importing native I/O
# dependencies.
_IMPORT_STUBS = {
    "backend.api.api.storm_watch_activities": _stub_module(
        "backend.api.api.storm_watch_activities",
        [
            "compare_radar_frames",
            "delete_push_token",
            "detect_storm_change",
            "fan_out_push_to_user",
            "fetch_nws_active_alerts",
            "mark_alerts_seen",
            "persist_push_token",
            "signal_matching_storm_watches",
        ],
    ),
    "backend.ingest_airquality.activities": _stub_module(
        "backend.ingest_airquality.activities",
        [
            "aqm_cleanup",
            "aqm_find_latest_run",
            "aqm_mark_processed",
            "aqm_publish_run",
            "aqm_render_chunk",
        ],
    ),
    "backend.ingest_hrrr.activities": _stub_module(
        "backend.ingest_hrrr.activities",
        [
            "hrrr_cleanup",
            "hrrr_find_latest_run",
            "hrrr_horizon_for_run",
            "hrrr_mark_processed",
            "hrrr_process_forecast_hour",
            "hrrr_publish_run",
        ],
    ),
    "backend.ingest_lightning.activities": _stub_module(
        "backend.ingest_lightning.activities",
        ["lightning_consume_stream"],
    ),
    "backend.ingest_mrms.activities": _stub_module(
        "backend.ingest_mrms.activities",
        [
            "mrms_cleanup",
            "mrms_list_unprocessed_keys",
            "mrms_mark_processed",
            "mrms_process_frame",
        ],
    ),
    "backend.ingest_tropical.activities": _stub_module(
        "backend.ingest_tropical.activities",
        ["tropical_fetch_and_publish"],
    ),
    "backend.nowcast.activities": _stub_module(
        "backend.nowcast.activities",
        ["nowcast_run"],
    ),
    "backend.tile_cleanup.activities": _stub_module(
        "backend.tile_cleanup.activities",
        ["tile_cleanup_sweep"],
    ),
    "temporal.shared.health": _stub_module(
        "temporal.shared.health",
        ["health_file_loop"],
    ),
    "temporal.shared.otel": _stub_module(
        "temporal.shared.otel",
        ["init_tracer"],
    ),
    "temporal.shared.push": _stub_module(
        "temporal.shared.push",
        ["send_push_notification"],
    ),
}

for module_name, workflow_name in {
    "temporal.workflows.ingest_airquality": "IngestAirQualityWorkflow",
    "temporal.workflows.ingest_hrrr": "IngestHrrrWorkflow",
    "temporal.workflows.ingest_lightning": "IngestLightningWorkflow",
    "temporal.workflows.ingest_mrms": "IngestMrmsWorkflow",
    "temporal.workflows.ingest_tropical": "IngestTropicalWorkflow",
    "temporal.workflows.nowcast": "NowcastWorkflow",
    "temporal.workflows.open_meteo_sync": "OpenMeteoSyncWorkflow",
    "temporal.workflows.poll_alerts": "PollAlertsWorkflow",
    "temporal.workflows.register_push_token": "RegisterPushTokenWorkflow",
    "temporal.workflows.tile_cleanup": "TileCleanupWorkflow",
    "temporal.workflows.watch_storm": "WatchStormWorkflow",
}.items():
    _IMPORT_STUBS[module_name] = _stub_module(module_name, [workflow_name])

with patch.dict(sys.modules, _IMPORT_STUBS):
    from temporal import worker


class _RecordingLogger:
    def __init__(self, records=None, fields=None):
        self.records = records if records is not None else []
        self.fields = fields or {}

    def bind(self, **fields):
        return _RecordingLogger(self.records, self.fields | fields)

    def info(self, message, *args):
        self.records.append(("info", self.fields, message, args))

    def critical(self, message, *args):
        self.records.append(("critical", self.fields, message, args))


class WorkerSupervisionTests(unittest.IsolatedAsyncioTestCase):
    async def test_reconciliation_failure_is_structured_and_non_fatal(self):
        failure = ScheduleSeedError({"nowcast": RuntimeError("unavailable")})
        recording_logger = _RecordingLogger()

        async def fail(_client):
            raise failure

        with (
            patch.object(worker, "seed_schedules", side_effect=fail),
            patch.object(worker, "logger", recording_logger),
        ):
            await worker._reconcile_schedules(object())

        critical = [
            record for record in recording_logger.records if record[0] == "critical"
        ]
        self.assertEqual(len(critical), 1)
        fields = critical[0][1]
        self.assertEqual(
            fields["event"],
            "TEMPORAL_SCHEDULE_RECONCILIATION_FAILED",
        )
        self.assertEqual(fields["schedule_ids"], ["nowcast"])
        self.assertEqual(fields["errors"], "nowcast:RuntimeError")

    async def test_default_message_render_has_ids_and_safe_error_labels(self):
        failure = ScheduleSeedError(
            {
                "nowcast": RPCError(
                    "secret payload must not render",
                    RPCStatusCode.DEADLINE_EXCEEDED,
                    b"private details",
                ),
                "poll-alerts": ValueError("sensitive upstream response"),
            }
        )
        rendered: list[str] = []

        async def fail(_client):
            raise failure

        sink_id = loguru_logger.add(
            lambda message: rendered.append(str(message).strip()),
            level="CRITICAL",
        )
        try:
            with patch.object(worker, "seed_schedules", side_effect=fail):
                await worker._reconcile_schedules(object())
        finally:
            loguru_logger.remove(sink_id)

        self.assertEqual(len(rendered), 1)
        for field in (
            "event=TEMPORAL_SCHEDULE_RECONCILIATION_FAILED",
            "schedule_ids=nowcast,poll-alerts",
            "errors=nowcast:RPCError[DEADLINE_EXCEEDED],poll-alerts:ValueError",
            "operator_action=inspect_schedule_reconciliation",
        ):
            with self.subTest(field=field):
                self.assertIn(field, rendered[0])
        self.assertNotIn("secret payload", rendered[0])
        self.assertNotIn("sensitive upstream", rendered[0])
        self.assertNotIn("private details", rendered[0])

        self.assertNotIn("worker_polling=active", rendered[0])

    async def test_reconciliation_waits_for_validation_and_poller_start(self):
        events: list[str] = []
        validation_started = asyncio.Event()
        allow_validation = asyncio.Event()
        poller_started = asyncio.Event()
        seed_started = asyncio.Event()

        async def fail_seed(_client):
            events.append("seed")
            seed_started.set()
            raise ScheduleSeedError({"nowcast": RuntimeError("timeout")})

        async def stay_alive(*_args, **_kwargs):
            await asyncio.Event().wait()

        class FakeWorker:
            is_running = False

            async def run(self):
                events.append("validate")
                validation_started.set()
                await allow_validation.wait()
                self.is_running = True

                async def poll():
                    events.append("poll")
                    poller_started.set()
                    await asyncio.Event().wait()

                poller = asyncio.create_task(poll())
                try:
                    await seed_started.wait()
                    # Give the reconciliation wrapper time to consume the error.
                    await asyncio.sleep(0)
                finally:
                    self.is_running = False
                    poller.cancel()
                    await asyncio.gather(poller, return_exceptions=True)

        with (
            patch.object(worker, "seed_schedules", side_effect=fail_seed),
            patch.object(worker, "health_file_loop", side_effect=stay_alive),
            patch.object(worker, "watch_schedules", side_effect=stay_alive),
            patch.object(worker, "logger", _RecordingLogger()),
        ):
            run_task = asyncio.create_task(
                worker._run_worker(FakeWorker(), object(), should_seed=True)
            )
            await validation_started.wait()
            await asyncio.sleep(worker.WORKER_START_POLL_SECONDS * 2)
            self.assertFalse(seed_started.is_set())
            self.assertFalse(poller_started.is_set())

            allow_validation.set()
            await run_task

        self.assertEqual(events, ["validate", "poll", "seed"])

    async def test_validation_failure_starts_no_background_work(self):
        calls: list[str] = []

        async def forbidden(name):
            calls.append(name)

        class FakeWorker:
            is_running = False

            async def run(self):
                await asyncio.sleep(0)
                raise RuntimeError("namespace validation failed")

        with (
            patch.object(
                worker,
                "health_file_loop",
                side_effect=lambda *_args: forbidden("health"),
            ),
            patch.object(
                worker,
                "_reconcile_schedules",
                side_effect=lambda *_args: forbidden("reconcile"),
            ),
            patch.object(
                worker,
                "watch_schedules",
                side_effect=lambda *_args: forbidden("watchdog"),
            ),
            self.assertRaisesRegex(RuntimeError, "namespace validation failed"),
        ):
            await worker._run_worker(FakeWorker(), object(), should_seed=True)

        self.assertEqual(calls, [])

    async def test_non_seeding_pool_does_not_start_schedule_tasks(self):
        health_started = asyncio.Event()

        async def health(*_args, **_kwargs):
            health_started.set()
            await asyncio.Event().wait()

        async def forbidden(*_args, **_kwargs):
            raise AssertionError("non-seeding pool started schedule control work")

        class FakeWorker:
            is_running = False

            async def run(self):
                self.is_running = True
                try:
                    await health_started.wait()
                finally:
                    self.is_running = False

        with (
            patch.object(worker, "health_file_loop", side_effect=health),
            patch.object(worker, "_reconcile_schedules", side_effect=forbidden),
            patch.object(worker, "watch_schedules", side_effect=forbidden),
        ):
            await worker._run_worker(FakeWorker(), object(), should_seed=False)


if __name__ == "__main__":
    unittest.main()
