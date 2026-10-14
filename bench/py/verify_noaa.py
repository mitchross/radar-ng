#!/usr/bin/env python3
"""Python side of the pre-cutover NOAA check (rust/scripts/preflight.sh, step "noaa").

Compares the production Python decode (pygrib, backend.* functions) with the Rust dump written by
`noaa_dump dump` (crates/radar-bench) on the same files. Covers the radar-grib README list
"Must verify against real NOAA files", items 1 to 8, and 9 and 10 when a NAQFC file was fetched.

Subcommands:
  compare   run every comparison, print a table, write --json; exit 1 when a check fails
  render    render an MRMS file with the production indexed renderer, for `radar parity --exact`
  make-idx  write a wgrib2-style .idx for a local GRIB2 file (local test fixtures have none)

Python reads the production modules from RADAR_NG (the repository root). Nothing is written there.
Status words: PASS, FAIL (exit 1), WARN (look at it, not blocking), INFO, SKIP.
"""

from __future__ import annotations

import argparse
import gzip
import hashlib
import json
import re
import shutil
import sys
import tempfile
from pathlib import Path

import numpy as np

sys.path.insert(0, str(Path(__file__).resolve().parent))
from common import RADAR_NG, write_json  # noqa: E402

AXIS_TOL = 1e-9
# The variables whose .idx record should be unique in f01 (README item 2).
UNIQUE_IN_F01 = ("refc", "tcdc", "apcp", "cape", "crain", "csnow", "cfrzr", "cicep")
IDX_LINE = re.compile(r"^\d+:\d+:d=\d{10}:[^:]+:[^:]+:[^:]*:")


class Results:
    def __init__(self) -> None:
        self.rows: list[dict] = []
        self.notes: dict = {}

    def add(self, group: str, check: str, status: str, detail: str = "", **data) -> None:
        self.rows.append({"group": group, "check": check, "status": status, "detail": detail, **data})

    @property
    def failed(self) -> bool:
        return any(r["status"] == "FAIL" for r in self.rows)


# ------------------------------------------------------------------ helpers

def canonical_f32(values) -> np.ndarray:
    """Values as production uses them: masked cells to NaN, then float32, flattened."""
    if np.ma.isMaskedArray(values):
        values = values.filled(np.nan)
    return np.ascontiguousarray(np.asarray(values, dtype=np.float32).ravel())


def canonical_hash(values: np.ndarray) -> str:
    """SHA-256 of LE f32 with every NaN as 0x7fc00000 (same as noaa_dump)."""
    bits = np.ascontiguousarray(values, dtype="<f4").view("<u4").copy()
    bits[np.isnan(values)] = 0x7FC00000
    return hashlib.sha256(bits.tobytes()).hexdigest()


def compare_f32(py: np.ndarray, rust: np.ndarray) -> dict:
    """Bitwise comparison; NaN equals NaN whatever the payload."""
    if py.shape != rust.shape:
        return {"equal": False, "detail": f"length {py.size} vs {rust.size}"}
    py_nan, rs_nan = np.isnan(py), np.isnan(rust)
    same_bits = py.view("<u4") == rust.view("<u4")
    ok = same_bits | (py_nan & rs_nan)
    bad = int((~ok).sum())
    both = ~(py_nan | rs_nan)
    diff = np.abs(py[both].astype(np.float64) - rust[both].astype(np.float64))
    return {
        "equal": bad == 0,
        "mismatches": bad,
        "nan_py": int(py_nan.sum()),
        "nan_rust": int(rs_nan.sum()),
        "max_abs_diff": float(diff.max()) if diff.size else 0.0,
        "detail": f"{bad} of {py.size} cells differ" if bad else f"{py.size} cells bitwise equal",
    }


def max_abs(a: np.ndarray, b: np.ndarray) -> float:
    a = np.asarray(a, dtype=np.float64).ravel()
    b = np.asarray(b, dtype=np.float64).ravel()
    if a.shape != b.shape:
        return float("inf")
    if a.size == 0:
        return 0.0
    return float(np.nanmax(np.abs(a - b)))


