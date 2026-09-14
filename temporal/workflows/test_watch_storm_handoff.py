"""Real-server version handoff with signals on both sides of Continue-as-New."""

import asyncio
import os
import unittest
import uuid
from datetime import timedelta

from temporalio import activity
from temporalio.api.workflowservice.v1 import SetWorkerDeploymentCurrentVersionRequest
from temporalio.client import Client
from temporalio.common import VersioningBehavior, WorkerDeploymentVersion
from temporalio.converter import DataConverter
from temporalio.testing import WorkflowEnvironment
from temporalio.worker import Worker, WorkerDeploymentConfig

from backend.api.api.storm_watch_activities import (
    CompareFramesInput, CompareFramesResult, DetectChangeInput, DetectChangeResult,
    FanOutPushInput, FanOutPushResult,
)
from temporal.workflows.watch_storm import WatchStormHandoff, WatchStormInput, WatchStormState, WatchStormWorkflow


class Fixtures:
    def __init__(self, build):
        self.build = build
        self.compares = []
        self.alerts = []
        self.comparing = asyncio.Event()
        self.release_compare = asyncio.Event()
        self.pushing = asyncio.Event()
        self.release_push = asyncio.Event()

    @activity.defn(name="compare_radar_frames")
    async def compare(self, inp: CompareFramesInput) -> CompareFramesResult:
        self.compares.append(inp)
        self.comparing.set()
        if self.build == "v1":
            await self.release_compare.wait()
            return CompareFramesResult(sampled=True, curr_timestamp="synthetic-frame", curr_max_dbz=42)
        return CompareFramesResult(sampled=False)

    @activity.defn(name="detect_storm_change")
    async def detect(self, inp: DetectChangeInput) -> DetectChangeResult:
        return DetectChangeResult(kind="", summary="")

    @activity.defn(name="fan_out_push_to_user")
    async def push(self, inp: FanOutPushInput) -> FanOutPushResult:
        self.alerts.append(inp.extra["alert_id"])
        self.pushing.set()
        await self.release_push.wait()
        return FanOutPushResult(sent=1)


class StormHandoffTests(unittest.IsolatedAsyncioTestCase):
    async def test_version_upgrade_preserves_state_and_alerts_during_handoff(self):
        env = None
        if address := os.environ.get("TEMPORAL_TEST_ADDRESS"):
            client = await Client.connect(address)
        else:
            env = await WorkflowEnvironment.start_local(
                dev_server_existing_path=os.environ.get("TEMPORAL_TEST_SERVER"),
            )
            client = env.client
        name = f"synthetic-handoff-{uuid.uuid4().hex}"
        first, second = Fixtures("v1"), Fixtures("v2")
        def make_worker(fixture):
            return Worker(
                client, task_queue=name, workflows=[WatchStormWorkflow],
                activities=[fixture.compare, fixture.detect, fixture.push],
                deployment_config=WorkerDeploymentConfig(
                    version=WorkerDeploymentVersion(deployment_name=name, build_id=fixture.build),
                    use_worker_versioning=True,
                    default_versioning_behavior=VersioningBehavior.PINNED,
                ),
            )

        async def promote(build):
            async with asyncio.timeout(20):
                while True:
                    try:
                        await client.workflow_service.set_worker_deployment_current_version(
                            SetWorkerDeploymentCurrentVersionRequest(
                                namespace=client.namespace, deployment_name=name,
                                build_id=build, identity="synthetic-handoff-test",
                            )
                        )
                        return
                    except Exception:
                        await asyncio.sleep(0.1)

        handle = None
        try:
            async with asyncio.timeout(60), make_worker(first), make_worker(second):
                await promote("v1")
                handle = await client.start_workflow(
                    WatchStormWorkflow.run,
                    WatchStormInput("synthetic-user", "synthetic-cell", 0, 0,
                        handoff=WatchStormHandoff(state=WatchStormState(
                            last_frame_ts="synthetic-before", last_max_dbz=30,
                            last_change_kind="intensifying", last_notified_at=123,
                            poll_count=4000, push_count=7,
                        ))),
                    id=name, task_queue=name,
                    execution_timeout=timedelta(minutes=2),
                )
                await first.comparing.wait()
                await promote("v2")
                await handle.signal(WatchStormWorkflow.alert_match, "synthetic-alert-a")
                await handle.signal(WatchStormWorkflow.alert_match, "synthetic-alert-b")
                # Matching routing propagates asynchronously; drive tasks until the server notifies V1.
                async with asyncio.timeout(20):
                    while True:
                        await handle.signal(WatchStormWorkflow.wake_up)
                        history = await handle.fetch_history()
                        if any(e.workflow_task_started_event_attributes.target_worker_deployment_version_changed for e in history.events):
                            break
                        await asyncio.sleep(0.1)
                first.release_compare.set()
                await second.pushing.wait()
                latest = client.get_workflow_handle(name)
                await latest.signal(WatchStormWorkflow.alert_match, "synthetic-alert-c")
                second.release_push.set()
                while len(second.alerts) < 3:
                    await asyncio.sleep(0.02)
                await latest.signal(WatchStormWorkflow.unpin)
                result = await handle.result()
                self.assertEqual(first.alerts, [])
                self.assertEqual(second.alerts, ["synthetic-alert-a", "synthetic-alert-b", "synthetic-alert-c"])
                self.assertEqual(result.last_frame_ts, "synthetic-frame")
                self.assertEqual(result.last_max_dbz, 42)
                self.assertGreaterEqual(result.poll_count, 4001)
                self.assertEqual(result.push_count, 10)
                self.assertTrue(second.compares)
                self.assertEqual(second.compares[0].prev_timestamp, "synthetic-frame")
                self.assertEqual(second.compares[0].prev_max_dbz, 42)

                history = await client.get_workflow_handle(name, run_id=handle.first_execution_run_id).fetch_history()
                continued = history.events[-1].workflow_execution_continued_as_new_event_attributes
                handoff_input, = await DataConverter.default.decode(continued.input.payloads, [WatchStormInput])
                self.assertEqual(handoff_input.handoff.pending_alert_ids, ["synthetic-alert-a", "synthetic-alert-b"])
                self.assertEqual(handoff_input.handoff.state.last_notified_at, 123)
                self.assertEqual(handoff_input.handoff.state.last_change_kind, "intensifying")
                self.assertEqual(handoff_input.handoff.state.push_count, 7)
        finally:
            first.release_compare.set()
            first.release_push.set()
            second.release_push.set()
            if handle:
                try:
                    await client.get_workflow_handle(name).terminate("synthetic test cleanup")
                except Exception:
                    pass
            if env:
                await env.shutdown()
