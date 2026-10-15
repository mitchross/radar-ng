//! Temporal Schedule definitions and seeding gates (spec-temporal sections 2.2, 6 and 10.5).
//!
//! [`all_schedules`] reproduces `temporal/schedules/seed.py` exactly: the same
//! 12 schedules, in the same order, with the same intervals, jitter, catch-up
//! windows, execution timeouts, queues and arguments. Two Rust-only schedules
//! (`ingest-gfs-radar`, `ingest-uv`) follow them. The seeder overwrites live
//! definitions on every worker start, so any drift here silently changes running
//! schedules.

use serde_json::{Value, json};

use crate::wire::{
    self, ALERTS_TASK_QUEUE, AUX_TASK_QUEUE, HRRR_TASK_QUEUE, MRMS_TASK_QUEUE, NOWCAST_TASK_QUEUE,
    WorkerRole,
};

/// Prefix of a schedule's action workflow ID (`sched-<schedule_id>`).
pub const SCHEDULE_ACTION_ID_PREFIX: &str = "sched-";

/// Schedule ID of the MRMS base reflectivity schedule.
pub const SCHEDULE_INGEST_MRMS_BASE: &str = "ingest-mrms-base";
/// Schedule ID of the MRMS composite reflectivity schedule.
pub const SCHEDULE_INGEST_MRMS_COMPOSITE: &str = "ingest-mrms-composite";
/// Schedule ID of the HRRR schedule.
pub const SCHEDULE_INGEST_HRRR: &str = "ingest-hrrr";
/// Schedule ID of the air-quality schedule.
pub const SCHEDULE_INGEST_AIRQUALITY: &str = "ingest-airquality";
/// Schedule ID of the lightning schedule.
pub const SCHEDULE_INGEST_LIGHTNING: &str = "ingest-lightning";
/// Schedule ID of the tropical schedule.
pub const SCHEDULE_INGEST_TROPICAL: &str = "ingest-tropical";
/// Schedule ID of the nowcast backstop schedule (also the MRMS kick target by default).
pub const SCHEDULE_NOWCAST: &str = "nowcast";
/// Schedule ID of the tile cleanup schedule.
pub const SCHEDULE_TILE_CLEANUP: &str = "tile-cleanup";
/// Schedule ID of the NWS alerts schedule.
pub const SCHEDULE_POLL_ALERTS: &str = "poll-alerts";
/// Schedule ID of the Open-Meteo GFS 0.13 degree schedule.
pub const SCHEDULE_OPEN_METEO_SYNC_GFS: &str = "open-meteo-sync-gfs";
/// Schedule ID of the Open-Meteo GFS 0.25 degree schedule.
pub const SCHEDULE_OPEN_METEO_SYNC_GFS025: &str = "open-meteo-sync-gfs025";
/// Schedule ID of the Open-Meteo HRRR schedule.
pub const SCHEDULE_OPEN_METEO_SYNC_HRRR: &str = "open-meteo-sync-hrrr";
/// Schedule ID of the GFS long-range radar schedule (`radar-gfs`).
pub const SCHEDULE_INGEST_GFS_RADAR: &str = "ingest-gfs-radar";
/// Schedule ID of the UV Index schedule (`uv`).
pub const SCHEDULE_INGEST_UV: &str = "ingest-uv";

/// `GFS013_VARIABLES` (`seed.py`), exact comma-joined list.
pub const GFS013_VARIABLES: &str = "temperature_2m,relative_humidity_2m,precipitation,showers,snowfall_water_equivalent,frozen_precipitation_percent,cloud_cover,wind_u_component_10m,wind_v_component_10m,shortwave_radiation,uv_index";
/// `GFS025_SURFACE_VARIABLES` (`seed.py`), exact comma-joined list.
pub const GFS025_SURFACE_VARIABLES: &str =
    "wind_gusts_10m,pressure_msl,cape,lifted_index,visibility,categorical_freezing_rain";
/// `HRRR_VARIABLES` (`seed.py`), exact comma-joined list.
pub const HRRR_VARIABLES: &str = "temperature_2m,relative_humidity_2m,precipitation,snowfall_water_equivalent,frozen_precipitation_percent,categorical_freezing_rain,cloud_cover,wind_u_component_10m,wind_v_component_10m,wind_gusts_10m,pressure_msl,cape,lifted_index,visibility,shortwave_radiation";

/// Catch-up window of the fast (1 to 2 minute) schedules: 5 minutes.
pub const FAST_CATCHUP_S: u64 = 5 * 60;
/// Jitter of the fast schedules: 20 seconds.
pub const FAST_JITTER_S: u64 = 20;
/// Default catch-up window for every other schedule: 1 hour.
pub const DEFAULT_CATCHUP_S: u64 = 60 * 60;
/// MRMS poll interval: 1 minute.
pub const MRMS_POLL_INTERVAL_S: u64 = 60;