def read_f32(path: Path) -> np.ndarray:
    return np.fromfile(path, dtype="<f4")


def read_f64(path: Path) -> np.ndarray:
    return np.fromfile(path, dtype="<f8")


def msg_key(grb, key: str):
    try:
        return grb[key]
    except Exception:  # noqa: BLE001 (eccodes raises for keys a template lacks)
        return None


def import_backend():
    sys.path.insert(0, str(RADAR_NG))


class _Resp:
    def __init__(self, content: bytes) -> None:
        self.content = content
        self.text = content.decode("utf-8", errors="replace")
        self.status_code = 200

    def raise_for_status(self) -> None:
        return None


class _StubClient:
    """Stands in for httpx.Client: get() returns local bytes, so production helpers run offline."""

    def __init__(self, content: bytes) -> None:
        self._content = content

    def get(self, url: str, timeout: float | None = None, **_kw) -> _Resp:
        return _Resp(self._content)


# ------------------------------------------------------------------ MRMS (items 5, 7, 8)

def python_mrms(path: Path, work: Path):
    """Production decode (`_download_and_decode_sync`) plus the process_frame south-up flip."""
    import_backend()
    from backend.ingest_mrms import activities as ing

    tmp = Path(tempfile.mkdtemp(prefix="mrms-", dir=work))
    try:
        decoded = ing._download_and_decode_sync(_StubClient(path.read_bytes()), path.name, tmp)
    finally:
        shutil.rmtree(tmp, ignore_errors=True)
    if decoded is None:
        raise RuntimeError("production decode returned None")
    data, lat, lon = decoded
    stored = (float(lat[0]), float(lat[-1]))
    flip = bool(lat[0] > lat[-1])
    if flip:
        lat = lat[::-1]
        data = np.flipud(data)
    return np.ascontiguousarray(data, dtype=np.float32), np.asarray(lat), np.asarray(lon), flip, stored


def compare_mrms(res: Results, path: Path, rust: dict, rust_dir: Path, work: Path) -> None:
    import pygrib

    g = "mrms"
    try:
        data, lat, lon, flip, stored = python_mrms(path, work)
    except Exception as exc:  # noqa: BLE001
        res.add(g, "python decode", "FAIL", f"{type(exc).__name__}: {exc}")
        return
    res.add(g, "file", "INFO", path.name, shape=list(data.shape))
    # Packing details for items 7 and 8.
    with gzip.open(path, "rb") as fh, tempfile.NamedTemporaryFile(dir=work, suffix=".grib2") as tmp:
        tmp.write(fh.read())
        tmp.flush()
        grbs = pygrib.open(tmp.name)
        grb = grbs[1]
        info = {k: msg_key(grb, k) for k in ("packingType", "binaryScaleFactor", "decimalScaleFactor",
                                             "bitmapPresent", "gridType", "Ni", "Nj")}
        grbs.close()
    res.add(g, "packing (items 7, 8)", "INFO",
            f"{info['packingType']}, E={info['binaryScaleFactor']}, D={info['decimalScaleFactor']}, "
            f"bitmap={info['bitmapPresent']}", pygrib=info)

    rs_shape = rust.get("shape")
    if list(data.shape) != rs_shape:
        res.add(g, "shape", "FAIL", f"python {list(data.shape)} vs rust {rs_shape}")
        return
    res.add(g, "shape", "PASS", f"{data.shape[0]} x {data.shape[1]}")

    flip_ok = flip == rust["flipped"] and lat[0] <= lat[-1] and rust["lat_first"] <= rust["lat_last"]
    res.add(g, "south-up flip", "PASS" if flip_ok else "FAIL",
            f"stored lat {stored[0]} .. {stored[1]}; python flipped={flip}, rust flipped={rust['flipped']}")

    cmp = compare_f32(data.ravel(), read_f32(rust_dir / rust["values_file"]))
    res.add(g, "values f32 bitwise (item 5)", "PASS" if cmp["equal"] else "FAIL", cmp["detail"],
            compare=cmp)
    dlat = max_abs(lat, read_f64(rust_dir / rust["lats_file"]))
    dlon = max_abs(lon, read_f64(rust_dir / rust["lons_file"]))
    ok = dlat <= AXIS_TOL and dlon <= AXIS_TOL
    res.add(g, "lat/lon axes <= 1e-9", "PASS" if ok else "FAIL", f"max |dlat| {dlat:.3g}, |dlon| {dlon:.3g}")


