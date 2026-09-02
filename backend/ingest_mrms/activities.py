"""Temporal activities for the MRMS radar ingest pipeline.

Orchestrated by IngestMrmsWorkflow.

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
from dataclasses import dataclass, field
from datetime import datetime, timedelta, timezone
from pathlib import Path

import httpx
import numpy as np
import pygrib
from temporalio import activity

from backend.shared.activity_heartbeat import run_sync_with_heartbeat
from backend.shared.grid_dump import cleanup_old_grids, prune_grid_layer, write_grid
from backend.shared.logger import get_logger
from backend.shared.manifest import update_manifest_file
from backend.shared.palettes import get_palette_names, load_palette
from backend.shared.state import ProcessedSet
from backend.shared.storms import write_storms_json
from backend.shared.tiler import render_frame_palettes


MRMS_BASE = "https://noaa-mrms-pds.s3.amazonaws.com"
TILE_DIR = os.environ.get("TILE_DIR", "/data/tiles")
STATE_DIR = os.environ.get("STATE_DIR", "/data/state")
TMP_ROOT = Path(os.environ.get("MRMS_TMP_ROOT", "/tmp/mrms_work"))
# z7 is approximately the native display ceiling of the ~1 km CONUS MRMS
# source. z8 was pure upsampling and accounted for roughly 75% of this
# pyramid's tile count, so it spent freshness budget without adding detail.
ZOOM_LEVELS = [4, 5, 6, 7]
BACKLOG_PER_CYCLE = int(os.environ.get("BACKLOG_PER_CYCLE", "3"))
NOWCAST_SCIENCE_GRID_MAX_CELLS = int(
    os.environ.get("NOWCAST_SCIENCE_GRID_MAX_CELLS", "7000000")
)


# Defaults — used when the workflow doesn't pass overrides.
DEFAULT_MRMS_PREFIX = os.environ.get("MRMS_PREFIX", "CONUS/MergedBaseReflectivityQC_00.50")
DEFAULT_LAYER_NAME = os.environ.get("LAYER_NAME", "radar")

log = get_logger("ingest-mrms-activities")


# ---------- serialisable activity I/O ----------


@dataclass
class IngestMrmsArgs:
    """Per-product config the workflow passes to every activity. Defaults
    match the QC-applied base reflectivity used by the original
    ingest-mrms CronJob; the radar-composite schedule overrides both
    fields to point at the full-atmosphere composite product.
    """
    mrms_prefix: str = DEFAULT_MRMS_PREFIX
    layer_name: str = DEFAULT_LAYER_NAME


@dataclass
class ListKeysResult:
    keys: list[str] = field(default_factory=list)
    backlog_total: int = 0


@dataclass
class ProcessFrameResult:
    key: str
    timestamp: str = ""
    rendered: bool = False
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


def _list_recent_files_sync(client: httpx.Client, mrms_prefix: str) -> list[str]:
    today = datetime.now(timezone.utc).strftime("%Y%m%d")
    keys = _list_keys_sync(client, f"{mrms_prefix}/{today}")
    if not keys:
        yesterday = (datetime.now(timezone.utc) - timedelta(days=1)).strftime("%Y%m%d")
        keys = _list_keys_sync(client, f"{mrms_prefix}/{yesterday}")
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
        try:
            grb = grbs[1]
            data = grb.values
            lats, lons = grb.latlons()
        finally:
            grbs.close()
        lat_col = lats[:, 0]
        lon_row = lons[0, :]
        lon_row = np.where(lon_row > 180.0, lon_row - 360.0, lon_row)
        if hasattr(data, "filled"):
            data = data.filled(np.nan)
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


def _render_all_palettes(
    palette_tables: dict[str, dict],
    data: np.ndarray,
    lats_arr: np.ndarray,
    lons_arr: np.ndarray,
    timestamp: str,
    tile_base: Path,
    layer_name: str,
) -> list[str]:
    """Sample values once per tile, then publish every palette."""
    out_dirs = {pname: str(tile_base / layer_name / pname / timestamp) for pname in palette_tables}
    result = render_frame_palettes(
        data,
        lats_arr,
        lons_arr,
        palette_tables,
        out_dirs,
        ZOOM_LEVELS,
        nodata_value=None,
        min_valid_weight=1.0,
    )
    return result.rendered_palettes


def _state_path(layer_name: str) -> Path:
    """Per-layer state file so multiple schedules (radar + radar-composite)
    don't race on a shared ProcessedSet.
    """
    return Path(STATE_DIR) / f"ingest-mrms-{layer_name}.json"


def _safe_path_part(value: object) -> str:
    raw = str(value)
    safe = "".join(ch if ch.isalnum() or ch in ("-", "_") else "_" for ch in raw)
    return safe.strip("_") or "unknown"


def _current_activity_tmp_dir(prefix: str, *parts: object) -> Path:
    info = activity.info()
    name_parts = [
        _safe_path_part(prefix),
        _safe_path_part(info.workflow_id),
        _safe_path_part(info.workflow_run_id),
        _safe_path_part(info.activity_id),
        f"attempt{info.attempt}",
        *(_safe_path_part(p) for p in parts),
    ]
    tmp_dir = TMP_ROOT / "-".join(name_parts)
    tmp_dir.mkdir(parents=True, exist_ok=True)
    return tmp_dir


# ---------- activities ----------


@dataclass
class MarkProcessedInput:
    key: str
    layer_name: str


@dataclass
class CleanupInput:
    layer_name: str
    retention_hours: int


@activity.defn(name="mrms_list_unprocessed_keys")
async def mrms_list_unprocessed_keys(args: IngestMrmsArgs) -> ListKeysResult:
    def _go() -> ListKeysResult:
        with httpx.Client() as client:
            keys = _list_recent_files_sync(client, args.mrms_prefix)
        state = ProcessedSet(_state_path(args.layer_name), max_entries=2000)
        new_keys = [k for k in keys if k not in state]
        ordered = list(reversed(new_keys))[:BACKLOG_PER_CYCLE]
        return ListKeysResult(keys=ordered, backlog_total=len(new_keys))

    return await asyncio.to_thread(_go)


@dataclass
class ProcessFrameInput:
    key: str
    layer_name: str


@activity.defn(name="mrms_process_frame")
async def mrms_process_frame(inp: ProcessFrameInput) -> ProcessFrameResult:
    started = time.time()
    # Palette JSON reads and the flock'd manifest update below are disk I/O:
    # on the event loop they stall every other activity's heartbeats.
    palette_tables = await asyncio.to_thread(_load_palette_tables)
    tile_base = Path(TILE_DIR)
    timestamp_hint = inp.key.rsplit("/", 1)[-1].replace(".grib2.gz", "")
    tmp_dir = _current_activity_tmp_dir("mrms", inp.layer_name, timestamp_hint)

    activity.heartbeat({"phase": "download", "key": inp.key})

    def _download() -> tuple[np.ndarray, np.ndarray, np.ndarray] | None:
        with httpx.Client() as client:
            return _download_and_decode_sync(client, inp.key, tmp_dir)

    try:
        decoded = await asyncio.to_thread(_download)
    except BaseException:
        shutil.rmtree(tmp_dir, ignore_errors=True)
        raise
    if decoded is None:
        shutil.rmtree(tmp_dir, ignore_errors=True)
        return ProcessFrameResult(key=inp.key, rendered=False)

    data, lats_arr, lons_arr = decoded
    flip = lats_arr[0] > lats_arr[-1]
    if flip:
        lats_arr = lats_arr[::-1]
    timestamp = _extract_timestamp(inp.key)
    grid_data = np.flipud(data) if flip else data

    def _grids() -> None:
        try:
            write_grid(inp.layer_name, timestamp, grid_data, lats_arr, lons_arr, unit="dBZ")
        except Exception as exc:  # noqa: BLE001
            log.warning("grid_dump_failed", extra={"err": str(exc)})
        # Only base reflectivity owns storms.json. The composite schedule runs
        # independently and otherwise races this file with different cells and
        # timestamps, making motion vectors jump between products.
        if inp.layer_name == "radar":
            try:
                write_storms_json(Path(STATE_DIR), grid_data, lats_arr, lons_arr, timestamp)
            except Exception as exc:  # noqa: BLE001
                log.warning("storm_detect_failed", extra={"err": str(exc)})

    activity.heartbeat({"phase": "render", "timestamp": timestamp, "palettes": list(palette_tables.keys())})

    def _render_all() -> list[str]:
        # One in-process pass samples physical values once per tile; every
        # palette is then a PLTE/tRNS rewrite of the same indexed PNG. Let
        # failures reach Temporal so a retry can converge on any complete,
        # immutable palette directories already published by the first try.
        return _render_all_palettes(
            palette_tables,
            grid_data,
            lats_arr,
            lons_arr,
            timestamp,
            tile_base,
            inp.layer_name,
        )

    try:
        rendered_palettes = await run_sync_with_heartbeat(
            _render_all,
            heartbeat_every=30,
            heartbeat_details=lambda: {
                "phase": "render",
                "timestamp": timestamp,
                "palettes": list(palette_tables.keys()),
            },
        )
    finally:
        # finally, not except Exception: CancelledError is a BaseException and
        # a cancelled frame otherwise leaks its tmp dir on the node disk.
        shutil.rmtree(tmp_dir, ignore_errors=True)
    expected_palettes = set(palette_tables)
    if set(rendered_palettes) != expected_palettes:
        # A fully transparent frame publishes no palettes and is not
        # advertised. Rendering failures raise above and are retried instead.
        log.error(
            "frame_incomplete",
            extra={
                "layer": inp.layer_name,
                "timestamp": timestamp,
                "expected_palettes": sorted(expected_palettes),
                "rendered_palettes": sorted(rendered_palettes),
            },
        )
        return ProcessFrameResult(
            key=inp.key,
            timestamp=timestamp,
            rendered=False,
            palettes=sorted(rendered_palettes),
            duration_s=round(time.time() - started, 2),
        )

    # Grids and storm metadata are derived from a frame only after all public
    # tiles are complete. Nowcast can never anchor to an unpublishable frame.
    activity.heartbeat({"phase": "grid", "timestamp": timestamp})
    await asyncio.to_thread(_grids)
    if inp.layer_name == "radar":
        await asyncio.to_thread(
            write_grid,
            "radar-nowcast-input",
            timestamp,
            grid_data,
            lats_arr,
            lons_arr,
            "dBZ",
            max_cells=NOWCAST_SCIENCE_GRID_MAX_CELLS,
        )
        # 24.5 MB every 2 min; nowcast only ever reads the newest N, so prune at write time.
        await asyncio.to_thread(prune_grid_layer, "radar-nowcast-input")

    duration = time.time() - started
    await asyncio.to_thread(
        update_manifest_file,
        inp.layer_name,
        timestamp,
        palettes=rendered_palettes,
        action="add",
        frame={
            "path": timestamp,
            "source": "mrms",
            "kind": "observation",
            "issued_at": timestamp,
            "lead_minutes": 0,
            "spatial_resolution_km": 1.0,
            "max_zoom": max(ZOOM_LEVELS),
        },
        layer_metadata={
            "title": "MRMS observed reflectivity",
            "kind": "observation",
        },
    )
    log.info("frame_done", extra={"layer": inp.layer_name, "timestamp": timestamp, "duration_s": round(duration, 1)})

    return ProcessFrameResult(
        key=inp.key,
        timestamp=timestamp,
        rendered=True,
        palettes=rendered_palettes,
        duration_s=round(duration, 2),
    )


@activity.defn(name="mrms_mark_processed")
async def mrms_mark_processed(inp: MarkProcessedInput) -> None:
    def _go() -> None:
        state = ProcessedSet(_state_path(inp.layer_name), max_entries=2000)
        state.add(inp.key)

    await asyncio.to_thread(_go)


@activity.defn(name="mrms_cleanup")
async def mrms_cleanup(inp: CleanupInput) -> CleanupResult:
    def _go() -> CleanupResult:
        tile_base = Path(TILE_DIR)
        layer_dir = tile_base / inp.layer_name
        cutoff = time.time() - (inp.retention_hours * 3600)
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
                    # Manifest first, then tiles — never advertise deleted tiles.
                    update_manifest_file(inp.layer_name, ts_dir.name, action="remove")
                    shutil.rmtree(ts_dir, ignore_errors=True)
                    removed += 1
        grids_removed = cleanup_old_grids()
        return CleanupResult(tile_dirs_removed=removed, grid_files_removed=grids_removed)

    return await asyncio.to_thread(_go)
