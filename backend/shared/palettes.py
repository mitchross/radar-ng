"""Palette loader — returns the color-table dict for a named palette.

Palettes live as JSON files in backend/shared/palettes/. Each file has the
same shape as the legacy backend/shared/color_tables.json. This helper also
supports the env var PALETTES (comma-separated list) which ingestors iterate
over to render the same frame into multiple palette subtrees:

    /data/tiles/{layer}/{palette}/{timestamp}/{z}/{x}/{y}.png

Default is "classic" — which falls back to color_tables.json if the dedicated
classic.json is missing, preserving the pre-palette install path.
"""

from __future__ import annotations

import json
import os
from pathlib import Path

_SHARED_DIR = Path(__file__).resolve().parent
_PALETTES_DIR = _SHARED_DIR / "palettes"
_LEGACY_COLOR_TABLES = _SHARED_DIR / "color_tables.json"


def get_palette_names() -> list[str]:
    """Return palettes to render for, from PALETTES env var. Defaults to ["classic"]."""
    raw = os.environ.get("PALETTES", "classic")
    names = [p.strip() for p in raw.split(",") if p.strip()]
    return names or ["classic"]


def load_palette(name: str) -> dict:
    """Load a palette by name. Falls back to legacy color_tables.json for 'classic'."""
    path = _PALETTES_DIR / f"{name}.json"
    if path.exists():
        with open(path) as f:
            return json.load(f)
    if name == "classic" and _LEGACY_COLOR_TABLES.exists():
        with open(_LEGACY_COLOR_TABLES) as f:
            return json.load(f)
    raise FileNotFoundError(f"palette not found: {name} (looked in {path})")