# ------------------------------------------------------------------ HRRR (items 2, 3, 4, 6, 7, 8)

def py_matches(grb, match: dict) -> bool:
    """The match loop of backend.ingest_hrrr.activities._extract_variable."""
    for k, v in match.items():
        if k == "name":
            if v.lower() not in grb.name.lower():
                return False
        elif getattr(grb, k, None) != v:
            return False
    return True


def compare_hrrr(res: Results, path: Path, idx_path: Path | None, rust: dict, rust_dir: Path,
                 allow_missing: bool) -> None:
    import pygrib

    import_backend()
    from backend.ingest_hrrr import activities as hr

    g = "hrrr"
    res.add(g, "file", "INFO", path.name)

    # Every message: pygrib metadata, packing and value hashes.
    grbs = pygrib.open(str(path))
    messages = []
    for i, grb in enumerate(grbs, start=1):
        vals = canonical_f32(grb.values)
        messages.append({
            "index": i,
            "shortName": grb.shortName, "name": grb.name,
            "typeOfLevel": grb.typeOfLevel, "level": grb.level,
            "discipline": msg_key(grb, "discipline"),
            "parameterCategory": msg_key(grb, "parameterCategory"),
            "parameterNumber": msg_key(grb, "parameterNumber"),
            "typeOfFirstFixedSurface": msg_key(grb, "typeOfFirstFixedSurface"),
            "packingType": msg_key(grb, "packingType"),
            "binaryScaleFactor": msg_key(grb, "binaryScaleFactor"),
            "decimalScaleFactor": msg_key(grb, "decimalScaleFactor"),
            "bitmapPresent": msg_key(grb, "bitmapPresent"),
            "groupSplittingMethodUsed": msg_key(grb, "groupSplittingMethodUsed"),
            "missingValueManagementUsed": msg_key(grb, "missingValueManagementUsed"),
            "values_sha256": canonical_hash(vals),
        })
    grbs.close()
    res.notes["hrrr_messages"] = messages

    n_py, n_rs, n_split = len(messages), rust["message_count_listed"], rust["message_count_split"]
    res.add(g, "message count", "PASS" if n_py == n_rs == n_split else "FAIL",
            f"pygrib {n_py}, rust list_messages {n_rs}, split by length {n_split}")

    # Item 7: every message decodes in Rust; item 4 for the whole file: hashes match pygrib.
    decode = {d["index"]: d for d in rust["decode"]}
    failed = [d for d in rust["decode"] if not d["ok"]]
    differ = [m["index"] for m in messages
              if decode.get(m["index"], {}).get("ok") and decode[m["index"]]["values_sha256"] != m["values_sha256"]]
    templates = sorted({str(m["packingType"]) for m in messages})
    res.add(g, "all messages decode in Rust (item 7)", "PASS" if not failed else "WARN",
            f"{n_rs - len(failed)}/{n_rs} decode; packings {', '.join(templates)}"
            + ("; failures: " + "; ".join(f"#{d['index']} {d['error'][:80]}" for d in failed[:5]) if failed else ""))
    res.add(g, "all messages bitwise vs pygrib", "PASS" if not differ else "WARN",
            f"{n_py - len(differ) - len(failed)} equal, {len(differ)} differ"
            + (f" (#{', #'.join(map(str, differ[:10]))})" if differ else ""))
    by_index = {m["index"]: m for m in messages}
    for i in differ[:5]:
        mm = by_index[i]
        res.add(g, f"differing message #{i}", "INFO",
                f"{mm['shortName']}/{mm['typeOfLevel']}/{mm['level']} codes {mm['discipline']}.{mm['parameterCategory']}."
                f"{mm['parameterNumber']} packing {mm['packingType']} E={mm['binaryScaleFactor']} "
                f"D={mm['decimalScaleFactor']} bitmap={mm['bitmapPresent']} "
                f"groupSplit={mm['groupSplittingMethodUsed']} missingMgmt={mm['missingValueManagementUsed']}")
    neg = [m for m in messages if (m["binaryScaleFactor"] or 0) < 0 or (m["decimalScaleFactor"] or 0) < 0]
    neg_bad = [m["index"] for m in neg if m["index"] in differ or not decode.get(m["index"], {}).get("ok")]
    used = {v.get("selected") for v in rust.get("vars", {}).values() if v.get("selected")}
    if neg:
        # Only the 13 variables radar-ng reads must be exact; another field that differs is reported.
        used_bad = [i for i in neg_bad if i in used]
        status = "PASS" if not neg_bad else ("FAIL" if used_bad else "WARN")
        res.add(g, "negative E/D scale factors (item 8)", status,
                f"{len(neg)} messages with E<0 or D<0, {len(neg) - len(neg_bad)} bitwise equal"
                + (f"; differing #{', #'.join(map(str, neg_bad))} are not fields radar-ng reads" if neg_bad and not used_bad else ""))
    else:
        res.add(g, "negative E/D scale factors (item 8)", "WARN", "no message in this file has E<0 or D<0")
    gs = [m["index"] for m in messages if m["groupSplittingMethodUsed"] not in (None, 1)]
    mv = [m["index"] for m in messages if m["missingValueManagementUsed"] not in (None, 0)]
    res.add(g, "complex-packing options (item 7)", "INFO",
            f"group splitting != 1: {len(gs)} messages; missing-value management != 0: {len(mv)} messages")

    # Item 2: .idx format and record picks.
    py_picks: dict[str, list[int]] = {}
    if idx_path is not None and rust.get("idx"):
        text = idx_path.read_text()
        lines = [ln for ln in text.strip().splitlines() if ln.strip()]
        bad_lines = [ln for ln in lines if not IDX_LINE.match(ln)]
        res.add(g, ".idx line format (item 2)", "PASS" if not bad_lines else "FAIL",
                f"{len(lines)} lines" + (f", {len(bad_lines)} malformed, e.g. {bad_lines[0][:80]!r}" if bad_lines else
                                         " match num:offset:d=YYYYMMDDHH:VAR:level:fcst:"))
        records = hr._get_idx_sync(_StubClient(text.encode()), str(idx_path))
        by_offset = {r["offset"]: r["num"] for r in records}
        for var, terms in hr.IDX_MATCHERS.items():
            ranges = hr._pick_ranges(records, [terms])
            py_picks[var] = [by_offset[s] for s, _ in ranges]
            rs = rust["idx"]["picks"].get(var, {}).get("records")
            rs_ranges = [(r["start"], r["end"]) for r in rust["idx"]["picks"].get(var, {}).get("ranges", [])]
            same = rs == py_picks[var] and rs_ranges == [(s, e) for s, e in ranges]
            status = "PASS" if same else "FAIL"
            detail = f"python records {py_picks[var]}, rust {rs}"
            if same and not py_picks[var]:
                status = "SKIP" if allow_missing else "FAIL"
                detail = "no record matches on either side"
            elif same and var in UNIQUE_IN_F01 and len(py_picks[var]) != 1:
                status = "WARN"
                detail += " (expected exactly one record in f01)"
            res.add(g, f"idx pick {var}", status, detail)
    else:
        res.add(g, ".idx picks (item 2)", "SKIP", "no .idx file")

    # Items 3, 4, 6: selector choice, values and Lambert lat/lon for the 13 variables.
    mesh_lat = mesh_lon = None
    if rust.get("mesh_files"):
        mesh_lat = read_f64(rust_dir / rust["mesh_files"][0])
        mesh_lon = read_f64(rust_dir / rust["mesh_files"][1])
    grbs = pygrib.open(str(path))
    try:
        for var, match in hr.VAR_SELECTORS.items():
            rv = rust["vars"].get(var, {})
            # Python reference: the message production's _extract_variable returns (matched by its
            # values), not a re-implemented match loop.
            py_idx = None
            ext = hr._extract_variable(path, match)
            if ext is not None:
                ext_hash = canonical_hash(canonical_f32(ext.data))
                hits = [mm["index"] for mm in messages if mm["values_sha256"] == ext_hash]
                rs_pick = rv.get("selected")
                py_idx = rs_pick if rs_pick in hits else (hits[0] if hits else None)
            rs_idx = rv.get("selected")
            if py_idx is None and rs_idx is None:
                res.add(g, f"select {var}", "SKIP" if allow_missing else "FAIL",
                        "no message matches on either side")
                continue
            m = messages[py_idx - 1] if py_idx else None
            names = (f"pygrib #{py_idx} {m['shortName']}/{m['typeOfLevel']}/{m['level']}" if m else "pygrib: none")
            rinfo = rv.get("info") or {}
            codes = (f"rust #{rs_idx} {rinfo.get('discipline')}.{rinfo.get('category')}.{rinfo.get('number')}"
                     f" surface {rinfo.get('surface_type')} level {rinfo.get('level_value')}") if rs_idx else "rust: none"
            sel_ok = py_idx == rs_idx
            extra = ""
            if len(rv.get("matching", [])) > 1:
                extra = f"; rust selector matches {rv['matching']}"
            status = "PASS" if sel_ok else "FAIL"
            if py_idx is None and rs_idx is not None:
                # Python's name selector finds nothing: production skips this layer, Rust renders it.
                # A behaviour change caused by the eccodes name tables, not a Rust decode error.
                rm = messages[rs_idx - 1]
                status = "WARN"
                extra += (f"; python name selector {match} matches nothing (rust's message is pygrib "
                          f"{rm['name']!r} {rm['shortName']}/{rm['typeOfLevel']}/{rm['level']}): "
                          "production Python would skip this layer")
            if status == "FAIL" and m is not None:
                extra += (f"; pygrib codes {m['discipline']}.{m['parameterCategory']}.{m['parameterNumber']}"
                          f" surface {m['typeOfFirstFixedSurface']}")
                if py_picks.get(var) == [py_idx] and rs_idx is None:
                    extra += " (the .idx record Rust fetches is this message)"
            res.add(g, f"select {var} (item 3)", status, f"{names}; {codes}{extra}")
            if py_picks.get(var) and py_idx and py_picks[var][0] != py_idx:
                res.add(g, f"idx record vs message {var}", "WARN",
                        f"idx picks record {py_picks[var]}, name selector picks message #{py_idx}")
            if "error" in rv:
                res.add(g, f"values {var} (item 4)", "FAIL", f"rust decode error: {rv['error'][:120]}")
                continue
            # Values and mesh are compared on the message both picked, or on Rust's pick when Python's
            # name selector found nothing (still a decode check of that message).
            cmp_idx = py_idx if sel_ok else (rs_idx if py_idx is None else None)
            if cmp_idx is None or "values_file" not in rv:
                continue
            m = messages[cmp_idx - 1]
            grb = grbs.message(cmp_idx)
            py_vals = canonical_f32(grb.values)
            cmp = compare_f32(py_vals, read_f32(rust_dir / rv["values_file"]))
            res.add(g, f"values {var} (item 4)", "PASS" if cmp["equal"] else "FAIL",
                    f"{cmp['detail']}; packing {m['packingType']}, E={m['binaryScaleFactor']}, "
                    f"D={m['decimalScaleFactor']}", compare=cmp)
            if mesh_lat is None:
                res.add(g, f"lat/lon {var} (item 6)", "FAIL", "rust grid is not projected (no mesh)")
                continue
            if not rv.get("mesh_same_as_written"):
                res.add(g, f"lat/lon {var} (item 6)", "FAIL", "rust mesh differs from the first variable's")
                continue
            lats, lons = grb.latlons()
            lons = hr._normalize_lons(lons)
            dlat, dlon = max_abs(lats, mesh_lat), max_abs(lons, mesh_lon)
            ok = dlat <= AXIS_TOL and dlon <= AXIS_TOL
            res.add(g, f"lat/lon {var} (item 6)", "PASS" if ok else "FAIL",
                    f"max |dlat| {dlat:.3g}, |dlon| {dlon:.3g} over {lats.size} points")
    finally:
        grbs.close()

    # The fetch_hour subset (ranged GETs, item 12) decodes to the same values as the whole file.
    sub = rust.get("subset")
    if sub:
        bad = []
        for var, info in sub["vars"].items():
            full = rust["vars"].get(var, {}).get("values_sha256")
            if full is None and info.get("missing"):
                continue
            if info.get("values_sha256") != full:
                bad.append(var)
        res.add(g, "fetch_hour subset == whole file (item 12)", "PASS" if not bad else "FAIL",
                f"{sub.get('messages')} messages in the ranged subset" + (f"; differ: {bad}" if bad else ""))
    else:
        res.add(g, "fetch_hour subset (item 12)", "SKIP", "no subset file (local run or fetch failed)")


