"""Golden fixture generator for radar-alerts.

Runs the ORIGINAL radar-ng Python functions (lightning activities, storm-watch activities and
push sender) with Temporal, websockets, httpx and the backend shared modules stubbed out, and
writes JSON fixtures next to this file. The Rust tests replay these fixtures.

Usage (from the crate directory):

    RADAR_NG_ROOT=. python3 testdata/gen_golden.py

Requires PyJWT and cryptography (pip install pyjwt cryptography). The test keys written under
testdata/keys/ are throwaway keys for the mock-server tests. They protect nothing.
"""

import asyncio
import importlib.util
import json
import logging
import math
import os
import random
import sys
import tempfile
import types
from pathlib import Path

import jwt
from cryptography.hazmat.primitives import serialization
from cryptography.hazmat.primitives.asymmetric import ec, rsa

SRC = Path(os.environ.get("RADAR_NG_ROOT", "."))
OUT = Path(__file__).resolve().parent
KEYS = OUT / "keys"
FIXED_NOW = 1_789_999_999.5  # time.time() used by the writer
SEED_NOW = 1_800_000_000.0   # now passed to _load_existing_strikes


# ---------- stubs for Temporal, websockets, httpx and backend.shared ----------

def _stub(name, **attrs):
    module = types.ModuleType(name)
    module.__dict__.update(attrs)
    sys.modules[name] = module
    return module


class ApplicationError(Exception):
    def __init__(self, message, *, non_retryable=False):
        super().__init__(message)
        self.non_retryable = non_retryable


def _defn(*args, **kwargs):
    if args and callable(args[0]) and not kwargs:
        return args[0]
    return lambda fn: fn


_activity = _stub("temporalio.activity", defn=_defn, heartbeat=lambda *a, **k: None)
_exceptions = _stub("temporalio.exceptions", ApplicationError=ApplicationError)
_temporalio = _stub("temporalio", activity=_activity, exceptions=_exceptions)
_stub("websockets")
_stub("httpx", AsyncClient=None)
_stub("backend")
_stub("backend.shared")
_stub("backend.shared.logger", get_logger=lambda name: logging.getLogger(name))
DELETED_TOKENS = []
_stub(
    "backend.shared.push_tokens",
    delete_by_token=lambda token: DELETED_TOKENS.append(token) or 1,
    list_for_user=lambda user_id: [],
    upsert=lambda *a: None,
)
_stub(
    "backend.shared.storm_watch",
    FrameDiff=None, FrameSample=None, detect_change=None,
    latest_radar_meta=None, sample_window=None,
)


def load(name, rel):
    spec = importlib.util.spec_from_file_location(name, SRC / rel)
    module = importlib.util.module_from_spec(spec)
    sys.modules[name] = module  # dataclasses look the module up while it executes
    spec.loader.exec_module(module)
    return module


lightning = load("lightning_activities", "backend/ingest_lightning/activities.py")
storm = load("storm_watch_activities", "backend/api/api/storm_watch_activities.py")
push = load("push_sender", "temporal/shared/push.py")


def write_json(name, obj):
    path = OUT / name
    path.write_text(json.dumps(obj, separators=(",", ":"), ensure_ascii=True) + "\n")
    print("wrote", path.relative_to(OUT.parent.parent), len(json.dumps(obj)))


# ---------- keys (throwaway, for mock-server tests) ----------

