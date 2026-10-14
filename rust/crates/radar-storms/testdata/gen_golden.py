"""Generate golden fixtures for the radar-storms Rust port.

Runs the ORIGINAL radar-ng Python (backend/shared/storms.py, storm_prefetch.py,
storm_watch.py and backend/api/api/storm_watch_activities.py) on deterministic
synthetic inputs and writes JSON the Rust tests compare against.

Usage (from any directory; -I keeps the interpreter isolated from the cwd):
    python3 -I gen_golden.py <radar-ng repo root> <output dir>

Requires numpy and scipy. temporalio and httpx are stubbed because only the
pure functions of storm_watch_activities.py are exercised.

Non-finite floats are written as {"$float": "-inf" | "inf" | "nan"} because
strict JSON cannot carry them. Values from detect_storms have generated_at
(wall clock) forced to 0.0 so outputs are deterministic.
"""

import asyncio
import json
import math
import os
import shutil
import sys
import tempfile
import types
from dataclasses import asdict, is_dataclass
from pathlib import Path

sys.dont_write_bytecode = True

REPO = Path(sys.argv[1]).resolve()
OUT = Path(sys.argv[2]).resolve()
OUT.mkdir(parents=True, exist_ok=True)
sys.path.insert(0, str(REPO))

# Stub the heavy third-party imports used only by the activity wrappers.
temporalio = types.ModuleType("temporalio")
activity_mod = types.ModuleType("temporalio.activity")


def _defn(fn=None, **_kwargs):
    if callable(fn):
        return fn
    return lambda f: f


activity_mod.defn = _defn
temporalio.activity = activity_mod
sys.modules["temporalio"] = temporalio
sys.modules["temporalio.activity"] = activity_mod
sys.modules["httpx"] = types.ModuleType("httpx")

import numpy as np  # noqa: E402

from backend.shared import storm_prefetch, storm_watch, storms  # noqa: E402
from backend.api.api import storm_watch_activities as swa  # noqa: E402


def enc(obj):
    """Make an object JSON-strict: non-finite floats become tagged strings."""
    if isinstance(obj, float) and not math.isfinite(obj):
        if math.isnan(obj):
            return {"$float": "nan"}
        return {"$float": "inf" if obj > 0 else "-inf"}
    if isinstance(obj, (np.floating,)):
        return enc(float(obj))
    if isinstance(obj, (np.integer,)):
        return int(obj)
    if isinstance(obj, dict):
        return {str(k): enc(v) for k, v in obj.items()}
    if isinstance(obj, (list, tuple)):
        return [enc(v) for v in obj]
    if is_dataclass(obj):
        return enc(asdict(obj))
    return obj


def write_json(name, obj):
    (OUT / name).write_text(json.dumps(enc(obj), indent=1, sort_keys=False) + "\n")
    print("wrote", OUT / name)


def linspace(a, b, n):
    return [float(x) for x in np.linspace(a, b, n)]