# ------------------------------------------------------------------ NAQFC (item 9, informational)

def compare_naqfc(res: Results, path: Path, rust: dict, rust_dir: Path) -> None:
    import pygrib

    import_backend()
    from backend.ingest_airquality import activities as aq

    g = "naqfc"
    grbs = pygrib.open(str(path))
    grb = grbs.message(1)
    grid_type = msg_key(grb, "gridType")
    pgrid = aq._grid_from_message(grb)
    grbs.close()
    res.add(g, "grid (item 9)", "INFO",
            f"pygrib gridType {grid_type}; rust first message template 3.{(rust.get('first_info') or {}).get('grid_template')}"
            f", data template 5.{(rust.get('first_info') or {}).get('data_template')}")
    if "error" in rust:
        res.add(g, "rust decode (item 9)", "WARN", rust["error"][:160])
        return
    cmp = compare_f32(canonical_f32(pgrid.data), read_f32(rust_dir / "naqfc_values.f32le"))
    res.add(g, "values (item 9)", "PASS" if cmp["equal"] else "WARN", cmp["detail"])
    if rust.get("geo") == "projected" and pgrid.lats.ndim == 2:
        dlat = max_abs(pgrid.lats, read_f64(rust_dir / "naqfc_lats.f64le"))
        dlon = max_abs(pgrid.lons, read_f64(rust_dir / "naqfc_lons.f64le"))
        res.add(g, "lat/lon mesh (item 9)", "PASS" if max(dlat, dlon) <= AXIS_TOL else "WARN",
                f"max |dlat| {dlat:.3g}, |dlon| {dlon:.3g}")
        if pgrid.source_x is not None:
            dx = max_abs(pgrid.source_x, read_f64(rust_dir / "naqfc_x.f64le"))
            dy = max_abs(pgrid.source_y, read_f64(rust_dir / "naqfc_y.f64le"))
            res.add(g, "projected axes vs _extract_native_projection (item 9)",
                    "PASS" if max(dx, dy) <= 1e-3 else "WARN", f"max |dx| {dx:.3g} m, |dy| {dy:.3g} m")
    else:
        res.add(g, "geometry (item 9)", "WARN",
                f"rust geo {rust.get('geo')}, python lats ndim {pgrid.lats.ndim}: compare by hand")


