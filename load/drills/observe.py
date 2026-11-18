#!/usr/bin/env python3
"""Timeline recorder for the Level II drills and the k6 run. Stdlib only.

Every INTERVAL seconds it appends one CSV row: what a client sees (sites, a real view submit,
MRMS manifest age) and what Prometheus says (MRMS age, Level II sweep age, pool occupancy,
fallbacks). Start it before the drill, stop it (Ctrl-C or --duration) after recovery, and attach
the CSV to the drill record.

  python3 load/drills/observe.py --base https://<shadow-host> --mrms https://<shadow-host> \
      --prom http://kube-prometheus-stack-prometheus.prometheus-stack.svc.cluster.local:9090 \
      --out drill-noaa-delay.csv --duration 1800

Sites 404 is recorded as `off`: Level II disabled at that host, which is the right answer on
the public hosts.
"""

import argparse
import csv
import json
import math
import sys
import time
import urllib.parse
import urllib.request
from datetime import datetime, timezone

PROM_QUERIES = {
    "prom_mrms_age_s": "max(radar_ng_mrms_age_seconds)",
    "prom_l2_sweep_age_max_s": "max(radar_level2_sweep_age_seconds)",
    "prom_l2_feed_age_max_s": "max(radar_level2_feed_age_seconds)",
    "prom_l2_active_stations": "max(radar_level2_active_stations)",
    "prom_l2_decodes_running": "max(radar_level2_decodes_running)",
    "prom_l2_fallback_per_min": "sum(rate(radar_level2_fallback_total[5m])) * 60",
    "prom_l2_pool_pods_up": 'count(up{job=~".*level2.*"} == 1)',
}


def get(url, data=None, timeout=10):
    # A named agent: the edge blocks the default Python user agent.
    req = urllib.request.Request(url, data=data, method="POST" if data else "GET", headers={"User-Agent": "radar-ng-level2-drill/1"})
    if data:
        req.add_header("Content-Type", "application/json")
    started = time.monotonic()
    try:
        with urllib.request.urlopen(req, timeout=timeout) as r:
            body = r.read()
            return r.status, body, (time.monotonic() - started) * 1000
    except urllib.error.HTTPError as e:
        return e.code, e.read(), (time.monotonic() - started) * 1000
    except Exception:  # noqa: BLE001 - any transport failure is a row, not a crash
        return 0, b"", (time.monotonic() - started) * 1000


def age_s(iso, now):
    if not iso:
        return ""
    return round(now - datetime.fromisoformat(iso.replace("Z", "+00:00")).timestamp(), 1)


def tile(lat, lon, z):
    n = 2**z
    r = math.radians(lat)
    return int((lon + 180) / 360 * n), int((1 - math.log(math.tan(r) + 1 / math.cos(r)) / math.pi) / 2 * n)


def view_body(site):
    z = 9
    x, y = tile(site["lat"], site["lon"], z)
    tiles = [[x + dx, y + dy] for dx in (-1, 0, 1) for dy in (-1, 0, 1)]
    return json.dumps(
        {"centre": {"lat": site["lat"], "lon": site["lon"]}, "zoom": z, "tiles": tiles, "palette": "classic"}
    ).encode()


def prom(base, query):
    if not base:
        return ""
    url = f"{base.rstrip('/')}/api/v1/query?{urllib.parse.urlencode({'query': query})}"
    status, body, _ = get(url)
    if status != 200:
        return "err"
    res = json.loads(body).get("data", {}).get("result", [])
    return round(float(res[0]["value"][1]), 2) if res else ""


def row(args):
    now = time.time()
    out = {"utc": datetime.now(timezone.utc).strftime("%Y-%m-%dT%H:%M:%SZ")}
    status, body, ms = get(f"{args.base}/api/level2/sites")
    out["sites_status"] = "off" if status == 404 else status
    out["sites_ms"] = round(ms)
    enabled = []
    if status == 200:
        enabled = [s for s in json.loads(body).get("sites", []) if s.get("enabled")]
    states = []
    for s in enabled:
        h = s.get("health") or {}
        states.append(f"{s['id']}:{h.get('state')}:{age_s(h.get('low_sweep_end'), now)}")
    out["station_state_age"] = " ".join(states)
    if enabled:
        site = enabled[0] if not args.station else next((s for s in enabled if s["id"] == args.station), enabled[0])
        st, vb, vms = get(f"{args.base}/api/level2/views", data=view_body(site))
        v = json.loads(vb) if vb[:1] == b"{" else {}
        out["view_station"] = site["id"]
        out["view_status"] = st
        out["view_state"] = v.get("state", "")
        out["view_reason"] = v.get("reason", "")
        out["view_ms"] = round(vms)
    status, body, _ = get(f"{args.mrms}/api/manifest.json")
    latest = ""
    if status == 200:
        latest = (json.loads(body).get("layers", {}).get("radar", {}) or {}).get("latest", "")
    out["mrms_manifest_status"] = status
    out["mrms_age_s"] = age_s(latest, now)
    for name, q in PROM_QUERIES.items():
        out[name] = prom(args.prom, q)
    return out


FIELDS = [
    "utc", "sites_status", "sites_ms", "station_state_age", "view_station", "view_status", "view_state",
    "view_reason", "view_ms", "mrms_manifest_status", "mrms_age_s", *PROM_QUERIES,
]


def main():
    p = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    p.add_argument("--base", required=True, help="Level II host: shadow host or level2-api Service")
    p.add_argument("--mrms", help="MRMS host (tile-server); defaults to --base")
    p.add_argument("--prom", default="", help="Prometheus base URL; empty skips the server columns")
    p.add_argument("--station", default="", help="station to submit views for; default first enabled")
    p.add_argument("--interval", type=float, default=15, help="seconds between rows (client refresh is 15)")
    p.add_argument("--duration", type=float, default=0, help="stop after this many seconds; 0 runs until Ctrl-C")
    p.add_argument("--out", default="-", help="CSV path, - for stdout")
    args = p.parse_args()
    args.base = args.base.rstrip("/")
    args.mrms = (args.mrms or args.base).rstrip("/")
    fh = sys.stdout if args.out == "-" else open(args.out, "a", newline="")
    w = csv.DictWriter(fh, fieldnames=FIELDS)
    if fh is sys.stdout or fh.tell() == 0:
        w.writeheader()
    end = time.time() + args.duration if args.duration else math.inf
    try:
        while time.time() < end:
            started = time.time()
            w.writerow(row(args))
            fh.flush()
            time.sleep(max(0, args.interval - (time.time() - started)))
    except KeyboardInterrupt:
        pass


if __name__ == "__main__":
    main()
