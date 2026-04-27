#!/usr/bin/env python3
"""MRMS radar ingest: poll S3 → decode GRIB2 → render PNG tiles.

Hardened for Phase 1.4:
- Structured JSON logs (see services/shared/logger.py)
- Exponential backoff on S3 listing + downloads
- Processed-files state persisted to /data/state/ingest-mrms.json so restarts
  do not re-render tiles we already have.
"""

from __future__ import annotations

import gzip
import os
import shutil
import sys
import time
import xml.etree.ElementTree as ET
from concurrent.futures import ThreadPoolExecutor
from datetime import datetime, timedelta, timezone
from pathlib import Path

import httpx
import numpy as np
import pygrib

sys.path.insert(0, str(Path(__file__).resolve().parent / "shared"))
sys.path.insert(0, "/app/shared")
from logger import get_logger, retry  # type: ignore  # noqa: E402
from state import ProcessedSet  # type: ignore  # noqa: E402
from tiler import apply_color_table, render_tiles  # type: ignore  # noqa: E402
from palettes import get_palette_names, load_palette  # type: ignore  # noqa: E402
from grid_dump import cleanup_old_grids, write_grid  # type: ignore  # noqa: E402
from storms import write_storms_json  # type: ignore  # noqa: E402

MRMS_BASE = "https://noaa-mrms-pds.s3.amazonaws.com"
# Default to the QC-applied base reflectivity — NOAA filters ground clutter,
# anomalous propagation, and biological returns (birds/bugs) so the rendered
# tiles match what consumer apps like AccuWeather/RadarScope show, instead of
# the speckly raw 0.5° slice. Override per-deployment via env var:
#   CONUS/MergedBaseReflectivityQC_00.50   default · clean low-elevation slice
#   CONUS/MergedReflectivityQComposite     full-atmosphere composite
#   CONUS/MergedBaseReflectivity_00.50     raw (pre-QC) — for diagnostics
MRMS_PREFIX = os.environ.get("MRMS_PREFIX", "CONUS/MergedBaseReflectivityQC_00.50")
LAYER_NAME = os.environ.get("LAYER_NAME", "radar")
TILE_DIR = os.environ.get("TILE_DIR", "/data/tiles")
STATE_DIR = os.environ.get("STATE_DIR", "/data/state")
POLL_INTERVAL = int(os.environ.get("POLL_INTERVAL", "120"))
ZOOM_LEVELS = [4, 5, 6, 7, 8, 9]
RETENTION_HOURS = 4
# How many backlog frames to catch up per cycle. Newest first; each rendered
# frame is committed to state immediately so a crash can't lose progress.
BACKLOG_PER_CYCLE = int(os.environ.get("BACKLOG_PER_CYCLE", "3"))

log = get_logger("ingest-mrms")


def load_palette_tables() -> dict[str, dict]:
    """Load reflectivity color-table per active palette name."""
    tables: dict[str, dict] = {}
    for name in get_palette_names():
        try:
            tables[name] = load_palette(name)["reflectivity"]
        except (FileNotFoundError, KeyError) as exc:
            log.warning("palette_missing", extra={"palette": name, "err": str(exc)})
    if not tables:
        # Absolute fallback so the container never fails cold.
        tables["classic"] = load_palette("classic")["reflectivity"]
    return tables


@retry(attempts=4, base_delay=2.0, log=log, exceptions=(httpx.HTTPError,))
def _list_keys(client: httpx.Client, prefix: str) -> list[str]:
    """List all GRIB2 keys under a prefix, paginating past S3's 1000-key cap."""
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
        is_truncated = root.findtext(".//s3:IsTruncated", default="false", namespaces=ns)
        if is_truncated != "true":
            break
        continuation = root.findtext(".//s3:NextContinuationToken", namespaces=ns)
        if not continuation:
            break
    return keys


def list_recent_files(client: httpx.Client) -> list[str]:
    today = datetime.now(timezone.utc).strftime("%Y%m%d")
    keys = _list_keys(client, f"{MRMS_PREFIX}/{today}")
    if not keys:
        yesterday = (datetime.now(timezone.utc) - timedelta(days=1)).strftime("%Y%m%d")
        keys = _list_keys(client, f"{MRMS_PREFIX}/{yesterday}")
    return sorted(keys)


@retry(attempts=4, base_delay=2.0, log=log, exceptions=(httpx.HTTPError,))
def _fetch(client: httpx.Client, url: str) -> bytes:
    resp = client.get(url, timeout=60)
    resp.raise_for_status()
    return resp.content


def download_and_decode(
    client: httpx.Client, key: str, tmp_dir: Path
) -> tuple[np.ndarray, np.ndarray, np.ndarray] | None:
    url = f"{MRMS_BASE}/{key}"
    try:
        body = _fetch(client, url)
    except httpx.HTTPError as exc:
        log.error("download_failed", extra={"key": key, "err": str(exc)})
        return None

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
        # MRMS GRIB2 uses 0..360 longitudes; the tiler and every downstream
        # consumer wants -180..180. Monotonicity is preserved for CONUS since
        # the domain (230..300) doesn't cross the dateline.
        lon_row = np.where(lon_row > 180.0, lon_row - 360.0, lon_row)
        if hasattr(data, "filled"):
            data = data.filled(np.nan)
        grbs.close()
        return data.astype(np.float32), lat_col.astype(np.float64), lon_row.astype(np.float64)
    except Exception as exc:  # noqa: BLE001
        log.error("decode_failed", extra={"key": key, "err": str(exc)})
        return None
    finally:
        gz_path.unlink(missing_ok=True)
        grib_path.unlink(missing_ok=True)