def make_keys():
    KEYS.mkdir(parents=True, exist_ok=True)
    ec_key = ec.generate_private_key(ec.SECP256R1())
    p8 = ec_key.private_bytes(
        serialization.Encoding.PEM, serialization.PrivateFormat.PKCS8, serialization.NoEncryption()
    )
    (KEYS / "apns-test.p8").write_bytes(p8)
    ec_pub = ec_key.public_key().public_bytes(
        serialization.Encoding.PEM, serialization.PublicFormat.SubjectPublicKeyInfo
    )
    (KEYS / "apns-test.pub.pem").write_bytes(ec_pub)

    rsa_key = rsa.generate_private_key(public_exponent=65537, key_size=2048)
    rsa_pem = rsa_key.private_bytes(
        serialization.Encoding.PEM, serialization.PrivateFormat.PKCS8, serialization.NoEncryption()
    ).decode()
    creds = {
        "type": "service_account",
        "project_id": "radar-test",
        "client_email": "radar-push@radar-test.iam.gserviceaccount.com",
        "private_key": rsa_pem,
        "token_uri": "https://oauth2.googleapis.com/token",
    }
    (KEYS / "fcm-test-service-account.json").write_text(json.dumps(creds, indent=2) + "\n")
    rsa_pub = rsa_key.public_key().public_bytes(
        serialization.Encoding.PEM, serialization.PublicFormat.SubjectPublicKeyInfo
    )
    (KEYS / "fcm-test.pub.pem").write_bytes(rsa_pub)


# ---------- 1. Blitzortung LZW-variant decode ----------

def gen_lzw(rng):
    cases = [
        {"raw": "", "out": ""},
        {"raw": "abc", "out": "abc"},
        {"raw": "aĀ", "out": "aaa"},
        {"raw": "abĀ", "out": "abab"},
        {"raw": "abĀā", "out": "ababba"},
        {"raw": '{"a":1}', "out": '{"a":1}'},
    ]
    lit = [chr(c) for c in range(32, 127)] + ["é", "ÿ", "\x00", " "]
    code = [chr(c) for c in range(256, 300)] + [chr(c) for c in (400, 511, 512, 1000, 65535)]
    for _ in range(600):
        n = rng.randint(0, 30)
        raw = "".join(rng.choice(lit if rng.random() < 0.6 else code) for _ in range(n))
        cases.append({"raw": raw, "out": lightning._decode_payload(raw)})
    # Frames that look like the real feed: JSON text with a few code points >= 256.
    for _ in range(200):
        obj = {"lat": round(rng.uniform(15, 55), 4), "lon": round(rng.uniform(-130, -50), 4),
               "time": rng.randint(10**18, 2 * 10**18), "pol": rng.choice([-1, 1]), "mds": rng.randint(0, 40)}
        text = json.dumps(obj, separators=(",", ":"))
        raw = "".join(ch if rng.random() < 0.9 else chr(256 + rng.randint(0, 40)) for ch in text)
        cases.append({"raw": raw, "out": lightning._decode_payload(raw)})
    write_json("lzw_decode.json", {"cases": cases})


# ---------- 2. Point in GeoJSON (ray casting, holes, multipolygons) ----------

def ring(rng, cx, cy, r, n):
    pts = []
    for i in range(n):
        ang = 2 * math.pi * i / n + rng.uniform(-0.2, 0.2)
        rad = r * rng.uniform(0.4, 1.2)
        pts.append([round(cx + rad * math.cos(ang), 4), round(cy + rad * math.sin(ang), 4)])
    pts.append(list(pts[0]))
    return pts


