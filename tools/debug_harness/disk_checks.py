"""On-disk checks — run these INSIDE a pod/container that mounts the PVCs
(tile-server or worker), or on the compose host against the named volumes.

These see what the HTTP surface can't: orphaned render staging dirs, manifest
entries whose tiles never landed (or vice versa), PVC headroom, and state-file
freshness straight from mtimes.
"""

from __future__ import annotations

import json
import os
import shutil
import time
from pathlib import Path

from .core import FAIL, OK, WARN, Check, age_str

TILE_DIR = os.environ.get("TILE_DIR", "/data/tiles")
GRID_DIR = os.environ.get("GRID_DIR", "/data/grids")
STATE_DIR = os.environ.get("STATE_DIR", "/data/state")
TMP_ROOTS = (
    os.environ.get("MRMS_TMP_ROOT", "/tmp/mrms_work"),
    os.environ.get("HRRR_TMP_ROOT", "/tmp/hrrr_work"),
)

# A .tmp staging dir older than this is an orphan from a crashed/cancelled
# render (render_tiles_atomic never rename()d it into place).
ORPHAN_TMP_AGE_S = 30 * 60


def check_disk() -> list[Check]:
    checks: list[Check] = []
    roots = {"tiles": Path(TILE_DIR), "grids": Path(GRID_DIR), "state": Path(STATE_DIR)}

    present = {name: p for name, p in roots.items() if p.is_dir()}
    if not present:
        return [Check(
            "disk.mounts", FAIL,
            f"none of {[str(p) for p in roots.values()]} exist — run this inside "
            "a container that mounts the data volumes (or set TILE_DIR/GRID_DIR/STATE_DIR)",
        )]

    # -- filesystem headroom --------------------------------------------------
    for name, p in present.items():
        try:
            usage = shutil.disk_usage(p)
        except OSError as exc:
            checks.append(Check(f"disk.{name}.usage", FAIL, f"disk_usage failed: {exc}"))
            continue
        pct = usage.used / usage.total * 100
        status = OK if pct < 85 else WARN if pct < 95 else FAIL
        checks.append(Check(
            f"disk.{name}.usage", status,
            f"{p}: {pct:.0f}% used ({usage.free / 1e9:.1f} GB free of {usage.total / 1e9:.1f} GB)",
            {"pct": round(pct, 1), "free_gb": round(usage.free / 1e9, 1)},
        ))

    # -- tile tree: per-layer frame counts + orphaned .tmp staging dirs -------
    now = time.time()
    tile_root = roots["tiles"]
    disk_layers: dict[str, dict[str, set[str]]] = {}
    orphans: list[str] = []
    if tile_root.is_dir():
        for layer_dir in sorted(tile_root.iterdir()):
            if not layer_dir.is_dir() or not layer_dir.name[0:1].isalpha():
                continue
            palettes: dict[str, set[str]] = {}
            for palette_dir in layer_dir.iterdir():
                if not palette_dir.is_dir():
                    continue
                stamps = set()
                for ts_dir in palette_dir.iterdir():
                    if not ts_dir.is_dir():
                        continue
                    if ts_dir.name.endswith(".tmp"):
                        try:
                            age = now - ts_dir.stat().st_mtime
                        except OSError:
                            continue
                        if age > ORPHAN_TMP_AGE_S:
                            orphans.append(f"{ts_dir} ({age_str(age)} old)")
                        continue
                    stamps.add(ts_dir.name)
                palettes[palette_dir.name] = stamps
            disk_layers[layer_dir.name] = palettes

        for layer, palettes in disk_layers.items():
            counts = {p: len(s) for p, s in palettes.items()}
            # Palettes of one layer are rendered from the same grid in the same
            # activity — a lopsided count means partial palette failures.
            lopsided = len(set(counts.values())) > 1
            checks.append(Check(
                f"disk.tiles.{layer}",
                WARN if lopsided else OK,
                f"frames per palette: {counts}"
                + (" — uneven counts mean some palette renders failed" if lopsided else ""),
                {"counts": counts},
            ))

    if orphans:
        checks.append(Check(
            "disk.tiles.orphans", WARN,
            f"{len(orphans)} stale .tmp staging dir(s) from crashed/cancelled renders: "
            + "; ".join(orphans[:5]),
        ))
    elif tile_root.is_dir():
        checks.append(Check("disk.tiles.orphans", OK, "no stale .tmp staging dirs"))

    # -- manifest vs disk drift ------------------------------------------------
    manifest_path = roots["state"] / "manifest.json"
    if manifest_path.is_file() and disk_layers:
        try:
            manifest = json.loads(manifest_path.read_text())
            layers = manifest.get("layers", {})
        except (OSError, json.JSONDecodeError) as exc:
            layers = {}
            checks.append(Check("disk.manifest", FAIL, f"manifest unreadable: {exc}"))
        for layer_name, layer in sorted(layers.items()):
            advertised = set(layer.get("timestamps", []))
            palettes = layer.get("palettes", ["classic"])
            on_disk = set()
            for pal in palettes:
                on_disk |= disk_layers.get(layer_name, {}).get(pal, set())
            ghost = advertised - on_disk  # advertised but no tiles → client 404s
            hidden = on_disk - advertised  # rendered but invisible to clients
            if ghost:
                checks.append(Check(
                    f"disk.manifest.{layer_name}", FAIL,
                    f"{len(ghost)} manifest timestamp(s) have NO tiles on disk "
                    f"(clients 404 these frames): {sorted(ghost)[:3]}...",
                    {"ghost": sorted(ghost)},
                ))
            elif hidden and len(hidden) > 2:  # 1-2 is normal mid-publish skew
                checks.append(Check(
                    f"disk.manifest.{layer_name}", WARN,
                    f"{len(hidden)} rendered timestamp(s) missing from manifest (invisible to clients)",
                    {"hidden": sorted(hidden)},
                ))
            else:
                checks.append(Check(f"disk.manifest.{layer_name}", OK,
                                    f"{len(advertised)} advertised frames all present on disk"))

    # -- state files: freshness + size ----------------------------------------
    state_root = roots["state"]
    if state_root.is_dir():
        expectations = {  # filename → max healthy age seconds (None = informational)
            "manifest.json": 600,
            "nowcast-status.json": 600,
            "lightning.json": 300,
            "storms.json": 600,
            "tropical.json": None,
        }
        for fname, max_age in expectations.items():
            f = state_root / fname
            if not f.is_file():
                checks.append(Check(f"disk.state.{fname}", WARN, "missing"))
                continue
            try:
                st = f.stat()
            except OSError as exc:
                checks.append(Check(f"disk.state.{fname}", FAIL, str(exc)))
                continue
            age = now - st.st_mtime
            status = OK if max_age is None or age <= max_age else WARN
            checks.append(Check(
                f"disk.state.{fname}", status,
                f"written {age_str(age)} ago · {st.st_size // 1024} KB",
                {"age_s": int(age), "bytes": st.st_size},
            ))

    # -- ingest scratch dirs: leaked attempt dirs ------------------------------
    for root in TMP_ROOTS:
        rp = Path(root)
        if not rp.is_dir():
            continue
        stale = []
        total_bytes = 0
        for child in rp.iterdir():
            try:
                age = now - child.stat().st_mtime
            except OSError:
                continue
            if age > ORPHAN_TMP_AGE_S:
                stale.append(child.name)
                total_bytes += sum(
                    f.stat().st_size for f in child.rglob("*") if f.is_file()
                )
        if stale:
            checks.append(Check(
                f"disk.tmp.{rp.name}", WARN,
                f"{len(stale)} leaked work dir(s), {total_bytes / 1e6:.0f} MB "
                "(cancelled activities skip their cleanup handler)",
            ))
        else:
            checks.append(Check(f"disk.tmp.{rp.name}", OK, "no leaked work dirs"))

    return checks
