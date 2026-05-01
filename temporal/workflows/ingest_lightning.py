from datetime import timedelta

from temporalio import workflow


@workflow.defn(name="IngestLightningWorkflow")
class IngestLightningWorkflow:
    """Replaces CronJob `ingest-lightning`. Runs every 5 minutes via Temporal Schedule.

    Phase 0 stub. Activity port lands in Phase 2.
    """

    @workflow.run
    async def run(self) -> None:
        workflow.logger.info("IngestLightningWorkflow stub")
        await workflow.sleep(timedelta(seconds=0))