def gen_point_in_geojson(rng):
    cases = []
    square = [[-98, 35], [-96, 35], [-96, 37], [-98, 37], [-98, 35]]
    hole = [[-97.5, 35.5], [-96.5, 35.5], [-96.5, 36.5], [-97.5, 36.5], [-97.5, 35.5]]
    geoms = [
        {"type": "Polygon", "coordinates": [square]},
        {"type": "Polygon", "coordinates": [square, hole]},
        {"type": "MultiPolygon", "coordinates": [[square], [[[-80, 30], [-79, 30], [-79, 31], [-80, 31], [-80, 30]]]]},
        {"type": "Polygon", "coordinates": [[[0, 0], [1, 0], [1, 1]]]},          # short ring
        {"type": "Polygon", "coordinates": [[[0, 0], [1, 0], None, [1, 1], [0, 0]]]},  # null vertex
        {"type": "Polygon", "coordinates": [[["0", "0"], ["2", "0"], ["2", "2"], ["0", "2"], ["0", "0"]]]},  # string numbers
        {"type": "Polygon", "coordinates": []},
        {"type": "Polygon", "coordinates": None},
        {"type": "Point", "coordinates": [-97, 36]},
        {"type": "MultiPolygon", "coordinates": "nope"},
        {},
        None,
    ]
    for g in geoms:
        for _ in range(40):
            lng = round(rng.uniform(-100, -76), 3)
            lat = round(rng.uniform(28, 40), 3)
            if g is None:
                expect = False
            else:
                expect = bool(storm._point_in_geojson(lng, lat, g))
            cases.append({"geometry": g, "lng": lng, "lat": lat, "expect": expect})
    for _ in range(600):
        cx, cy = rng.uniform(-95, -85), rng.uniform(30, 40)
        if rng.random() < 0.5:
            g = {"type": "Polygon", "coordinates": [ring(rng, cx, cy, 3, rng.randint(4, 12))]}
        else:
            g = {"type": "MultiPolygon", "coordinates": [
                [ring(rng, cx, cy, 2, rng.randint(4, 9))],
                [ring(rng, cx + 4, cy - 2, 1.5, rng.randint(4, 9)), ring(rng, cx + 4, cy - 2, 0.4, 5)],
            ]}
        for _ in range(8):
            lng = round(cx + rng.uniform(-4, 6), 3)
            lat = round(cy + rng.uniform(-4, 4), 3)
            cases.append({"geometry": g, "lng": lng, "lat": lat,
                          "expect": bool(storm._point_in_geojson(lng, lat, g))})
    write_json("point_in_geojson.json", {"cases": cases})


# ---------- 3. EndpointPicker scripts ----------

def run_picker(primary, legacy, ops, rands_init=0.5):
    now = [0.0]
    rnd = [rands_init]
    picker = lightning.EndpointPicker(primary, legacy, clock=lambda: now[0], rand=lambda: rnd[0])
    results = []
    for op in ops:
        kind = op[0]
        if kind == "next":
            results.append(picker.next())
        elif kind == "failed":
            results.append(picker.failed(op[1]))
        elif kind == "succeeded":
            results.append(picker.succeeded(op[1]))
        elif kind == "advance":
            now[0] += op[1]
            results.append(None)
        elif kind == "rand":
            rnd[0] = op[1]
            results.append(None)
    return results


def gen_picker(rng):
    primary = list(lightning.PRIMARY_ENDPOINTS)
    legacy = list(lightning.LEGACY_ENDPOINTS)
    scenarios = []
    # Spec table: consecutive failures on one endpoint, rand = 0.5.
    ops = [("failed", primary[0]) for _ in range(9)]
    scenarios.append({"name": "spec_table", "primary": primary, "legacy": legacy,
                      "ops": ops, "expect": run_picker(primary, legacy, ops)})
    # Round robin and failover ordering.
    ops = [("next",), ("next",), ("next",), ("next",), ("failed", primary[0]), ("next",), ("next",),
           ("failed", primary[1]), ("failed", primary[2]), ("failed", primary[3]), ("next",), ("next",),
           ("advance", 61), ("next",), ("succeeded", primary[0]), ("next",)]
    scenarios.append({"name": "failover", "primary": primary, "legacy": legacy,
                      "ops": ops, "expect": run_picker(primary, legacy, ops)})
    # Override with a single primary and no legacy: never stalls.
    ops = [("failed", "wss://relay.local/"), ("next",), ("next",), ("advance", 5), ("next",)]
    scenarios.append({"name": "single_primary", "primary": ["wss://relay.local/"], "legacy": [],
                      "ops": ops, "expect": run_picker(["wss://relay.local/"], [], ops)})
    # Randomized scripts over the production lists.
    pool = primary + legacy[:6]
    for k in range(40):
        ops = []
        for _ in range(80):
            roll = rng.random()
            if roll < 0.35:
                ops.append(("next",))
            elif roll < 0.6:
                ops.append(("failed", rng.choice(pool)))
            elif roll < 0.7:
                ops.append(("succeeded", rng.choice(pool)))
            elif roll < 0.85:
                ops.append(("advance", rng.randint(0, 120)))
            else:
                ops.append(("rand", round(rng.random(), 6)))
        init = round(rng.random(), 6)
        ops = [("rand", init)] + ops
        scenarios.append({"name": f"random_{k}", "primary": primary, "legacy": legacy,
                          "ops": ops, "expect": run_picker(primary, legacy, ops, init)})
    write_json("picker.json", {"scenarios": scenarios})


