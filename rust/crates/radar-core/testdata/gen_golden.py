#!/usr/bin/env python3
"""Generate the byte-exact golden fixtures for radar-core from the Python originals.

Run from anywhere with a Python that has numpy (see README.md, "Regenerating the golden fixtures"):

    /tmp/venv-radar-core/bin/python crates/radar-core/testdata/gen_golden.py

The Python modules are loaded by file path from SHARED (the radar-ng source tree, read-only). Clocks and
UUIDs are pinned so the output is deterministic. Non-deterministic inputs (wall-clock `updated_at`,
`uuid4`) are replaced by fixed values, and the Rust tests use the same fixed values.
"""

import importlib.util
import json
import logging
import os
import pathlib
import random
import shutil
import struct
import sys
import tempfile
import time
import types

import numpy as np
from datetime import datetime, timedelta, timezone

HERE = pathlib.Path(__file__).resolve().parent
SHARED = pathlib.Path(os.environ.get("RADAR_NG_SHARED", "./backend/shared"))
OUT = HERE


def load(name: str):
    spec = importlib.util.spec_from_file_location(f"radar_shared_{name}", SHARED / f"{name}.py")
    module = importlib.util.module_from_spec(spec)
    sys.modules[spec.name] = module  # dataclasses looks the module up by name
    spec.loader.exec_module(module)
    return module


os.environ["TZ"] = "UTC"
time.tzset()

NOW = datetime(2026, 5, 20, 12, 5, 3, 418221, tzinfo=timezone.utc)
SKILL_NOW = datetime(2026, 9, 30, 12, 0, 0, tzinfo=timezone.utc)


def write(path: pathlib.Path, text: str) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(text, encoding="utf-8")


# ---------------------------------------------------------------- manifest


def gen_manifest() -> None:
    manifest = load("manifest")

    class FixedDatetime(datetime):
        @classmethod
        def now(cls, tz=None):
            return NOW

    manifest.datetime = FixedDatetime
    with tempfile.TemporaryDirectory() as tmp:
        state = pathlib.Path(tmp)
        v1 = {
            "layers": {
                "radar": {
                    "palettes": ["classic"],
                    "timestamps": ["2026-05-20T12:00:00+00:00"],
                    "title": "MRMS observed reflectivity",
                }
            },
            "updated_at": "2026-05-20T12:00:00+00:00",
        }
        write(OUT / "manifest" / "v1_input.json", json.dumps(v1, separators=(",", ":"), sort_keys=True) + "\n")
        (state / "manifest.json").write_text((OUT / "manifest" / "v1_input.json").read_text())
        t1 = "2026-05-20T12:05:00+00:00"
        t2 = "2026-05-20T12:10:00+00:00"
        manifest.update_manifest_file(
            "radar",
            t1,
            palettes=["classic", "vivid"],
            frame={
                "source": "mrms",
                "kind": "observation",
                "issued_at": t1,
                "lead_minutes": 0,
                "spatial_resolution_km": 1.0,
                "max_zoom": 7,
            },
            layer_metadata={"title": "MRMS observed reflectivity", "kind": "observation"},
            state_dir=state,
        )
        manifest.replace_layer_manifest(
            "nowcast",
            [t2],
            palettes=["classic"],
            state_dir=state,
            frames=[
                {
                    "timestamp": t2,
                    "path": "runs/2026-05-20T12:00:00+00:00/2026-05-20T12:10:00+00:00",
                    "palettes": [],
                    "grid_key": "runs/2026-05-20T12:00:00+00:00/2026-05-20T12:10:00+00:00",
                    "source": "mrms-nowcast",
                    "kind": "nowcast",
                    "issued_at": "2026-05-20T12:00:00+00:00",
                    "lead_minutes": 10,
                    "input_interval_minutes": 2.0,
                    "method": "pysteps-sprog",
                    "spatial_resolution_km": 1.0,
                    "max_zoom": 6,
                }
            ],
            layer_metadata={
                "horizon_minutes": 60,
                "kind": "nowcast",
                "method": "pysteps-sprog",
                "run_id": "2026-05-20T12:00:00+00:00",
                "step_minutes": 5,
                "title": "MRMS motion nowcast",
            },
        )
        manifest.update_manifest_file("radar", "2026-05-20T11:55:00+00:00", action="remove", state_dir=state)
        final = (state / "manifest.json").read_text()
        write(OUT / "manifest" / "v1_migrated_final.json", final)


