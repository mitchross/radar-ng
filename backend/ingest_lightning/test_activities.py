import json

from backend.ingest_lightning import activities


def test_write_geojson_is_atomic_and_leaves_no_temp_files(tmp_path, monkeypatch):
    out = tmp_path / "state" / "lightning.json"
    monkeypatch.setattr(activities, "STATE_DIR", out.parent)
    monkeypatch.setattr(activities, "OUT_PATH", out)

    strikes = [{"t": 1.0, "lat": 40.0, "lon": -100.0, "pol": 1, "mds": 3}]
    activities._write_geojson(strikes)
    activities._write_geojson(iter(strikes))  # any iterable snapshot works

    body = json.loads(out.read_text())
    assert body["type"] == "FeatureCollection"
    assert body["features"][0]["geometry"]["coordinates"] == [-100.0, 40.0]
    assert [p.name for p in out.parent.iterdir()] == ["lightning.json"]


def test_primary_endpoints_are_the_verified_wss_hosts():
    primary, legacy = activities.configured_endpoints()
    assert primary == [
        "wss://ws1.blitzortung.org:443/",
        "wss://ws2.blitzortung.org:443/",
        "wss://ws7.blitzortung.org:443/",
        "wss://ws8.blitzortung.org:443/",
    ]
    assert all(url.startswith("ws://") for url in legacy)
    assert not set(primary) & set(legacy)


def test_endpoint_override_replaces_primaries_only(monkeypatch):
    monkeypatch.setattr(activities, "_ENDPOINT_OVERRIDE", " wss://relay.local/ ,, ws://other.local:8087/ ")
    primary, legacy = activities.configured_endpoints()
    assert primary == ["wss://relay.local/", "ws://other.local:8087/"]
    assert legacy == list(activities.LEGACY_ENDPOINTS)


class _Clock:
    def __init__(self) -> None:
        self.now = 1000.0

    def __call__(self) -> float:
        return self.now


def _picker(clock: _Clock) -> activities.EndpointPicker:
    return activities.EndpointPicker(
        ["wss://a/", "wss://b/"], ["ws://legacy-1/", "ws://legacy-2/"], clock=clock, rand=lambda: 0.5
    )


def test_picker_round_robins_healthy_primaries_and_never_touches_legacy():
    picker = _picker(_Clock())
    assert [picker.next() for _ in range(4)] == ["wss://a/", "wss://b/", "wss://a/", "wss://b/"]


def test_picker_demotes_failed_primaries_then_falls_back_to_legacy_then_recovers():
    clock = _Clock()
    picker = _picker(clock)
    first = picker.next()
    delay_1 = picker.failed(first)
    second = picker.next()
    assert second != first and second.startswith("wss://")
    delay_2 = picker.failed(second)
    # Both primaries demoted: legacy ports are the last resort.
    assert picker.next().startswith("ws://legacy")
    assert delay_2 > delay_1 > 0
    assert delay_2 <= activities.FAILURE_BACKOFF_MAX_S
    # Once the primary demotion expires the picker goes back to wss.
    clock.now += activities.DEMOTE_PRIMARY_S + 1
    assert picker.next().startswith("wss://")


def test_picker_never_stalls_when_everything_is_demoted():
    clock = _Clock()
    picker = _picker(clock)
    for url in ["wss://a/", "wss://b/", "ws://legacy-1/", "ws://legacy-2/"]:
        picker.failed(url)
    assert picker.next() in {"wss://a/", "wss://b/"}


def test_picker_success_resets_backoff_and_reconnects_fast():
    picker = _picker(_Clock())
    url = picker.next()
    picker.failed(url)
    picker.failed(url)
    assert picker.consecutive_failures == 2
    assert picker.succeeded(url) == activities.RECONNECT_AFTER_CLOSE_S
    assert picker.consecutive_failures == 0
    # A healed endpoint is eligible again immediately.
    assert url in [picker.next() for _ in range(2)]


def test_picker_backoff_is_capped():
    picker = _picker(_Clock())
    delays = [picker.failed("wss://a/") for _ in range(12)]
    assert max(delays) <= activities.FAILURE_BACKOFF_MAX_S
    assert delays[-1] >= delays[0]


def _feature(t: float, lon: float = -100.0, lat: float = 40.0, **props) -> dict:
    return {
        "type": "Feature",
        "geometry": {"type": "Point", "coordinates": [lon, lat]},
        "properties": {"time": t, "polarity": 1, "mds": 4, **props},
    }


def test_seed_keeps_only_strikes_inside_retention_and_ignores_junk(tmp_path):
    out = tmp_path / "lightning.json"
    now = 10_000.0
    body = {
        "type": "FeatureCollection",
        "features": [
            _feature(now - 20 * 60),  # expired
            _feature(now - 60, lon=-90.0, lat=35.0),
            _feature(now - 30),
            {"type": "Feature", "geometry": {}, "properties": {}},  # malformed
            {"type": "Feature", "geometry": {"coordinates": [1, 2]}, "properties": {"time": "nope"}},
        ],
    }
    out.write_text(json.dumps(body))
    strikes = activities._load_existing_strikes(out, retention_min=15, now=now)
    assert [s["t"] for s in strikes] == [now - 60, now - 30]
    assert strikes[0] == {"t": now - 60, "lat": 35.0, "lon": -90.0, "pol": 1, "mds": 4}


def test_seed_tolerates_missing_or_corrupt_file(tmp_path):
    assert activities._load_existing_strikes(tmp_path / "missing.json") == []
    bad = tmp_path / "bad.json"
    bad.write_text("{not json")
    assert activities._load_existing_strikes(bad) == []
    bad.write_text("[]")
    assert activities._load_existing_strikes(bad) == []


def test_seed_round_trips_through_the_writer(tmp_path, monkeypatch):
    out = tmp_path / "state" / "lightning.json"
    monkeypatch.setattr(activities, "STATE_DIR", out.parent)
    monkeypatch.setattr(activities, "OUT_PATH", out)
    now = 5_000.0
    strikes = [{"t": now - 10, "lat": 41.0, "lon": -99.0, "pol": -1, "mds": 7}]
    activities._write_geojson(strikes)
    assert activities._load_existing_strikes(now=now) == strikes
