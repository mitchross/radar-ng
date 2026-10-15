//! Temporal wire names: workflow types, activity names, signals, queries, patch
//! IDs, task queues, environment variable names and worker roles.
//!
//! Every string here is part of the wire contract. Changing one breaks in-flight
//! executions and replay (spec-temporal section 10.3). Source: `temporal/task_queues.py`,
//! `temporal/workflows/*.py`, `backend/*/activities.py`, `backend/api/api/storm_watch_activities.py`.

use std::fmt;
use std::str::FromStr;

use thiserror::Error;

// ---------------------------------------------------------------------------
// Workflow types (13 from temporal/workflows/__init__.py, plus one Rust-only)
// ---------------------------------------------------------------------------

/// `RadarDeploymentSmokeWorkflow` (deployment gate, every role).
pub const WORKFLOW_DEPLOYMENT_SMOKE: &str = "RadarDeploymentSmokeWorkflow";
/// `IngestMrmsWorkflow`.
pub const WORKFLOW_INGEST_MRMS: &str = "IngestMrmsWorkflow";
/// `IngestHrrrWorkflow`.
pub const WORKFLOW_INGEST_HRRR: &str = "IngestHrrrWorkflow";
/// `IngestAirQualityWorkflow`.
pub const WORKFLOW_INGEST_AIRQUALITY: &str = "IngestAirQualityWorkflow";
/// `IngestLightningWorkflow`.
pub const WORKFLOW_INGEST_LIGHTNING: &str = "IngestLightningWorkflow";
/// `IngestTropicalWorkflow`.
pub const WORKFLOW_INGEST_TROPICAL: &str = "IngestTropicalWorkflow";
/// `NowcastWorkflow`.
pub const WORKFLOW_NOWCAST: &str = "NowcastWorkflow";
/// `TileCleanupWorkflow`.
pub const WORKFLOW_TILE_CLEANUP: &str = "TileCleanupWorkflow";
/// `OpenMeteoSyncWorkflow`.
pub const WORKFLOW_OPEN_METEO_SYNC: &str = "OpenMeteoSyncWorkflow";
/// `PollAlertsWorkflow`.
pub const WORKFLOW_POLL_ALERTS: &str = "PollAlertsWorkflow";
/// `WatchStormWorkflow` (entity workflow, one per user and storm cell).
pub const WORKFLOW_WATCH_STORM: &str = "WatchStormWorkflow";
/// `RegisterPushTokenWorkflow` (compatibility only; replay of retained histories).
pub const WORKFLOW_REGISTER_PUSH_TOKEN: &str = "RegisterPushTokenWorkflow";
/// `DeletePushTokenWorkflow` (compatibility only; replay of retained histories).
pub const WORKFLOW_DELETE_PUSH_TOKEN: &str = "DeletePushTokenWorkflow";
/// `IngestGlobalModelWorkflow` (Rust-only): GFS long-range radar and the UV Index, one layer per run.
pub const WORKFLOW_INGEST_GLOBAL_MODEL: &str = "IngestGlobalModelWorkflow";

/// All 14 registered workflow type names.
pub const ALL_WORKFLOW_TYPES: [&str; 14] = [
    WORKFLOW_DEPLOYMENT_SMOKE,
    WORKFLOW_INGEST_MRMS,
    WORKFLOW_INGEST_HRRR,
    WORKFLOW_INGEST_AIRQUALITY,
    WORKFLOW_INGEST_LIGHTNING,
    WORKFLOW_INGEST_TROPICAL,
    WORKFLOW_NOWCAST,
    WORKFLOW_TILE_CLEANUP,
    WORKFLOW_OPEN_METEO_SYNC,
    WORKFLOW_POLL_ALERTS,
    WORKFLOW_WATCH_STORM,
    WORKFLOW_REGISTER_PUSH_TOKEN,
    WORKFLOW_DELETE_PUSH_TOKEN,
    WORKFLOW_INGEST_GLOBAL_MODEL,
];

// ---------------------------------------------------------------------------
// Activity names (30 from `@activity.defn(name=...)`, plus five Rust-only)
// ---------------------------------------------------------------------------

/// `radar_deployment_smoke`.
pub const ACTIVITY_RADAR_DEPLOYMENT_SMOKE: &str = "radar_deployment_smoke";

