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

from dataclasses import dataclass, field, replace
from datetime import timedelta

from temporalio import workflow
from temporalio.common import RetryPolicy
from temporalio.exceptions import ActivityError
from temporalio.exceptions import ApplicationError

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
    handoff: WatchStormHandoff | None = None


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


@dataclass
class WatchStormHandoff:
    schema_version: int = 1
    state: WatchStormState = field(default_factory=WatchStormState)
    pending_alert_ids: list[str] = field(default_factory=list)


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
    @workflow.init
    def __init__(self, inp: WatchStormInput) -> None:
        if inp.handoff is not None and inp.handoff.schema_version != 1:
            raise ApplicationError("Unsupported storm-watch handoff schema", non_retryable=True)
        self._state = replace(inp.handoff.state) if inp.handoff else WatchStormState()
        self._unpinned = False
        self._continue_requested = False
        self._pending_alert_ids = list(inp.handoff.pending_alert_ids) if inp.handoff else []

    @workflow.signal(name="unpinSignal")
    def unpin(self) -> None:
        self._unpinned = True

    @workflow.signal(name="wakeUpSignal")
    def wake_up(self) -> None:
        self._observe_continue_request()

    @workflow.signal(name="alertMatchSignal")
    def alert_match(self, alert_id: str) -> None:
        # Keep historical last-alert behavior during replay; new handlers queue every signal.
        if not workflow.patched("watch-storm-alert-queue-v1"):
            self._pending_alert_ids.clear()
        self._pending_alert_ids.append(alert_id)
        self._observe_continue_request()

    def _observe_continue_request(self) -> None:
        # The server's notification can clear on the next task; remember it until the safe boundary.
        info = workflow.info()
        self._continue_requested = self._continue_requested or (
            info.is_continue_as_new_suggested()
            or info.is_target_worker_deployment_version_changed()
        )

    @workflow.query(name="getCurrentState")
    def get_state(self) -> WatchStormState:
        return self._state

    @workflow.run
    async def run(self, inp: WatchStormInput) -> WatchStormState:
        deadline = workflow.now() + MAX_RUN_DURATION
        polls_this_run = 0
        self._state.user_id = inp.user_id
        self._state.storm_cell_id = inp.storm_cell_id
        self._state.lat = inp.lat
        self._state.lng = inp.lng
        workflow.logger.info(
            "watch start: user=%s storm=%s @ (%.4f,%.4f)",
            inp.user_id, inp.storm_cell_id, inp.lat, inp.lng,
        )

        while not self._unpinned:
            self._observe_continue_request()
            # Continue-as-new bound — keep history under 50K events.
            if (
                polls_this_run >= MAX_FRAMES_PER_RUN
                or workflow.now() >= deadline
                or (
                    self._continue_requested
                    and workflow.patched("watch-storm-version-boundary-v1")
                )
            ):
                workflow.logger.info("continue-as-new (polls=%d)", self._state.poll_count)
                if workflow.patched("watch-storm-handoff-v1"):
                    await workflow.wait_condition(workflow.all_handlers_finished)
                    if self._unpinned:
                        break
                    workflow.continue_as_new(
                        replace(inp, handoff=WatchStormHandoff(
                            state=replace(self._state),
                            pending_alert_ids=list(self._pending_alert_ids),
                        )),
                        initial_versioning_behavior=workflow.ContinueAsNewVersioningBehavior.AUTO_UPGRADE,
                    )
                workflow.continue_as_new(inp)

            # Drain any pending alert match signal first (high-priority).
            if self._pending_alert_ids:
                alert_id = self._pending_alert_ids.pop(0)
                await self._push_alert(inp, alert_id)
                self._observe_continue_request()

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
            self._observe_continue_request()

            self._state.poll_count += 1
            polls_this_run += 1

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
                self._observe_continue_request()

                if det.kind:
                    await self._push_change(inp, det)
                    self._observe_continue_request()

            try:
                await workflow.wait_condition(
                    lambda: self._unpinned or bool(self._pending_alert_ids),
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
