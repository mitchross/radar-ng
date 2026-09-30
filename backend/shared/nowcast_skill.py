"""Rolling verification log for the radar nowcast, shared by worker and API.

The nowcast worker appends one entry per (run, lead time) whenever the radar
frame that a retained run predicted actually arrives (see
``backend/nowcast/skill.py`` for the grid comparison). This module owns the
file format and the summary the API serves, and deliberately imports nothing
heavier than the standard library so the tile-server image can use it.

Counts are the classic contingency table per dBZ threshold: ``hits`` (rain
forecast and observed), ``misses`` (observed, not forecast) and
``false_alarms`` (forecast, not observed). From them:

- POD  = hits / (hits + misses)          "how much of the rain did we catch"
- FAR  = false_alarms / (hits + false_alarms)  "how much forecast rain never came"
- CSI  = hits / (hits + misses + false_alarms)  the usual single skill number

``persistence`` holds the same counts for a "nothing moves" forecast made
from the run's own anchor observation, so the app can show whether the motion
forecast beats simply assuming the radar stays put.
"""

from __future__ import annotations

import json
import os
import tempfile
from datetime import datetime, timedelta, timezone
from pathlib import Path
from typing import Any

SKILL_FILE = "nowcast-skill.json"
SCHEMA_VERSION = 1
THRESHOLDS_DBZ: dict[str, float] = {"rain": 20.0, "heavy": 35.0}
WINDOW_HOURS = int(os.environ.get("NOWCAST_SKILL_WINDOW_HOURS", "24"))
HEADLINE_LEAD_MIN = int(os.environ.get("NOWCAST_SKILL_HEADLINE_LEAD_MIN", "30"))
# Fewer scored runs than this and the headline is withheld: a single storm's
# first minutes would otherwise read as a track record.
MIN_RUNS_FOR_HEADLINE = int(os.environ.get("NOWCAST_SKILL_MIN_RUNS", "6"))


def skill_path(state_dir: str | Path) -> Path:
    return Path(state_dir) / SKILL_FILE


def empty_log() -> dict[str, Any]:
    return {
        "schema_version": SCHEMA_VERSION,
        "window_hours": WINDOW_HOURS,
        "thresholds_dbz": dict(THRESHOLDS_DBZ),
        "updated_at": None,
        "entries": [],
    }


def read_log(state_dir: str | Path) -> dict[str, Any]:
    """The log, or an empty one when the file is missing or unreadable."""
    try:
        body = json.loads(skill_path(state_dir).read_text())
    except (OSError, ValueError):
        return empty_log()
    if not isinstance(body, dict) or not isinstance(body.get("entries"), list):
        return empty_log()
    return body


def _parse_ts(value: object) -> datetime | None:
    try:
        dt = datetime.fromisoformat(str(value).replace("Z", "+00:00"))
    except (TypeError, ValueError):
        return None
    return dt if dt.tzinfo else dt.replace(tzinfo=timezone.utc)


def append_entries(
    state_dir: str | Path,
    entries: list[dict[str, Any]],
    *,
    now: datetime | None = None,
    window_hours: int | None = None,
) -> dict[str, Any]:
    """Append verified entries, drop everything older than the window, write atomically."""
    now = now or datetime.now(timezone.utc)
    window = timedelta(hours=window_hours or WINDOW_HOURS)
    log = read_log(state_dir)
    kept = [
        entry
        for entry in [*log["entries"], *entries]
        if (scored := _parse_ts(entry.get("scored_at"))) is not None and now - scored <= window
    ]
    kept.sort(key=lambda e: (str(e.get("scored_at")), int(e.get("lead_minutes") or 0)))
    log.update(
        schema_version=SCHEMA_VERSION,
        window_hours=window_hours or WINDOW_HOURS,
        thresholds_dbz=dict(THRESHOLDS_DBZ),
        updated_at=now.isoformat(),
        entries=kept,
    )
    path = skill_path(state_dir)
    path.parent.mkdir(parents=True, exist_ok=True)
    fd, tmp_name = tempfile.mkstemp(prefix=f".{path.name}.", suffix=".tmp", dir=str(path.parent))
    try:
        with os.fdopen(fd, "w") as fh:
            json.dump(log, fh, separators=(",", ":"), sort_keys=True)
            fh.write("\n")
        os.replace(tmp_name, path)
    finally:
        try:
            os.unlink(tmp_name)
        except FileNotFoundError:
            pass
    return log


def _ratio(numerator: int, denominator: int) -> float | None:
    return round(numerator / denominator, 3) if denominator > 0 else None


