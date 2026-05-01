from datetime import timedelta

from temporalio import workflow


@workflow.defn(name="TileCleanupWorkflow")
class TileCleanupWorkflow:
    """Replaces CronJob `tile-cleanup`. Runs hourly via Temporal Schedule.

    Phase 0 stub. Activity port lands in Phase 2.
    """

    @workflow.run
    async def run(self) -> None:
        workflow.logger.info("TileCleanupWorkflow stub")
        await workflow.sleep(timedelta(seconds=0))
