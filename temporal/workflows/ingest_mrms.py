from datetime import timedelta

from temporalio import workflow


@workflow.defn(name="IngestMrmsWorkflow")
class IngestMrmsWorkflow:
    """Replaces CronJob `ingest-mrms`. Runs every 2 minutes via Temporal Schedule.

    Phase 0 stub. Activity port lands in Phase 1.
    """

    @workflow.run
    async def run(self) -> None:
        workflow.logger.info("IngestMrmsWorkflow stub — not yet implemented")
        # Phase 1:
        # files = await workflow.execute_activity(poll_mrms_catalog, "MRMS_REFL", ...)
        # for f in files:
        #     key = await workflow.execute_activity(download_grib2, f, ...)
        #     grid = await workflow.execute_activity(decode_grib2, key, ...)
        #     ref = await workflow.execute_activity(build_mbtiles, grid, ...)
        #     await workflow.execute_activity(invalidate_cache, ("reflectivity", grid.ts), ...)
        #     await workflow.execute_activity(publish_manifest, ("reflectivity", grid.ts, ref), ...)
        await workflow.sleep(timedelta(seconds=0))
