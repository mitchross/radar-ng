#!/usr/bin/env python3
"""Local stand-in for the Level II routes and the MRMS manifest, for k6 dry runs only.

  python3 load/dev/level2-stub.py 38611            # fresh frames: thresholds pass
  STALE_S=900 python3 load/dev/level2-stub.py 38612  # 15-minute-old frames: the age rule must fail

Views turn ready 3 s after the first submit. Centres near KIWX get an out_of_range fallback.
It checks the script, not the service.
"""
import json, os, sys, time, base64
from datetime import datetime, timezone, timedelta
from http.server import ThreadingHTTPServer, BaseHTTPRequestHandler
STALE_S = int(os.environ.get("STALE_S", "120"))
PNG = base64.b64decode("iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR42mNkYPhfDwAChwGA60e6kgAAAABJRU5ErkJggg==")
VIEWS = {}
def iso(dt): return dt.strftime("%Y-%m-%dT%H:%M:%SZ")
class H(BaseHTTPRequestHandler):
    def log_message(self, *a): pass
    def send(self, code, obj=None, ctype="application/json", raw=None):
        body = raw if raw is not None else json.dumps(obj).encode()
        self.send_response(code); self.send_header("Content-Type", ctype); self.send_header("Content-Length", str(len(body))); self.end_headers(); self.wfile.write(body)
    def do_GET(self):
        p = self.path.split("?")[0]
        now = datetime.now(timezone.utc)
        if p == "/api/level2/sites":
            return self.send(200, {"schema_version": 1, "sites": [
                {"id": "KGRR", "name": "GRAND RAPIDS", "lat": 42.8939, "lon": -85.5449, "enabled": True, "health": {"state": "healthy"}},
                {"id": "KDTX", "name": "DETROIT", "lat": 42.6999, "lon": -83.4718, "enabled": True, "health": {"state": "healthy"}},
                {"id": "KIWX", "name": "N WEBSTER", "lat": 41.3586, "lon": -85.7000, "enabled": False, "health": None}]})
        if p.startswith("/api/level2/views/") and p.endswith("/manifest.json"):
            vid = p.split("/")[4]; v = VIEWS.get(vid)
            if not v: return self.send(404, {"detail": "Not Found"})
            frames = []
            for i in range(6):
                end = now - timedelta(seconds=STALE_S + (5 - i) * 270)
                frames.append({"timestamp": iso(end), "scan_end": iso(end), "path": f"s{i}/r0", "station_id": v["station"], "elevation_deg": 0.5})
            return self.send(200, {"schema_version": 1, "view_id": vid, "revision": "0123456789abcdef",
                "station": {"id": v["station"], "name": v["station"], "lat": 42.9, "lon": -85.5}, "source_zoom": v["zoom"],
                "palettes": [v["palette"]], "required_tiles": [{"z": v["zoom"], "x": t[0], "y": t[1]} for t in v["tiles"]],
                "tile_url_template": "/level2/tiles/{station}/{path}/{palette}/{z}/{x}/{y}.png", "expires_at": iso(now + timedelta(seconds=60)),
                "manifest": {"schema_version": 2, "layers": {f"level2-ref-{v['station']}": {"frames": frames}}}})
        if p.startswith("/api/level2/views/"):
            vid = p.split("/")[4]; v = VIEWS.get(vid)
            if not v: return self.send(404, {"detail": "Not Found"})
            return self.send(200, self.status(vid, v))
        if p.startswith("/level2/tiles/"): return self.send(200, raw=PNG, ctype="image/png")
        if p == "/api/manifest.json":
            t = iso(now - timedelta(seconds=90))
            return self.send(200, {"layers": {"radar": {"latest": t, "frames": [{"timestamp": t, "path": t}]}}})
        if p.startswith("/tiles/radar/"): return self.send(404, {"detail": "Not Found"})
        self.send(404, {"detail": "Not Found"})
    def status(self, vid, v):
        ready = time.time() - v["at"] > 3
        base = {"schema_version": 1, "view_id": vid, "station_id": v["station"], "poll_after_s": 2 if not ready else 15}
        if ready: base.update(state="ready", manifest_url=f"/api/level2/views/{vid}/0123456789abcdef/manifest.json")
        else: base.update(state="pending", reason="warming")
        return base
    def do_POST(self):
        body = json.loads(self.rfile.read(int(self.headers.get("Content-Length", 0))))
        lat, lon = body["centre"]["lat"], body["centre"]["lon"]
        station = "KGRR" if lon < -84.5 else "KDTX"
        if abs(lat - 41.36) < 0.6 and abs(lon + 85.7) < 0.7 and lat < 41.9:
            return self.send(200, {"schema_version": 1, "view_id": None, "state": "fallback", "reason": "out_of_range", "poll_after_s": 15})
        vid = f"{abs(hash(json.dumps(body, sort_keys=True))):032x}"[:32]
        VIEWS.setdefault(vid, {"at": time.time(), "station": station, "zoom": body["zoom"], "tiles": body["tiles"], "palette": body["palette"]})
        s = self.status(vid, VIEWS[vid])
        self.send(202 if s["state"] == "pending" else 200, s)
ThreadingHTTPServer(("127.0.0.1", int(sys.argv[1])), H).serve_forever()
