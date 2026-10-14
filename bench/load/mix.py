#!/usr/bin/env python3
"""Build the request mix of the k6 script (load/k6-radar.js in the original repository) as an oha URL list.

k6 runs whole user sessions (a widget refresh, or an app session with playback). oha sends
independent requests, so the session structure is flattened into per-request weights:

  widget   (20 % of sessions): manifest + 1 tile at z7 (k6: tiles(..., span 0))
  app      (80 %):             manifest, forecast, alerts, nowcast, geocode (10 % of sessions),
                               playback of the last 6 frames as 3 x 3 tiles at z6

The result is written as one URL per line, repeated in proportion to its expected count per
100 sessions, so the file reproduces the mix when oha samples from it. Tile URLs dominate
(about 90 % of requests), as in the k6 script.

    python3 mix.py --base http://127.0.0.1:8080 --manifest served/api/manifest.json --out urls.txt

With --tiles-root, tile URLs whose file is not in the served tree are dropped and counted.

By default forecast, alerts and geocode are left out: they call upstream services (Open-Meteo,
NWS, Photon) that the bench sandbox cannot reach, so they would measure DNS failures. Pass
--with-upstream to include them.

The manifest is the /api/manifest.json body (layers.radar.frames[].path). Use the same manifest
for the Python and the Rust run so both serve the same paths.
"""

from __future__ import annotations

import argparse
import json
import math
from pathlib import Path
from urllib.parse import quote

# Same places as the k6 script.
PLACES = [
    (42.96, -85.67), (41.88, -87.63), (40.71, -74.01), (34.05, -118.24), (29.76, -95.37),
    (33.45, -112.07), (39.74, -104.99), (47.61, -122.33), (25.76, -80.19), (36.16, -86.78),
    (44.98, -93.27), (32.78, -96.8), (38.63, -90.2), (35.96, -83.92), (42.36, -71.06),
]
CITIES = ["Chicago", "Denver", "Knoxville", "Seattle", "Miami", "Boston"]


def tile_for(lat: float, lon: float, z: int) -> tuple[int, int]:
    n = 2**z
    x = math.floor((lon + 180) / 360 * n)
    r = math.radians(lat)
    y = math.floor((1 - math.log(math.tan(r) + 1 / math.cos(r)) / math.pi) / 2 * n)
    return x, y


def encode_path(path: str) -> str:
    return "/".join(quote(seg, safe="") for seg in path.split("/"))


def tile_urls(base: str, path: str, lat: float, lon: float, z: int, span: int) -> list[str]:
    cx, cy = tile_for(lat, lon, z)
    out = []
    for dx in range(-span, span + 1):
        for dy in range(-span, span + 1):
            out.append(f"{base}/tiles/radar/classic/{encode_path(path)}/{z}/{cx + dx}/{cy + dy}.png")
    return out


def main() -> None:
    ap = argparse.ArgumentParser()
    ap.add_argument("--base", required=True)
    ap.add_argument("--manifest", type=Path, required=True)
    ap.add_argument("--out", type=Path, required=True)
    ap.add_argument("--sessions", type=int, default=100, help="sessions per weight unit")
    ap.add_argument(
        "--tiles-root", type=Path, default=None,
        help="served TILE_DIR; tile URLs with no file here are dropped (empty tiles would 404 "
             "on both servers and dilute the comparison)",
    )
    ap.add_argument(
        "--with-upstream", action="store_true",
        help="also request forecast, alerts and geocode (they call Open-Meteo, NWS and Photon; "
             "not reachable from the bench sandbox, so the default leaves them out)",
    )
    args = ap.parse_args()
    base = args.base.rstrip("/")
    man = json.loads(args.manifest.read_text())
    frames = (man.get("layers", {}).get("radar", {}) or {}).get("frames", [])
    if not frames:
        raise SystemExit("manifest has no layers.radar.frames; seed the served tree first")
    latest = frames[-1]["path"]
    last6 = [f["path"] for f in frames[-6:]]

    # Per 100 sessions: 20 widget, 80 app. Weights are request counts per 100 sessions.
    weights: dict[str, int] = {}

    def add(url: str, n: float) -> None:
        weights[url] = weights.get(url, 0) + round(n)

    widget, app = 20, 80
    add(f"{base}/api/manifest.json", widget + app)
    for i in range(widget):
        lat, lon = PLACES[i % len(PLACES)]
        for u in tile_urls(base, latest, lat, lon, 7, 0):
            add(u, 1)
    for i in range(app):
        lat, lon = PLACES[i % len(PLACES)]
        if args.with_upstream:
            add(f"{base}/api/forecast/{lat}/{lon}", 1)
            add(f"{base}/api/alerts?lat={lat}&lon={lon}", 1)
        add(f"{base}/api/nowcast/{lat}/{lon}", 1)
        if args.with_upstream and i % 10 == 0:
            add(f"{base}/api/geocode?q={CITIES[i % len(CITIES)]}", 1)
        for p in last6:
            for u in tile_urls(base, p, lat, lon, 6, 1):
                add(u, 1)

    dropped_missing = 0
    if args.tiles_root is not None:
        from urllib.parse import unquote
        for url in list(weights):
            if "/tiles/" not in url:
                continue
            rel = unquote(url.split("/tiles/", 1)[1].split("?", 1)[0])
            if not (args.tiles_root / rel).is_file():
                dropped_missing += weights.pop(url)

    lines: list[str] = []
    for url, n in weights.items():
        lines.extend([url] * max(1, n))
    args.out.write_text("\n".join(lines) + "\n")
    total = sum(weights.values())
    tiles = sum(n for u, n in weights.items() if "/tiles/" in u)
    summary = {
        "sessions_per_unit": args.sessions,
        "requests_per_100_sessions": total,
        "tile_share": tiles / total,
        "distinct_urls": len(weights),
        "list_lines": len(lines),
        "manifest_frames": len(frames),
        "with_upstream": args.with_upstream,
        "tile_requests_dropped_missing": dropped_missing,
    }
    args.out.with_suffix(".summary.json").write_text(json.dumps(summary, indent=2) + "\n")
    print(json.dumps(summary))


if __name__ == "__main__":
    main()