# ------------------------------------------------------------------ fetch report (items 1, 10)

def report_fetch(res: Results, fetch: dict) -> None:
    probes = fetch.get("missing_object_status")
    if probes:
        for bucket, p in probes.items():
            status = p.get("head_status")
            if status == 404:
                meaning = "404: the pending and .idx-fallback paths work as written"
            elif status == 403:
                meaning = ("403: is_not_found() is false, so pending hours error out and the .idx "
                           "fallback never runs; decide the 403 mapping (radar-grib README item 1)")
            else:
                meaning = f"unexpected: {p.get('head_error') or status}"
            res.add("s3", f"missing key in {bucket} (item 1)", "INFO", f"HEAD {status}: {meaning}",
                    head_status=status, url=p.get("url"))
        res.notes["missing_object_status"] = {b: p.get("head_status") for b, p in probes.items()}
    else:
        res.add("s3", "missing-key status (item 1)", "SKIP", "no network probe (cached or local run)")
    fh = (fetch.get("hrrr") or {}).get("fetch_hour")
    if fh is not None and not fh.get("ok"):
        res.add("s3", "hrrr fetch_hour ranged GETs (item 12)", "FAIL", str(fh.get("error"))[:200])
    nq = fetch.get("naqfc")
    if nq:
        if nq.get("error"):
            res.add("naqfc", "latest run (item 10)", "WARN", nq["error"][:200])
        for h in nq.get("heads", []):
            st = h.get("status")
            res.add("naqfc", "HEAD " + ("AWS mirror" if "amazonaws" in h["url"] else "NOMADS") + " (item 10)",
                    "PASS" if st == 200 else "WARN",
                    f"{st or h.get('error')}" + (f" -> {h['location']}" if h.get("location") else "")
                    + f" {h['url']}")


