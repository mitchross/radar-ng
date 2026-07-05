"""PollAlertsWorkflow — server-side NWS alert poll.

Replaces the mobile-only NWS poll: by running it server-side we can push
severe alerts to phones that are backgrounded or asleep. Schedule fires
every 5 minutes, OverlapPolicy.SKIP.

Pipeline (at-least-once):
  1. fetch_nws_active_alerts — diff vs last seen, return new alert geometry
     (read-only: it does NOT commit the seen set)
  2. for each new alert: signal_matching_storm_watches → fans out
     `alertMatchSignal` to matching running WatchStormWorkflow instances;
     a per-alert failure is isolated so the rest of the batch still delivers
  3. mark_alerts_seen — commit only the alerts that were handled; failed
     ones stay unseen and are retried by the next poll (duplicate signals
     are possible after a partial failure — for severe weather that is the
     right trade against a silently dropped alert)
"""

from __future__ import annotations

import asyncio
from dataclasses import dataclass, field
from datetime import timedelta

from temporalio import workflow
from temporalio.common import RetryPolicy
from temporalio.exceptions import ActivityError

with workflow.unsafe.imports_passed_through():
    from backend.api.api.storm_watch_activities import (
        FetchAlertsResult,
        MarkAlertsSeenInput,
        SignalWatchesInput,
        SignalWatchesResult,
        fetch_nws_active_alerts,
        mark_alerts_seen,
        signal_matching_storm_watches,
    )


_RETRY = RetryPolicy(
    initial_interval=timedelta(seconds=2),
    backoff_coefficient=2.0,
    maximum_interval=timedelta(seconds=60),
    maximum_attempts=4,
)


@dataclass
class PollAlertsResult:
    total: int
    new: int
    signaled_watches: int = 0
    failed_alerts: int = 0
    new_ids: list[str] = field(default_factory=list)


@workflow.defn(name="PollAlertsWorkflow")
class PollAlertsWorkflow:
    @workflow.run
    async def run(self) -> PollAlertsResult:
        fetched: FetchAlertsResult = await workflow.execute_activity(
            fetch_nws_active_alerts,
            start_to_close_timeout=timedelta(seconds=60),
            retry_policy=_RETRY,
        )

        # Normalize to (id, geometry) tuples up front — the payload converter
        # hands back AlertForSignal dataclasses, but keeping the loop body free
        # of hasattr/type sniffing means a future payload-shape change can't
        # silently flip code paths on replay.
        normalized: list[tuple[str, dict]] = [(a.alert_id, a.geometry or {}) for a in fetched.new_alerts]

        # Zoneless alerts can never match a watch polygon — the activity
        # would return matched=0 immediately. Nothing to deliver, so they
        # count as handled without an activity execution.
        handled_ids: list[str] = [aid for aid, geo in normalized if not geo]
        to_signal = [(aid, geo) for aid, geo in normalized if geo]

        async def signal_one(alert_id: str, geometry: dict) -> tuple[str, int] | None:
            try:
                res: SignalWatchesResult = await workflow.execute_activity(
                    signal_matching_storm_watches,
                    SignalWatchesInput(alert_id=alert_id, geometry=geometry),
                    start_to_close_timeout=timedelta(seconds=60),
                    retry_policy=_RETRY,
                )
            except ActivityError:
                # This alert stays out of handled_ids: it remains unseen and
                # the next poll (≤5 min) retries it. Without this isolation,
                # one bad alert dropped every alert after it in the batch.
                workflow.logger.warning("signaling failed for alert %s; will retry next poll", alert_id)
                return None
            return (alert_id, res.matched)

        # Fan out concurrently: alerts are independent, and sequential
        # signaling made an outbreak batch (100+ new alerts) — or a few
        # alerts grinding through the full retry ladder — outlast the 5-min
        # schedule window while SKIP dropped the next polls. Wall time is now
        # the slowest single alert; worker slots bound actual concurrency.
        results = await asyncio.gather(*(signal_one(aid, geo) for aid, geo in to_signal))
        signaled = 0
        failed = 0
        for r in results:
            if r is None:
                failed += 1
                continue
            handled_ids.append(r[0])
            signaled += r[1]

        await workflow.execute_activity(
            mark_alerts_seen,
            MarkAlertsSeenInput(handled_ids=handled_ids),
            start_to_close_timeout=timedelta(seconds=30),
            retry_policy=_RETRY,
        )

        return PollAlertsResult(
            total=fetched.alert_count,
            new=len(fetched.new_alert_ids),
            signaled_watches=signaled,
            failed_alerts=failed,
            new_ids=fetched.new_alert_ids,
        )