# ---------- 4. Seeding from the previous lightning.json ----------

def gen_seed():
    def feat(t, lon=-97.0, lat=36.0, pol=1, mds=4, extra=None):
        props = {"time": t, "age_s": 0, "polarity": pol, "mds": mds}
        if extra:
            props.update(extra)
        return {"type": "Feature", "geometry": {"type": "Point", "coordinates": [lon, lat]}, "properties": props}

    cutoff = SEED_NOW - 15 * 60
    cases = [
        {"name": "missing_file", "file": None},
        {"name": "invalid_json", "file": "{not json"},
        {"name": "top_level_array", "file": "[1, 2, 3]"},
        {"name": "no_features", "file": json.dumps({"type": "FeatureCollection"})},
        {"name": "features_null", "file": json.dumps({"features": None})},
    ]
    body = {
        "type": "FeatureCollection",
        "features": [
            feat(SEED_NOW - 1200),                  # 20 min old: dropped
            feat(SEED_NOW - 60, lon=-98.25, lat=35.5, pol=-1, mds=12),
            feat(SEED_NOW - 30, lon=-90.0, lat=30.0),
            feat(cutoff),                           # exactly at cutoff: kept
            feat(cutoff - 0.001),                   # just outside: dropped
            {"type": "Feature", "geometry": {"coordinates": [1, 2]}},  # no properties
            {"type": "Feature", "properties": {"time": SEED_NOW - 10}},  # no geometry
            {"type": "Feature", "geometry": {"coordinates": [1]}, "properties": {"time": SEED_NOW - 10}},
            {"type": "Feature", "geometry": {"coordinates": [1, 2]}, "properties": {"time": None}},
            {"type": "Feature", "geometry": {"coordinates": [1, 2]}, "properties": {"time": SEED_NOW - 9, "polarity": None}},
            {"type": "Feature", "geometry": {"coordinates": [-96.0, 33.0]}, "properties": {"time": SEED_NOW - 5, "polarity": "2", "mds": 1.9}},
            {"type": "Feature", "geometry": {"coordinates": ["-95.5", "34.25"]}, "properties": {"time": str(SEED_NOW - 4)}},
            "not-an-object",
        ],
    }
    cases.append({"name": "mixed_features", "file": json.dumps(body)})
    body2 = {"features": [feat(SEED_NOW - 2), feat(SEED_NOW - 7), feat(SEED_NOW - 3)]}
    cases.append({"name": "unsorted_input", "file": json.dumps(body2)})

    tmp = Path(tempfile.mkdtemp())
    for case in cases:
        path = tmp / "lightning.json"
        if path.exists():
            path.unlink()
        if case["file"] is not None:
            path.write_text(case["file"])
        strikes = lightning._load_existing_strikes(path=path, retention_min=15, now=SEED_NOW)
        case["expect"] = [[s["t"], s["lat"], s["lon"], s["pol"], s["mds"]] for s in strikes]
    write_json("seed.json", {"now": SEED_NOW, "retention_min": 15, "cases": cases})


# ---------- 5. Writer format (byte-exact) ----------

def gen_writer():
    tmp = Path(tempfile.mkdtemp())
    lightning.STATE_DIR = tmp
    lightning.OUT_PATH = tmp / "lightning.json"
    strikes = [
        {"t": FIXED_NOW - 12.7, "lat": 36.123456, "lon": -97.5, "pol": 1, "mds": 4},
        {"t": 1789999999.123456789, "lat": 40.0, "lon": -75.0, "pol": -1, "mds": 0},
        {"t": FIXED_NOW + 0.5, "lat": 15.0, "lon": -130.0, "pol": 0, "mds": 1234},
        {"t": 1789999000.0, "lat": 55.0, "lon": -50.0, "pol": 1, "mds": 7},
    ]
    real_time = lightning.time.time
    lightning.time.time = lambda: FIXED_NOW
    try:
        lightning._write_geojson(strikes)
    finally:
        lightning.time.time = real_time
    text = lightning.OUT_PATH.read_text()
    leftovers = sorted(p.name for p in tmp.iterdir())
    write_json("writer.json", {
        "now": FIXED_NOW,
        "retention_min": 15,
        "strikes": strikes,
        "expect_text": text,
        "leftovers": leftovers,
    })


