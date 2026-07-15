"""Temporal control-plane checks: schedules, recent failures, stuck runs.

Needs the `temporalio` package (present in both worker images; `pip install
temporalio` elsewhere). Import errors surface as a single actionable check
instead of a traceback so `doctor` still runs the HTTP-side checks without it.
"""

from __future__ import annotations

from datetime import timedelta
from typing import Any

from .core import FAIL, OK, WARN, Check, age_str, utcnow

# Mirrors temporal/schedules/seed.py. Imported lazily below when the repo is
# on sys.path so drift is caught; this literal is the fallback for a
# pip-installed-temporalio-but-no-repo environment (e.g. a debug pod).
FALLBACK_SCHEDULE_IDS = [
    "ingest-mrms-base", "ingest-mrms-composite", "ingest-hrrr",
    "ingest-lightning", "ingest-tropical", "nowcast", "tile-cleanup",
    "poll-alerts", "open-meteo-sync-gfs", "open-meteo-sync-hrrr",
]

# How stale a schedule's most recent action may be before we flag it,
# per schedule cadence (seed.py) with slack for SKIP-dropped runs.
RECENT_ACTION_SLACK = 3.0


def _expected_schedules() -> dict[str, timedelta | None]:
    try:
        from temporal.schedules.seed import SCHEDULES  # type: ignore
        return {s.schedule_id: s.interval for s in SCHEDULES}
    except Exception:
        return {sid: None for sid in FALLBACK_SCHEDULE_IDS}


