from datetime import timedelta

from temporalio import workflow


@workflow.defn(name="IngestTropicalWorkflow")
class IngestTropicalWorkflow:
    """Replaces CronJob `ingest-tropical`. Runs hourly via Temporal Schedule.

    Phase 0 stub. Activity port lands in Phase 2.
    """

    @workflow.run
    async def run(self) -> None:
        workflow.logger.info("IngestTropicalWorkflow stub")
        await workflow.sleep(timedelta(seconds=0))
