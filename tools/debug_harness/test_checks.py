"""Pure-logic tests for the debug harness (no network, no temporalio)."""

from __future__ import annotations

from datetime import timedelta

from tools.debug_harness import checks
from tools.debug_harness.core import FAIL, OK, WARN, Check, parse_iso, percentiles, quote_ts, utcnow, worst


def test_percentiles_basic():
    stats = percentiles([10.0, 20.0, 30.0, 40.0, 100.0])
    assert stats["n"] == 5
    assert stats["min_ms"] == 10.0
    assert stats["p50_ms"] == 30.0
    assert stats["max_ms"] == 100.0
    assert percentiles([]) == {}


def test_parse_iso_handles_ingest_format():
    # ingest writes tz-aware isoformat (backend/ingest_mrms/activities.py)
    dt = parse_iso("2026-07-04T20:02:00+00:00")
    assert dt is not None and dt.tzinfo is not None
    # tolerate naive strings as UTC
    naive = parse_iso("2026-07-04T20:02:00")
    assert naive is not None and naive.tzinfo is not None
    assert parse_iso("not-a-timestamp") is None


def test_quote_ts_encodes_colon_and_plus():
    assert quote_ts("2026-07-04T20:02:00+00:00") == "2026-07-04T20%3A02%3A00%2B00%3A00"


def test_slippy_conus_center():
    # CONUS center at z4 lands inside the continental tile range
    x, y = checks._slippy(39.5, -98.35, 4)
    assert (x, y) == (3, 6)


def test_worst_severity_ordering():
    assert worst([Check("a", OK, ""), Check("b", WARN, "")]) == WARN
    assert worst([Check("a", WARN, ""), Check("b", FAIL, "")]) == FAIL
    assert worst([]) == OK


def test_pipeline_freshness_logic(monkeypatch):
    now = utcnow()
    fresh = [(now - timedelta(minutes=2 * i)).isoformat() for i in range(29, -1, -1)]
    stale_forecast = [(now - timedelta(hours=2)).isoformat()]
    manifest = {
        "layers": {
            "radar": {"timestamps": fresh, "latest": fresh[-1], "palettes": ["classic"]},
            "nowcast": {"timestamps": stale_forecast, "latest": stale_forecast[-1]},
        },
        "tile_url_template": "/tiles/{layer}/{palette}/{timestamp}/{z}/{x}/{y}.png",
    }

    monkeypatch.setattr(checks, "_get_manifest", lambda server: (None, manifest))
    monkeypatch.setattr(
        checks, "fetch_json",
        lambda url, **kw: (None, {"status": "ok", "reasons": [], "nowcast": {"status": "ok"}})
        if "health" in url else (None, {"features": [], "generated_at": now.timestamp()}),
    )

    results = {c.name: c for c in checks.check_pipeline("http://x")}
    assert results["pipeline.radar"].status == OK
    assert results["pipeline.radar"].data["frames_hour"] == 30
    # a forecast layer whose horizon has collapsed into the past must fail
    assert results["pipeline.nowcast"].status == FAIL
    # HRRR layers absent from the manifest only warn
    assert results["pipeline.temperature"].status == WARN
