from datetime import timedelta

from temporalio import workflow


@workflow.defn(name="NowcastWorkflow")
class NowcastWorkflow:
    """Replaces CronJob `nowcast`. Runs every 2 minutes via Temporal Schedule.

    Calls pysteps in a heartbeating activity (CPU-expensive).

    Phase 0 stub. Activity port lands in Phase 2.
    """

    @workflow.run
    async def run(self) -> None:
        workflow.logger.info("NowcastWorkflow stub")
        await workflow.sleep(timedelta(seconds=0))
