"""PollAlertsWorkflow — server-side NWS alert poll.

Replaces the mobile-only NWS poll: by running it server-side we can push
severe alerts to phones that are backgrounded or asleep. Schedule fires
every 5 minutes, OverlapPolicy.SKIP.

Pipeline:
  1. fetch_nws_active_alerts — diff vs last seen, return new alert geometry
  2. for each new alert: signal_matching_storm_watches → fans out
     `alertMatchSignal` to matching running WatchStormWorkflow instances
"""

from __future__ import annotations

from dataclasses import dataclass, field
from datetime import timedelta

from temporalio import workflow
from temporalio.common import RetryPolicy

with workflow.unsafe.imports_passed_through():
    from backend.api.api.storm_watch_activities import (
        FetchAlertsResult,
        SignalWatchesInput,
        SignalWatchesResult,
        fetch_nws_active_alerts,
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

        signaled = 0
        for alert_id, geometry in normalized:
            if not geometry:
                # Zoneless alerts can never match a watch polygon — the
                # activity would return matched=0 immediately. Skipping here
                # saves an activity execution per alert.
                continue
            res: SignalWatchesResult = await workflow.execute_activity(
                signal_matching_storm_watches,
                SignalWatchesInput(alert_id=alert_id, geometry=geometry),
                start_to_close_timeout=timedelta(seconds=60),
                retry_policy=_RETRY,
            )
            signaled += res.matched

        return PollAlertsResult(
            total=fetched.alert_count,
            new=len(fetched.new_alert_ids),
            signaled_watches=signaled,
            new_ids=fetched.new_alert_ids,
        )
