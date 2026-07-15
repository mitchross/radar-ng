"""WatchStormWorkflow — entity workflow, one running instance per
(user_id, storm_cell_id). Hours-to-days lifetime.

  - Polls the latest MRMS frame every POLL_S (default 60s)
  - Compares to the previous sample → detects intensify/dissipate/severe
  - On change: fan out push notification(s) to every registered device token
    for the user
  - On `unpinSignal`: returns cleanly
  - On `alertMatchSignal`: high-priority push regardless of frame change
  - Continue-as-new every 24h or 1000 frame compares (per spec)

Workflow id convention: `watch:{user_id}:{storm_cell_id}` — unique, stable.
"""

from __future__ import annotations

from dataclasses import dataclass
from datetime import timedelta

from temporalio import workflow
from temporalio.common import RetryPolicy
from temporalio.exceptions import ActivityError

with workflow.unsafe.imports_passed_through():
    from backend.api.api.storm_watch_activities import (
        CompareFramesInput,
        CompareFramesResult,
        DetectChangeInput,
        DetectChangeResult,
        FanOutPushInput,
        FanOutPushResult,
        compare_radar_frames,
        detect_storm_change,
        fan_out_push_to_user,
    )


POLL_S = 60
MAX_FRAMES_PER_RUN = 1000
MAX_RUN_DURATION = timedelta(hours=24)


@dataclass
class WatchStormInput:
    user_id: str
    storm_cell_id: str
    lat: float
    lng: float


@dataclass
class WatchStormState:
    user_id: str = ""
    storm_cell_id: str = ""
    lat: float = 0.0
    lng: float = 0.0
    last_frame_ts: str | None = None
    last_max_dbz: float | None = None
    last_change_kind: str | None = None
    last_notified_at: float | None = None
    poll_count: int = 0
    push_count: int = 0


_DEFAULT_RETRY = RetryPolicy(
    initial_interval=timedelta(seconds=2),
    backoff_coefficient=2.0,
    maximum_interval=timedelta(seconds=60),
    maximum_attempts=5,
)
_PUSH_RETRY = RetryPolicy(
    initial_interval=timedelta(seconds=1),
    backoff_coefficient=2.0,
    maximum_interval=timedelta(seconds=30),
    maximum_attempts=3,
)


