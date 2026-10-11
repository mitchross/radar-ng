import json
import os

import httpx
import pytest
from fastapi.testclient import TestClient

os.environ.setdefault("DISABLE_WORKFLOW_ROUTES", "1")

from backend.api.api import routes_places, server


@pytest.fixture
def upstream(monkeypatch):
    """Route every upstream call through a handler the test controls."""
    calls: list[httpx.Request] = []
    state = {"handler": lambda req: httpx.Response(500)}

    def dispatch(req: httpx.Request) -> httpx.Response:
        calls.append(req)
        return state["handler"](req)

    for cache in (routes_places._alerts_cache, routes_places._geocode_cache, routes_places._reverse_cache):
        cache.clear()
    monkeypatch.setattr(routes_places, "PHOTON_URL", "http://photon.test:2322")
    monkeypatch.setattr(routes_places, "GEOCODER_URL", "")
    with TestClient(server.app) as client:
        server.app.state.forecast_http = httpx.AsyncClient(transport=httpx.MockTransport(dispatch))
        yield client, calls, state


ALERT = {
    "type": "FeatureCollection",
    "features": [{"id": "urn:1", "properties": {"id": "urn:1", "event": "Tornado Warning"}}],
}


def test_alerts_proxy_rounds_point_and_passes_nws_body_through(upstream):
    client, calls, state = upstream
    state["handler"] = lambda req: httpx.Response(200, json=ALERT)

    resp = client.get("/api/alerts", params={"lat": 42.9634567, "lon": -85.6681234})

    assert resp.status_code == 200
    assert resp.json() == ALERT
    assert calls[0].url.params["point"] == "42.963,-85.668"
    assert calls[0].headers["User-Agent"] == routes_places.NWS_USER_AGENT


def test_alerts_are_cached_per_rounded_point(upstream):
    client, calls, state = upstream
    state["handler"] = lambda req: httpx.Response(200, json=ALERT)

    client.get("/api/alerts", params={"lat": 42.96341, "lon": -85.66812})
    client.get("/api/alerts", params={"lat": 42.96339, "lon": -85.66808})

    assert len(calls) == 1


def test_alert_upstream_failure_is_502_never_an_empty_list(upstream):
    client, _, state = upstream
    state["handler"] = lambda req: httpx.Response(503, text="busy")

    resp = client.get("/api/alerts", params={"lat": 42.96, "lon": -85.67})

    assert resp.status_code == 502
    assert resp.headers["cache-control"] == "no-store"
    assert "features" not in resp.json()


def test_alert_failure_is_not_cached(upstream):
    client, calls, state = upstream
    state["handler"] = lambda req: httpx.Response(503)
    client.get("/api/alerts", params={"lat": 42.96, "lon": -85.67})
    state["handler"] = lambda req: httpx.Response(200, json=ALERT)

    resp = client.get("/api/alerts", params={"lat": 42.96, "lon": -85.67})

    assert resp.status_code == 200
    assert len(calls) == 2


def _feature(name, osm_value, lon=-85.67, lat=42.96, osm_id=1, **props):
    return {
        "type": "Feature",
        "geometry": {"type": "Point", "coordinates": [lon, lat]},
        "properties": {
            "osm_id": osm_id,
            "osm_key": "place",
            "osm_value": osm_value,
            "name": name,
            "state": "Michigan",
            "country": "United States",
            "countrycode": "us",
            **props,
        },
    }


def test_geocode_keeps_settlements_in_app_shape(upstream):
    client, calls, state = upstream
    state["handler"] = lambda req: httpx.Response(
        200,
        json={
            "features": [
                _feature("Grand Rapids", "city", osm_id=10),
                _feature("Grand Rapids", "house", osm_id=11),
                _feature("Grandville", "town", osm_id=12, lon=-85.76, lat=42.91),
            ]
        },
    )

    resp = client.get("/api/geocode", params={"q": "grand  rap"})

    assert resp.status_code == 200
    assert resp.json()["results"] == [
        {"id": 10, "name": "Grand Rapids", "latitude": 42.96, "longitude": -85.67,
         "admin1": "Michigan", "country": "United States", "countryCode": "US"},
        {"id": 12, "name": "Grandville", "latitude": 42.91, "longitude": -85.76,
         "admin1": "Michigan", "country": "United States", "countryCode": "US"},
    ]
    assert calls[0].url.path == "/api"
    assert calls[0].url.params["q"] == "grand rap"
    assert calls[0].url.params["osm_tag"] == "place"


