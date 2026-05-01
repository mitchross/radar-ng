from datetime import timedelta

from temporalio import workflow


@workflow.defn(name="PollAlertsWorkflow")
class PollAlertsWorkflow:
    """New workflow — polls NWS active alerts every 5 minutes and signals
    matching `WatchStormWorkflow` instances when a new alert affects them.

    Phase 0 stub. Lands in Phase 3 with storm-watch.
    """

    @workflow.run
    async def run(self) -> None:
        workflow.logger.info("PollAlertsWorkflow stub")
        await workflow.sleep(timedelta(seconds=0))
