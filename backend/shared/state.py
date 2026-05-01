"""Disk-backed state so ingestors survive restarts without re-processing work."""

from __future__ import annotations

import json
import os
import tempfile
from pathlib import Path
from typing import Iterable


class ProcessedSet:
    """Bounded on-disk set of ids that have already been processed."""

    def __init__(self, path: Path | str, max_entries: int = 5000) -> None:
        self.path = Path(path)
        self.max_entries = max_entries
        self._items: list[str] = []
        self._set: set[str] = set()
        self._load()

    def _load(self) -> None:
        if not self.path.exists():
            return
        try:
            data = json.loads(self.path.read_text())
            items = data.get("items", []) if isinstance(data, dict) else list(data)
            self._items = [str(x) for x in items]
            self._set = set(self._items)
        except (json.JSONDecodeError, OSError):
            self._items = []
            self._set = set()

    def __contains__(self, key: object) -> bool:
        return key in self._set

    def add(self, key: str) -> None:
        if key in self._set:
            return
        self._items.append(key)
        self._set.add(key)
        if len(self._items) > self.max_entries:
            dropped = self._items[: -self.max_entries]
            self._items = self._items[-self.max_entries:]
            for d in dropped:
                self._set.discard(d)
        self._flush()

    def update(self, keys: Iterable[str]) -> None:
        dirty = False
        for k in keys:
            if k in self._set:
                continue
            self._items.append(k)
            self._set.add(k)
            dirty = True
        if dirty:
            if len(self._items) > self.max_entries:
                dropped = self._items[: -self.max_entries]
                self._items = self._items[-self.max_entries:]
                for d in dropped:
                    self._set.discard(d)
            self._flush()

    def _flush(self) -> None:
        self.path.parent.mkdir(parents=True, exist_ok=True)
        # Atomic write: tempfile in same dir, then os.replace
        fd, tmp_path = tempfile.mkstemp(dir=self.path.parent, prefix=".state-", suffix=".json.tmp")
        try:
            with os.fdopen(fd, "w") as f:
                json.dump({"items": self._items}, f)
            os.replace(tmp_path, self.path)
        except Exception:
            try:
                os.unlink(tmp_path)
            except OSError:
                pass
            raise