def build_grid(spec, lats, lons):
    h, w = len(lats), len(lons)
    if spec["kind"] == "rects":
        d = np.zeros((h, w), dtype=np.float32)
        for r0, r1, c0, c1, v in spec["rects"]:
            d[r0:r1, c0:c1] = np.float32(v)
        return d
    if spec["kind"] == "formula":
        r = np.arange(h, dtype=np.int64)[:, None]
        c = np.arange(w, dtype=np.int64)[None, :]
        if spec["name"] == "linear_mod45":
            k = (r * 37 + c * 53 + r * c * 7 + spec["shift"]) % 45
            return (30 + k).astype(np.float32)
        if spec["name"] == "blocks4":
            cond = ((r // 4) * 7 + (c // 4) * 5 + spec["shift"]) % 3 != 0
            var = ((r * c + spec["shift"]) % 4).astype(np.float32) * np.float32(0.5)
            return np.where(cond, np.float32(42.0) + var, np.float32(10.0)).astype(np.float32)
    raise ValueError(spec)


def run_chain(lats, lons, frames):
    """Run detect_storms on a chain of frames, feeding each output as previous."""
    prev = None
    outs = []
    for frame in frames:
        data = build_grid(frame["grid"], lats, lons)
        out = storms.detect_storms(
            data,
            np.asarray(lats, dtype=np.float64),
            np.asarray(lons, dtype=np.float64),
            timestamp=frame["timestamp"],
            previous=prev,
        )
        out["generated_at"] = 0.0
        outs.append(out)
        prev = out
    return outs


def rects_grid(*rects):
    return {"kind": "rects", "rects": [list(r) for r in rects]}


def cell_rect(lon_center, lat_center=42.0, size=5):
    """Same placement as test_storms._cell_frame, returned as a rect."""
    r = int(round((lat_center - 41.0) / 0.01))
    c = int(round((lon_center + 86.0) / 0.01))
    return [r, r + size, c, c + size, 55.0]


# --------------------------------------------------------------------------
# 1. detect_storms chains
# --------------------------------------------------------------------------
detect_cases = []

# (a) test_detect_storms_tracks_cell_and_emits_three_predicted_bboxes
lats20 = linspace(43.0, 42.81, 20)
lons20 = linspace(-85.0, -84.81, 20)
detect_cases.append({
    "name": "pair_moves_one_pixel_east",
    "lats": lats20, "lons": lons20,
    "frames": [
        {"timestamp": "2026-07-10T12:00:00+00:00", "grid": rects_grid([7, 10, 7, 10, 55.0])},
        {"timestamp": "2026-07-10T12:05:00+00:00", "grid": rects_grid([7, 10, 8, 11, 55.0])},
    ],
})

# (b) test_motion_is_fitted_over_history_not_one_noisy_step
lats200 = [float(x) for x in (41.0 + 0.01 * np.arange(200))]
lons300 = [float(x) for x in (-86.0 + 0.01 * np.arange(300))]
km_per_deg = 111.320 * np.cos(np.radians(42.0))
wobble = [0.0, 0.01, -0.01, 0.01, 0.0, -0.01, 0.01, 0.0]
frames = []
for step, jitter in enumerate(wobble):
    lon = -85.5 + (60.0 * step * 2 / 60.0) / km_per_deg + jitter
    frames.append({
        "timestamp": f"2026-07-10T12:{step * 2:02d}:00+00:00",
        "grid": rects_grid(cell_rect(lon)),
    })
detect_cases.append({"name": "eight_frame_motion_fit", "lats": lats200, "lons": lons300, "frames": frames})

# (c) test_new_cells_report_no_motion_until_history_spans_enough_time
detect_cases.append({
    "name": "gap_two_minutes_no_motion",
    "lats": lats200, "lons": lons300,
    "frames": [
        {"timestamp": "2026-07-10T12:00:00+00:00", "grid": rects_grid(cell_rect(-85.5))},
        {"timestamp": "2026-07-10T12:02:00+00:00", "grid": rects_grid(cell_rect(-85.48))},
    ],
})

# (d) test_distant_cell_is_not_associated_as_impossible_motion
detect_cases.append({
    "name": "distant_cell_new_id",
    "lats": lats200, "lons": lons300,
    "frames": [
        {"timestamp": "2026-07-10T12:00:00+00:00", "grid": rects_grid(cell_rect(-85.5))},
        {"timestamp": "2026-07-10T12:02:00+00:00", "grid": rects_grid(cell_rect(-85.3))},
    ],
})

# (e) test_area_uses_the_source_grid_cell_size
detect_cases.append({
    "name": "single_ten_by_ten_block",
    "lats": lats200, "lons": lons300,
    "frames": [
        {"timestamp": "2026-07-10T12:00:00+00:00", "grid": rects_grid(cell_rect(-85.5, size=10))},
    ],
})

# (f) two cells, one new cell, one below MIN_PIXELS, descending lats, lons > 180
lats60 = linspace(44.0, 43.4, 60)
lons60 = linspace(-86.0, -85.4, 60)
lons60_east = [x + 360.0 if x < 0 else x for x in lons60]  # exercise lons > 180 normalisation
detect_cases.append({
    "name": "two_cells_assignment_and_small_blob",
    "lats": lats60, "lons": lons60_east,
    "frames": [
        {"timestamp": "2026-07-10T12:00:00+00:00",
         "grid": rects_grid([10, 14, 10, 14, 55.0], [40, 44, 30, 34, 60.0])},
        {"timestamp": "2026-07-10T12:05:00+00:00",
         "grid": rects_grid([10, 14, 11, 15, 55.0], [40, 44, 29, 33, 60.0],
                            [25, 27, 25, 27, 45.0], [50, 52, 50, 52, 46.0])},
    ],
})

# (g) many ties in peak and components across block boundaries (label order matters)
lats64 = linspace(40.0, 40.63, 64)
lons64 = linspace(-100.0, -99.37, 64)
detect_cases.append({
    "name": "blocks_ties_label_order",
    "lats": lats64, "lons": lons64,
    "frames": [
        {"timestamp": "2026-07-10T12:00:00+00:00", "grid": {"kind": "formula", "name": "blocks4", "shift": 0}},
        {"timestamp": "2026-07-10T12:05:00+00:00", "grid": {"kind": "formula", "name": "blocks4", "shift": 1}},
    ],
})

# (h) decreasing lats and lons, linear mod formula, three frames
detect_cases.append({
    "name": "formula_three_frames_descending_lat",
    "lats": linspace(45.0, 40.0, 48),
    "lons": linspace(-90.0, -85.0, 48),
    "frames": [
        {"timestamp": "2026-07-10T12:00:00+00:00", "grid": {"kind": "formula", "name": "linear_mod45", "shift": 0}},
        {"timestamp": "2026-07-10T12:03:00+00:00", "grid": {"kind": "formula", "name": "linear_mod45", "shift": 3}},
        {"timestamp": "2026-07-10T12:06:00+00:00", "grid": {"kind": "formula", "name": "linear_mod45", "shift": 9}},
    ],
})

# (i) D3 reproduction: a cell disappears and a new cell appears two frames later.
lats40 = linspace(43.0, 42.6, 40)
lons40 = linspace(-85.0, -84.6, 40)
detect_cases.append({
    "name": "d3_id_reuse_reference",
    "defect": "D3",
    "lats": lats40, "lons": lons40,
    "frames": [
        {"timestamp": "2026-07-10T12:00:00+00:00",
         "grid": rects_grid([2, 5, 2, 5, 60.0], [10, 13, 10, 13, 60.0], [30, 33, 30, 33, 60.0])},
        {"timestamp": "2026-07-10T12:05:00+00:00",
         "grid": rects_grid([2, 5, 2, 5, 60.0], [10, 13, 10, 13, 60.0])},
        {"timestamp": "2026-07-10T12:10:00+00:00",
         "grid": rects_grid([2, 5, 2, 5, 60.0], [10, 13, 10, 13, 60.0], [20, 23, 20, 23, 60.0])},
    ],
})

detect_out = []
for case in detect_cases:
    outs = run_chain(case["lats"], case["lons"], case["frames"])
    detect_out.append({
        "name": case["name"],
        "defect": case.get("defect"),
        "lats": case["lats"],
        "lons": case["lons"],
        "frames": case["frames"],
        "expected": outs,
    })
write_json("storms_detect.json", {"cases": detect_out})

# --------------------------------------------------------------------------
# 2. write_storms_json: a backfilled frame must not regress the file.
# --------------------------------------------------------------------------
with tempfile.TemporaryDirectory() as tmp:
    tmp_path = Path(tmp)
    data = build_grid(rects_grid([7, 10, 7, 10, 55.0]), lats20, lons20)
    la = np.asarray(lats20, dtype=np.float64)
    lo = np.asarray(lons20, dtype=np.float64)
    storms.write_storms_json(tmp_path, data, la, lo, "2026-07-10T12:05:00+00:00")
    storms.write_storms_json(tmp_path, data, la, lo, "2026-07-10T12:00:00+00:00")
    payload = json.loads((tmp_path / "storms.json").read_text())
    after_backfill = payload["timestamp"]
    storms.write_storms_json(tmp_path, data, la, lo, "2026-07-10T12:05:00+00:00")
    after_equal = json.loads((tmp_path / "storms.json").read_text())["timestamp"]
    storms.write_storms_json(tmp_path, data, la, lo, "2026-07-10T12:10:00+00:00")
    after_newer = json.loads((tmp_path / "storms.json").read_text())["timestamp"]
    write_json("storms_write.json", {
        "after_newer_then_backfill": after_backfill,
        "after_equal": after_equal,
        "after_newer": after_newer,
    })

# --------------------------------------------------------------------------
# 3. build_storm_prefetch_plan
# --------------------------------------------------------------------------
def tiles_for(bbox, zoom):
    return storm_prefetch._tiles_for_bbox(bbox, zoom)


def fixed_tile_path(layer, palette, path, zoom, x, y):
    """Reference fix for D1: the frame path, with each '/'-separated segment quoted."""
    from urllib.parse import quote
    segs = ["tiles", layer, palette] + path.split("/") + [str(zoom), str(x), f"{y}.png"]
    return "/" + "/".join(quote(seg, safe=":+-.T_") for seg in segs)


def plan_with_fix(storms_doc, state_dir, tile_dir, base_url, lat, lon, zoom, palette, path_map):
    """Run the ORIGINAL Python plan, but with the D1 fix applied to the tile path only.

    path_map maps (layer, timestamp) -> frame path. This is the reference
    oracle for the Rust fix: everything except the tile path is unchanged.
    """
    original = storm_prefetch._tile_path

    def patched(layer, palette_, timestamp, zoom_, x, y):
        path = path_map.get((layer, timestamp), timestamp)
        return fixed_tile_path(layer, palette_, path, zoom_, x, y)

    storm_prefetch._tile_path = patched
    try:
        return storm_prefetch.build_storm_prefetch_plan(
            storms=storms_doc, state_dir=state_dir, tile_dir=tile_dir, base_url=base_url,
            lat=lat, lon=lon, zoom=zoom, palette=palette,
        )
    finally:
        storm_prefetch._tile_path = original


def write_existing_tiles(tile_dir, rel_paths):
    for rel in rel_paths:
        p = Path(tile_dir) / rel
        p.parent.mkdir(parents=True, exist_ok=True)
        p.write_bytes(b"png")


def existing_for_case(storms_doc, manifest, anchor_zoom_ts, path_map, palette, zoom):
    """Every tile the plan could ask for, using the given path map (used to create fixtures)."""
    rels = []
    anchor = storms_doc["timestamp"]
    feature = next(f for f in storms_doc["features"] if f["properties"]["cell_id"] == 1)
    del anchor_zoom_ts
    for pred in feature["properties"]["predicted_bboxes"][:3]:
        lead = pred["lead_minutes"]
        frame = storm_prefetch._frame_for_lead(manifest, storm_prefetch._parse_timestamp(anchor), lead)
        if frame is None:
            continue
        layer, ts = frame
        fz = min(max(4, int(zoom)), storm_prefetch.MAX_ZOOM[layer])
        for x, y in tiles_for(pred["bbox"], fz):
            path = path_map.get((layer, ts), ts)
            rels.append(f"{layer}/{palette}/{path}/{fz}/{x}/{y}.png")
    return rels


prefetch_cases = []
base_storms_lats = lats20
base_storms_lons = lons20
single = run_chain(base_storms_lats, base_storms_lons,
                   [{"timestamp": "2026-07-10T12:00:00+00:00", "grid": rects_grid([7, 10, 7, 10, 55.0])}])[0]
single["generated_at"] = 1783512000.123
base_storms = single

legacy_manifest = {
    "schema_version": 2,
    "layers": {
        "radar": {"timestamps": ["2026-07-10T12:00:00+00:00"], "latest": "2026-07-10T12:00:00+00:00",
                  "palettes": ["classic"],
                  "frames": [{"timestamp": "2026-07-10T12:00:00+00:00", "path": "2026-07-10T12:00:00+00:00", "palettes": ["classic"]}]},
        "nowcast": {"timestamps": ["2026-07-10T12:05:00+00:00", "2026-07-10T12:10:00+00:00"],
                    "latest": "2026-07-10T12:10:00+00:00", "palettes": ["classic"],
                    "frames": [
                        {"timestamp": "2026-07-10T12:05:00+00:00", "path": "2026-07-10T12:05:00+00:00", "palettes": ["classic"]},
                        {"timestamp": "2026-07-10T12:10:00+00:00", "path": "2026-07-10T12:10:00+00:00", "palettes": ["classic"]},
                    ]},
    },
    "tile_url_template": "/tiles/{layer}/{palette}/{path}/{z}/{x}/{y}.png",
    "updated_at": "2026-07-10T12:00:00+00:00",
}
run_anchor = "runs/2026-07-10T12:00:00+00:00"
run_manifest = json.loads(json.dumps(legacy_manifest))
run_manifest["layers"]["nowcast"]["frames"] = [
    {"timestamp": "2026-07-10T12:05:00+00:00", "path": f"{run_anchor}/2026-07-10T12:05:00+00:00", "palettes": ["classic"]},
    {"timestamp": "2026-07-10T12:10:00+00:00", "path": f"{run_anchor}/2026-07-10T12:10:00+00:00", "palettes": ["classic"]},
]
run_path_map = {
    ("nowcast", "2026-07-10T12:05:00+00:00"): f"{run_anchor}/2026-07-10T12:05:00+00:00",
    ("nowcast", "2026-07-10T12:10:00+00:00"): f"{run_anchor}/2026-07-10T12:10:00+00:00",
}
legacy_path_map = {}

REQ = {"base_url": "https://radar.example", "lat": 42.9, "lon": -84.9, "zoom": 6, "palette": "classic"}


def record_prefetch(name, storms_doc, manifest, existing, path_map, req=REQ, defect=None, with_fixed=False):
    tile_dir = Path(tempfile.mkdtemp())
    try:
        write_existing_tiles(tile_dir, existing)
        sdir = Path(tempfile.mkdtemp())
        (sdir / "manifest.json").write_text(json.dumps(manifest))
        python_plan = storm_prefetch.build_storm_prefetch_plan(
            storms=storms_doc, state_dir=sdir, tile_dir=tile_dir, base_url=req["base_url"],
            lat=req["lat"], lon=req["lon"], zoom=req["zoom"], palette=req["palette"],
        )
        entry = {
            "name": name,
            "defect": defect,
            "storms": storms_doc,
            "manifest": manifest,
            "existing_tiles": existing,
            "request": req,
            "expected_python": python_plan,
        }
        if with_fixed:
            entry["expected_fixed"] = plan_with_fix(
                storms_doc, sdir, tile_dir, req["base_url"], req["lat"], req["lon"],
                req["zoom"], req["palette"], path_map,
            )
        return entry
    finally:
        shutil.rmtree(tile_dir, ignore_errors=True)


# P1 legacy paths (path == timestamp): the test's own fixture shape.
p1_existing = existing_for_case(base_storms, legacy_manifest, None, legacy_path_map, "classic", 6)
prefetch_cases.append(record_prefetch("legacy_paths_match_timestamp", base_storms, legacy_manifest,
                                      p1_existing, legacy_path_map))

# P2 run-scoped nowcast paths: D1. Tiles exist on disk under runs/<anchor>/<valid>.
p2_existing = existing_for_case(base_storms, run_manifest, None, run_path_map, "classic", 6)
prefetch_cases.append(record_prefetch("nowcast_run_paths_D1", base_storms, run_manifest,
                                      p2_existing, run_path_map, defect="D1", with_fixed=True))

# P3 zoom clamp: requested 8, radar clamps to 7, nowcast to 6.
req8 = dict(REQ, zoom=8)
p3_existing = existing_for_case(base_storms, run_manifest, None, run_path_map, "classic", 8)
prefetch_cases.append(record_prefetch("zoom_clamp_run_paths", base_storms, run_manifest,
                                      p3_existing, run_path_map, req=req8, defect="D1", with_fixed=True))

# P4 no tiles on disk: bboxes present, tile_urls empty.
prefetch_cases.append(record_prefetch("no_tiles_on_disk", base_storms, legacy_manifest, [], legacy_path_map))

# P5 no nowcast frame close enough to lead 5 or 10.
far_manifest = json.loads(json.dumps(legacy_manifest))
far_manifest["layers"]["nowcast"] = {"timestamps": ["2026-07-10T12:30:00+00:00"], "palettes": ["classic"],
                                     "frames": [{"timestamp": "2026-07-10T12:30:00+00:00", "path": "2026-07-10T12:30:00+00:00", "palettes": ["classic"]}]}
p5_existing = existing_for_case(base_storms, far_manifest, None, legacy_path_map, "classic", 6)
prefetch_cases.append(record_prefetch("lead_without_frame", base_storms, far_manifest,
                                      p5_existing, legacy_path_map))

# P6 nothing within range of the user: plan is the empty shape.
prefetch_cases.append(record_prefetch("no_storm_in_range", base_storms, legacy_manifest, [], legacy_path_map,
                                      req=dict(REQ, lat=10.0, lon=10.0)))

# P7 a two-storm feature list where the far cell must not be chosen.
two_storms = json.loads(json.dumps(base_storms))
far_feature = json.loads(json.dumps(two_storms["features"][0]))
far_feature["geometry"]["coordinates"] = [-70.0, 30.0]
far_feature["properties"]["cell_id"] = 9
for pred in far_feature["properties"]["predicted_bboxes"]:
    w, s, e, n = pred["bbox"]
    pred["bbox"] = [-70.5, 29.5, -69.5, 30.5]
two_storms["features"].append(far_feature)
p7_existing = existing_for_case(two_storms, legacy_manifest, None, legacy_path_map, "classic", 6)
prefetch_cases.append(record_prefetch("nearest_storm_wins", two_storms, legacy_manifest,
                                      p7_existing, legacy_path_map))

write_json("prefetch.json", {"cases": prefetch_cases})

# --------------------------------------------------------------------------
# 4. storm_watch: sample_window, latest_radar_meta, detect_change
# --------------------------------------------------------------------------
H, W = 100, 100
LAT_MIN, LAT_MAX, LON_MIN, LON_MAX = 42.0, 43.0, -85.0, -84.0


def watch_grid():
    r = np.arange(H, dtype=np.int64)[:, None]
    c = np.arange(W, dtype=np.int64)[None, :]
    v = (20 + (r * 3 + c * 5) % 40).astype(np.float32)  # background peaks at 59 dBZ
    v[90:100, 90:100] = np.float32(-9999.0)
    v[50:53, 50:53] = np.float32(np.nan)
    v[10:14, 10:14] = np.float32(62.0)
    return v


def write_meta_pair(dirpath, name, values, with_data_file=True):
    dirpath.mkdir(parents=True, exist_ok=True)
    bin_name = name.replace(".meta.json", ".bin")
    values.astype("<f4").tofile(dirpath / bin_name)
    meta = {
        "height": int(values.shape[0]), "width": int(values.shape[1]),
        "lat_min": LAT_MIN, "lat_max": LAT_MAX, "lon_min": LON_MIN, "lon_max": LON_MAX,
        "unit": "dBZ", "fill": -9999.0, "stride": 1,
    }
    if with_data_file:
        meta["data_file"] = bin_name
    (dirpath / name).write_text(json.dumps(meta, sort_keys=True) + "\n")


tmp_root = Path(tempfile.mkdtemp())
grid_dir = tmp_root / "grids"
radar_dir = grid_dir / "radar"
vals = watch_grid()
write_meta_pair(radar_dir, "2026-07-10T12:00:00+00:00.meta.json", vals)
write_meta_pair(radar_dir, "2026-07-10T13:00:00+00:00.meta.json", vals, with_data_file=False)
write_meta_pair(radar_dir, "2026-07-10T12:30:00+00:00.meta.json", vals)  # sorts before 13:00
(radar_dir / ".hidden.meta.json").write_text("{}")  # glob('*') skips dotfiles

storm_watch.GRID_DIR = grid_dir
latest = storm_watch.latest_radar_meta()
latest_candidates = sorted(p.name for p in radar_dir.glob("*.meta.json"))

sample_cases = [
    ("normal_center", 42.5, -84.5, 20.0),
    ("out_of_bounds_south", 41.5, -84.5, 20.0),
    ("on_lat_min_boundary", 42.0, -84.5, 20.0),
    ("corner_small_radius", 43.0, -85.0, 5.0),
    ("fill_only_window", 43.0 - 95 / 99, -85.0 + 95 / 99, 1.0),
    ("nan_only_window", 43.0 - 51 / 99, -85.0 + 51 / 99, 1.0),
    ("half_even_rows", 42.5, -84.5, 2.5 * 111.32 / 99),
    ("hail_window", 43.0 - 11 / 99, -85.0 + 11 / 99, 3.0),
    ("no_data_file_key", 43.0 - 11 / 99, -85.0 + 11 / 99, 3.0),
]
sample_out = []
for name, lat, lon, rad in sample_cases:
    meta_path = radar_dir / ("2026-07-10T13:00:00+00:00.meta.json" if name == "no_data_file_key"
                             else "2026-07-10T12:00:00+00:00.meta.json")
    s = storm_watch.sample_window(meta_path, lat, lon, rad)
    sample_out.append({"name": name, "meta": meta_path.name, "lat": lat, "lon": lon, "radius_km": rad,
                       "sample": asdict(s) if s is not None else None})

# Missing bin file and malformed meta: both must come back as None.
(radar_dir / "2026-07-10T14:00:00+00:00.meta.json").write_text(json.dumps({
    "height": 2, "width": 2, "lat_min": LAT_MIN, "lat_max": LAT_MAX,
    "lon_min": LON_MIN, "lon_max": LON_MAX, "data_file": "missing.bin"}))
sample_out.append({"name": "missing_bin", "meta": "2026-07-10T14:00:00+00:00.meta.json", "lat": 42.5, "lon": -84.5,
                   "radius_km": 5.0,
                   "sample": asdict(s) if (s := storm_watch.sample_window(radar_dir / "2026-07-10T14:00:00+00:00.meta.json", 42.5, -84.5, 5.0)) else None})
(radar_dir / "2026-07-10T15:00:00+00:00.meta.json").write_text("{not json")
sample_out.append({"name": "malformed_meta", "meta": "2026-07-10T15:00:00+00:00.meta.json", "lat": 42.5, "lon": -84.5,
                   "radius_km": 5.0,
                   "sample": asdict(s) if (s := storm_watch.sample_window(radar_dir / "2026-07-10T15:00:00+00:00.meta.json", 42.5, -84.5, 5.0)) else None})

# detect_change: pure table.
change_cases = [
    ("no_prev", None, {"max_dbz": 30.0, "timestamp": "b"}, 0.0),
    ("severe_takes_precedence", {"max_dbz": 45.0}, {"max_dbz": 62.0}, 17.0),
    ("severe_exact_60", {"max_dbz": 40.0}, {"max_dbz": 60.0}, 20.0),
    ("intensify_exact_10", {"max_dbz": 30.0}, {"max_dbz": 40.0}, 10.0),
    ("intensify_rounding", {"max_dbz": 30.4}, {"max_dbz": 41.2}, 10.8),
    ("dissipate_exact_10", {"max_dbz": 50.0}, {"max_dbz": 40.0}, -10.0),
    ("dissipate_summary_sign", {"max_dbz": 50.0}, {"max_dbz": 38.0}, -12.0),
    ("small_change_none", {"max_dbz": 40.0}, {"max_dbz": 45.0}, 5.0),
    ("half_even_format", {"max_dbz": 40.5}, {"max_dbz": 40.5}, 0.0),
    ("format_2_5", {"max_dbz": 42.5}, {"max_dbz": 52.5}, 10.0),
]
change_out = []
for name, prev, curr, delta in change_cases:
    diff = storm_watch.FrameDiff(
        prev=None if prev is None else storm_watch.FrameSample("p", prev["max_dbz"], float("nan"), 0),
        curr=storm_watch.FrameSample("c", curr["max_dbz"], float("nan"), 0),
        max_dbz_delta=delta,
    )
    ch = storm_watch.detect_change(diff)
    change_out.append({"name": name, "prev": prev and prev["max_dbz"], "curr": curr["max_dbz"],
                       "delta": delta, "kind": ch.kind if ch else None, "summary": ch.summary if ch else None})
# D2 reproduction: -inf in the curr sample, delta computed unclamped as the API does.
d2 = storm_watch.FrameDiff(
    prev=storm_watch.FrameSample("p", 30.0, float("nan"), 0),
    curr=storm_watch.FrameSample("c", float("-inf"), float("nan"), 0),
    max_dbz_delta=float("-inf") - 30.0,
)
ch = storm_watch.detect_change(d2)
change_out.append({"name": "D2_minus_inf_curr", "defect": "D2", "prev": 30.0, "curr": float("-inf"),
                   "delta": float("-inf"), "kind": ch.kind if ch else None, "summary": ch.summary if ch else None})

# compare_radar_frames and detect_storm_change: run the ORIGINAL activity bodies
# against a grid directory that holds only the valid frame, so "latest" is known.
swa.STATE_DIR = tmp_root / "state"
cmp_grids = tmp_root / "cmp-grids"
cmp_radar = cmp_grids / "radar"
write_meta_pair(cmp_radar, "2026-07-10T12:00:00+00:00.meta.json", vals)
storm_watch.GRID_DIR = cmp_grids
compare_cases = [
    ("first_poll_sampled", dict(lat=42.5, lng=-84.5, prev_timestamp=None, prev_max_dbz=None), True),
    ("same_frame_skipped", dict(lat=42.5, lng=-84.5, prev_timestamp="2026-07-10T12:00:00+00:00", prev_max_dbz=30.0), True),
    ("second_poll_delta", dict(lat=42.5, lng=-84.5, prev_timestamp="2026-07-10T11:55:00+00:00", prev_max_dbz=30.0), True),
    ("out_of_bounds_D2", dict(lat=41.5, lng=-84.5, prev_timestamp="2026-07-10T11:55:00+00:00", prev_max_dbz=30.0), True),
    ("sentinel_prev_D2", dict(lat=42.5, lng=-84.5, prev_timestamp="2026-07-10T11:55:00+00:00", prev_max_dbz=-99.0), True),
    ("dissipate_on_poll", dict(lat=42.5, lng=-84.5, prev_timestamp="2026-07-10T11:55:00+00:00", prev_max_dbz=80.0), True),
    ("severe_on_poll", dict(lat=43.0 - 11 / 99, lng=-85.0 + 11 / 99, prev_timestamp="2026-07-10T11:55:00+00:00", prev_max_dbz=30.0), True),
]
compare_out = []
for name, kwargs, _ in compare_cases:
    inp = swa.CompareFramesInput(**kwargs)
    res = asyncio.run(swa.compare_radar_frames(inp))
    detect_inp = swa.DetectChangeInput(
        has_prev=res.has_prev, curr_max_dbz=res.curr_max_dbz,
        prev_max_dbz=res.prev_max_dbz, max_dbz_delta=res.max_dbz_delta,
    )
    change = asyncio.run(swa.detect_storm_change(detect_inp)) if res.sampled else None
    compare_out.append({"name": name, "input": kwargs, "result": asdict(res),
                        "change": asdict(change) if change is not None else None,
                        "defect": "D2" if name.endswith("D2") else None})

# No radar directory at all.
storm_watch.GRID_DIR = tmp_root / "absent"
res = asyncio.run(swa.compare_radar_frames(swa.CompareFramesInput(lat=42.5, lng=-84.5, prev_timestamp=None, prev_max_dbz=None)))
compare_out.append({"name": "no_grid_dir", "input": {"lat": 42.5, "lng": -84.5}, "result": asdict(res),
                    "change": None, "defect": None})

write_json("storm_watch.json", {
    "grid": {"height": H, "width": W, "lat_min": LAT_MIN, "lat_max": LAT_MAX,
             "lon_min": LON_MIN, "lon_max": LON_MAX, "formula": "watch_grid"},
    "latest_meta_name": latest.name if latest else None,
    "latest_candidates": latest_candidates,
    "samples": sample_out,
    "change": change_out,
    "compare": compare_out,
})

# Keep the grid bytes for the Rust side to regenerate instead of shipping .bin files.
# The formula is documented in gen_golden.py (watch_grid).
shutil.rmtree(tmp_root, ignore_errors=True)

# --------------------------------------------------------------------------
# 5. Numerics: numpy pairwise sums and Python rounding/formatting.
# --------------------------------------------------------------------------
lengths = [1, 2, 3, 7, 8, 9, 15, 16, 17, 31, 64, 100, 127, 128, 129, 200, 257, 300, 1000, 5000]
f32_means, f64_sums, f32_sums = [], [], []
for n in lengths:
    ks = [(k * 2654435761) % 1000 for k in range(n)]
    a32 = np.array([np.float32(v) * np.float32(0.1) for v in ks], dtype=np.float32)
    a64 = np.array([float(v) * 0.1 for v in ks], dtype=np.float64)
    f32_means.append({"n": n, "value": float(a32.mean())})
    f32_sums.append({"n": n, "value": float(a32.sum())})
    f64_sums.append({"n": n, "value": float(a64.sum())})

rounding_inputs = [0.5, 1.5, 2.5, -0.5, 2.675, 1.005, 0.125, 0.375, 12.345, 1e-7, 123456.7895, 0.0,
                   -12.5, 99.95, 0.0049999, 40.5, 52.5, 2.5e-5, 1783512000.123, -84.123456789]
rounding_rows = []
for x in rounding_inputs:
    rounding_rows.append({
        "x": x,
        "round0": round(x),
        "round1": round(x, 1),
        "round2": round(x, 2),
        "round3": round(x, 3),
        "round5": round(x, 5),
        "round6": round(x, 6),
        "fmt0": f"{x:.0f}",
    })

write_json("numerics.json", {"f32_mean": f32_means, "f32_sum": f32_sums, "f64_sum": f64_sums,
                             "rounding": rounding_rows})
print("done")