/// `mrms_list_unprocessed_keys`.
pub const ACTIVITY_MRMS_LIST_UNPROCESSED_KEYS: &str = "mrms_list_unprocessed_keys";
/// `mrms_process_frame`.
pub const ACTIVITY_MRMS_PROCESS_FRAME: &str = "mrms_process_frame";
/// `mrms_mark_processed`.
pub const ACTIVITY_MRMS_MARK_PROCESSED: &str = "mrms_mark_processed";
/// `mrms_cleanup`.
pub const ACTIVITY_MRMS_CLEANUP: &str = "mrms_cleanup";

/// `hrrr_find_latest_run`.
pub const ACTIVITY_HRRR_FIND_LATEST_RUN: &str = "hrrr_find_latest_run";
/// `hrrr_horizon_for_run`.
pub const ACTIVITY_HRRR_HORIZON_FOR_RUN: &str = "hrrr_horizon_for_run";
/// `hrrr_process_forecast_hour`.
pub const ACTIVITY_HRRR_PROCESS_FORECAST_HOUR: &str = "hrrr_process_forecast_hour";
/// `hrrr_publish_run`.
pub const ACTIVITY_HRRR_PUBLISH_RUN: &str = "hrrr_publish_run";
/// `hrrr_mark_processed`.
pub const ACTIVITY_HRRR_MARK_PROCESSED: &str = "hrrr_mark_processed";
/// `hrrr_cleanup`.
pub const ACTIVITY_HRRR_CLEANUP: &str = "hrrr_cleanup";

/// `global_model_find_latest_run`.
pub const ACTIVITY_GLOBAL_MODEL_FIND_LATEST_RUN: &str = "global_model_find_latest_run";
/// `global_model_process_hour`.
pub const ACTIVITY_GLOBAL_MODEL_PROCESS_HOUR: &str = "global_model_process_hour";
/// `global_model_publish_run`.
pub const ACTIVITY_GLOBAL_MODEL_PUBLISH_RUN: &str = "global_model_publish_run";
/// `global_model_mark_processed`.
pub const ACTIVITY_GLOBAL_MODEL_MARK_PROCESSED: &str = "global_model_mark_processed";
/// `global_model_cleanup`.
pub const ACTIVITY_GLOBAL_MODEL_CLEANUP: &str = "global_model_cleanup";

/// `aqm_find_latest_run`.
pub const ACTIVITY_AQM_FIND_LATEST_RUN: &str = "aqm_find_latest_run";
/// `aqm_render_chunk`.
pub const ACTIVITY_AQM_RENDER_CHUNK: &str = "aqm_render_chunk";
/// `aqm_publish_run`.
pub const ACTIVITY_AQM_PUBLISH_RUN: &str = "aqm_publish_run";
/// `aqm_mark_processed`.
pub const ACTIVITY_AQM_MARK_PROCESSED: &str = "aqm_mark_processed";
/// `aqm_cleanup`.
pub const ACTIVITY_AQM_CLEANUP: &str = "aqm_cleanup";

/// `lightning_consume_stream`.
pub const ACTIVITY_LIGHTNING_CONSUME_STREAM: &str = "lightning_consume_stream";
/// `tropical_fetch_and_publish`.
pub const ACTIVITY_TROPICAL_FETCH_AND_PUBLISH: &str = "tropical_fetch_and_publish";
/// `nowcast_run`.
pub const ACTIVITY_NOWCAST_RUN: &str = "nowcast_run";
/// `tile_cleanup_sweep`.
pub const ACTIVITY_TILE_CLEANUP_SWEEP: &str = "tile_cleanup_sweep";
/// `open_meteo_sync` (runs on [`OPEN_METEO_TASK_QUEUE`] regardless of the workflow's queue).
pub const ACTIVITY_OPEN_METEO_SYNC: &str = "open_meteo_sync";

/// `persist_push_token`.
pub const ACTIVITY_PERSIST_PUSH_TOKEN: &str = "persist_push_token";
/// `delete_push_token`.
pub const ACTIVITY_DELETE_PUSH_TOKEN: &str = "delete_push_token";
/// `compare_radar_frames`.
pub const ACTIVITY_COMPARE_RADAR_FRAMES: &str = "compare_radar_frames";
/// `detect_storm_change`.
pub const ACTIVITY_DETECT_STORM_CHANGE: &str = "detect_storm_change";
/// `fan_out_push_to_user`.
pub const ACTIVITY_FAN_OUT_PUSH_TO_USER: &str = "fan_out_push_to_user";
/// `fetch_nws_active_alerts`.
pub const ACTIVITY_FETCH_NWS_ACTIVE_ALERTS: &str = "fetch_nws_active_alerts";
/// `mark_alerts_seen`.
pub const ACTIVITY_MARK_ALERTS_SEEN: &str = "mark_alerts_seen";
/// `signal_matching_storm_watches`.
pub const ACTIVITY_SIGNAL_MATCHING_STORM_WATCHES: &str = "signal_matching_storm_watches";
/// `send_push_notification`. Registered but never scheduled by a workflow; the
/// name stays reserved for compatibility (spec-temporal section 8.1).
pub const ACTIVITY_SEND_PUSH_NOTIFICATION: &str = "send_push_notification";