def extract_timestamp(key: str) -> str:
    """CONUS/MergedBaseReflectivity_00.50/MergedBaseReflectivity_00.50_20260414-200200.grib2.gz"""
    basename = key.split("/")[-1]
    parts = basename.replace(".grib2.gz", "").split("_")
    dt_str = parts[-1]
    dt = datetime.strptime(dt_str, "%Y%m%d-%H%M%S").replace(tzinfo=timezone.utc)
    return dt.isoformat()


def cleanup_old_tiles(base_dir: Path, retention_hours: int) -> None:
    """Remove timestamp subtrees older than cutoff across every palette."""
    layer_dir = base_dir / LAYER_NAME
    if not layer_dir.exists():
        return
    cutoff = time.time() - (retention_hours * 3600)

    # Two layouts are possible for backward compat:
    #   /{layer}/{timestamp}/{z}/...            (legacy, no palette)
    #   /{layer}/{palette}/{timestamp}/{z}/...  (multi-palette)
    candidates: list[Path] = []
    for entry in sorted(layer_dir.iterdir()):
        if not entry.is_dir():
            continue
        # Heuristic: timestamp dirs start with a digit (ISO date); palette dirs are alpha.
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
            log.info("retention_expired", extra={"layer": LAYER_NAME, "timestamp": ts_dir.name, "path": str(ts_dir)})


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
    """Render a single palette's tile pyramid. Safe to run in a thread —
    PIL releases the GIL during PNG encode."""
    rgba = apply_color_table(data, ctable)
    if flip:
        rgba = np.flipud(rgba)
    out_dir = str(tile_base / LAYER_NAME / pname / timestamp)
    return render_tiles(
        rgba=rgba,
        lats=lats_arr,
        lons=lons_arr,
        output_dir=out_dir,
        zoom_levels=ZOOM_LEVELS,
    )


def _process_key(
    client: httpx.Client,
    key: str,
    tmp_dir: Path,
    tile_base: Path,
    palette_tables: dict[str, dict],
    pool: ThreadPoolExecutor,
) -> bool:
    """Decode one MRMS key + render every palette in parallel. Returns True
    on success so the caller can commit the key to state."""
    result = download_and_decode(client, key, tmp_dir)
    if result is None:
        return False

    data, lats_arr, lons_arr = result
    flip = lats_arr[0] > lats_arr[-1]
    if flip:
        lats_arr = lats_arr[::-1]
    timestamp = extract_timestamp(key)

    grid_data = np.flipud(data) if flip else data
    try:
        write_grid(LAYER_NAME, timestamp, grid_data, lats_arr, lons_arr, unit="dBZ")
    except Exception as exc:  # noqa: BLE001
        log.warning("grid_dump_failed", extra={"err": str(exc)})

    try:
        write_storms_json(Path(STATE_DIR), grid_data, lats_arr, lons_arr, timestamp)
    except Exception as exc:  # noqa: BLE001
        log.warning("storm_detect_failed", extra={"err": str(exc)})

    started = time.time()
    futures = {
        pool.submit(
            _render_palette, pname, ctable, data, lats_arr, lons_arr, flip, timestamp, tile_base
        ): pname
        for pname, ctable in palette_tables.items()
    }
    for fut in futures:
        pname = futures[fut]
        try:
            count = fut.result()
            log.info(
                "rendered",
                extra={"layer": LAYER_NAME, "palette": pname, "timestamp": timestamp, "tiles": count},
            )
        except Exception as exc:  # noqa: BLE001
            log.error("palette_render_failed", extra={"palette": pname, "err": str(exc)})
    log.info(
        "frame_done",
        extra={"timestamp": timestamp, "duration_s": round(time.time() - started, 1)},
    )
    return True


def run() -> None:
    palette_tables = load_palette_tables()
    client = httpx.Client()
    tile_base = Path(TILE_DIR)
    tmp_dir = Path("/tmp/mrms_work")
    tmp_dir.mkdir(parents=True, exist_ok=True)

    state = ProcessedSet(Path(STATE_DIR) / "ingest-mrms.json", max_entries=2000)
    log.info(
        "startup",
        extra={
            "tile_dir": str(tile_base),
            "poll_interval_s": POLL_INTERVAL,
            "processed": len(state._items),
            "palettes": list(palette_tables.keys()),
            "backlog_per_cycle": BACKLOG_PER_CYCLE,
        },
    )

    # Reused across iterations — one thread per palette is enough since we
    # render one frame at a time and palettes are the parallelism unit.
    pool = ThreadPoolExecutor(max_workers=max(1, len(palette_tables)))

    while True:
        started = time.time()
        try:
            keys = list_recent_files(client)
            new_keys = [k for k in keys if k not in state]

            if new_keys:
                # Newest first so the user always sees the freshest frame
                # ASAP; older backlog frames fill the timeline behind it.
                ordered = list(reversed(new_keys))[:BACKLOG_PER_CYCLE]
                log.info(
                    "processing_batch",
                    extra={"backlog": len(new_keys), "rendering": len(ordered)},
                )
                for key in ordered:
                    if _process_key(client, key, tmp_dir, tile_base, palette_tables, pool):
                        state.add(key)

            cleanup_old_tiles(tile_base, RETENTION_HOURS)
            cleanup_old_grids()

        except Exception as exc:  # noqa: BLE001
            log.exception("loop_error", extra={"err": str(exc)})

        elapsed = time.time() - started
        sleep_for = max(5.0, POLL_INTERVAL - elapsed)
        time.sleep(sleep_for)


if __name__ == "__main__":
    run()