def test_reverse_geocode_names_the_enclosing_city_at_the_requested_point(upstream):
    client, calls, state = upstream
    street = _feature("Monroe Center St", "residential", osm_id=20, city="Grand Rapids")
    street["properties"]["osm_key"] = "highway"
    state["handler"] = lambda req: httpx.Response(200, json={"features": [street]})

    resp = client.get("/api/reverse-geocode", params={"lat": 42.9634567, "lon": -85.6681234})

    assert resp.json()["place"]["name"] == "Grand Rapids"
    assert resp.json()["place"]["latitude"] == 42.963
    assert calls[0].url.params["lat"] == "42.963"


def test_reverse_geocode_with_no_match_returns_null_place(upstream):
    client, _, state = upstream
    state["handler"] = lambda req: httpx.Response(200, json={"features": []})

    resp = client.get("/api/reverse-geocode", params={"lat": 0.0, "lon": 0.0})

    assert resp.status_code == 200
    assert resp.json() == {"place": None}


GEOCODER_ROW = {"id": 4994358, "name": "Grand Rapids", "admin1": "Michigan", "admin1_code": "MI",
                "country": "United States", "country_code": "US", "feature_code": "PPLA2",
                "population": 195097, "timezone": "America/Detroit", "lat": 42.96336, "lon": -85.66809}


def test_geocode_prefers_our_geocoder_and_keeps_app_shape(upstream, monkeypatch):
    client, calls, state = upstream
    monkeypatch.setattr(routes_places, "GEOCODER_URL", "http://geocoder-api.geocoder:8000")
    state["handler"] = lambda req: httpx.Response(200, json={"query": "grand rap", "results": [GEOCODER_ROW, {"bad": 1}]})

    resp = client.get("/api/geocode", params={"q": "grand  rap", "limit": 5})

    assert resp.json()["results"] == [
        {"id": 4994358, "name": "Grand Rapids", "latitude": 42.96336, "longitude": -85.66809,
         "admin1": "Michigan", "country": "United States", "countryCode": "US"},
    ]
    assert calls[0].url.host == "geocoder-api.geocoder" and calls[0].url.path == "/v1/places"
    assert calls[0].url.params["q"] == "grand rap" and calls[0].url.params["limit"] == "5"


def test_reverse_geocode_via_our_geocoder_labels_the_requested_point(upstream, monkeypatch):
    client, calls, state = upstream
    monkeypatch.setattr(routes_places, "GEOCODER_URL", "http://geocoder-api.geocoder:8000")
    state["handler"] = lambda req: httpx.Response(200, json={**GEOCODER_ROW, "km": 2.7})

    resp = client.get("/api/reverse-geocode", params={"lat": 42.9634567, "lon": -85.6681234})

    assert resp.json()["place"]["name"] == "Grand Rapids"
    assert resp.json()["place"]["latitude"] == 42.963 and resp.json()["place"]["admin1"] == "Michigan"
    assert calls[0].url.path == "/v1/reverse-place" and calls[0].url.params["lat"] == "42.963"

    state["handler"] = lambda req: httpx.Response(404, json={"detail": "no place within 50 km"})
    assert client.get("/api/reverse-geocode", params={"lat": 0.0, "lon": 0.0}).json() == {"place": None}
    state["handler"] = lambda req: httpx.Response(500)
    assert client.get("/api/reverse-geocode", params={"lat": 10.0, "lon": 10.0}).status_code == 502


def test_geocoding_without_photon_is_503_and_makes_no_request(upstream, monkeypatch):
    client, calls, _ = upstream
    monkeypatch.setattr(routes_places, "PHOTON_URL", "")

    assert client.get("/api/geocode", params={"q": "grand"}).status_code == 503
    assert client.get("/api/reverse-geocode", params={"lat": 1, "lon": 1}).status_code == 503
    assert calls == []