/// All 35 activity names: the 30 the Python code registered (including the reserved
/// `send_push_notification`) and the five Rust-only `global_model_*` ones. The legacy pool serves all
/// of them except `open_meteo_sync`.
pub const ALL_ACTIVITY_NAMES: [&str; 35] = [
    ACTIVITY_RADAR_DEPLOYMENT_SMOKE,
    ACTIVITY_MRMS_LIST_UNPROCESSED_KEYS,
    ACTIVITY_MRMS_PROCESS_FRAME,
    ACTIVITY_MRMS_MARK_PROCESSED,
    ACTIVITY_MRMS_CLEANUP,
    ACTIVITY_HRRR_FIND_LATEST_RUN,
    ACTIVITY_HRRR_HORIZON_FOR_RUN,
    ACTIVITY_HRRR_PROCESS_FORECAST_HOUR,
    ACTIVITY_HRRR_PUBLISH_RUN,
    ACTIVITY_HRRR_MARK_PROCESSED,
    ACTIVITY_HRRR_CLEANUP,
    ACTIVITY_GLOBAL_MODEL_FIND_LATEST_RUN,
    ACTIVITY_GLOBAL_MODEL_PROCESS_HOUR,
    ACTIVITY_GLOBAL_MODEL_PUBLISH_RUN,
    ACTIVITY_GLOBAL_MODEL_MARK_PROCESSED,
    ACTIVITY_GLOBAL_MODEL_CLEANUP,
    ACTIVITY_AQM_FIND_LATEST_RUN,
    ACTIVITY_AQM_RENDER_CHUNK,
    ACTIVITY_AQM_PUBLISH_RUN,
    ACTIVITY_AQM_MARK_PROCESSED,
    ACTIVITY_AQM_CLEANUP,
    ACTIVITY_LIGHTNING_CONSUME_STREAM,
    ACTIVITY_TROPICAL_FETCH_AND_PUBLISH,
    ACTIVITY_NOWCAST_RUN,
    ACTIVITY_TILE_CLEANUP_SWEEP,
    ACTIVITY_OPEN_METEO_SYNC,
    ACTIVITY_PERSIST_PUSH_TOKEN,
    ACTIVITY_DELETE_PUSH_TOKEN,
    ACTIVITY_COMPARE_RADAR_FRAMES,
    ACTIVITY_DETECT_STORM_CHANGE,
    ACTIVITY_FAN_OUT_PUSH_TO_USER,
    ACTIVITY_FETCH_NWS_ACTIVE_ALERTS,
    ACTIVITY_MARK_ALERTS_SEEN,
    ACTIVITY_SIGNAL_MATCHING_STORM_WATCHES,
    ACTIVITY_SEND_PUSH_NOTIFICATION,
];

// ---------------------------------------------------------------------------
// Signals and queries (exact case)
// ---------------------------------------------------------------------------

/// `unpinSignal()`: no arguments. Sets the watch's unpinned flag.
pub const SIGNAL_UNPIN: &str = "unpinSignal";
/// `wakeUpSignal()`: no arguments. Only observes the continue-as-new request.
pub const SIGNAL_WAKE_UP: &str = "wakeUpSignal";
/// `alertMatchSignal(alert_id: str)`: one positional string argument.
pub const SIGNAL_ALERT_MATCH: &str = "alertMatchSignal";
/// Query `getCurrentState()`: returns `WatchStormState`.
pub const QUERY_GET_CURRENT_STATE: &str = "getCurrentState";

// ---------------------------------------------------------------------------
// Patch IDs (`workflow.patched`)
// ---------------------------------------------------------------------------

