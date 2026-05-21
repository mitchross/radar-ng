"""Atomic tile manifest state.

The API serves this file directly so request paths never need to crawl the
tile PVC. Ingest and cleanup activities update it as frames are added/removed.
"""

from __future__ import annotations

import json
import os
import tempfile
from datetime import datetime, timezone
from pathlib import Path
from typing import Any

TILE_URL_TEMPLATE = "/tiles/{layer}/{palette}/{timestamp}/{z}/{x}/{y}.png"
TILE_URL_TEMPLATE_LEGACY = "/tiles/{layer}/{timestamp}/{z}/{x}/{y}.png"
STATE_DIR = os.environ.get("STATE_DIR", "/data/state")


def empty_manifest() -> dict[str, Any]:
    return {
        "layers": {},
        "tile_url_template": TILE_URL_TEMPLATE,
        "tile_url_template_legacy": TILE_URL_TEMPLATE_LEGACY,
        "updated_at": datetime.now(timezone.utc).isoformat(),
    }


def manifest_path(state_dir: str | Path | None = None) -> Path:
    return Path(state_dir or STATE_DIR) / "manifest.json"


def read_manifest_file(state_dir: str | Path | None = None) -> dict[str, Any]:
    path = manifest_path(state_dir)
    try:
        body = json.loads(path.read_text())
    except (OSError, json.JSONDecodeError):
        return empty_manifest()
    if not isinstance(body, dict) or not isinstance(body.get("layers"), dict):
        return empty_manifest()
    body.setdefault("tile_url_template", TILE_URL_TEMPLATE)
    body.setdefault("tile_url_template_legacy", TILE_URL_TEMPLATE_LEGACY)
    body.setdefault("updated_at", datetime.now(timezone.utc).isoformat())
    return body


def update_manifest_file(
    layer_name: str,
    timestamp: str,
    *,
    palettes: list[str] | set[str] | tuple[str, ...] | None = None,
    action: str = "add",
    state_dir: str | Path | None = None,
) -> dict[str, Any]:
    """Atomically add or remove one layer timestamp under a cross-process lock."""
    import fcntl

    path = manifest_path(state_dir)
    path.parent.mkdir(parents=True, exist_ok=True)
    lock_path = path.with_suffix(".lock")

    with lock_path.open("w") as lock:
        fcntl.flock(lock.fileno(), fcntl.LOCK_EX)
        manifest = read_manifest_file(path.parent)
        layers = manifest.setdefault("layers", {})

        if action == "add":
            layer = layers.setdefault(layer_name, {"timestamps": [], "palettes": []})
            timestamps = set(layer.get("timestamps", []))
            timestamps.add(timestamp)
            layer["timestamps"] = sorted(timestamps)
            layer["latest"] = layer["timestamps"][-1]

            if palettes:
                palette_set = set(layer.get("palettes", []))
                palette_set.update(str(p) for p in palettes)
                layer["palettes"] = sorted(palette_set)
            elif not layer.get("palettes"):
                layer["palettes"] = ["classic"]
        elif action == "remove":
            layer = layers.get(layer_name)
            if layer:
                layer["timestamps"] = [ts for ts in layer.get("timestamps", []) if ts != timestamp]
                if layer["timestamps"]:
                    layer["latest"] = layer["timestamps"][-1]
                else:
                    layers.pop(layer_name, None)
        else:
            raise ValueError(f"unsupported manifest action: {action}")

        manifest["updated_at"] = datetime.now(timezone.utc).isoformat()
        _atomic_write_json(path, manifest)
        fcntl.flock(lock.fileno(), fcntl.LOCK_UN)
        return manifest


def _atomic_write_json(path: Path, body: dict[str, Any]) -> None:
    fd, tmp_name = tempfile.mkstemp(prefix=f".{path.name}.", suffix=".tmp", dir=str(path.parent))
    try:
        with os.fdopen(fd, "w") as fh:
            json.dump(body, fh, separators=(",", ":"), sort_keys=True)
            fh.write("\n")
        os.replace(tmp_name, path)
    finally:
        try:
            os.unlink(tmp_name)
        except FileNotFoundError:
            pass
