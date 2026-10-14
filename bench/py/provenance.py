#!/usr/bin/env python3
"""Record what was measured: git state and source hashes of the Rust crates, the bench
binaries, the Python package versions and the production files used.

Written to work/raw/provenance.json at the start of every run, so a result can be traced to
the exact source it was built from (other work may change the crates while a run is planned).

    provenance.py <rust_root> <target_release_dir> <out.json>
"""

from __future__ import annotations

import hashlib
import importlib.metadata as md
import json
import subprocess
import sys
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
from common import RADAR_NG, write_json  # noqa: E402

CRATES = ["radar-geo", "radar-tiler", "radar-grib", "radar-storms", "radar-nowcast", "radar-bench"]
PY_PACKAGES = ["numpy", "scipy", "opencv-python-headless", "pysteps", "Pillow", "pygrib",
               "eccodes", "temporalio", "httpx"]


def sha_tree(root: Path) -> str:
    h = hashlib.sha256()
    for p in sorted(root.rglob("*")):
        if p.is_file() and "target" not in p.parts:
            h.update(str(p.relative_to(root)).encode())
            h.update(p.read_bytes())
    return h.hexdigest()


def git(rust: Path, *args: str) -> str:
    try:
        return subprocess.run(["git", "-C", str(rust), *args], capture_output=True, text=True,
                              timeout=30).stdout.strip()
    except (OSError, subprocess.TimeoutExpired):
        return "unavailable"


def main() -> None:
    rust, release, out = Path(sys.argv[1]), Path(sys.argv[2]), Path(sys.argv[3])
    crate_hashes = {c: sha_tree(rust / "crates" / c) for c in CRATES if (rust / "crates" / c).exists()}
    binaries = {}
    for b in ("bench_tile", "bench_grib", "bench_storms", "bench_nowcast"):
        p = release / b
        if p.exists():
            binaries[b] = hashlib.sha256(p.read_bytes()).hexdigest()
    py_versions = {}
    for pkg in PY_PACKAGES:
        try:
            py_versions[pkg] = md.version(pkg)
        except md.PackageNotFoundError:
            py_versions[pkg] = None
    write_json(out, {
        "git_head": git(rust, "rev-parse", "HEAD"),
        "git_status_relevant": git(rust, "status", "--short", "--", *[f"crates/{c}" for c in CRATES]).splitlines(),
        "crate_source_sha256": crate_hashes,
        "bench_binary_sha256": binaries,
        "python": sys.version.split()[0],
        "python_packages": py_versions,
        "radar_ng_palettes_sha256": {
            p: hashlib.sha256((RADAR_NG / "backend/shared/palettes" / f"{p}.json").read_bytes()).hexdigest()
            for p in ("classic", "muted", "vivid") if (RADAR_NG / "backend/shared/palettes" / f"{p}.json").exists()
        },
    })


if __name__ == "__main__":
    main()
