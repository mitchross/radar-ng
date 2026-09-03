"""Independent discovery of every ``@workflow.defn`` class under ``temporal/``.

The registry, the worker roles, the schedule seed, the API routes, and the
replay fixture tree must all agree, so a new workflow type cannot ship without
a replay gate or a worker that polls for it.
"""

from __future__ import annotations

import ast
import importlib
import unittest
from pathlib import Path

from temporalio import workflow

from temporal.schedules.seed import SCHEDULES
from temporal.task_queues import ALERTS_TASK_QUEUE, LEGACY_TASK_QUEUE
from temporal.worker import ROLE_CONFIG
from temporal.workflows import WORKFLOW_REGISTRY

REPO_ROOT = Path(__file__).resolve().parents[2]
TEMPORAL_DIR = REPO_ROOT / "temporal"
API_DIR = REPO_ROOT / "backend" / "api"
FIXTURE_DIR = Path(__file__).with_name("replay_fixtures")
SKIP_PARTS = {".venv", "__pycache__", "node_modules"}
STARTERS = {"start_workflow", "execute_workflow", "ScheduleActionStartWorkflow"}


def _source_files(root: Path) -> list[Path]:
    return sorted(
        p
        for p in root.rglob("*.py")
        if not SKIP_PARTS & set(p.parts) and not p.name.startswith("test_")
    )


def _is_workflow_defn(decorator: ast.expr) -> bool:
    func = decorator.func if isinstance(decorator, ast.Call) else decorator
    return (
        isinstance(func, ast.Attribute)
        and func.attr == "defn"
        and isinstance(func.value, ast.Name)
        and func.value.id == "workflow"
    )


def _qualified(cls: type) -> str:
    return f"{cls.__module__}.{cls.__qualname__}"


def _decorated_workflows() -> dict[str, str]:
    """Map workflow type name -> qualified class name for every @workflow.defn under temporal/."""
    found: dict[str, str] = {}
    for path in _source_files(TEMPORAL_DIR):
        module_name = ".".join(path.relative_to(REPO_ROOT).with_suffix("").parts)
        for node in ast.walk(ast.parse(path.read_text())):
            if not isinstance(node, ast.ClassDef):
                continue
            for decorator in node.decorator_list:
                if not _is_workflow_defn(decorator):
                    continue
                type_name = node.name
                if isinstance(decorator, ast.Call):
                    for keyword in decorator.keywords:
                        if keyword.arg == "name" and isinstance(
                            keyword.value, ast.Constant
                        ):
                            type_name = keyword.value.value
                qualified = _qualified(
                    getattr(importlib.import_module(module_name), node.name)
                )
                if previous := found.get(type_name):
                    raise AssertionError(
                        f"duplicate @workflow.defn type {type_name!r}: "
                        f"{previous} and {qualified}"
                    )
                found[type_name] = qualified
    return found


def _started_workflow_types(root: Path) -> set[str]:
    """Workflow type names passed to start_workflow/execute_workflow/ScheduleActionStartWorkflow."""
    names: set[str] = set()
    for path in _source_files(root):
        for node in ast.walk(ast.parse(path.read_text())):
            if not isinstance(node, ast.Call) or not node.args:
                continue
            func = node.func
            callee = (
                func.attr
                if isinstance(func, ast.Attribute)
                else getattr(func, "id", None)
            )
            if callee not in STARTERS:
                continue
            target = node.args[0]
            if isinstance(target, ast.Constant) and isinstance(target.value, str):
                names.add(target.value)
            elif (
                isinstance(target, ast.Attribute)
                and target.attr == "run"
                and isinstance(target.value, ast.Name)
            ):
                names.add(target.value.id)  # WorkflowClass.run
    return names


class WorkflowDiscoveryTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls) -> None:
        cls.discovered = _decorated_workflows()

    def test_registry_is_exactly_the_decorated_classes(self) -> None:
        registry = {name: _qualified(cls) for name, cls in WORKFLOW_REGISTRY.items()}
        self.assertEqual(self.discovered, registry)
        for type_name, cls in WORKFLOW_REGISTRY.items():
            self.assertEqual(workflow._Definition.must_from_class(cls).name, type_name)

    def test_every_workflow_has_a_replay_fixture_directory(self) -> None:
        fixture_dirs = {
            p.name for p in FIXTURE_DIR.iterdir() if p.is_dir() and p.name[0].isupper()
        }
        self.assertEqual(fixture_dirs, set(self.discovered))

    def test_schedules_and_api_only_start_registered_types(self) -> None:
        scheduled = {s.workflow_name for s in SCHEDULES}
        api_started = _started_workflow_types(API_DIR)
        self.assertTrue(scheduled and api_started)
        self.assertLessEqual(scheduled | api_started, set(self.discovered))
        self.assertLessEqual(
            _started_workflow_types(TEMPORAL_DIR), set(self.discovered)
        )

    def test_roles_cover_every_workflow_and_legacy_registers_all(self) -> None:
        expected = set(WORKFLOW_REGISTRY.values())  # same objects the roles import
        full_roles = {"legacy", "all"}
        for role in full_roles:
            self.assertEqual(set(ROLE_CONFIG[role][1]), expected, role)
        by_role = {
            role: set(cfg[1])
            for role, cfg in ROLE_CONFIG.items()
            if role not in full_roles
        }
        self.assertEqual(set().union(*by_role.values()), expected)
        for role, cfg in ROLE_CONFIG.items():
            self.assertEqual(
                len(cfg[1]), len(set(cfg[1])), f"duplicate workflow in role {role}"
            )

    def test_schedule_and_api_queues_are_served_by_a_role_registering_the_type(
        self,
    ) -> None:
        by_queue: dict[str, set[type]] = {}
        for queue, workflows, _activities in ROLE_CONFIG.values():
            by_queue.setdefault(queue, set()).update(workflows)
        for schedule in SCHEDULES:
            with self.subTest(schedule=schedule.schedule_id):
                cls = WORKFLOW_REGISTRY[schedule.workflow_name]
                self.assertIn(cls, by_queue[schedule.task_queue])
                self.assertIn(cls, by_queue[LEGACY_TASK_QUEUE])
        for type_name in _started_workflow_types(API_DIR):
            with self.subTest(api_workflow=type_name):
                self.assertIn(WORKFLOW_REGISTRY[type_name], by_queue[ALERTS_TASK_QUEUE])


if __name__ == "__main__":
    unittest.main()