# ---------------------------------------------------------------- grid dumps


GRID_GENERATIONS = [f"{i:02x}" * 16 for i in range(1, 40)]


def gen_grids() -> None:
    grid_dump = load("grid_dump")
    scenarios = []

    def w(layer, ts, data, lats, lons, unit, gen, fill=None, max_cells=None, grid_key=None):
        return {
            "op": "write",
            "layer": layer,
            "timestamp": ts,
            "data": data,
            "lats": lats,
            "lons": lons,
            "unit": unit,
            "fill": fill,  # None means NaN
            "max_cells": max_cells,
            "grid_key": grid_key,
            "generation": gen,
        }

    ramp = lambda h, w_: [[float(r * w_ + c) for c in range(w_)] for r in range(h)]
    scenarios.append({
        "name": "basic_nan_fill",
        "ops": [w("radar", "2026-07-15T12:00:00+00:00", [[1.0, 1.0], [1.0, 1.0]], [43.0, 42.0], [-86.0, -85.0], "dBZ", GRID_GENERATIONS[0])],
    })
    scenarios.append({
        "name": "flip_lon_wrap_nan",
        "ops": [w(
            "radar",
            "2026-07-15T12:05:00+00:00",
            [[1.5, None, 3.25], [4.0, 5.0, -0.0], [7.0, 8.0, 9.0]],
            [40.0, 41.0, 42.0],
            [200.0, 201.0, 350.0],
            "°F",
            GRID_GENERATIONS[1],
        )],
    })
    scenarios.append({
        "name": "stride_10x10_cap25",
        "ops": [w("radar", "2026-07-15T12:10:00+00:00", ramp(10, 10), [50.0 - 0.5 * i for i in range(10)], [-130.0 + 0.5 * i for i in range(10)], "dBZ", GRID_GENERATIONS[2], max_cells=25)],
    })
    scenarios.append({
        "name": "stride_odd_9x9_cap16",
        "ops": [w("radar", "2026-07-15T12:15:00+00:00", ramp(9, 9), [9.0 - i for i in range(9)], [float(i) for i in range(9)], "J/kg", GRID_GENERATIONS[3], max_cells=16)],
    })
    scenarios.append({
        "name": "explicit_fill_keeps_nan",
        "ops": [w("radar", "2026-07-15T12:20:00+00:00", [[1.0, None], [-1.5, 2.0]], [43.0, 42.0], [-86.0, -85.0], "mph", GRID_GENERATIONS[4], fill=-1.5)],
    })
    scenarios.append({
        "name": "run_grid_key_and_finalize",
        "ops": [
            w("radar-nowcast-input", "2026-07-15T12:00:00+00:00", [[2.0, 3.0], [4.0, 5.0]], [43.0, 42.0], [-86.0, -85.0], "dBZ", GRID_GENERATIONS[5]),
            w("nowcast", "2026-07-15T12:05:00+00:00", [[10.0, 11.0], [12.0, 13.0]], [43.0, 42.0], [-86.0, -85.0], "dBZ", GRID_GENERATIONS[6], fill=-9999.0, grid_key="runs/2026-07-15T12:00:00+00:00/2026-07-15T12:05:00+00:00"),
            {"op": "finalize", "layer": "nowcast", "generation": "2026-07-15T12:00:00+00:00"},
        ],
    })
    scenarios.append({
        "name": "shape_mismatch_writes_nothing",
        "ops": [{"op": "write_expect_none", "layer": "radar", "timestamp": "2026-07-15T12:25:00+00:00", "data": [[1.0, 2.0, 3.0]], "lats": [43.0, 42.0], "lons": [-86.0, -85.0], "unit": "dBZ", "fill": None, "max_cells": None, "grid_key": None, "generation": GRID_GENERATIONS[7]}],
    })

    manifest_meta = {}
    for scenario in scenarios:
        with tempfile.TemporaryDirectory() as tmp:
            root = pathlib.Path(tmp) / "grids"
            grid_dump.GRID_DIR = str(root)
            for op in scenario["ops"]:
                if op["op"] in ("write", "write_expect_none"):
                    class _UUID:
                        def __init__(self, hex_value):
                            self.hex = hex_value
                    grid_dump.uuid = types.SimpleNamespace(uuid4=lambda g=op["generation"]: _UUID(g))
                    data = [[float("nan") if v is None else v for v in row] for row in op["data"]]
                    fill = float("nan") if op["fill"] is None else op["fill"]
                    kwargs = {"max_cells": op["max_cells"], "grid_key": op["grid_key"]}
                    result = grid_dump.write_grid(
                        op["layer"],
                        op["timestamp"],
                        np.array(data, dtype=np.float64),
                        np.array(op["lats"], dtype=np.float64),
                        np.array(op["lons"], dtype=np.float64),
                        op["unit"],
                        fill,
                        **kwargs,
                    )
                    if op["op"] == "write_expect_none":
                        assert result is None, result
                elif op["op"] == "finalize":
                    grid_dump.finalize_grid_generation(op["layer"], op["generation"])
            expected = OUT / "grid" / "expected" / scenario["name"]
            if expected.exists():
                shutil.rmtree(expected)
            if root.exists():
                shutil.copytree(root, expected)
            else:
                expected.mkdir(parents=True)
    write(OUT / "grid" / "scenarios.json", json.dumps(
        [{k: v for k, v in s.items()} for s in scenarios], indent=1, ensure_ascii=True) + "\n")


