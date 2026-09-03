"""Offline determinism gate: replay every retained synthetic history with the current code.

Fixtures live at ``replay_fixtures/<WorkflowType>/<version>/<scenario>.json[.gz]``.
Version directories are immutable; the gate replays all of them, so a workflow
change must stay compatible with every retained release, not just the newest.
"""

from __future__ import annotations

import asyncio
import base64
import gzip
import json
import re
import unittest
from collections.abc import Iterator
from dataclasses import dataclass
from pathlib import Path
from typing import Any

from temporalio.api.enums.v1 import EventType
from temporalio.client import WorkflowHistory
from temporalio.worker import Replayer

from temporal.workflows import ALL_WORKFLOWS, WORKFLOW_REGISTRY

FIXTURE_DIR = Path(__file__).with_name("replay_fixtures")
VERSION_RE = re.compile(r"^(v\d+\.\d+\.\d+|src-[0-9a-f]{7,40})$")

_COMPLETED = EventType.EVENT_TYPE_WORKFLOW_EXECUTION_COMPLETED
_FAILED = EventType.EVENT_TYPE_WORKFLOW_EXECUTION_FAILED
_ACTIVITY_FAILED = EventType.EVENT_TYPE_ACTIVITY_TASK_FAILED
_SIGNALED = EventType.EVENT_TYPE_WORKFLOW_EXECUTION_SIGNALED
_TIMER_FIRED = EventType.EVENT_TYPE_TIMER_FIRED
_CONTINUED = EventType.EVENT_TYPE_WORKFLOW_EXECUTION_CONTINUED_AS_NEW

# An EventType value, or the name of an activity type that must have been scheduled.
Marker = int | str
_SINGLE_ACTIVITY: dict[str, set[Marker]] = {
    "success": {_COMPLETED},
    "activity-error": {_ACTIVITY_FAILED, _FAILED},
}
# Scenarios every workflow must retain in at least one version, with proof they took the
# path they are named for. Dropping a scenario here weakens the gate; add, do not remove.
REQUIRED_SCENARIOS: dict[str, dict[str, set[Marker]]] = {
    "IngestMrmsWorkflow": {
        "success": {_COMPLETED, "mrms_mark_processed"},
        "partial": {_COMPLETED},
        "frame-activity-error": {_ACTIVITY_FAILED, _COMPLETED},
        "list-activity-error": {_ACTIVITY_FAILED, _FAILED},
    },
    "IngestHrrrWorkflow": {
        "success": {_COMPLETED, "hrrr_mark_processed"},
        "partial": {_COMPLETED, "hrrr_publish_run"},
        "hour-activity-error": {_ACTIVITY_FAILED, _COMPLETED},
        "find-activity-error": {_ACTIVITY_FAILED, _FAILED},
    },
    "IngestAirQualityWorkflow": {
        "success": {_COMPLETED, "aqm_mark_processed"},
        "partial": {_COMPLETED, "aqm_publish_run"},
        "chunk-activity-error": {_ACTIVITY_FAILED, _COMPLETED},
        "find-activity-error": {_ACTIVITY_FAILED, _FAILED},
    },
    "PollAlertsWorkflow": {
        "geometry-fanout": {_COMPLETED, "signal_matching_storm_watches"},
        "signal-activity-error": {_ACTIVITY_FAILED, _COMPLETED},
        "fetch-activity-error": {_ACTIVITY_FAILED, _FAILED},
    },
    "WatchStormWorkflow": {
        "signal-unpin": {_SIGNALED, _COMPLETED},
        "timer-poll": {_TIMER_FIRED, _COMPLETED},
        "push-change": {"detect_storm_change", "fan_out_push_to_user", _COMPLETED},
        "alert-signal-push": {_SIGNALED, "fan_out_push_to_user", _COMPLETED},
        "push-activity-error": {_ACTIVITY_FAILED, _COMPLETED},
        "compare-activity-error": {_ACTIVITY_FAILED, _FAILED},
        "continue-as-new": {_CONTINUED},
    },
    "IngestLightningWorkflow": _SINGLE_ACTIVITY,
    "IngestTropicalWorkflow": _SINGLE_ACTIVITY,
    "NowcastWorkflow": _SINGLE_ACTIVITY,
    "TileCleanupWorkflow": _SINGLE_ACTIVITY,
    "RegisterPushTokenWorkflow": _SINGLE_ACTIVITY,
    "DeletePushTokenWorkflow": _SINGLE_ACTIVITY,
    "OpenMeteoSyncWorkflow": _SINGLE_ACTIVITY,
}


@dataclass(frozen=True)
class Fixture:
    workflow: str
    version: str
    scenario: str
    path: Path
    document: dict[str, Any]
    history: WorkflowHistory

    @property
    def label(self) -> str:
        return f"{self.workflow}/{self.version}/{self.scenario}"

    def markers(self) -> set[Marker]:
        found: set[Marker] = set()
        for event in self.history.events:
            found.add(event.event_type)
            if event.HasField("activity_task_scheduled_event_attributes"):
                found.add(
                    event.activity_task_scheduled_event_attributes.activity_type.name
                )
        return found