/// Overlap policy of a schedule. The only policy used is `SKIP` (a slow run
/// means the next fire is dropped).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OverlapPolicy {
    /// `ScheduleOverlapPolicy.SKIP`.
    Skip,
}

/// One Temporal Schedule definition (spec-temporal section 6.1).
///
/// The schedule has no cron, calendar, timezone or start/end time, and no
/// retry policy or memo. It is created unpaused.
#[derive(Debug, Clone, PartialEq)]
pub struct ScheduleDef {
    /// Schedule ID.
    pub id: &'static str,
    /// Workflow type started on each fire.
    pub workflow_type: &'static str,
    /// Fire interval in seconds.
    pub interval_s: u64,
    /// Jitter in seconds, `None` when unset.
    pub jitter_s: Option<u64>,
    /// Catch-up window in seconds.
    pub catchup_window_s: u64,
    /// Workflow execution timeout in seconds (`max_runtime`).
    pub execution_timeout_s: u64,
    /// Workflow arguments, one JSON value per positional argument.
    pub args: Vec<Value>,
    /// Declared task queue. Applied only when `USE_ISOLATED_TASK_QUEUES == "1"`
    /// (see [`schedule_action_queue`]).
    pub task_queue: &'static str,
    /// Overlap policy.
    pub overlap: OverlapPolicy,
    /// Whether the schedule pauses after a failed run. Always `false` here.
    pub pause_on_failure: bool,
}

impl ScheduleDef {
    fn new(
        id: &'static str,
        workflow_type: &'static str,
        interval_s: u64,
        execution_timeout_s: u64,
        task_queue: &'static str,
    ) -> Self {
        Self {
            id,
            workflow_type,
            interval_s,
            jitter_s: None,
            catchup_window_s: DEFAULT_CATCHUP_S,
            execution_timeout_s,
            args: Vec::new(),
            task_queue,
            overlap: OverlapPolicy::Skip,
            pause_on_failure: false,
        }
    }

    fn fast(mut self) -> Self {
        self.catchup_window_s = FAST_CATCHUP_S;
        self.jitter_s = Some(FAST_JITTER_S);
        self
    }

    fn with_catchup(mut self, catchup_window_s: u64) -> Self {
        self.catchup_window_s = catchup_window_s;
        self
    }

    fn with_args(mut self, args: Vec<Value>) -> Self {
        self.args = args;
        self
    }

    /// The action workflow ID prefix, `sched-<id>` (the server adds a per-fire suffix).
    pub fn action_workflow_id(&self) -> String {
        action_workflow_id(self.id)
    }
}

/// The action workflow ID prefix for a schedule: `"sched-" + schedule_id`.
pub fn action_workflow_id(schedule_id: &str) -> String {
    format!("{SCHEDULE_ACTION_ID_PREFIX}{schedule_id}")
}