# ---------------------------------------------------------------- nowcast skill


def gen_skill() -> None:
    skill = load("nowcast_skill")

    def entry(anchor, lead, scored_at, hits=10, misses=2, false_alarms=3, persist=(6, 6, 6)):
        return {
            "anchor": anchor,
            "valid": "v",
            "lead_minutes": lead,
            "scored_at": scored_at.isoformat(),
            "cells": 100,
            "thresholds": {
                "rain": {"hits": hits, "misses": misses, "false_alarms": false_alarms},
                "heavy": {"hits": 1, "misses": 1, "false_alarms": 0},
            },
            "persistence": {
                "rain": {"hits": persist[0], "misses": persist[1], "false_alarms": persist[2]},
                "heavy": {"hits": 0, "misses": 2, "false_alarms": 0},
            },
        }

    entries = []
    for i in range(8):
        entries.append(entry(f"a{i}", 30, SKILL_NOW - timedelta(minutes=i)))
        entries.append(entry(f"a{i}", 5, SKILL_NOW - timedelta(minutes=i), hits=20, misses=0, false_alarms=0))
    entries.append(entry("old", 30, SKILL_NOW - timedelta(hours=30)))
    entries.append(entry("future", 15, SKILL_NOW + timedelta(minutes=2)))
    entries.append(entry("b0", 15, SKILL_NOW - timedelta(minutes=1), hits=0, misses=0, false_alarms=0, persist=(0, 0, 0)))
    write(OUT / "skill" / "entries.json", json.dumps(entries, indent=1, ensure_ascii=True) + "\n")

    with tempfile.TemporaryDirectory() as tmp:
        state = pathlib.Path(tmp)
        log = skill.append_entries(state, entries, now=SKILL_NOW)
        text = (state / "nowcast-skill.json").read_text(encoding="utf-8")
        names = sorted(p.name for p in state.iterdir())
        assert names == [skill.SKILL_FILE], names
        write(OUT / "skill" / "nowcast-skill.json", text)
        summary = skill.summarize(log, now=SKILL_NOW)
        write(OUT / "skill" / "summary.json", json.dumps(summary, indent=1, sort_keys=True, ensure_ascii=True, allow_nan=True) + "\n")
        write(OUT / "skill" / "prometheus.txt", "\n".join(skill.prometheus_lines(summary)) + "\n")

    # NSK-01 ratio table, Python round(num/den, 3) with its repr.
    rng = random.Random(20260930)
    cases = []
    for _ in range(600):
        num = rng.randint(0, 500)
        den = rng.randint(0, 500)
        value = skill._ratio(num, den)
        cases.append({"num": num, "den": den, "repr": "None" if value is None else repr(value)})
    write(OUT / "skill" / "ratios.json", json.dumps(cases, indent=1) + "\n")


# ---------------------------------------------------------------- floats and json strings