# ---------- 6. NWS alert feature parsing and seen-state commits ----------

def gen_alerts():
    feats = [
        {"properties": {"id": "urn:oid:a"}, "geometry": {"type": "Polygon", "coordinates": []}},
        {"id": "urn:oid:b", "properties": {}},
        {"properties": {"id": ""}, "id": "urn:oid:c"},
        {"properties": {"id": 0}, "id": None},
        {"properties": None, "id": "urn:oid:d", "geometry": None},
        {"properties": {"id": 12345}},
        {"properties": {"id": True}},
        {"properties": {"id": 1.5}},
        {"geometry": "string-geometry", "id": "urn:oid:e"},
        {"properties": {"id": "urn:oid:f"}, "geometry": {"type": "MultiPolygon", "coordinates": [[]]}},
        {"id": "urn:oid:g", "properties": {"id": "urn:oid:h"}},
    ]
    out = []
    for f in feats:
        alert = storm._alert_from_feature(f)
        out.append({"feature": f,
                    "alert_id": None if alert is None else alert.alert_id,
                    "geometry": None if alert is None else alert.geometry})
    write_json("alert_features.json", {"cases": out})

    tmp = Path(tempfile.mkdtemp())
    storm.STATE_DIR = tmp
    storm._ALERT_STATE_PATH = tmp / "alerts_seen.json"
    storm._ACTIVE_SNAPSHOT_PATH = tmp / "alerts_active_snapshot.json"
    rng = random.Random(7)

    def scenario(name, seen, snapshot, handled):
        if storm._ALERT_STATE_PATH.exists():
            storm._ALERT_STATE_PATH.unlink()
        if storm._ACTIVE_SNAPSHOT_PATH.exists():
            storm._ACTIVE_SNAPSHOT_PATH.unlink()
        if seen is not None:
            storm._ALERT_STATE_PATH.write_text(json.dumps(seen))
        if snapshot is not None:
            storm._ACTIVE_SNAPSHOT_PATH.write_text(json.dumps(snapshot))
        asyncio.run(storm.mark_alerts_seen(storm.MarkAlertsSeenInput(handled_ids=list(handled))))
        final = json.loads(storm._ALERT_STATE_PATH.read_text())
        return {"name": name, "seen": seen, "snapshot": snapshot, "handled": handled, "expect": final}

    cases = [
        scenario("spec_handled_and_expired", ["old-1"], ["old-1", "new-ok", "new-failed"], ["new-ok"]),
        scenario("expired_fills_cap", ["expired-1", "active-1"], ["active-1"], []),
        scenario("missing_snapshot_never_forgets", ["old-1"], None, ["new-ok"]),
        scenario("no_seen_file", None, ["a", "b"], ["b"]),
        scenario("corrupt_snapshot_is_missing", ["old-1"], "not-a-list", ["x"]),
        scenario("duplicate_handled", ["a"], ["a", "b"], ["b", "b", "c"]),
    ]
    seen_big = [f"old-{i:05d}" for i in range(4000)]
    active_big = [f"act-{i:04d}" for i in range(1500)] + [f"old-{i:05d}" for i in range(0, 4000, 3)]
    cases.append(scenario("cap_keeps_all_active", seen_big, active_big, ["new-1"]))
    seen_big2 = [f"old-{i:05d}" for i in range(4990)]
    cases.append(scenario("cap_trims_expired_only", seen_big2, ["old-00001"], ["x-1", "x-2"]))
    for k in range(30):
        seen = sorted({f"id-{rng.randint(0, 60)}" for _ in range(rng.randint(0, 40))})
        active = sorted({f"id-{rng.randint(0, 60)}" for _ in range(rng.randint(0, 40))})
        handled = [f"id-{rng.randint(0, 80)}" for _ in range(rng.randint(0, 10))]
        cases.append(scenario(f"random_{k}", seen, active, handled))
    write_json("seen_state.json", {"cases": cases})

    # Snapshot file format (json.dump default separators, sorted).
    storm._write_active_snapshot({"urn:b", "urn:a", "é-x"})
    write_json("snapshot_format.json", {"expect_text": storm._ACTIVE_SNAPSHOT_PATH.read_text()})


