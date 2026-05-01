"""Temporal activities for the MRMS radar ingest pipeline.

These wrap the same logic as ingest.py but expose it as Temporal activities
so it can be orchestrated by IngestMrmsWorkflow.

State-on-disk strategy: the ProcessedSet (state/ingest-mrms.json) is
loaded + saved by individual activities. Schedule overlap policy is SKIP,
so only one workflow runs at a time and there is no concurrent writer.
"""

from __future__ import annotations

import asyncio
import gzip
import os
import shutil
import time
import xml.etree.ElementTree as ET
from concurrent.futures import ThreadPoolExecutor
from dataclasses import dataclass, field
from datetime import datetime, timedelta, timezone
from pathlib import Path

import httpx
import numpy as np
import pygrib
from temporalio import activity

from backend.shared.grid_dump import cleanup_old_grids, write_grid
from backend.shared.logger import get_logger
from backend.shared.palettes import get_palette_names, load_palette
from backend.shared.state import ProcessedSet
from backend.shared.storms import write_storms_json
from backend.shared.tiler import apply_color_table, render_tiles


MRMS_BASE = "https://noaa-mrms-pds.s3.amazonaws.com"
MRMS_PREFIX = os.environ.get("MRMS_PREFIX", "CONUS/MergedBaseReflectivityQC_00.50")
LAYER_NAME = os.environ.get("LAYER_NAME", "radar")
TILE_DIR = os.environ.get("TILE_DIR", "/data/tiles")
STATE_DIR = os.environ.get("STATE_DIR", "/data/state")
ZOOM_LEVELS = [4, 5, 6, 7, 8, 9]
BACKLOG_PER_CYCLE = int(os.environ.get("BACKLOG_PER_CYCLE", "3"))

log = get_logger("ingest-mrms-activities")


# ---------- serialisable activity I/O ----------


@dataclass
class ListKeysResult:
    keys: list[str] = field(default_factory=list)
    backlog_total: int = 0


@dataclass
class ProcessFrameResult:
    key: str
    timestamp: str | None
    rendered: bool
    palettes: list[str] = field(default_factory=list)
    duration_s: float = 0.0


@dataclass
class CleanupResult:
    tile_dirs_removed: int
    grid_files_removed: int


# ---------- helpers (sync, called from activity bodies) ----------


def _list_keys_sync(client: httpx.Client, prefix: str) -> list[str]:
    """List GRIB2 keys under a prefix, paginating past S3's 1000-key cap."""
    ns = {"s3": "http://s3.amazonaws.com/doc/2006-03-01/"}
    keys: list[str] = []
    continuation: str | None = None
    while True:
        url = f"{MRMS_BASE}?prefix={prefix}&list-type=2&max-keys=1000"
        if continuation:
            url += f"&continuation-token={continuation}"
        resp = client.get(url, timeout=30)
        resp.raise_for_status()
        root = ET.fromstring(resp.text)
        for content in root.findall(".//s3:Contents/s3:Key", ns):
            if content.text and content.text.endswith(".grib2.gz"):
                keys.append(content.text)
        if root.findtext(".//s3:IsTruncated", default="false", namespaces=ns) != "true":
            break
        continuation = root.findtext(".//s3:NextContinuationToken", namespaces=ns)
        if not continuation:
            break
    return keys


def _list_recent_files_sync(client: httpx.Client) -> list[str]:
    today = datetime.now(timezone.utc).strftime("%Y%m%d")
    keys = _list_keys_sync(client, f"{MRMS_PREFIX}/{today}")
    if not keys:
        yesterday = (datetime.now(timezone.utc) - timedelta(days=1)).strftime("%Y%m%d")
        keys = _list_keys_sync(client, f"{MRMS_PREFIX}/{yesterday}")
    return sorted(keys)


def _extract_timestamp(key: str) -> str:
    basename = key.split("/")[-1]
    parts = basename.replace(".grib2.gz", "").split("_")
    dt = datetime.strptime(parts[-1], "%Y%m%d-%H%M%S").replace(tzinfo=timezone.utc)
    return dt.isoformat()


def _download_and_decode_sync(
    client: httpx.Client, key: str, tmp_dir: Path
) -> tuple[np.ndarray, np.ndarray, np.ndarray] | None:
    url = f"{MRMS_BASE}/{key}"
    resp = client.get(url, timeout=60)
    resp.raise_for_status()
    body = resp.content

    gz_path = tmp_dir / "mrms.grib2.gz"
    grib_path = tmp_dir / "mrms.grib2"
    gz_path.write_bytes(body)
    with gzip.open(gz_path, "rb") as src:
        grib_path.write_bytes(src.read())

    try:
        grbs = pygrib.open(str(grib_path))
        grb = grbs[1]
        data = grb.values
        lats, lons = grb.latlons()
        lat_col = lats[:, 0]
        lon_row = lons[0, :]
        lon_row = np.where(lon_row > 180.0, lon_row - 360.0, lon_row)
        if hasattr(data, "filled"):
            data = data.filled(np.nan)
        grbs.close()
        return data.astype(np.float32), lat_col.astype(np.float64), lon_row.astype(np.float64)
    finally:
        gz_path.unlink(missing_ok=True)
        grib_path.unlink(missing_ok=True)


def _load_palette_tables() -> dict[str, dict]:
    tables: dict[str, dict] = {}
    for name in get_palette_names():
        try:
            tables[name] = load_palette(name)["reflectivity"]
        except (FileNotFoundError, KeyError):
            log.warning("palette_missing", extra={"palette": name})
    if not tables:
        tables["classic"] = load_palette("classic")["reflectivity"]
    return tables