def test_out_of_range_point_is_rejected(upstream):
    client, calls, _ = upstream
    assert client.get("/api/alerts", params={"lat": 91, "lon": 0}).status_code == 422
    assert calls == []


def test_basemap_style_serves_absolute_self_hosted_glyphs(monkeypatch, tmp_path):
    (tmp_path / "positron.json").write_text(
        '{"version": 8, "glyphs": "/basemap/fonts/{fontstack}/{range}.pbf",'
        ' "sources": {"b": {"type": "vector", "tiles": ["/basemap/tiles/{z}/{x}/{y}.mvt"]}}, "layers": []}'
    )
    monkeypatch.setattr(server, "STYLE_DIR", str(tmp_path))
    with TestClient(server.app) as client:
        style = client.get("/api/basemap/style/positron", headers={"host": "radar.example"}).json()

    assert style["glyphs"] == "http://radar.example/basemap/fonts/{fontstack}/{range}.pbf"
    assert style["sources"]["b"]["tiles"] == ["http://radar.example/basemap/tiles/{z}/{x}/{y}.mvt"]


def test_basemap_style_keeps_https_from_forwarded_proto(monkeypatch, tmp_path):
    # Caddy forwards X-Forwarded-Proto from trusted proxies; the app hands these
    # URLs to MapLibre offline packs, so they must not fall back to http://.
    (tmp_path / "positron.json").write_text(
        json.dumps({"glyphs": "/basemap/fonts/{fontstack}/{range}.pbf", "sources": {"b": {"tiles": ["/basemap/tiles/{z}/{x}/{y}.mvt"]}}})
    )
    monkeypatch.setattr(server, "STYLE_DIR", str(tmp_path))
    with TestClient(server.app) as client:
        style = client.get(
            "/api/basemap/style/positron",
            headers={"host": "radar.example", "x-forwarded-proto": "https", "x-forwarded-host": "radar-ng-api.vanillax.me"},
        ).json()
    assert style["glyphs"].startswith("https://radar-ng-api.vanillax.me/")
    assert style["sources"]["b"]["tiles"] == ["https://radar-ng-api.vanillax.me/basemap/tiles/{z}/{x}/{y}.mvt"]


def test_bundled_styles_reference_no_external_hosts():
    import json
    from pathlib import Path

    styles = Path(__file__).resolve().parents[2] / "basemap" / "styles"
    for path in styles.glob("*.json"):
        style = json.loads(path.read_text())
        urls = [style.get("glyphs"), style.get("sprite")]
        for src in style.get("sources", {}).values():
            urls += src.get("tiles", []) + [src.get("url")]
        external = [u for u in urls if isinstance(u, str) and u.startswith("http")]
        assert external == [], f"{path.name} fetches from {external}"


MAP_ALERTS = {
    "type": "FeatureCollection",
    "features": [
        {
            "geometry": {"type": "Polygon", "coordinates": [[[-86, 42], [-85, 42], [-85, 43], [-86, 42]]]},
            "properties": {"id": "urn:2", "event": "Severe Thunderstorm Warning", "severity": "Severe", "description": "long text"},
        },
        {"geometry": None, "properties": {"id": "urn:3", "event": "Small Craft Advisory"}},
    ],
}


def test_map_alerts_keep_only_polygons_and_slim_properties(upstream):
    client, calls, state = upstream
    state["handler"] = lambda req: httpx.Response(200, json=MAP_ALERTS)

    resp = client.get("/api/alerts/map")

    assert resp.status_code == 200
    features = resp.json()["features"]
    assert [f["properties"]["id"] for f in features] == ["urn:2"]
    assert "description" not in features[0]["properties"]
    assert calls[0].url.params["status"] == "actual"
    client.get("/api/alerts/map")
    assert len(calls) == 1


def test_map_alert_upstream_failure_is_502(upstream):
    client, _, state = upstream
    state["handler"] = lambda req: httpx.Response(503)

    assert client.get("/api/alerts/map").status_code == 502
