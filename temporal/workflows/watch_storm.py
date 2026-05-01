from dataclasses import dataclass
from datetime import timedelta

from temporalio import workflow


@dataclass
class WatchStormInput:
    user_id: str
    storm_cell_id: str
    lat: float
    lng: float


@dataclass
class WatchStormState:
    last_frame_ts: int | None = None
    last_change_kind: str | None = None
    last_notified_at: int | None = None


@workflow.defn(name="WatchStormWorkflow")
class WatchStormWorkflow:
    """Entity workflow: one running instance per (user, storm_cell). Polls
    radar frames, detects changes, sends push notifications. Receives signals
    from `PollAlertsWorkflow` when severe alerts overlap the watched cell.

    Workflow ID convention: `watch:{user_id}:{storm_cell_id}`.
    Continue-as-new every 24h or 1000 frame comparisons.

    Phase 0 stub. Lands in Phase 3.
    """

    def __init__(self) -> None:
        self._state = WatchStormState()
        self._unpinned = False
        self._alert_match = None

    @workflow.signal(name="unpinSignal")
    def unpin(self) -> None:
        self._unpinned = True

    @workflow.signal(name="alertMatchSignal")
    def alert_match(self, alert_id: str) -> None:
        self._alert_match = alert_id

    @workflow.query(name="getCurrentState")
    def get_state(self) -> WatchStormState:
        return self._state

    @workflow.run
    async def run(self, input: WatchStormInput) -> None:
        workflow.logger.info(
            "WatchStormWorkflow stub for user=%s storm=%s",
            input.user_id,
            input.storm_cell_id,
        )
        # Phase 3: poll loop with detect_storm_change + send_push_notification
        await workflow.wait_condition(lambda: self._unpinned, timeout=timedelta(hours=24))