/// Alert queueing patch: absent in histories that predate it (last alert wins).
pub const PATCH_WATCH_STORM_ALERT_QUEUE_V1: &str = "watch-storm-alert-queue-v1";
/// Version-boundary continue-as-new patch.
pub const PATCH_WATCH_STORM_VERSION_BOUNDARY_V1: &str = "watch-storm-version-boundary-v1";
/// Handoff continue-as-new patch (state and pending alerts carried forward).
pub const PATCH_WATCH_STORM_HANDOFF_V1: &str = "watch-storm-handoff-v1";

// ---------------------------------------------------------------------------
// Task queues (temporal/task_queues.py)
// ---------------------------------------------------------------------------

/// `radar-ng`: all-in-one (legacy and all) pool. Default queue for every schedule
/// while `USE_ISOLATED_TASK_QUEUES != "1"`.
pub const LEGACY_TASK_QUEUE: &str = "radar-ng";
/// `radar-ng-mrms`.
pub const MRMS_TASK_QUEUE: &str = "radar-ng-mrms";
/// `radar-ng-nowcast`.
pub const NOWCAST_TASK_QUEUE: &str = "radar-ng-nowcast";
/// `radar-ng-hrrr`.
pub const HRRR_TASK_QUEUE: &str = "radar-ng-hrrr";
/// `radar-ng-aux`: default queue for schedules that do not name one.
pub const AUX_TASK_QUEUE: &str = "radar-ng-aux";
/// `radar-ng-alerts`: also the API's WatchStorm start queue by default.
pub const ALERTS_TASK_QUEUE: &str = "radar-ng-alerts";
/// `radar-ng-open-meteo`: the open-meteo pool; `open_meteo_sync` always targets it.
pub const OPEN_METEO_TASK_QUEUE: &str = "radar-ng-open-meteo";

// ---------------------------------------------------------------------------
// Environment variable names (spec-temporal section 3.1 and section 2.2)
// ---------------------------------------------------------------------------

/// `WORKER_ROLE`: which pool this process runs (default `legacy`).
pub const ENV_WORKER_ROLE: &str = "WORKER_ROLE";
/// `TEMPORAL_TASK_QUEUE`: poll-queue override (default: the role's queue).
pub const ENV_TEMPORAL_TASK_QUEUE: &str = "TEMPORAL_TASK_QUEUE";
/// `USE_ISOLATED_TASK_QUEUES`: `"1"` routes schedules to their role queues.
pub const ENV_USE_ISOLATED_TASK_QUEUES: &str = "USE_ISOLATED_TASK_QUEUES";
/// `SEED_SCHEDULES`: `"1"` forces seeding; any other set value disables it.
pub const ENV_SEED_SCHEDULES: &str = "SEED_SCHEDULES";
/// `SKIP_SCHEDULE_SEED`: `"1"` disables seeding.
pub const ENV_SKIP_SCHEDULE_SEED: &str = "SKIP_SCHEDULE_SEED";
/// `TEMPORAL_ALERTS_TASK_QUEUE`: API-only WatchStorm start queue override.
pub const ENV_TEMPORAL_ALERTS_TASK_QUEUE: &str = "TEMPORAL_ALERTS_TASK_QUEUE";
/// `NOWCAST_SCHEDULE_ID`: schedule ID the MRMS producer triggers (default `nowcast`).
pub const ENV_NOWCAST_SCHEDULE_ID: &str = "NOWCAST_SCHEDULE_ID";

/// Error returned when a worker role string is not recognised.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
#[error(
    "unknown WORKER_ROLE {0:?}; expected one of legacy, all, mrms, nowcast, hrrr, aux, alerts, open-meteo"
)]
pub struct UnknownWorkerRole(pub String);

/// A worker pool role. Mirrors the Python `ROLE_CONFIG` keys plus the open-meteo pool.
///
/// Deviation (spec-temporal D1): the Python worker has no `open-meteo` role and
/// refuses to boot with it. The Rust worker accepts `open-meteo` (and the
/// `open_meteo` spelling) and maps it to [`OPEN_METEO_TASK_QUEUE`], the pool the
/// docs describe. It never polls a different queue.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum WorkerRole {
    /// All-in-one pool on [`LEGACY_TASK_QUEUE`]. Seeds schedules by default.
    Legacy,
    /// Same queue and registrations as [`WorkerRole::Legacy`].
    All,
    /// MRMS ingest pool.
    Mrms,
    /// Nowcast pool.
    Nowcast,
    /// HRRR ingest pool.
    Hrrr,
    /// Auxiliary ingest pool (lightning, tropical, air quality, tile cleanup, open-meteo workflow).
    Aux,
    /// Alerts and WatchStorm pool.
    Alerts,
    /// Open-meteo sync pool (activity only).
    OpenMeteo,
}