def _render_palette(
    pname: str,
    ctable: dict,
    data: np.ndarray,
    lats_arr: np.ndarray,
    lons_arr: np.ndarray,
    flip: bool,
    timestamp: str,
    tile_base: Path,
) -> int:
    rgba = apply_color_table(data, ctable)
    if flip:
        rgba = np.flipud(rgba)
    out_dir = str(tile_base / LAYER_NAME / pname / timestamp)
    return render_tiles(rgba=rgba, lats=lats_arr, lons=lons_arr, output_dir=out_dir, zoom_levels=ZOOM_LEVELS)


def _state_path() -> Path:
    return Path(STATE_DIR) / "ingest-mrms.json"


# ---------- activities ----------


@activity.defn(name="mrms_list_unprocessed_keys")
async def mrms_list_unprocessed_keys() -> ListKeysResult:
    """List S3 keys, filter out ones already in ProcessedSet, return newest-first
    capped at BACKLOG_PER_CYCLE.
    """
    def _go() -> ListKeysResult:
        with httpx.Client() as client:
            keys = _list_recent_files_sync(client)
        state = ProcessedSet(_state_path(), max_entries=2000)
        new_keys = [k for k in keys if k not in state]
        # newest first so the user sees fresh frames ASAP
        ordered = list(reversed(new_keys))[:BACKLOG_PER_CYCLE]
        return ListKeysResult(keys=ordered, backlog_total=len(new_keys))

    return await asyncio.to_thread(_go)


@activity.defn(name="mrms_process_frame")
async def mrms_process_frame(key: str) -> ProcessFrameResult:
    """Download + decode + render every palette + write grid + detect storms
    for one MRMS key. Heartbeats every 30s while rendering.
    """
    started = time.time()
    palette_tables = _load_palette_tables()
    tile_base = Path(TILE_DIR)
    tmp_dir = Path("/tmp/mrms_work")
    tmp_dir.mkdir(parents=True, exist_ok=True)

    activity.heartbeat({"phase": "download", "key": key})

    def _download() -> tuple[np.ndarray, np.ndarray, np.ndarray] | None:
        with httpx.Client() as client:
            return _download_and_decode_sync(client, key, tmp_dir)

    decoded = await asyncio.to_thread(_download)
    if decoded is None:
        return ProcessFrameResult(key=key, timestamp=None, rendered=False)

    data, lats_arr, lons_arr = decoded
    flip = lats_arr[0] > lats_arr[-1]
    if flip:
        lats_arr = lats_arr[::-1]
    timestamp = _extract_timestamp(key)
    grid_data = np.flipud(data) if flip else data

    activity.heartbeat({"phase": "grid", "timestamp": timestamp})

    def _grids() -> None:
        try:
            write_grid(LAYER_NAME, timestamp, grid_data, lats_arr, lons_arr, unit="dBZ")
        except Exception as exc:  # noqa: BLE001
            log.warning("grid_dump_failed", extra={"err": str(exc)})
        try:
            write_storms_json(Path(STATE_DIR), grid_data, lats_arr, lons_arr, timestamp)
        except Exception as exc:  # noqa: BLE001
            log.warning("storm_detect_failed", extra={"err": str(exc)})

    await asyncio.to_thread(_grids)

    activity.heartbeat({"phase": "render", "timestamp": timestamp, "palettes": list(palette_tables.keys())})

    def _render_all() -> list[str]:
        rendered: list[str] = []
        with ThreadPoolExecutor(max_workers=max(1, len(palette_tables))) as pool:
            futures = {
                pool.submit(_render_palette, pname, ctable, data, lats_arr, lons_arr, flip, timestamp, tile_base): pname
                for pname, ctable in palette_tables.items()
            }
            for fut in futures:
                pname = futures[fut]
                try:
                    fut.result()
                    rendered.append(pname)
                except Exception as exc:  # noqa: BLE001
                    log.error("palette_render_failed", extra={"palette": pname, "err": str(exc)})
        return rendered

    rendered_palettes = await asyncio.to_thread(_render_all)
    duration = time.time() - started
    log.info("frame_done", extra={"timestamp": timestamp, "duration_s": round(duration, 1)})

    return ProcessFrameResult(
        key=key,
        timestamp=timestamp,
        rendered=True,
        palettes=rendered_palettes,
        duration_s=round(duration, 2),
    )


@activity.defn(name="mrms_mark_processed")
async def mrms_mark_processed(key: str) -> None:
    def _go() -> None:
        state = ProcessedSet(_state_path(), max_entries=2000)
        state.add(key)

    await asyncio.to_thread(_go)


@activity.defn(name="mrms_cleanup")
async def mrms_cleanup(retention_hours: int) -> CleanupResult:
    def _go() -> CleanupResult:
        tile_base = Path(TILE_DIR)
        layer_dir = tile_base / LAYER_NAME
        cutoff = time.time() - (retention_hours * 3600)
        removed = 0
        if layer_dir.exists():
            candidates: list[Path] = []
            for entry in sorted(layer_dir.iterdir()):
                if not entry.is_dir():
                    continue
                if entry.name[:1].isdigit():
                    candidates.append(entry)
                else:
                    candidates.extend(p for p in entry.iterdir() if p.is_dir())
            for ts_dir in candidates:
                try:
                    dt = datetime.fromisoformat(ts_dir.name)
                except ValueError:
                    continue
                if dt.timestamp() < cutoff:
                    shutil.rmtree(ts_dir, ignore_errors=True)
                    removed += 1
        grids_removed = cleanup_old_grids()
        return CleanupResult(tile_dirs_removed=removed, grid_files_removed=grids_removed)

    return await asyncio.to_thread(_go)
