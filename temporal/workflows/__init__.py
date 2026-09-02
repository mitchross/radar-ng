"""Canonical registry for every radar-ng Temporal workflow type.

The worker and replay suite both consume this registry. Keep compatibility-only
workflow types here until their retained histories can no longer be replayed.
"""

from temporal.workflows.ingest_airquality import IngestAirQualityWorkflow
from temporal.workflows.ingest_hrrr import IngestHrrrWorkflow
from temporal.workflows.ingest_lightning import IngestLightningWorkflow
from temporal.workflows.ingest_mrms import IngestMrmsWorkflow
from temporal.workflows.ingest_tropical import IngestTropicalWorkflow
from temporal.workflows.nowcast import NowcastWorkflow
from temporal.workflows.open_meteo_sync import OpenMeteoSyncWorkflow
from temporal.workflows.poll_alerts import PollAlertsWorkflow
from temporal.workflows.register_push_token import (
    DeletePushTokenWorkflow,
    RegisterPushTokenWorkflow,
)
from temporal.workflows.tile_cleanup import TileCleanupWorkflow
from temporal.workflows.watch_storm import WatchStormWorkflow


# Keys are the externally stable Temporal workflow type names. RegisterPushToken
# and DeletePushToken are no longer started by the API (tokens must not enter new
# histories), but remain registered so retained executions can still replay.
WORKFLOW_REGISTRY: dict[str, type] = {
    "IngestMrmsWorkflow": IngestMrmsWorkflow,
    "IngestHrrrWorkflow": IngestHrrrWorkflow,
    "IngestAirQualityWorkflow": IngestAirQualityWorkflow,
    "IngestLightningWorkflow": IngestLightningWorkflow,
    "IngestTropicalWorkflow": IngestTropicalWorkflow,
    "NowcastWorkflow": NowcastWorkflow,
    "TileCleanupWorkflow": TileCleanupWorkflow,
    "PollAlertsWorkflow": PollAlertsWorkflow,
    "WatchStormWorkflow": WatchStormWorkflow,
    "RegisterPushTokenWorkflow": RegisterPushTokenWorkflow,
    "DeletePushTokenWorkflow": DeletePushTokenWorkflow,
    "OpenMeteoSyncWorkflow": OpenMeteoSyncWorkflow,
}

ALL_WORKFLOWS: tuple[type, ...] = tuple(WORKFLOW_REGISTRY.values())