# ------------------------------------------------------------------ commands

def print_table(res: Results) -> None:
    width = max((len(f"{r['group']}: {r['check']}") for r in res.rows), default=20)
    for r in res.rows:
        label = f"{r['group']}: {r['check']}"
        print(f"  {r['status']:<4}  {label:<{width}}  {r['detail']}")
    counts = {s: sum(1 for r in res.rows if r["status"] == s) for s in ("PASS", "FAIL", "WARN", "SKIP", "INFO")}
    print("  " + ", ".join(f"{k} {v}" for k, v in counts.items()))
    probes = res.notes.get("missing_object_status")
    if probes:
        print("\n  *** Missing-object status (radar-grib README item 1, decides the 404/403 mapping):")
        for bucket, st in probes.items():
            print(f"  ***   {bucket}: HEAD {st}")


def cmd_compare(args) -> int:
    res = Results()
    rust_dir = args.rust_dir
    rust = json.loads((rust_dir / "rust.json").read_text())
    work = args.work
    work.mkdir(parents=True, exist_ok=True)
    if args.fetch_json and args.fetch_json.exists():
        report_fetch(res, json.loads(args.fetch_json.read_text()))
    if args.mrms:
        if "mrms" not in rust:
            res.add("mrms", "rust dump", "FAIL", "rust.json has no mrms section")
        else:
            compare_mrms(res, args.mrms, rust["mrms"], rust_dir, work)
    if args.hrrr:
        if "hrrr" not in rust:
            res.add("hrrr", "rust dump", "FAIL", "rust.json has no hrrr section")
        else:
            compare_hrrr(res, args.hrrr, args.hrrr_idx, rust["hrrr"], rust_dir, args.allow_missing_vars)
    if args.naqfc:
        try:
            compare_naqfc(res, args.naqfc, rust.get("naqfc", {}), rust_dir)
        except Exception as exc:  # noqa: BLE001 (informational section)
            res.add("naqfc", "compare (item 9)", "WARN", f"{type(exc).__name__}: {exc}")
    print_table(res)
    if args.json:
        write_json(args.json, {"rows": res.rows, "notes": res.notes, "failed": res.failed})
    return 1 if res.failed else 0


