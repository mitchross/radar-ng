from datetime import timedelta

from temporalio import workflow


@workflow.defn(name="IngestHrrrWorkflow")
class IngestHrrrWorkflow:
    """Replaces CronJob `ingest-hrrr`. Runs every 15 minutes via Temporal Schedule.

    Phase 0 stub. Activity port lands in Phase 2.
    """

    @workflow.run
    async def run(self) -> None:
        workflow.logger.info("IngestHrrrWorkflow stub")
        await workflow.sleep(timedelta(seconds=0))