async def check_temporal(
    address: str,
    namespace: str,
    *,
    failure_window_h: int = 6,
    stuck_after_h: int = 2,
) -> list[Check]:
    try:
        from temporalio.client import Client
    except ImportError:
        # WARN, not FAIL: a missing optional dependency on the machine running
        # the harness says nothing about the health of the stack.
        return [Check(
            "temporal.client", WARN,
            "temporalio not installed — run inside the worker image or `pip install temporalio`",
        )]

    checks: list[Check] = []
    try:
        client = await Client.connect(address, namespace=namespace)
    except Exception as exc:
        return [Check("temporal.connect", FAIL, f"cannot reach {address} ({namespace}): {exc}")]
    checks.append(Check("temporal.connect", OK, f"connected to {address} namespace={namespace}"))

    now = utcnow()

    # -- schedules: existence, pause state, recent-action recency ------------
    expected = _expected_schedules()
    seen: dict[str, Any] = {}
    try:
        # list_schedules changed from sync-returning to async-returning across
        # temporalio 1.x releases; tolerate both.
        import inspect

        it = client.list_schedules()
        if inspect.isawaitable(it):
            it = await it
        async for entry in it:
            seen[entry.id] = entry
    except Exception as exc:
        checks.append(Check("temporal.schedules", FAIL, f"list_schedules failed: {exc}"))
        return checks

    missing = sorted(set(expected) - set(seen))
    if missing:
        checks.append(Check(
            "temporal.schedules", FAIL,
            f"{len(missing)} expected schedule(s) missing: {missing} "
            "(worker seed never ran? check worker logs for '[seed]')",
        ))
    else:
        checks.append(Check("temporal.schedules", OK, f"all {len(expected)} expected schedules present"))

    for sid in sorted(seen):
        entry = seen[sid]
        handle = client.get_schedule_handle(sid)
        try:
            desc = await handle.describe()
        except Exception as exc:
            checks.append(Check(f"temporal.schedule.{sid}", FAIL, f"describe failed: {exc}"))
            continue

        state = desc.schedule.state
        if state.paused:
            checks.append(Check(
                f"temporal.schedule.{sid}", WARN,
                f"PAUSED ({state.note or 'no note'})",
            ))
            continue

        recent = desc.info.recent_actions
        interval = expected.get(sid)
        if not recent:
            checks.append(Check(f"temporal.schedule.{sid}", WARN, "no actions recorded yet"))
            continue
        last = recent[-1]
        last_age = (now - last.started_at).total_seconds()
        detail = f"last run {age_str(last_age)} ago"
        status = OK
        if interval is not None:
            allowed = interval.total_seconds() * RECENT_ACTION_SLACK
            if last_age > allowed:
                # SKIP overlap means one long run legitimately drops triggers,
                # but 3× the cadence with nothing started means stalled.
                status = FAIL
                detail += f" — expected every {age_str(interval.total_seconds())} (stalled or worker down)"
        # Did the most recent action actually succeed?
        try:
            wf = client.get_workflow_handle(
                last.action.workflow_id, run_id=last.action.first_execution_run_id
            )
            wf_desc = await wf.describe()
            wf_status = wf_desc.status.name if wf_desc.status else "UNKNOWN"
            if wf_status in ("FAILED", "TIMED_OUT", "TERMINATED"):
                status = FAIL
                detail += f", last result {wf_status}"
            elif wf_status == "RUNNING":
                detail += ", currently running"
            else:
                detail += f", last result {wf_status}"
        except Exception:
            detail += ", result unknown"
        checks.append(Check(f"temporal.schedule.{sid}", status, detail))

    # -- recent failed / timed-out / terminated workflows --------------------
    cutoff = (now - timedelta(hours=failure_window_h)).strftime("%Y-%m-%dT%H:%M:%SZ")
    failures: list[Any] = []
    for state in ("Failed", "TimedOut", "Terminated"):
        try:
            async for wf in client.list_workflows(
                f"ExecutionStatus = '{state}' AND StartTime > '{cutoff}'"
            ):
                failures.append(wf)
        except Exception as exc:
            checks.append(Check("temporal.failures", WARN, f"visibility query for {state} failed: {exc}"))
            break

    if failures:
        failures.sort(key=lambda w: w.start_time, reverse=True)
        by_type: dict[str, int] = {}
        for wf in failures:
            by_type[wf.workflow_type] = by_type.get(wf.workflow_type, 0) + 1
        summary = ", ".join(f"{t}×{n}" for t, n in sorted(by_type.items(), key=lambda kv: -kv[1]))
        checks.append(Check(
            "temporal.failures", WARN if len(failures) < 10 else FAIL,
            f"{len(failures)} failed/timed-out runs in last {failure_window_h}h: {summary}",
            {"count": len(failures), "by_type": by_type},
        ))
        # Pull the terminal failure message for the freshest few — this is
        # usually the whole diagnosis (which activity, which exception).
        for wf in failures[:5]:
            msg = await _terminal_failure_message(client, wf)
            checks.append(Check(
                f"temporal.failure.{wf.id}", WARN,
                f"{wf.workflow_type} started {age_str((now - wf.start_time).total_seconds())} ago"
                + (f": {msg}" if msg else ""),
            ))
    else:
        checks.append(Check("temporal.failures", OK, f"no failed runs in last {failure_window_h}h"))

    # -- running workflows: stuck long-runners + retrying activities ----------
    stuck: list[str] = []
    running: list[Any] = []
    try:
        async for wf in client.list_workflows("ExecutionStatus = 'Running'"):
            running.append(wf)
            run_h = (now - wf.start_time).total_seconds() / 3600
            # ingest-lightning legitimately streams ~50 min; everything else
            # in this system should finish within its schedule interval.
            if run_h > stuck_after_h:
                stuck.append(f"{wf.workflow_type}({wf.id}) {run_h:.1f}h")
    except Exception as exc:
        checks.append(Check("temporal.running", WARN, f"visibility query failed: {exc}"))
    if stuck:
        checks.append(Check(
            "temporal.stuck", WARN,
            f"{len(stuck)} workflow(s) running > {stuck_after_h}h: {'; '.join(stuck[:5])}",
        ))
    else:
        checks.append(Check("temporal.stuck", OK, f"no workflows running > {stuck_after_h}h"))

    # Activities retrying inside otherwise-healthy runs are the early warning
    # for the failures above. Ingest activities heartbeat structured dicts
    # ({"phase": "download"|"grid"|"render", ...}), so surface those too.
    retrying = 0
    for wf in running[:20]:
        try:
            desc = await client.get_workflow_handle(wf.id, run_id=wf.run_id).describe()
        except Exception:
            continue
        # The Python SDK doesn't surface pending activities as a dataclass
        # field — they live on the raw proto DescribeWorkflowExecutionResponse.
        raw = getattr(desc, "raw_description", None)
        for act in getattr(raw, "pending_activities", None) or []:
            if act.attempt <= 1:
                continue
            retrying += 1
            failure_msg = act.last_failure.message if act.HasField("last_failure") else ""
            hb = _heartbeat_summary(act)
            checks.append(Check(
                f"temporal.retrying.{wf.id}", WARN,
                f"{act.activity_type.name} attempt {act.attempt}/{act.maximum_attempts or '∞'}"
                + (f", last failure: {failure_msg}" if failure_msg else "")
                + (f", heartbeat: {hb}" if hb else ""),
            ))
    if not retrying:
        checks.append(Check("temporal.retrying", OK, "no activities on attempt > 1"))

    return checks


def _heartbeat_summary(act: Any) -> str | None:
    """Ingest activities heartbeat structured dicts (e.g. {"phase": "render",
    "timestamp": ...}) — decode the first payload for live progress."""
    try:
        payloads = act.heartbeat_details.payloads
        if not payloads:
            return None
        import temporalio.converter as conv
        values = conv.default().payload_converter.from_payloads(list(payloads))
        return str(values[0])[:120] if values else None
    except Exception:
        return None


async def _terminal_failure_message(client: Any, wf: Any) -> str | None:
    try:
        handle = client.get_workflow_handle(wf.id, run_id=wf.run_id)
        history = await handle.fetch_history()
        for event in reversed(history.events):
            attrs = getattr(event, "workflow_execution_failed_event_attributes", None)
            if attrs and attrs.failure and attrs.failure.message:
                cause = attrs.failure.cause
                inner = f" ← {cause.message}" if cause and cause.message else ""
                return f"{attrs.failure.message}{inner}"
    except Exception:
        pass
    return None