def cmd_render(args) -> int:
    """Production indexed renderer, same options as `radar render-once` (zooms 4-7, overview max)."""
    import_backend()
    from backend.shared import tiler

    args.work.mkdir(parents=True, exist_ok=True)
    data, lat, lon, _, _ = python_mrms(args.mrms, args.work)
    palettes = ("classic", "muted", "vivid")
    tables = {p: json.loads((RADAR_NG / "backend/shared/palettes" / f"{p}.json").read_text())["reflectivity"]
              for p in palettes}
    layer_root = args.out / "radar"
    shutil.rmtree(layer_root, ignore_errors=True)
    layer_root.mkdir(parents=True)
    out_dirs = {p: str(layer_root / p / "bench") for p in palettes}
    result = tiler.render_frame_palettes(
        data, lat, lon, tables, out_dirs, [4, 5, 6, 7],
        nodata_value=None, min_valid_weight=1.0, renderer="indexed", overview="max",
        source_id="bench:radar", publication_lock_root=str(layer_root),
    )
    n = sum(1 for _ in layer_root.rglob("*.png"))
    print(f"python indexed render: {n} tiles, palettes {sorted(result.rendered_palettes)}")
    return 0


# wgrib2 abbreviations and level strings for the codes the local fixtures use.
_ABBREV = {(0, 16, 196): "REFC", (0, 0, 0): "TMP", (0, 0, 6): "DPT", (0, 7, 6): "CAPE",
           (0, 2, 2): "UGRD", (0, 2, 3): "VGRD", (0, 1, 192): "CRAIN", (0, 1, 193): "CSNOW",
           (0, 1, 194): "CFRZR", (0, 1, 195): "CICEP", (0, 1, 1): "RH", (0, 1, 8): "APCP",
           (0, 6, 1): "TCDC"}


