"""ASGI entry point for the Python side of the API load test.

The real FastAPI app (backend/api/api/server.py) plus a StaticFiles mount at /tiles for the
pyramids. In production Caddy serves /tiles, so this mount stands in for Caddy here (Caddy is
not installable in this sandbox). Tile responses therefore carry no Cache-Control or gzip
precompression, which makes the Python tile numbers a lower bound for the Caddy setup.

    TILE_DIR=/path/to/served RADAR_NG=. \
      uvicorn --app-dir bench/load py_server:app --port 8081 --workers 1
"""

import os
import sys
from pathlib import Path

RADAR_NG = Path(os.environ.get("RADAR_NG", "."))
sys.path.insert(0, str(RADAR_NG))

from fastapi.staticfiles import StaticFiles  # noqa: E402

from backend.api.api import server  # noqa: E402  (production app, read-only tree)

app = server.app
tile_dir = os.environ.get("TILE_DIR", "/data/tiles")
app.mount("/tiles", StaticFiles(directory=tile_dir), name="tiles")