def gen_pycompat() -> None:
    rng = random.Random(1234)
    values = [0.0, -0.0, 1.0, -1.0, 43.0, -9999.0, 0.0001, 0.00001, 1e15, 1e16, 1.5e300, 5e-324,
              1.7976931348623157e308, 0.1 + 0.2, 21.10000000000001, 123.456, 0.833, 1e22, 9007199254740993.0,
              float("1e-4"), 2.5e-5, 3.0000000000000004e-1]
    for _ in range(600):
        values.append(rng.uniform(-1, 1) * 10 ** rng.randint(-30, 30))
    floats = []
    for v in values:
        floats.append({"bits": struct.pack(">d", v).hex(), "repr": repr(v)})
    write(OUT / "pycompat" / "floats.json", json.dumps(floats, indent=1) + "\n")

    strings = ["plain", "°F", "a\"b\\c", "tab\tnew\nline\rcr", "\u0001\u001f", "\u007f", "→ ✓ 😀", "é", ""]
    write(OUT / "pycompat" / "strings.json", json.dumps(
        [{"value": s, "json": json.dumps(s)} for s in strings], indent=1, ensure_ascii=True) + "\n")


# ---------------------------------------------------------------- logger


def gen_logger() -> None:
    logger_mod = load("logger")
    formatter = logger_mod.JsonFormatter()
    cases = [
        {"service": "radar-ingest", "level": logging.WARNING, "msg": "retry",
         "extras": [("fn", "fetch_grib"), ("attempt", 1), ("max_attempts", 4), ("delay", 1.0), ("err", "connection reset")]},
        {"service": "radar-api", "level": logging.INFO, "msg": "unit °F → ✓", "extras": [("ok", True), ("n", 0), ("ratio", 0.833)]},
        {"service": "radar-api", "level": logging.ERROR, "msg": "plain", "extras": []},
    ]
    out = []
    for case in cases:
        record = logging.LogRecord(case["service"], case["level"], __file__, 1, case["msg"], None, None)
        record.created = 1781956800.0  # 2026-06-20T12:00:00 UTC
        record.msecs = 0.0
        for key, value in case["extras"]:
            setattr(record, key, value)
        out.append({"service": case["service"], "level": logging.getLevelName(case["level"]), "msg": case["msg"],
                    "extras": [[k, v] for k, v in case["extras"]], "line": formatter.format(record)})
    write(OUT / "logger" / "lines.json", json.dumps(out, indent=1, ensure_ascii=True) + "\n")


# ---------------------------------------------------------------- processed set


def gen_processed_set() -> None:
    state = load("state")
    with tempfile.TemporaryDirectory() as tmp:
        path = pathlib.Path(tmp) / "ingest-hrrr.json"
        ps = state.ProcessedSet(path, max_entries=3)
        ps.add("a")
        ps.add("b")
        ps.update(["c", "a", "d"])
        write(OUT / "processed_set" / "expected.json", path.read_text(encoding="utf-8"))
        write(OUT / "processed_set" / "ops.json", json.dumps({
            "max_entries": 3,
            "ops": [["add", "a"], ["add", "b"], ["update", ["c", "a", "d"]]],
            "expected_items": list(ps._items),
        }, indent=1) + "\n")


# ---------------------------------------------------------------- push tokens schema


def gen_push_tokens() -> None:
    import sqlite3
    push = load("push_tokens")
    with tempfile.TemporaryDirectory() as tmp:
        push.STATE_DIR = pathlib.Path(tmp)
        push.DB_PATH = push.STATE_DIR / "push_tokens.sqlite"
        push.upsert("u", "t", "ios")
        conn = sqlite3.connect(str(push.DB_PATH))
        rows = conn.execute("SELECT name, sql FROM sqlite_master WHERE sql IS NOT NULL ORDER BY name").fetchall()
        conn.close()
    write(OUT / "push_tokens" / "schema.json", json.dumps([{"name": n, "sql": s} for n, s in rows], indent=1) + "\n")


def main() -> None:
    gen_manifest()
    gen_grids()
    gen_skill()
    gen_pycompat()
    gen_logger()
    gen_processed_set()
    gen_push_tokens()
    print("golden fixtures written to", OUT)


if __name__ == "__main__":
    sys.exit(main())
