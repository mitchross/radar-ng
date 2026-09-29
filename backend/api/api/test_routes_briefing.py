import json
import os

import httpx
import pytest
from fastapi.testclient import TestClient

os.environ.setdefault("DISABLE_WORKFLOW_ROUTES", "1")

from backend.api.api import routes_briefing, routes_places, server

FORECAST = {
    "utc_offset_seconds": -14400,
    "current": {"temperature_2m": 61, "apparent_temperature": 60, "weather_code": 61, "wind_speed_10m": 8, "wind_gusts_10m": 15},
    "hourly": {"time": [f"2099-01-01T{h:02d}:00" for h in range(24)], "temperature_2m": [60] * 24, "weather_code": [61] * 24, "precipitation": [0.02] * 24},
    "daily": {"temperature_2m_max": [66], "temperature_2m_min": [52], "precipitation_sum": [0.3]},
}
LLM_REPLY = {"choices": [{"message": {"content": json.dumps({"headline": "Showers rolling in", "body": "Light rain by 2:40 PM, gone by 3:30."})}}]}


@pytest.fixture
def api(monkeypatch, tmp_path):
    calls = {"llm": 0, "forecast": 0}
    state = {"llm": lambda req: httpx.Response(200, json=LLM_REPLY)}

    def dispatch(req: httpx.Request) -> httpx.Response:
        if req.url.path.endswith("/chat/completions"):
            calls["llm"] += 1
            calls["last_llm"] = json.loads(req.content)
            return state["llm"](req)
        if "alerts" in req.url.path:
            if req.url.path.startswith("/alerts/urn"):
                return httpx.Response(200, json={"properties": {"event": "Flood Warning", "areaDesc": "Kent", "description": "River rising."}})
            return httpx.Response(200, json={"features": []})
        calls["forecast"] += 1
        return httpx.Response(200, json=FORECAST)

    monkeypatch.setattr(routes_briefing, "LLM_BASE_URL", "http://litellm.test/v1")
    monkeypatch.setattr(routes_briefing, "STATE_DIR", str(tmp_path))
    monkeypatch.setattr(routes_briefing, "_breaker", routes_briefing._Breaker())
    monkeypatch.setattr(routes_briefing, "_semaphore", None)
    routes_briefing._cache.clear()
    routes_places._alerts_cache.clear()
    server._forecast_cache.clear()
    with TestClient(server.app) as client:
        server.app.state.forecast_http = httpx.AsyncClient(transport=httpx.MockTransport(dispatch))
        yield client, calls, state


def test_unconfigured_llm_is_quietly_unavailable(monkeypatch):
    monkeypatch.setattr(routes_briefing, "LLM_BASE_URL", "")
    with TestClient(server.app) as client:
        resp = client.get("/api/briefing", params={"lat": 42.96, "lon": -85.67})
    assert resp.status_code == 200
    assert resp.json() == {"available": False, "reason": "not_configured"}


def test_briefing_narrates_facts_with_thinking_off_and_caches(api):
    client, calls, _ = api
    body = client.get("/api/briefing", params={"lat": 42.96, "lon": -85.67, "place": "Grand Rapids"}).json()
    assert body["available"] and body["headline"] == "Showers rolling in"
    sent = calls["last_llm"]
    assert sent["chat_template_kwargs"] == {"enable_thinking": False, "preserve_thinking": False}
    facts = json.loads(sent["messages"][1]["content"])
    assert facts["place"] == "Grand Rapids" and facts["now"]["sky"] == "light rain"
    client.get("/api/briefing", params={"lat": 42.97, "lon": -85.66, "place": "Grand Rapids"})
    assert calls["llm"] == 1


@pytest.mark.parametrize("failure", [
    lambda req: httpx.Response(500),
    lambda req: httpx.Response(200, json={"choices": [{"message": {"content": "not json"}}]}),
    lambda req: (_ for _ in ()).throw(httpx.ReadTimeout("slow")),
])
def test_llm_failures_never_error_the_api(api, failure):
    client, _, state = api
    state["llm"] = failure
    resp = client.get("/api/briefing", params={"lat": 42.96, "lon": -85.67})
    assert resp.status_code == 200 and resp.json()["available"] is False


def test_breaker_stops_calling_a_failing_llm(api, monkeypatch):
    client, calls, state = api
    state["llm"] = lambda req: httpx.Response(503)
    for i in range(5):
        client.get("/api/briefing", params={"lat": 40 + i, "lon": -85.0})
    assert calls["llm"] == routes_briefing.BREAKER_FAILURES
    assert client.get("/api/briefing", params={"lat": 30, "lon": -85}).json()["reason"] == "llm_unavailable"


def test_missing_forecast_means_no_briefing_not_an_error(api):
    client, calls, _ = api
    server.app.state.forecast_http = httpx.AsyncClient(transport=httpx.MockTransport(lambda req: httpx.Response(502)))
    resp = client.get("/api/briefing", params={"lat": 42.96, "lon": -85.67})
    assert resp.status_code == 200 and resp.json()["available"] is False


def test_alert_explainer(api):
    client, calls, state = api
    state["llm"] = lambda req: httpx.Response(200, json={"choices": [{"message": {"content": '{"what": "The Grand River is flooding in Kent County.", "do": "Avoid low roads."}'}}]})
    body = client.get("/api/alerts/explain", params={"id": "urn:oid:2.49.0.1.840.0.abc"}).json()
    assert body["available"] and body["do"] == "Avoid low roads."
    assert client.get("/api/alerts/explain", params={"id": "javascript:alert(1)"}).json()["available"] is False


def test_nowcast_summary_and_incoming_storm_helpers():
    pts = [{"timestamp": f"2099-01-01T18:{m:02d}:00+00:00", "dbz": d} for m, d in [(5, 0), (10, 20), (15, 38), (20, 12)]]
    assert routes_briefing.summarize_nowcast(pts, -14400) == "rain from about 2:10 PM, peaking near 2:15 PM at 38 dBZ, ending around 2:20 PM"
    assert routes_briefing.summarize_nowcast([{"timestamp": "2099-01-01T18:05:00+00:00", "dbz": 3}], 0) == "dry for the next hour"
    storms = {"features": [{
        "geometry": {"coordinates": [-86.2, 42.96]},
        "properties": {"peak_dbz": 57, "tracking_confidence": 0.9, "track_history": [[0, 0, 0]] * 3,
                       "tracking_vector": {"east_kmh": 60, "north_kmh": 0, "speed_kmh": 60}},
    }]}
    t = routes_briefing.incoming_storm(storms, 42.96, -85.67)
    assert t and 30 <= t["minutes"] <= 50 and t["peak_dbz"] == 57
