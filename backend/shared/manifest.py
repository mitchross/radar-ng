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

MANIFEST_SCHEMA_VERSION = 2
TILE_URL_TEMPLATE = "/tiles/{layer}/{palette}/{path}/{z}/{x}/{y}.png"
TILE_URL_TEMPLATE_LEGACY = "/tiles/{layer}/{timestamp}/{z}/{x}/{y}.png"
STATE_DIR = os.environ.get("STATE_DIR", "/data/state")


def empty_manifest() -> dict[str, Any]:
    return {
        "schema_version": MANIFEST_SCHEMA_VERSION,
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
    body.setdefault("schema_version", 1)
    for layer in body["layers"].values():
        if not isinstance(layer, dict):
            continue
        timestamps = [str(ts) for ts in layer.get("timestamps", [])]
        if not isinstance(layer.get("frames"), list):
            layer["frames"] = [
                {
                    "timestamp": ts,
                    "path": ts,
                    "palettes": list(layer.get("palettes", ["classic"])),
                }
                for ts in timestamps
            ]
    return body


def _normalize_frame(
    timestamp: str,
    *,
    palettes: list[str] | set[str] | tuple[str, ...] | None,
    frame: dict[str, Any] | None,
) -> dict[str, Any]:
    normalized = dict(frame or {})
    normalized["timestamp"] = timestamp
    normalized["path"] = str(normalized.get("path") or timestamp)
    normalized["palettes"] = sorted(
        {str(p) for p in (palettes or normalized.get("palettes") or ["classic"])}
    )
    return normalized


def _sync_layer_indexes(layer: dict[str, Any]) -> None:
    frames_by_timestamp = {
        str(frame["timestamp"]): frame
        for frame in layer.get("frames", [])
        if isinstance(frame, dict) and frame.get("timestamp")
    }
    frames = [frames_by_timestamp[key] for key in sorted(frames_by_timestamp)]
    layer["frames"] = frames
    layer["timestamps"] = [str(frame["timestamp"]) for frame in frames]
    if frames:
        layer["latest"] = frames[-1]["timestamp"]
        palette_sets = [set(frame.get("palettes", [])) for frame in frames]
        common = set.intersection(*palette_sets) if palette_sets else set()
        layer["palettes"] = sorted(common)
    else:
        layer.pop("latest", None)
        layer["palettes"] = []


def update_manifest_file(
    layer_name: str,
    timestamp: str,
    *,
    palettes: list[str] | set[str] | tuple[str, ...] | None = None,
    action: str = "add",
    state_dir: str | Path | None = None,
    frame: dict[str, Any] | None = None,
    layer_metadata: dict[str, Any] | None = None,
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
            layer = layers.setdefault(layer_name, {"timestamps": [], "frames": [], "palettes": []})
            existing = [
                candidate for candidate in layer.get("frames", [])
                if isinstance(candidate, dict) and candidate.get("timestamp") != timestamp
            ]
            existing.append(_normalize_frame(timestamp, palettes=palettes, frame=frame))
            layer["frames"] = existing
            if layer_metadata:
                layer.update(layer_metadata)
            _sync_layer_indexes(layer)
        elif action == "remove":
            layer = layers.get(layer_name)
            if layer:
                layer["frames"] = [
                    candidate for candidate in layer.get("frames", [])
                    if isinstance(candidate, dict) and candidate.get("timestamp") != timestamp
                ]
                _sync_layer_indexes(layer)
                if not layer["frames"]:
                    layers.pop(layer_name, None)
        else:
            raise ValueError(f"unsupported manifest action: {action}")

        manifest["schema_version"] = MANIFEST_SCHEMA_VERSION
        manifest["updated_at"] = datetime.now(timezone.utc).isoformat()
        _atomic_write_json(path, manifest)
        fcntl.flock(lock.fileno(), fcntl.LOCK_UN)
        return manifest


def replace_layer_manifest(
    layer_name: str,
    timestamps: list[str],
    *,
    palettes: list[str] | set[str] | tuple[str, ...] | None = None,
    state_dir: str | Path | None = None,
    frames: list[dict[str, Any]] | None = None,
    layer_metadata: dict[str, Any] | None = None,
) -> dict[str, Any]:
    """Atomically swap a layer's ENTIRE timestamp list.

    Built for forecast-style layers (nowcast) where every run supersedes the
    previous one: per-frame `action="add"` calls would accumulate predictions
    from every past anchor run, so the app's future window ends up mixing
    fresh and stale vintages. One replace per run keeps only the latest
    model's frames and makes them visible all-at-once, after tiles exist.
    """
    import fcntl

    path = manifest_path(state_dir)
    path.parent.mkdir(parents=True, exist_ok=True)
    lock_path = path.with_suffix(".lock")

    with lock_path.open("w") as lock:
        fcntl.flock(lock.fileno(), fcntl.LOCK_EX)
        manifest = read_manifest_file(path.parent)
        layers = manifest.setdefault("layers", {})

        if timestamps:
            layer = layers.setdefault(
                layer_name, {"timestamps": [], "frames": [], "palettes": []}
            )
            supplied = {
                str(frame["timestamp"]): frame
                for frame in (frames or [])
                if isinstance(frame, dict) and frame.get("timestamp")
            }
            layer["frames"] = [
                _normalize_frame(ts, palettes=palettes, frame=supplied.get(ts))
                for ts in sorted(set(timestamps))
            ]
            if layer_metadata:
                layer.update(layer_metadata)
            _sync_layer_indexes(layer)
        else:
            layers.pop(layer_name, None)

        manifest["schema_version"] = MANIFEST_SCHEMA_VERSION
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