def cmd_make_idx(args) -> int:
    """A wgrib2-style index for a local multi-message GRIB2 (test fixtures only)."""
    import pygrib

    raw = args.grib.read_bytes()
    offsets = []
    at = 0
    while True:
        pos = raw.find(b"GRIB", at)
        if pos < 0 or pos + 16 > len(raw):
            break
        offsets.append(pos)
        at = pos + int.from_bytes(raw[pos + 8:pos + 16], "big")
    grbs = pygrib.open(str(args.grib))
    lines = []
    for i, grb in enumerate(grbs, start=1):
        code = (grb["discipline"], grb["parameterCategory"], grb["parameterNumber"])
        abbrev = _ABBREV.get(code, grb.shortName.upper())
        level = {"atmosphere": "entire atmosphere", "surface": "surface"}.get(
            grb.typeOfLevel, f"{grb.level} m above ground")
        date = f"{grb.dataDate:08d}{grb.dataTime // 100:02d}"
        fcst = "0-1 hour acc fcst" if abbrev == "APCP" else "1 hour fcst"
        lines.append(f"{i}:{offsets[i - 1]}:d={date}:{abbrev}:{level}:{fcst}:")
    grbs.close()
    args.out.write_text("\n".join(lines) + "\n")
    print(f"wrote {args.out} ({len(lines)} records)")
    return 0


def main() -> int:
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    sub = ap.add_subparsers(dest="cmd", required=True)
    c = sub.add_parser("compare")
    c.add_argument("--rust-dir", type=Path, required=True, help="noaa_dump dump --out directory")
    c.add_argument("--mrms", type=Path)
    c.add_argument("--hrrr", type=Path)
    c.add_argument("--hrrr-idx", type=Path)
    c.add_argument("--naqfc", type=Path)
    c.add_argument("--fetch-json", type=Path, help="noaa_dump fetch report (status probes)")
    c.add_argument("--allow-missing-vars", action="store_true",
                   help="a variable absent on both sides is SKIP, not FAIL (local fixtures)")
    c.add_argument("--work", type=Path, default=Path(tempfile.gettempdir()) / "verify-noaa")
    c.add_argument("--json", type=Path)
    r = sub.add_parser("render")
    r.add_argument("--mrms", type=Path, required=True)
    r.add_argument("--out", type=Path, required=True, help="tile root; writes <out>/radar/<palette>/bench")
    r.add_argument("--work", type=Path, default=Path(tempfile.gettempdir()) / "verify-noaa")
    m = sub.add_parser("make-idx")
    m.add_argument("--grib", type=Path, required=True)
    m.add_argument("--out", type=Path, required=True)
    args = ap.parse_args()
    return {"compare": cmd_compare, "render": cmd_render, "make-idx": cmd_make_idx}[args.cmd](args)


if __name__ == "__main__":
    sys.exit(main())