def _load_fixtures() -> list[Fixture]:
    fixtures: list[Fixture] = []
    for path in sorted(
        p for p in FIXTURE_DIR.rglob("*") if p.name.endswith((".json", ".json.gz"))
    ):
        raw = (
            gzip.decompress(path.read_bytes())
            if path.suffix == ".gz"
            else path.read_bytes()
        )
        document = json.loads(raw)
        workflow, version, filename = path.relative_to(FIXTURE_DIR).parts
        scenario = filename.removesuffix(".gz").removesuffix(".json")
        history = WorkflowHistory.from_json(
            f"{workflow}/{version}/{scenario}", document
        )
        fixtures.append(Fixture(workflow, version, scenario, path, document, history))
    return fixtures


def _field_values(value: Any, field_name: str) -> Iterator[Any]:
    if isinstance(value, dict):
        for key, child in value.items():
            if key == field_name:
                yield child
            yield from _field_values(child, field_name)
    elif isinstance(value, list):
        for child in value:
            yield from _field_values(child, field_name)


def _decoded_json_payloads(value: Any) -> Iterator[Any]:
    if isinstance(value, dict):
        metadata = value.get("metadata")
        data = value.get("data")
        if isinstance(metadata, dict) and isinstance(data, str):
            if base64.b64decode(metadata.get("encoding", "")) == b"json/plain":
                yield json.loads(base64.b64decode(data))
        for child in value.values():
            yield from _decoded_json_payloads(child)
    elif isinstance(value, list):
        for child in value:
            yield from _decoded_json_payloads(child)


def _coordinates(geometry: Any) -> Iterator[Any]:
    if isinstance(geometry, list):
        for child in geometry:
            yield from _coordinates(child)
    else:
        yield geometry


class WorkflowReplayTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls) -> None:
        cls.fixtures = _load_fixtures()

    def test_fixture_tree_matches_canonical_registry(self) -> None:
        by_workflow: dict[str, list[Fixture]] = {}
        for fixture in self.fixtures:
            by_workflow.setdefault(fixture.workflow, []).append(fixture)
        self.assertEqual(set(by_workflow), set(WORKFLOW_REGISTRY))
        self.assertEqual(set(REQUIRED_SCENARIOS), set(WORKFLOW_REGISTRY))
        labels = [fixture.label for fixture in self.fixtures]
        self.assertEqual(len(labels), len(set(labels)), "duplicate fixture scenario")

        for fixture in self.fixtures:
            with self.subTest(fixture=fixture.label):
                self.assertRegex(fixture.version, VERSION_RE)
                self.assertTrue(fixture.history.events)
                started = fixture.history.events[
                    0
                ].workflow_execution_started_event_attributes
                self.assertEqual(started.workflow_type.name, fixture.workflow)

    def test_required_scenarios_are_retained_and_prove_their_path(self) -> None:
        by_release: dict[tuple[str, str], list[Fixture]] = {}
        for fixture in self.fixtures:
            by_release.setdefault((fixture.workflow, fixture.version), []).append(
                fixture
            )

        for (workflow, version), fixtures in by_release.items():
            required = REQUIRED_SCENARIOS[workflow]
            present = {fixture.scenario for fixture in fixtures}
            with self.subTest(workflow=workflow, version=version):
                self.assertLessEqual(
                    set(required), present, "release is missing required scenarios"
                )
            for fixture in fixtures:
                if markers := required.get(fixture.scenario):
                    with self.subTest(fixture=fixture.label):
                        self.assertLessEqual(markers, fixture.markers(), fixture.label)

    def test_fixtures_are_synthetic(self) -> None:
        for fixture in self.fixtures:
            document = fixture.document
            with self.subTest(fixture=fixture.label):
                identities = set(_field_values(document, "identity"))
                self.assertTrue(identities)
                self.assertTrue(
                    all(i.startswith("synthetic-") for i in identities), identities
                )
                self.assertEqual(list(_field_values(document, "stackTrace")), [])
                for message in _field_values(document, "message"):
                    self.assertTrue(
                        message.startswith("synthetic")
                        or message == "Activity task failed",
                        message,
                    )

                payloads = list(_decoded_json_payloads(document))
                for field_name in ("user_id", "storm_cell_id", "alert_id", "run_id"):
                    for value in _field_values(payloads, field_name):
                        if value is not None:
                            self.assertTrue(
                                value.startswith("synthetic-"), (field_name, value)
                            )
                for value in _field_values(payloads, "token"):
                    self.assertEqual(value, "synthetic-placeholder")
                for field_name in ("lat", "lng"):
                    self.assertTrue(
                        all(v == 0.0 for v in _field_values(payloads, field_name))
                    )
                for geometry in _field_values(payloads, "geometry"):
                    coordinates = (
                        list(_coordinates(geometry.get("coordinates", [])))
                        if geometry
                        else []
                    )
                    self.assertTrue(all(c in (0, 1) for c in coordinates), geometry)

    def test_every_fixture_replays(self) -> None:
        replayer = Replayer(workflows=ALL_WORKFLOWS)
        for fixture in self.fixtures:
            with self.subTest(fixture=fixture.label):
                asyncio.run(replayer.replay_workflow(fixture.history))


if __name__ == "__main__":
    unittest.main()