# ---------- 7. Push: APNS and FCM request shapes ----------

class FakeResp:
    def __init__(self, status, text="", payload=None):
        self.status_code = status
        self.text = text
        self._payload = payload

    def raise_for_status(self):
        if self.status_code >= 400:
            raise RuntimeError(f"HTTP {self.status_code}")

    def json(self):
        return self._payload


class FakeClient:
    calls = []
    responses = []

    def __init__(self, *args, **kwargs):
        self.kwargs = kwargs

    async def __aenter__(self):
        return self

    async def __aexit__(self, *exc):
        return False

    async def post(self, url, headers=None, json=None, data=None):
        FakeClient.calls.append({"url": url, "headers": headers, "json": json, "data": data})
        return FakeClient.responses.pop(0)


def gen_push():
    push.httpx.AsyncClient = FakeClient
    push.APNS_HOST = "https://api.push.apple.com"
    push.APNS_TOPIC = "me.example.radar"
    push.APNS_KEY_ID = "ABC123DEFG"
    push.APNS_TEAM_ID = "TEAM123456"
    push.APNS_KEY_PATH = str(KEYS / "apns-test.p8")
    push.FCM_PROJECT_ID = "radar-test"
    push.FCM_CREDENTIALS_PATH = str(KEYS / "fcm-test-service-account.json")
    tmp = Path(tempfile.mkdtemp())

    apns_payloads = [
        {"token": "tok-ios-1", "title": "Storm intensifying", "body": "+12 dBZ since last frame (40→52)",
         "collapse_id": "watch:" + "ab" * 32 + ":2026-10-14T00:00:00+00:00:intensifying",
         "extra": {"storm_cell_id": "cell-7", "kind": "intensifying"}},
        {"token": "tok-ios-2", "title": "Severe weather alert", "body": "NWS issued an alert",
         "collapse_id": "short-id",
         "extra": {"aps": "overridden", "alert_id": "urn:oid:1"}},
    ]
    apns_cases = []
    for p in apns_payloads:
        payload = push.PushPayload(title=p["title"], body=p["body"], collapse_id=p["collapse_id"], extra=p["extra"])
        FakeClient.calls.clear()
        FakeClient.responses = [FakeResp(200)]
        asyncio.run(push._send_apns(p["token"], payload))
        call = FakeClient.calls[0]
        auth = call["headers"]["authorization"]
        token_jwt = auth.split(" ", 1)[1]
        header = jwt.get_unverified_header(token_jwt)
        claims = jwt.decode(token_jwt, options={"verify_signature": False})
        apns_cases.append({
            "token": p["token"], "title": p["title"], "body": p["body"],
            "collapse_id": p["collapse_id"], "extra": p["extra"],
            "url": call["url"], "headers": {k: v for k, v in call["headers"].items() if k != "authorization"},
            "authorization_prefix": auth.split(" ", 1)[0],
            "jwt_header": {"alg": header["alg"], "kid": header["kid"], "typ": header.get("typ")},
            "jwt_claims_keys": sorted(claims.keys()),
            "jwt_iss": claims["iss"],
            "body_json": call["json"],
            "expect": "ok",
        })

    # Response handling.
    apns_status_cases = []
    for status, text in [(410, ""), (400, '{"reason":"BadDeviceToken"}'), (400, '{"reason":"PayloadTooLarge"}'),
                         (429, "TooManyRequests"), (503, "ServiceUnavailable"), (200, "")]:
        DELETED_TOKENS.clear()
        FakeClient.calls.clear()
        FakeClient.responses = [FakeResp(status, text)]
        payload = push.PushPayload(title="t", body="b", collapse_id="c", extra={})
        try:
            asyncio.run(push._send_apns("tok-x", payload))
            outcome = {"raised": False}
        except Exception as exc:  # noqa: BLE001
            outcome = {"raised": True, "non_retryable": bool(getattr(exc, "non_retryable", False)),
                       "message": str(exc)}
        apns_status_cases.append({"status": status, "text": text, "outcome": outcome,
                                  "deleted": list(DELETED_TOKENS)})

    # FCM: token mint + send.
    push._FCM_TOKEN = None
    fcm_cases = []
    FakeClient.calls.clear()
    FakeClient.responses = [FakeResp(200, payload={"access_token": "ya29.test"}), FakeResp(200, "{}")]
    payload = push.PushPayload(title="Storm intensifying", body="+12 dBZ",
                               collapse_id="watch:abc:2026:intensifying",
                               extra={"storm_cell_id": "cell-7", "kind": "intensifying"})
    asyncio.run(push._send_fcm("fcm-tok-1", payload))
    oauth, send = FakeClient.calls
    form = oauth["data"]
    assertion_header = jwt.get_unverified_header(form["assertion"])
    assertion_claims = jwt.decode(form["assertion"], options={"verify_signature": False})
    fcm_cases.append({
        "oauth_url": oauth["url"],
        "oauth_form_keys": sorted(form.keys()),
        "oauth_grant_type": form["grant_type"],
        "assertion_alg": assertion_header["alg"],
        "assertion_claims": {k: assertion_claims[k] for k in sorted(assertion_claims)},
        "send_url": send["url"],
        "send_headers": send["headers"],
        "send_body": send["json"],
        "expect": "ok",
    })
    # Second send reuses the cached token: no second oauth call.
    FakeClient.calls.clear()
    FakeClient.responses = [FakeResp(200, "{}")]
    asyncio.run(push._send_fcm("fcm-tok-2", payload))
    fcm_cases.append({"reused_token_calls": len(FakeClient.calls),
                      "reused_auth": FakeClient.calls[0]["headers"]["authorization"]})

    fcm_status_cases = []
    for status, text in [(404, "{}"), (400, '{"error":{"status":"INVALID_ARGUMENT"}}'),
                         (400, '{"error":{"details":[{"errorCode":"UNREGISTERED"}],"status":"NOT_REGISTERED"}}'),
                         (403, "PERMISSION_DENIED"), (500, "INTERNAL")]:
        push._FCM_TOKEN = ("ya29.cached", push.time.time() + 3000)
        DELETED_TOKENS.clear()
        FakeClient.calls.clear()
        FakeClient.responses = [FakeResp(status, text)]
        try:
            asyncio.run(push._send_fcm("fcm-tok-x", payload))
            outcome = {"raised": False}
        except Exception as exc:  # noqa: BLE001
            outcome = {"raised": True, "non_retryable": bool(getattr(exc, "non_retryable", False)),
                       "message": str(exc)}
        fcm_status_cases.append({"status": status, "text": text, "outcome": outcome,
                                 "deleted": list(DELETED_TOKENS)})

    write_json("push_apns.json", {"config": {"host": "https://api.push.apple.com", "topic": "me.example.radar",
                                             "key_id": "ABC123DEFG", "team_id": "TEAM123456"},
                                  "cases": apns_cases, "status_cases": apns_status_cases})
    write_json("push_fcm.json", {"cases": fcm_cases, "status_cases": fcm_status_cases})


def main():
    rng = random.Random(20261014)
    make_keys()
    gen_lzw(rng)
    gen_point_in_geojson(rng)
    gen_picker(rng)
    gen_seed()
    gen_writer()
    gen_alerts()
    gen_push()


if __name__ == "__main__":
    main()