/// The 12 `seed.py` schedules in their order, then the two global-model ones. Seed-time queue
/// routing is applied by [`schedule_action_queue`], not here.
pub fn all_schedules() -> Vec<ScheduleDef> {
    vec![
        ScheduleDef::new(
            SCHEDULE_INGEST_MRMS_BASE,
            wire::WORKFLOW_INGEST_MRMS,
            MRMS_POLL_INTERVAL_S,
            6 * 60,
            MRMS_TASK_QUEUE,
        )
        .fast()
        .with_args(vec![json!({
            "mrms_prefix": "CONUS/MergedBaseReflectivityQC_00.50",
            "layer_name": "radar",
        })]),
        ScheduleDef::new(
            SCHEDULE_INGEST_MRMS_COMPOSITE,
            wire::WORKFLOW_INGEST_MRMS,
            MRMS_POLL_INTERVAL_S,
            6 * 60,
            MRMS_TASK_QUEUE,
        )
        .fast()
        .with_args(vec![json!({
            "mrms_prefix": "CONUS/MergedReflectivityComposite_00.50",
            "layer_name": "radar-composite",
        })]),
        ScheduleDef::new(
            SCHEDULE_INGEST_HRRR,
            wire::WORKFLOW_INGEST_HRRR,
            15 * 60,
            45 * 60,
            HRRR_TASK_QUEUE,
        ),
        ScheduleDef::new(
            SCHEDULE_INGEST_AIRQUALITY,
            wire::WORKFLOW_INGEST_AIRQUALITY,
            30 * 60,
            75 * 60,
            AUX_TASK_QUEUE,
        ),
        ScheduleDef::new(
            SCHEDULE_INGEST_LIGHTNING,
            wire::WORKFLOW_INGEST_LIGHTNING,
            60 * 60,
            56 * 60,
            AUX_TASK_QUEUE,
        ),
        ScheduleDef::new(
            SCHEDULE_INGEST_TROPICAL,
            wire::WORKFLOW_INGEST_TROPICAL,
            60 * 60,
            5 * 60,
            AUX_TASK_QUEUE,
        ),
        ScheduleDef::new(
            SCHEDULE_NOWCAST,
            wire::WORKFLOW_NOWCAST,
            2 * 60,
            12 * 60,
            NOWCAST_TASK_QUEUE,
        )
        .fast(),
        ScheduleDef::new(
            SCHEDULE_TILE_CLEANUP,
            wire::WORKFLOW_TILE_CLEANUP,
            60 * 60,
            15 * 60,
            AUX_TASK_QUEUE,
        ),
        ScheduleDef::new(
            SCHEDULE_POLL_ALERTS,
            wire::WORKFLOW_POLL_ALERTS,
            5 * 60,
            4 * 60 + 30,
            ALERTS_TASK_QUEUE,
        )
        .with_catchup(10 * 60),
        ScheduleDef::new(
            SCHEDULE_OPEN_METEO_SYNC_GFS,
            wire::WORKFLOW_OPEN_METEO_SYNC,
            6 * 60 * 60,
            60 * 60,
            AUX_TASK_QUEUE,
        )
        .with_args(vec![json!({
            "model": "ncep_gfs013",
            "variables": GFS013_VARIABLES,
            "past_days": 2,
        })]),
        ScheduleDef::new(
            SCHEDULE_OPEN_METEO_SYNC_GFS025,
            wire::WORKFLOW_OPEN_METEO_SYNC,
            6 * 60 * 60,
            60 * 60,
            AUX_TASK_QUEUE,
        )
        .with_args(vec![json!({
            "model": "ncep_gfs025",
            "variables": GFS025_SURFACE_VARIABLES,
            "past_days": 2,
        })]),
        ScheduleDef::new(
            SCHEDULE_OPEN_METEO_SYNC_HRRR,
            wire::WORKFLOW_OPEN_METEO_SYNC,
            60 * 60,
            55 * 60,
            AUX_TASK_QUEUE,
        )
        .with_args(vec![json!({
            "model": "ncep_hrrr_conus",
            "variables": HRRR_VARIABLES,
            "past_days": 1,
        })]),
        // GFS posts a run every 6 h; a 30 min poll picks it up soon after its last hour lands.
        ScheduleDef::new(
            SCHEDULE_INGEST_GFS_RADAR,
            wire::WORKFLOW_INGEST_GLOBAL_MODEL,
            30 * 60,
            40 * 60,
            HRRR_TASK_QUEUE,
        )
        .with_args(vec![json!("radar-gfs")]),
        ScheduleDef::new(
            SCHEDULE_INGEST_UV,
            wire::WORKFLOW_INGEST_GLOBAL_MODEL,
            60 * 60,
            40 * 60,
            HRRR_TASK_QUEUE,
        )
        .with_args(vec![json!("uv")]),
    ]
}

/// The task queue a schedule's action uses: the declared queue when
/// `use_isolated` is true, otherwise the legacy queue (`seed.py:297-301`).
pub fn schedule_action_queue(def: &ScheduleDef, use_isolated: bool) -> &str {
    wire::schedule_task_queue(def.task_queue, use_isolated)
}

/// Whether this process seeds schedules.
///
/// Mirrors `temporal/worker.py`: `SEED_SCHEDULES == "1"` seeds; when unset,
/// only the `legacy` and `all` roles seed; any other set value disables seeding.
/// `SKIP_SCHEDULE_SEED == "1"` always disables seeding. Values are passed raw
/// (no trimming), and both match exactly.
pub fn should_seed_schedules(
    role: WorkerRole,
    seed_schedules: Option<&str>,
    skip_schedule_seed: Option<&str>,
) -> bool {
    let seed = match seed_schedules {
        Some(value) => value == "1",
        None => matches!(role, WorkerRole::Legacy | WorkerRole::All),
    };
    seed && skip_schedule_seed != Some("1")
}

/// [`should_seed_schedules`] reading `SEED_SCHEDULES` and `SKIP_SCHEDULE_SEED` from the environment.
pub fn should_seed_schedules_from_env(role: WorkerRole) -> bool {
    let seed = std::env::var(wire::ENV_SEED_SCHEDULES).ok();
    let skip = std::env::var(wire::ENV_SKIP_SCHEDULE_SEED).ok();
    should_seed_schedules(role, seed.as_deref(), skip.as_deref())
}
