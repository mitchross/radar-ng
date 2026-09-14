import os
import tempfile
import unittest
from pathlib import Path
from unittest.mock import patch

from temporal.shared.deployment_smoke import check_release


class DeploymentSmokeTests(unittest.TestCase):
    def test_all_production_roles_exercise_fixture_and_leave_volumes_unchanged(self):
        with tempfile.TemporaryDirectory() as root:
            paths = {key: str(Path(root) / key) for key in ("TILE_DIR", "GRID_DIR", "STATE_DIR")}
            for path in paths.values():
                Path(path).mkdir()
                (Path(path) / "existing-data").write_text("preserve me")
            for role in ("mrms", "nowcast", "hrrr", "aux", "alerts"):
                with self.subTest(role=role), patch.dict(os.environ, {**paths, "WORKER_ROLE": role}):
                    self.assertEqual(check_release(), {"ok": True, "role": role})
                for path in paths.values():
                    self.assertEqual([p.name for p in Path(path).iterdir()], ["existing-data"])
                    self.assertEqual((Path(path) / "existing-data").read_text(), "preserve me")

    def test_missing_mount_fails_without_creating_the_root(self):
        with tempfile.TemporaryDirectory() as root:
            missing = Path(root) / "missing"
            with patch.dict(os.environ, {"TILE_DIR": str(missing), "WORKER_ROLE": "alerts"}):
                with self.assertRaises(FileNotFoundError):
                    check_release()
            self.assertFalse(missing.exists())

    def test_missing_configured_palette_fails(self):
        with patch.dict(os.environ, {"PALETTES": "nonexistent", "WORKER_ROLE": "alerts"}):
            with self.assertRaisesRegex(ValueError, "palette"):
                check_release()
