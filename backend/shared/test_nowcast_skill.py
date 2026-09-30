from datetime import datetime, timedelta, timezone

from backend.shared import nowcast_skill as skill


def _entry(anchor: str, lead: int, scored_at: datetime, hits=10, misses=2, false_alarms=3, persist=(6, 6, 6)):
    return {
        "anchor": anchor,
        "valid": "v",
        "lead_minutes": lead,
        "scored_at": scored_at.isoformat(),
        "cells": 100,
        "thresholds": {
            "rain": {"hits": hits, "misses": misses, "false_alarms": false_alarms},
            "heavy": {"hits": 1, "misses": 1, "false_alarms": 0},
        },
        "persistence": {
            "rain": {"hits": persist[0], "misses": persist[1], "false_alarms": persist[2]},
            "heavy": {"hits": 0, "misses": 2, "false_alarms": 0},
        },
    }


def test_scores_from_counts_is_the_classic_contingency_table():
    scores = skill.scores_from_counts({"hits": 8, "misses": 2, "false_alarms": 2})
    assert scores["pod"] == 0.8 and scores["far"] == 0.2 and scores["csi"] == round(8 / 12, 3)
    assert scores["bias"] == 1.0
    empty = skill.scores_from_counts({})
    assert empty["pod"] is None and empty["far"] is None and empty["csi"] is None


def test_append_keeps_only_the_window_and_writes_atomically(tmp_path):
    now = datetime(2026, 9, 30, 12, tzinfo=timezone.utc)
    old = _entry("a", 30, now - timedelta(hours=30))
    fresh = _entry("b", 30, now - timedelta(minutes=5))
    skill.append_entries(tmp_path, [old, fresh], now=now)
    log = skill.read_log(tmp_path)
    assert [e["anchor"] for e in log["entries"]] == ["b"]
    assert log["updated_at"] == now.isoformat()
    assert [p.name for p in tmp_path.iterdir()] == [skill.SKILL_FILE]

    skill.append_entries(tmp_path, [_entry("c", 5, now)], now=now)
    assert [e["anchor"] for e in skill.read_log(tmp_path)["entries"]] == ["b", "c"]


def test_unreadable_log_is_empty(tmp_path):
    (tmp_path / skill.SKILL_FILE).write_text("{nope")
    assert skill.read_log(tmp_path)["entries"] == []
    (tmp_path / skill.SKILL_FILE).write_text("[]")
    assert skill.read_log(tmp_path)["entries"] == []


def test_summary_withholds_a_headline_until_enough_runs_scored():
    now = datetime(2026, 9, 30, 12, tzinfo=timezone.utc)
    log = skill.empty_log()
    log["entries"] = [_entry(f"a{i}", 30, now) for i in range(skill.MIN_RUNS_FOR_HEADLINE - 1)]
    summary = skill.summarize(log, now=now)
    assert summary["available"] is False and summary["reason"] == "warming_up"
    assert summary["by_lead"][0]["runs"] == skill.MIN_RUNS_FOR_HEADLINE - 1


def test_summary_aggregates_per_lead_and_picks_the_30_minute_headline():
    now = datetime(2026, 9, 30, 12, tzinfo=timezone.utc)
    log = skill.empty_log()
    entries = []
    for i in range(8):
        entries.append(_entry(f"a{i}", 30, now - timedelta(minutes=i)))
        entries.append(_entry(f"a{i}", 5, now - timedelta(minutes=i), hits=20, misses=0, false_alarms=0))
    log["entries"] = entries
    summary = skill.summarize(log, now=now)
    assert summary["available"] is True
    assert summary["runs_scored"] == 8
    by_lead = {row["lead_minutes"]: row for row in summary["by_lead"]}
    assert by_lead[5]["rain"]["pod"] == 1.0 and by_lead[5]["rain"]["far"] == 0.0
    assert by_lead[30]["rain"]["hits"] == 80
    headline = summary["headline"]
    assert headline["lead_minutes"] == 30 and headline["runs"] == 8
    assert headline["pod"] == round(80 / 96, 3)
    assert headline["far"] == round(24 / 104, 3)
    assert headline["persistence_csi"] == round(48 / 144, 3)


def test_summary_reports_no_rain_when_nothing_was_observed():
    now = datetime(2026, 9, 30, 12, tzinfo=timezone.utc)
    log = skill.empty_log()
    log["entries"] = [_entry(f"a{i}", 30, now, hits=0, misses=0, false_alarms=0, persist=(0, 0, 0)) for i in range(8)]
    summary = skill.summarize(log, now=now)
    assert summary["available"] is False and summary["reason"] == "no_rain_observed"


def test_prometheus_lines_expose_each_lead_and_threshold():
    now = datetime(2026, 9, 30, 12, tzinfo=timezone.utc)
    log = skill.empty_log()
    log["entries"] = [_entry(f"a{i}", 15, now) for i in range(8)]
    lines = skill.prometheus_lines(skill.summarize(log, now=now))
    assert "radar_ng_nowcast_skill_runs 8" in lines
    assert 'radar_ng_nowcast_pod{lead_minutes="15",threshold="rain"} 0.833' in lines
    assert any(line.startswith('radar_ng_nowcast_csi{lead_minutes="15",threshold="heavy"}') for line in lines)