impl WorkerRole {
    /// Every role, in declaration order.
    pub const ALL: [WorkerRole; 8] = [
        WorkerRole::Legacy,
        WorkerRole::All,
        WorkerRole::Mrms,
        WorkerRole::Nowcast,
        WorkerRole::Hrrr,
        WorkerRole::Aux,
        WorkerRole::Alerts,
        WorkerRole::OpenMeteo,
    ];

    /// Canonical lowercase name (`open-meteo` for the open-meteo pool).
    pub fn as_str(self) -> &'static str {
        match self {
            WorkerRole::Legacy => "legacy",
            WorkerRole::All => "all",
            WorkerRole::Mrms => "mrms",
            WorkerRole::Nowcast => "nowcast",
            WorkerRole::Hrrr => "hrrr",
            WorkerRole::Aux => "aux",
            WorkerRole::Alerts => "alerts",
            WorkerRole::OpenMeteo => "open-meteo",
        }
    }

    /// The queue this role polls when no `TEMPORAL_TASK_QUEUE` override is set.
    pub fn default_task_queue(self) -> &'static str {
        match self {
            WorkerRole::Legacy | WorkerRole::All => LEGACY_TASK_QUEUE,
            WorkerRole::Mrms => MRMS_TASK_QUEUE,
            WorkerRole::Nowcast => NOWCAST_TASK_QUEUE,
            WorkerRole::Hrrr => HRRR_TASK_QUEUE,
            WorkerRole::Aux => AUX_TASK_QUEUE,
            WorkerRole::Alerts => ALERTS_TASK_QUEUE,
            WorkerRole::OpenMeteo => OPEN_METEO_TASK_QUEUE,
        }
    }
}

impl fmt::Display for WorkerRole {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl FromStr for WorkerRole {
    type Err = UnknownWorkerRole;

    /// Parses a role the way the Python worker does: surrounding whitespace is
    /// stripped and the match is case-insensitive. `open_meteo` is accepted as
    /// an alias of `open-meteo`.
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.trim().to_ascii_lowercase().as_str() {
            "legacy" => Ok(WorkerRole::Legacy),
            "all" => Ok(WorkerRole::All),
            "mrms" => Ok(WorkerRole::Mrms),
            "nowcast" => Ok(WorkerRole::Nowcast),
            "hrrr" => Ok(WorkerRole::Hrrr),
            "aux" => Ok(WorkerRole::Aux),
            "alerts" => Ok(WorkerRole::Alerts),
            "open-meteo" | "open_meteo" => Ok(WorkerRole::OpenMeteo),
            _ => Err(UnknownWorkerRole(s.to_owned())),
        }
    }
}

/// Schedule action queue rule (spec-temporal section 2.2, `seed.py:297-301`).
///
/// Returns `declared` when isolated queues are enabled, otherwise [`LEGACY_TASK_QUEUE`].
/// Use [`use_isolated_task_queues`] to derive `use_isolated` from the raw env value.
pub fn schedule_task_queue(declared: &str, use_isolated: bool) -> &str {
    if use_isolated {
        declared
    } else {
        LEGACY_TASK_QUEUE
    }
}

/// `USE_ISOLATED_TASK_QUEUES == "1"` (exact match, as in Python).
pub fn use_isolated_task_queues(env_value: Option<&str>) -> bool {
    env_value == Some("1")
}

/// Poll queue for a worker: a non-empty `TEMPORAL_TASK_QUEUE` override, else the role default.
///
/// Deviation: Python uses an empty override verbatim (polling the invalid queue
/// name `""`). Here an empty value is treated as unset.
pub fn poll_task_queue(role: WorkerRole, override_value: Option<&str>) -> String {
    match override_value {
        Some(q) if !q.is_empty() => q.to_owned(),
        _ => role.default_task_queue().to_owned(),
    }
}

/// Start queue for `WatchStormWorkflow` from the API: `TEMPORAL_ALERTS_TASK_QUEUE`
/// (non-empty) or `radar-ng-alerts`. This ignores `USE_ISOLATED_TASK_QUEUES` (D3).
pub fn watch_start_task_queue(env_value: Option<&str>) -> String {
    match env_value {
        Some(q) if !q.is_empty() => q.to_owned(),
        _ => ALERTS_TASK_QUEUE.to_owned(),
    }
}