@workflow.defn(name="WatchStormWorkflow")
class WatchStormWorkflow:
    def __init__(self) -> None:
        self._state = WatchStormState()
        self._unpinned = False
        self._pending_alert_id: str | None = None

    @workflow.signal(name="unpinSignal")
    def unpin(self) -> None:
        self._unpinned = True

    @workflow.signal(name="alertMatchSignal")
    def alert_match(self, alert_id: str) -> None:
        self._pending_alert_id = alert_id

    @workflow.query(name="getCurrentState")
    def get_state(self) -> WatchStormState:
        return self._state

    @workflow.run
    async def run(self, inp: WatchStormInput) -> WatchStormState:
        deadline = workflow.now() + MAX_RUN_DURATION
        self._state.user_id = inp.user_id
        self._state.storm_cell_id = inp.storm_cell_id
        self._state.lat = inp.lat
        self._state.lng = inp.lng
        workflow.logger.info(
            "watch start: user=%s storm=%s @ (%.4f,%.4f)",
            inp.user_id, inp.storm_cell_id, inp.lat, inp.lng,
        )

        while not self._unpinned:
            # Continue-as-new bound — keep history under 50K events.
            if self._state.poll_count >= MAX_FRAMES_PER_RUN or workflow.now() >= deadline:
                workflow.logger.info("continue-as-new (polls=%d)", self._state.poll_count)
                workflow.continue_as_new(inp)

            # Drain any pending alert match signal first (high-priority).
            if self._pending_alert_id is not None:
                alert_id = self._pending_alert_id
                self._pending_alert_id = None
                await self._push_alert(inp, alert_id)

            cmp_in = CompareFramesInput(
                lat=inp.lat,
                lng=inp.lng,
                prev_timestamp=self._state.last_frame_ts,
                prev_max_dbz=self._state.last_max_dbz,
            )
            cmp_out: CompareFramesResult = await workflow.execute_activity(
                compare_radar_frames, cmp_in,
                start_to_close_timeout=timedelta(seconds=30),
                retry_policy=_DEFAULT_RETRY,
            )

            self._state.poll_count += 1

            if cmp_out.sampled:
                self._state.last_frame_ts = cmp_out.curr_timestamp
                self._state.last_max_dbz = cmp_out.curr_max_dbz

                det: DetectChangeResult = await workflow.execute_activity(
                    detect_storm_change,
                    DetectChangeInput(
                        has_prev=cmp_out.has_prev,
                        curr_max_dbz=cmp_out.curr_max_dbz,
                        prev_max_dbz=cmp_out.prev_max_dbz,
                        max_dbz_delta=cmp_out.max_dbz_delta,
                    ),
                    start_to_close_timeout=timedelta(seconds=10),
                    retry_policy=_DEFAULT_RETRY,
                )

                if det.kind:
                    await self._push_change(inp, det)

            try:
                await workflow.wait_condition(
                    lambda: self._unpinned or self._pending_alert_id is not None,
                    timeout=timedelta(seconds=POLL_S),
                )
            except TimeoutError:
                pass

        workflow.logger.info(
            "watch unpinned: user=%s storm=%s polls=%d pushes=%d",
            inp.user_id, inp.storm_cell_id, self._state.poll_count, self._state.push_count,
        )
        return self._state

    async def _push_change(self, inp: WatchStormInput, det: DetectChangeResult) -> None:
        collapse = f"{workflow.info().workflow_id}:{self._state.last_frame_ts}:{det.kind}"
        result = await self._fan_out(
            FanOutPushInput(
                user_id=inp.user_id,
                title=self._title_for(det.kind),
                body=det.summary or "",
                collapse_id=collapse,
                extra={"storm_cell_id": inp.storm_cell_id, "kind": det.kind or ""},
            ),
        )
        self._state.last_change_kind = det.kind
        self._state.last_notified_at = workflow.now().timestamp()
        self._state.push_count += result.sent if result else 0

    async def _push_alert(self, inp: WatchStormInput, alert_id: str) -> None:
        collapse = f"{workflow.info().workflow_id}:alert:{alert_id}"
        result = await self._fan_out(
            FanOutPushInput(
                user_id=inp.user_id,
                title="Severe weather alert near your storm",
                body="NWS issued an alert in your watched area",
                collapse_id=collapse,
                extra={"storm_cell_id": inp.storm_cell_id, "alert_id": alert_id, "kind": "alert"},
            ),
        )
        self._state.last_change_kind = "alert"
        self._state.last_notified_at = workflow.now().timestamp()
        self._state.push_count += result.sent if result else 0

    async def _fan_out(self, push: FanOutPushInput) -> FanOutPushResult | None:
        """Push failure must never kill the watch: fan_out_push_to_user raises
        when EVERY send fails (so transient APNS/FCM outages get the retry
        policy), but past the retry budget — e.g. every token for this user is
        permanently stale — the watch itself keeps polling. The user may
        re-register a token tomorrow; an hours-to-days entity workflow dying
        over one undeliverable notification is the wrong trade."""
        try:
            return await workflow.execute_activity(
                fan_out_push_to_user,
                push,
                start_to_close_timeout=timedelta(seconds=60),
                retry_policy=_PUSH_RETRY,
            )
        except ActivityError:
            workflow.logger.warning("push fan-out failed after retries; watch continues")
            return None

    @staticmethod
    def _title_for(kind: str | None) -> str:
        return {
            "intensifying": "Storm intensifying",
            "dissipating": "Storm weakening",
            "severe": "Severe storm cell",
            "alert": "Severe weather alert",
        }.get(kind or "", "Storm update")