def scores_from_counts(counts: dict[str, Any]) -> dict[str, Any]:
    hits = int(counts.get("hits", 0))
    misses = int(counts.get("misses", 0))
    false_alarms = int(counts.get("false_alarms", 0))
    return {
        "hits": hits,
        "misses": misses,
        "false_alarms": false_alarms,
        "pod": _ratio(hits, hits + misses),
        "far": _ratio(false_alarms, hits + false_alarms),
        "csi": _ratio(hits, hits + misses + false_alarms),
        "bias": _ratio(hits + false_alarms, hits + misses),
    }


def _add_counts(into: dict[str, int], counts: dict[str, Any] | None) -> None:
    if not isinstance(counts, dict):
        return
    for key in ("hits", "misses", "false_alarms"):
        into[key] = into.get(key, 0) + int(counts.get(key, 0) or 0)


def summarize(
    log: dict[str, Any],
    *,
    now: datetime | None = None,
    window_hours: int | None = None,
) -> dict[str, Any]:
    """Aggregate the log per lead time and pick the headline the app shows."""
    now = now or datetime.now(timezone.utc)
    window = timedelta(hours=window_hours or int(log.get("window_hours") or WINDOW_HOURS))
    by_lead: dict[int, dict[str, Any]] = {}
    anchors: set[str] = set()
    for entry in log.get("entries", []):
        if not isinstance(entry, dict):
            continue
        scored = _parse_ts(entry.get("scored_at"))
        if scored is None or now - scored > window:
            continue
        try:
            lead = int(entry["lead_minutes"])
        except (KeyError, TypeError, ValueError):
            continue
        bucket = by_lead.setdefault(
            lead,
            {"n": 0, "anchors": set(), "counts": {}, "persistence": {}},
        )
        bucket["n"] += 1
        bucket["anchors"].add(str(entry.get("anchor")))
        anchors.add(str(entry.get("anchor")))
        thresholds = entry.get("thresholds") or {}
        persistence = entry.get("persistence") or {}
        for label in THRESHOLDS_DBZ:
            _add_counts(bucket["counts"].setdefault(label, {}), thresholds.get(label))
            _add_counts(bucket["persistence"].setdefault(label, {}), persistence.get(label))

    leads = []
    for lead in sorted(by_lead):
        bucket = by_lead[lead]
        row: dict[str, Any] = {"lead_minutes": lead, "runs": len(bucket["anchors"])}
        for label in THRESHOLDS_DBZ:
            scores = scores_from_counts(bucket["counts"].get(label, {}))
            persistence = scores_from_counts(bucket["persistence"].get(label, {}))
            scores["persistence_csi"] = persistence["csi"] if persistence["hits"] + persistence["misses"] else None
            row[label] = scores
        leads.append(row)

    summary: dict[str, Any] = {
        "available": False,
        "window_hours": int(window.total_seconds() // 3600),
        "thresholds_dbz": dict(THRESHOLDS_DBZ),
        "runs_scored": len(anchors),
        "updated_at": log.get("updated_at"),
        "by_lead": leads,
        "headline": None,
    }
    if not leads:
        summary["reason"] = "no_scored_runs"
        return summary

    headline_row = min(leads, key=lambda row: (abs(row["lead_minutes"] - HEADLINE_LEAD_MIN), row["lead_minutes"]))
    if headline_row["runs"] < MIN_RUNS_FOR_HEADLINE:
        summary["reason"] = "warming_up"
        return summary
    rain = headline_row["rain"]
    if rain["pod"] is None:
        # No observed rain at this lead in the window: nothing to be right about yet.
        summary["reason"] = "no_rain_observed"
        return summary
    summary["available"] = True
    summary["headline"] = {
        "lead_minutes": headline_row["lead_minutes"],
        "window_hours": summary["window_hours"],
        "runs": headline_row["runs"],
        "pod": rain["pod"],
        "far": rain["far"],
        "csi": rain["csi"],
        "persistence_csi": rain["persistence_csi"],
    }
    return summary


def prometheus_lines(summary: dict[str, Any]) -> list[str]:
    """Gauges for the API's /api/metrics."""
    lines = [
        "# TYPE radar_ng_nowcast_skill_runs gauge",
        f"radar_ng_nowcast_skill_runs {int(summary.get('runs_scored', 0))}",
    ]
    rows = summary.get("by_lead") or []
    if rows:
        for metric in ("pod", "far", "csi"):
            lines.append(f"# TYPE radar_ng_nowcast_{metric} gauge")
            for row in rows:
                for label in THRESHOLDS_DBZ:
                    value = (row.get(label) or {}).get(metric)
                    if value is None:
                        continue
                    lines.append(
                        f'radar_ng_nowcast_{metric}{{lead_minutes="{row["lead_minutes"]}",threshold="{label}"}} {value}'
                    )
    return lines
