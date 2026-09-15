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
        for role in ("mrms", "nowcast", "hrrr", "aux", "alerts"):
            required = ("GRID_DIR", "STATE_DIR") if role == "alerts" else ("TILE_DIR", "GRID_DIR", "STATE_DIR")
            for variable in required:
                with self.subTest(role=role, variable=variable), tempfile.TemporaryDirectory() as root:
                    paths = {key: str(Path(root) / key) for key in ("TILE_DIR", "GRID_DIR", "STATE_DIR")}
                    for key, path in paths.items():
                        if key != variable:
                            Path(path).mkdir()
                    with patch.dict(os.environ, {**paths, "WORKER_ROLE": role}):
                        with self.assertRaises(FileNotFoundError):
                            check_release()
                    self.assertFalse(Path(paths[variable]).exists())

    def test_alerts_accepts_read_only_grids_without_tiles(self):
        with tempfile.TemporaryDirectory() as root:
            paths = {key: str(Path(root) / key) for key in ("TILE_DIR", "GRID_DIR", "STATE_DIR")}
            grid, state = Path(paths["GRID_DIR"]), Path(paths["STATE_DIR"])
            grid.mkdir()
            state.mkdir()
            fixture = grid / "existing-grid"
            fixture.write_bytes(b"preserve grid")
            grid.chmod(0o555)
            try:
                with patch.dict(os.environ, {**paths, "WORKER_ROLE": "alerts"}):
                    self.assertEqual(check_release(), {"ok": True, "role": "alerts"})
                self.assertFalse(Path(paths["TILE_DIR"]).exists())
                self.assertEqual(list(grid.iterdir()), [fixture])
                self.assertEqual(fixture.read_bytes(), b"preserve grid")
                self.assertEqual(list(state.iterdir()), [])
            finally:
                grid.chmod(0o755)

    @unittest.skipIf(os.geteuid() == 0, "Run as a non-root worker to enforce permissions")
    def test_alerts_requires_readable_grids_and_writable_state(self):
        for variable, mode in (("GRID_DIR", 0o000), ("GRID_DIR", 0o444), ("STATE_DIR", 0o555)):
            with self.subTest(variable=variable, mode=mode), tempfile.TemporaryDirectory() as root:
                paths = {key: str(Path(root) / key) for key in ("TILE_DIR", "GRID_DIR", "STATE_DIR")}
                for key in ("GRID_DIR", "STATE_DIR"):
                    Path(paths[key]).mkdir()
                restricted = Path(paths[variable])
                restricted.chmod(mode)
                try:
                    with patch.dict(os.environ, {**paths, "WORKER_ROLE": "alerts"}):
                        with self.assertRaises(PermissionError):
                            check_release()
                finally:
                    restricted.chmod(0o755)

    def test_missing_configured_palette_fails(self):
        with patch.dict(os.environ, {"PALETTES": "nonexistent", "WORKER_ROLE": "alerts"}):
            with self.assertRaisesRegex(ValueError, "palette"):
                check_release()
