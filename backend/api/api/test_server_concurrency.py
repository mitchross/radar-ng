"""Source contracts for the endpoint concurrency rule.

Filesystem-bound endpoints must be plain `def` (threadpool) so a hung NFS
mount parks worker threads instead of the event loop; the k8s probe target
must stay `async def` so it keeps answering during exactly that failure.
Text-based on purpose — importing server.py would drag in fastapi/httpx,
and this contract is about the source, not the runtime.
"""

import re
from pathlib import Path

SOURCE = (Path(__file__).parent / "server.py").read_text()

# Endpoints that read the NFS-backed PVCs (tiles/grids/state) or style dir.
FS_BOUND = [
    "get_manifest",
    "inspect_point",
    "wind_field",
    "lightning",
    "storms",
    "tropical",
    "health",
    "get_basemap_style",
    "metrics",
]

# Event-loop endpoints: pure httpx, or the probe that must never block.
ASYNC_ONLY = ["get_forecast", "livez"]


def _def_kind(name: str) -> str:
    match = re.search(rf"^(async def|def) {name}\(", SOURCE, re.MULTILINE)
    assert match, f"endpoint {name} not found in server.py"
    return match.group(1)


def test_fs_bound_endpoints_run_in_threadpool():
    for name in FS_BOUND:
        assert _def_kind(name) == "def", (
            f"{name} touches the filesystem and must be plain `def` "
            "(threadpool) — `async def` blocks the event loop on NFS stalls"
        )


def test_probe_and_proxy_stay_async():
    for name in ASYNC_ONLY:
        assert _def_kind(name) == "async def", (
            f"{name} must stay `async def` on the event loop"
        )


def test_livez_never_touches_the_filesystem():
    body = SOURCE.split("async def livez(")[1].split("\n@app.")[0]
    for needle in ("Path(", "open(", "read_text", "read_bytes", "iterdir", "glob("):
        assert needle not in body, f"livez must not do filesystem I/O ({needle})"
