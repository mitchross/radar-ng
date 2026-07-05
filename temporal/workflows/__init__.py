from temporal.workflows.ingest_mrms import IngestMrmsWorkflow
from temporal.workflows.ingest_hrrr import IngestHrrrWorkflow
from temporal.workflows.ingest_lightning import IngestLightningWorkflow
from temporal.workflows.ingest_tropical import IngestTropicalWorkflow
from temporal.workflows.nowcast import NowcastWorkflow
from temporal.workflows.tile_cleanup import TileCleanupWorkflow
from temporal.workflows.poll_alerts import PollAlertsWorkflow
from temporal.workflows.watch_storm import WatchStormWorkflow
from temporal.workflows.register_push_token import (
    DeletePushTokenWorkflow,
    RegisterPushTokenWorkflow,
)
from temporal.workflows.open_meteo_sync import OpenMeteoSyncWorkflow

ALL_WORKFLOWS = [
    IngestMrmsWorkflow,
    IngestHrrrWorkflow,
    IngestLightningWorkflow,
    IngestTropicalWorkflow,
    NowcastWorkflow,
    TileCleanupWorkflow,
    PollAlertsWorkflow,
    WatchStormWorkflow,
    RegisterPushTokenWorkflow,
    DeletePushTokenWorkflow,
    OpenMeteoSyncWorkflow,
]
