//! Worker assembly: role registrations, runtime and metrics, Worker Versioning, supervision, and graceful
//! shutdown (spec section 3).

use std::collections::BTreeMap;
use std::sync::Arc;
use std::time::Duration;

use anyhow::Context;
use radar_alerts::alerts::AlertsConfig;
use radar_alerts::push::{PushConfig, PushSender};
use radar_aux::aqm::AqmConfig;
use radar_aux::open_meteo::OpenMeteoConfig;
use radar_aux::tropical::TropicalConfig;
use radar_core::env::Paths;
use radar_core::push_tokens::PushTokenStore;
use radar_ingest::{IngestConfig, IngestCtx};
use radar_nowcast::NowcastConfig;
use radar_storms::watch::WatchConfig;
use radar_types::schedules::all_schedules;
use radar_types::wire::{self, WorkerRole};
use temporalio_client::Client;
use temporalio_common::WorkflowDefinition;
use temporalio_common::telemetry::metrics::CoreMeter;
use temporalio_common::telemetry::{
    PrometheusExporterOptions, TelemetryOptions, start_prometheus_metric_exporter,
};
use temporalio_common::worker::{
    VersioningBehavior, WorkerDeploymentOptions, WorkerDeploymentVersion, build_id_from_current_exe,
};
use temporalio_sdk::runtime::worker_tuner::{FixedSizeSlotSupplier, TunerHolder, WorkerTuner};
use temporalio_sdk::runtime::{PollerBehavior, RuntimeOptions};
use temporalio_sdk::workflow_replayer::WorkflowReplayerOptions;
use temporalio_sdk::workflows::WorkflowImplementation;
use temporalio_sdk::{Runtime, Worker, WorkerOptions, WorkflowRegistrationError};

use crate::activities::alerts::{AlertsActivities, AlertsDeps};
use crate::activities::aqm::AqmActivities;
use crate::activities::aux::AuxActivities;
use crate::activities::global_model::GlobalModelActivities;
use crate::activities::hrrr::HrrrActivities;
use crate::activities::mrms::MrmsActivities;
use crate::activities::nowcast::NowcastActivities;
use crate::activities::open_meteo::OpenMeteoActivities;
use crate::activities::smoke::SmokeActivities;
use crate::config::WorkerConfig;
use crate::health::health_file_loop;
use crate::seed::{
    BASE_DELAY, MAX_ATTEMPTS, MAX_DELAY, Outcome, reconcile_schedules_once, seed_with_retry,
};
use crate::temporal_io::{ScheduleKicker, TemporalWatchSignaler, connect};
use crate::watchdog::{watch_schedules, watched_schedules};
use crate::workflows::airquality::IngestAirQualityWorkflow;
use crate::workflows::deployment_smoke::RadarDeploymentSmokeWorkflow;
use crate::workflows::global_model::IngestGlobalModelWorkflow;
use crate::workflows::hrrr::IngestHrrrWorkflow;
use crate::workflows::lightning::IngestLightningWorkflow;
use crate::workflows::mrms::IngestMrmsWorkflow;
use crate::workflows::nowcast::NowcastWorkflow;
use crate::workflows::open_meteo_sync::OpenMeteoSyncWorkflow;
use crate::workflows::poll_alerts::PollAlertsWorkflow;
use crate::workflows::push_tokens::{DeletePushTokenWorkflow, RegisterPushTokenWorkflow};
use crate::workflows::tile_cleanup::TileCleanupWorkflow;
use crate::workflows::tropical::IngestTropicalWorkflow;
use crate::workflows::watch_storm::WatchStormWorkflow;

/// How long the worker runs alone before background work starts. The SDK exposes no running-state check, so
/// this stands in for the Python `is_running` poll (spec 3.4). A worker that fails within this window starts
/// no background work.
pub const POLLER_START_GRACE: Duration = Duration::from_millis(250);

/// How long a cancelled or timed-out activity's blocking work may keep running before the worker exits.
/// The restart releases the flocks and memory that work holds.
pub const ABANDONED_WORK_GRACE: Duration = Duration::from_secs(60);

/// Grace period for the open-meteo pool, which the Python worker hard-codes to 25 s.
pub const OPEN_METEO_GRACEFUL_SHUTDOWN: Duration = Duration::from_secs(25);
/// Local-activity and Nexus slot counts. The Python worker does not set them (spec 3.3), so 100 is an assumed
/// SDK default that this port has not verified against the Python SDK.
pub const DEFAULT_SLOTS: usize = 100;

/// Workflow and activity names one role registers (spec 3.2). The open-meteo pool is not a
/// `ROLE_CONFIG` key in Python; it registers only `open_meteo_sync`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Registration {
    /// Workflow type names.
    pub workflows: Vec<&'static str>,
    /// Activity names.
    pub activities: Vec<&'static str>,
}

/// The activity names every role except `legacy`/`all` gets for the deployment smoke gate.
const SMOKE_ACTIVITY: &str = wire::ACTIVITY_RADAR_DEPLOYMENT_SMOKE;
const SMOKE_WORKFLOW: &str = wire::WORKFLOW_DEPLOYMENT_SMOKE;

/// Registration table for a role (spec 3.2, `worker.py:140-216`). `legacy` and `all` register everything
/// except `open_meteo_sync`, which runs only on the open-meteo pool.
pub fn registration(role: WorkerRole) -> Registration {
    let mut workflows: Vec<&'static str> = match role {
        WorkerRole::Legacy | WorkerRole::All => wire::ALL_WORKFLOW_TYPES.to_vec(),
        WorkerRole::Mrms => vec![wire::WORKFLOW_INGEST_MRMS],
        WorkerRole::Nowcast => vec![wire::WORKFLOW_NOWCAST],
        WorkerRole::Hrrr => vec![
            wire::WORKFLOW_INGEST_HRRR,
            wire::WORKFLOW_INGEST_GLOBAL_MODEL,
        ],
        WorkerRole::Aux => vec![
            wire::WORKFLOW_INGEST_LIGHTNING,
            wire::WORKFLOW_INGEST_TROPICAL,
            wire::WORKFLOW_INGEST_AIRQUALITY,
            wire::WORKFLOW_TILE_CLEANUP,
            wire::WORKFLOW_OPEN_METEO_SYNC,
        ],
        WorkerRole::Alerts => vec![
            wire::WORKFLOW_POLL_ALERTS,
            wire::WORKFLOW_WATCH_STORM,
            wire::WORKFLOW_REGISTER_PUSH_TOKEN,
            wire::WORKFLOW_DELETE_PUSH_TOKEN,
        ],
        WorkerRole::OpenMeteo => vec![],
    };
    let mut activities: Vec<&'static str> = match role {
        WorkerRole::Legacy | WorkerRole::All => wire::ALL_ACTIVITY_NAMES
            .iter()
            .copied()
            .filter(|name| *name != wire::ACTIVITY_OPEN_METEO_SYNC)
            .collect(),
        WorkerRole::Mrms => vec![
            wire::ACTIVITY_MRMS_LIST_UNPROCESSED_KEYS,
            wire::ACTIVITY_MRMS_PROCESS_FRAME,
            wire::ACTIVITY_MRMS_MARK_PROCESSED,
            wire::ACTIVITY_MRMS_CLEANUP,
        ],
        WorkerRole::Nowcast => vec![wire::ACTIVITY_NOWCAST_RUN],
        WorkerRole::Hrrr => vec![
            wire::ACTIVITY_HRRR_FIND_LATEST_RUN,
            wire::ACTIVITY_HRRR_HORIZON_FOR_RUN,
            wire::ACTIVITY_HRRR_PROCESS_FORECAST_HOUR,
            wire::ACTIVITY_HRRR_PUBLISH_RUN,
            wire::ACTIVITY_HRRR_MARK_PROCESSED,
            wire::ACTIVITY_HRRR_CLEANUP,
            wire::ACTIVITY_GLOBAL_MODEL_FIND_LATEST_RUN,
            wire::ACTIVITY_GLOBAL_MODEL_PROCESS_HOUR,
            wire::ACTIVITY_GLOBAL_MODEL_PUBLISH_RUN,
            wire::ACTIVITY_GLOBAL_MODEL_MARK_PROCESSED,
            wire::ACTIVITY_GLOBAL_MODEL_CLEANUP,
        ],
        WorkerRole::Aux => vec![
            wire::ACTIVITY_LIGHTNING_CONSUME_STREAM,
            wire::ACTIVITY_TROPICAL_FETCH_AND_PUBLISH,
            wire::ACTIVITY_AQM_FIND_LATEST_RUN,
            wire::ACTIVITY_AQM_RENDER_CHUNK,
            wire::ACTIVITY_AQM_PUBLISH_RUN,
            wire::ACTIVITY_AQM_MARK_PROCESSED,
            wire::ACTIVITY_AQM_CLEANUP,
            wire::ACTIVITY_TILE_CLEANUP_SWEEP,
        ],
        WorkerRole::Alerts => vec![
            wire::ACTIVITY_PERSIST_PUSH_TOKEN,
            wire::ACTIVITY_DELETE_PUSH_TOKEN,
            wire::ACTIVITY_COMPARE_RADAR_FRAMES,
            wire::ACTIVITY_DETECT_STORM_CHANGE,
            wire::ACTIVITY_FAN_OUT_PUSH_TO_USER,
            wire::ACTIVITY_FETCH_NWS_ACTIVE_ALERTS,
            wire::ACTIVITY_MARK_ALERTS_SEEN,
            wire::ACTIVITY_SIGNAL_MATCHING_STORM_WATCHES,
            wire::ACTIVITY_SEND_PUSH_NOTIFICATION,
        ],
        WorkerRole::OpenMeteo => vec![wire::ACTIVITY_OPEN_METEO_SYNC],
    };
    if role != WorkerRole::OpenMeteo && role != WorkerRole::Legacy && role != WorkerRole::All {
        workflows.push(SMOKE_WORKFLOW);
        activities.push(SMOKE_ACTIVITY);
    }
    Registration {
        workflows,
        activities,
    }
}

/// Build the tuner from the configured activity slots.
fn tuner(cfg: &WorkerConfig) -> WorkerTuner {
    WorkerTuner::from(
        TunerHolder::builder()
            .workflow_task_slot_supplier(FixedSizeSlotSupplier::new(
                crate::config::WORKFLOW_TASK_SLOTS,
            ))
            .activity_task_slot_supplier(FixedSizeSlotSupplier::new(cfg.max_concurrent_activities))
            .local_activity_task_slot_supplier(FixedSizeSlotSupplier::new(DEFAULT_SLOTS))
            .nexus_task_slot_supplier(FixedSizeSlotSupplier::new(DEFAULT_SLOTS))
            .build(),
    )
}

/// Worker Versioning block (spec 3.6). Versioning is off unless both names are set.
fn deployment_options(cfg: &WorkerConfig) -> WorkerDeploymentOptions {
    match &cfg.versioning {
        Some(v) => WorkerDeploymentOptions::new(
            WorkerDeploymentVersion::builder()
                .deployment_name(v.deployment_name.clone())
                .build_id(v.build_id.clone())
                .build(),
        )
        .use_worker_versioning(true)
        .default_versioning_behavior(VersioningBehavior::Pinned)
        .build(),
        None => WorkerDeploymentOptions::from_build_id(build_id_from_current_exe().to_owned()),
    }
}

/// Clients and configs the role's activity implementers need.
struct Deps {
    ingest: Option<IngestCtx>,
    aqm: Option<AqmActivities>,
    aux: Option<AuxActivities>,
    nowcast: Option<NowcastConfig>,
    alerts: Option<AlertsDeps>,
    open_meteo: Option<OpenMeteoConfig>,
}

fn needs(role: WorkerRole, pool: WorkerRole) -> bool {
    role == pool || matches!(role, WorkerRole::Legacy | WorkerRole::All)
}

impl Deps {
    /// Build only what the role registers. Configuration errors surface here, before polling starts.
    fn for_role(role: WorkerRole, client: &Client) -> anyhow::Result<Self> {
        let http = radar_grib::http::client().context("building HTTP client")?;
        let ingest = if needs(role, WorkerRole::Mrms) || needs(role, WorkerRole::Hrrr) {
            let kicker: Arc<dyn radar_ingest::NowcastKicker> =
                Arc::new(ScheduleKicker::new(client.clone()));
            Some(IngestCtx::new(IngestConfig::from_env()?, Some(kicker))?)
        } else {
            None
        };
        let aqm = if needs(role, WorkerRole::Aux) {
            Some(AqmActivities::new(AqmConfig::from_env()?, http.clone()))
        } else {
            None
        };
        let aux = if needs(role, WorkerRole::Aux) {
            Some(AuxActivities::new(http.clone(), TropicalConfig::from_env()))
        } else {
            None
        };
        let nowcast = if needs(role, WorkerRole::Nowcast) {
            Some(NowcastConfig::from_env()?)
        } else {
            None
        };
        let alerts = if needs(role, WorkerRole::Alerts) {
            let paths = Paths::from_env();
            let tokens = PushTokenStore::new(&paths.state_dir);
            let push = PushSender::new(PushConfig::from_env(), tokens.clone())?;
            Some(AlertsDeps {
                alerts: AlertsConfig::from_env(),
                push,
                tokens,
                grid_dir: paths.grid_dir.clone(),
                signaler: Arc::new(TemporalWatchSignaler::new(client.clone())),
                watch: WatchConfig::from_env(),
            })
        } else {
            None
        };
        let open_meteo = if role == WorkerRole::OpenMeteo {
            Some(OpenMeteoConfig::from_env())
        } else {
            None
        };
        Ok(Self {
            ingest,
            aqm,
            aux,
            nowcast,
            alerts,
            open_meteo,
        })
    }
}

/// Something workflow implementations can be registered on. The worker's [`WorkerOptions`] and the replay
/// gate's [`WorkflowReplayerOptions`] both implement it, so `radar worker` and the replay test share one
/// registration path ([`register_workflows`]).
pub trait WorkflowTarget {
    /// Register workflow implementation `W`.
    fn add<W>(&mut self) -> Result<(), WorkflowRegistrationError>
    where
        W: WorkflowImplementation,
        <W::Run as WorkflowDefinition>::Input: Send;
}

impl WorkflowTarget for WorkerOptions {
    fn add<W>(&mut self) -> Result<(), WorkflowRegistrationError>
    where
        W: WorkflowImplementation,
        <W::Run as WorkflowDefinition>::Input: Send,
    {
        self.register_workflow::<W>().map(|_| ())
    }
}

impl WorkflowTarget for WorkflowReplayerOptions {
    fn add<W>(&mut self) -> Result<(), WorkflowRegistrationError>
    where
        W: WorkflowImplementation,
        <W::Run as WorkflowDefinition>::Input: Send,
    {
        self.register_workflow::<W>().map(|_| ())
    }
}

/// Register the role's workflow types on `target`. Each workflow appears once. This is the registration
/// `radar worker` uses; the offline replay gate (`tests/replay.rs`) calls it with role `all`.
pub fn register_workflows<T: WorkflowTarget>(
    target: &mut T,
    role: WorkerRole,
) -> anyhow::Result<()> {
    for name in registration(role).workflows {
        match name {
            wire::WORKFLOW_INGEST_MRMS => target.add::<IngestMrmsWorkflow>()?,
            wire::WORKFLOW_INGEST_HRRR => target.add::<IngestHrrrWorkflow>()?,
            wire::WORKFLOW_INGEST_AIRQUALITY => target.add::<IngestAirQualityWorkflow>()?,
            wire::WORKFLOW_INGEST_LIGHTNING => target.add::<IngestLightningWorkflow>()?,
            wire::WORKFLOW_INGEST_TROPICAL => target.add::<IngestTropicalWorkflow>()?,
            wire::WORKFLOW_NOWCAST => target.add::<NowcastWorkflow>()?,
            wire::WORKFLOW_TILE_CLEANUP => target.add::<TileCleanupWorkflow>()?,
            wire::WORKFLOW_OPEN_METEO_SYNC => target.add::<OpenMeteoSyncWorkflow>()?,
            wire::WORKFLOW_POLL_ALERTS => target.add::<PollAlertsWorkflow>()?,
            wire::WORKFLOW_WATCH_STORM => target.add::<WatchStormWorkflow>()?,
            wire::WORKFLOW_REGISTER_PUSH_TOKEN => target.add::<RegisterPushTokenWorkflow>()?,
            wire::WORKFLOW_DELETE_PUSH_TOKEN => target.add::<DeletePushTokenWorkflow>()?,
            wire::WORKFLOW_DEPLOYMENT_SMOKE => target.add::<RadarDeploymentSmokeWorkflow>()?,
            wire::WORKFLOW_INGEST_GLOBAL_MODEL => target.add::<IngestGlobalModelWorkflow>()?,
            other => anyhow::bail!("no workflow implementation for {other}"),
        }
    }
    Ok(())
}

/// Register activity implementers for the role. Each group registers only its own method names.
fn add_activities(
    mut builder: WorkerOptionsBuilder,
    role: WorkerRole,
    deps: Deps,
) -> anyhow::Result<WorkerOptionsBuilder> {
    if let Some(ctx) = deps.ingest {
        if needs(role, WorkerRole::Mrms) {
            builder = builder.register_activities(MrmsActivities::new(ctx.clone()));
        }
        if needs(role, WorkerRole::Hrrr) {
            builder = builder.register_activities(GlobalModelActivities::new(ctx.clone()));
            builder = builder.register_activities(HrrrActivities::new(ctx));
        }
    }
    if let Some(aqm) = deps.aqm {
        builder = builder.register_activities(aqm);
    }
    if let Some(aux) = deps.aux {
        builder = builder.register_activities(aux);
    }
    if let Some(cfg) = deps.nowcast {
        builder = builder.register_activities(NowcastActivities::new(cfg));
    }
    if let Some(alerts) = deps.alerts {
        builder = builder.register_activities(AlertsActivities::new(alerts));
    }
    if let Some(cfg) = deps.open_meteo {
        builder = builder.register_activities(OpenMeteoActivities::new(cfg));
    }
    builder = builder.register_activities(SmokeActivities::new());
    Ok(builder)
}

type WorkerOptionsBuilder = temporalio_sdk::WorkerOptionsBuilder;

/// Assemble the worker options for a role.
fn worker_options(cfg: &WorkerConfig, deps: Deps) -> anyhow::Result<WorkerOptions> {
    let builder = WorkerOptions::new(cfg.task_queue.clone());
    let builder = add_activities(builder, cfg.role, deps)?;
    let mut options = builder
        .deployment_options(deployment_options(cfg))
        .tuner(tuner(cfg))
        .activity_task_poller_behavior(PollerBehavior::SimpleMaximum(
            cfg.max_concurrent_activity_polls,
        ))
        .graceful_shutdown_period(cfg.graceful_shutdown)
        .build();
    register_workflows(&mut options, cfg.role)?;
    Ok(options)
}

/// Telemetry for the runtime. Metrics use Prometheus at `TEMPORAL_METRICS_BIND` unless it is `off`. The
/// returned exporter handle must stay alive for the life of the worker.
fn telemetry_options(
    cfg: &WorkerConfig,
) -> anyhow::Result<(TelemetryOptions, Option<Box<dyn std::any::Any + Send>>)> {
    match cfg.metrics_bind {
        Some(addr) => {
            let started = start_prometheus_metric_exporter(
                PrometheusExporterOptions::builder()
                    .socket_addr(addr)
                    .build(),
            )
            .context("starting Prometheus metrics exporter")?;
            let meter: Arc<dyn CoreMeter> = started.meter.clone();
            let telemetry = TelemetryOptions::builder().metrics(meter).build();
            Ok((telemetry, Some(Box::new(started))))
        }
        None => Ok((TelemetryOptions::builder().build(), None)),
    }
}

/// Resolve on SIGTERM or SIGINT. Either one starts the graceful drain.
async fn wait_for_termination() {
    use tokio::signal::unix::{SignalKind, signal};
    match (
        signal(SignalKind::terminate()),
        signal(SignalKind::interrupt()),
    ) {
        (Ok(mut term), Ok(mut int)) => {
            tokio::select! {
                _ = term.recv() => {}
                _ = int.recv() => {}
            }
        }
        _ => std::future::pending::<()>().await,
    }
}

/// Run the worker for `cfg.role` until shutdown (spec 3.3, 3.4, 3.5).
///
/// Validation and client construction happen first. The SDK's `Worker::run` future is not `Send`, so it runs
/// on this task. Background tasks (health loop, and for the seeding pool the reconciler and watchdog) start
/// only after [`POLLER_START_GRACE`] and are aborted when the worker returns. A reconciliation failure is
/// logged and never stops the worker. SIGTERM or SIGINT drains in-flight work for `TEMPORAL_GRACEFUL_SHUTDOWN_S`.
pub async fn run_worker(cfg: WorkerConfig) -> anyhow::Result<()> {
    radar_core::heartbeat::exit_when_abandoned_after(ABANDONED_WORK_GRACE);
    if cfg.role == WorkerRole::OpenMeteo {
        return run_open_meteo_pool(cfg).await;
    }
    let (telemetry, _prom) = telemetry_options(&cfg)?;
    let runtime_options = RuntimeOptions::builder()
        .telemetry_options(telemetry)
        .build()
        .map_err(anyhow::Error::msg)?;
    let runtime =
        Runtime::from_current_tokio(runtime_options).context("creating Temporal runtime")?;
    let client = connect().await?;
    let namespace = temporalio_client::NamespacedClient::namespace(&client);
    let deps = Deps::for_role(cfg.role, &client)?;
    let options = worker_options(&cfg, deps)?;
    let mut worker = Worker::new(&runtime, client.clone(), options).context("creating worker")?;
    let shutdown = worker.shutdown_handle();
    tokio::spawn(async move {
        wait_for_termination().await;
        tracing::info!("shutdown requested; draining in-flight activities");
        shutdown();
    });

    let mut run = std::pin::pin!(worker.run());
    let outcome = tokio::select! {
        biased;
        // Startup failed before the pollers settled: no background work has started.
        outcome = &mut run => return finish(outcome),
        () = tokio::time::sleep(POLLER_START_GRACE) => {
            let background = start_background(&cfg, &client, &namespace);
            let outcome = run.await;
            for task in background {
                task.abort();
            }
            outcome
        }
    };
    finish(outcome)
}

/// Health loop always; the seeder's reconciler and the watchdog only when this process seeds (spec 3.4).
fn start_background(
    cfg: &WorkerConfig,
    client: &Client,
    namespace: &str,
) -> Vec<tokio::task::JoinHandle<()>> {
    let mut background = vec![tokio::spawn(health_file_loop(
        client.clone(),
        cfg.health_file.clone(),
        cfg.alive_file.clone(),
    ))];
    if cfg.seed_schedules {
        let isolated = cfg.use_isolated_task_queues;
        let (seed_client, seed_ns) = (client.clone(), namespace.to_owned());
        background.push(tokio::spawn(async move {
            reconcile_schedules_once(&seed_client, &seed_ns, isolated).await;
        }));
        background.push(tokio::spawn(watch_schedules(
            client.clone(),
            watched_schedules(),
        )));
    }
    background
}

fn finish(outcome: Result<(), temporalio_sdk::WorkerRunError>) -> anyhow::Result<()> {
    match outcome {
        Ok(()) => {
            tracing::info!("worker drained, exiting");
            Ok(())
        }
        Err(err) => Err(anyhow::anyhow!("worker stopped: {err}")),
    }
}

/// The open-meteo pool: activity only, no workflows, no metrics, no tracing, 25 s drain.
///
/// The health loop runs here too (F2). The liveness probe needs `/tmp/worker-alive`, and the loop is what
/// touches it. The loop is a separate task, so it keeps ticking while a multi-minute `open_meteo_sync`
/// activity runs. It starts right away, before the poller, because this pool has no start-up grace.
async fn run_open_meteo_pool(cfg: WorkerConfig) -> anyhow::Result<()> {
    let runtime_options = RuntimeOptions::builder()
        .build()
        .map_err(anyhow::Error::msg)?;
    let runtime =
        Runtime::from_current_tokio(runtime_options).context("creating Temporal runtime")?;
    let client = connect().await?;
    let health = tokio::spawn(health_file_loop(
        client.clone(),
        cfg.health_file.clone(),
        cfg.alive_file.clone(),
    ));
    let options = WorkerOptions::new(cfg.task_queue.clone())
        .register_activities(OpenMeteoActivities::new(OpenMeteoConfig::from_env()))
        .deployment_options(deployment_options(&cfg))
        .graceful_shutdown_period(OPEN_METEO_GRACEFUL_SHUTDOWN)
        .build();
    let mut worker =
        Worker::new(&runtime, client, options).context("creating open-meteo worker")?;
    let shutdown = worker.shutdown_handle();
    tokio::spawn(async move {
        wait_for_termination().await;
        shutdown();
    });
    let outcome = worker.run().await;
    health.abort();
    finish(outcome)
}

/// One-shot schedule seeding, for an operator or job. Creates or updates every declared schedule with bounded
/// retries and returns the per-schedule outcome.
pub async fn seed_schedules(cfg: WorkerConfig) -> anyhow::Result<BTreeMap<String, Outcome>> {
    let client = connect().await?;
    let namespace = temporalio_client::NamespacedClient::namespace(&client);
    let outcomes = seed_with_retry(
        &client,
        &namespace,
        cfg.use_isolated_task_queues,
        MAX_ATTEMPTS,
        BASE_DELAY,
        MAX_DELAY,
    )
    .await
    .map_err(anyhow::Error::from)?;
    Ok(outcomes)
}

/// Run the schedule watchdog forever (read-only observer; spec 7).
pub async fn run_watchdog(_cfg: WorkerConfig) -> anyhow::Result<()> {
    let client = connect().await?;
    watch_schedules(client, all_schedules()).await;
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::collections::{BTreeMap, BTreeSet};

    use radar_types::wire::{self, WorkerRole};

    use super::*;

    /// Every pool that owns a set of workflows or activities. Legacy and all are the union and are checked
    /// separately.
    const POOLS: [WorkerRole; 6] = [
        WorkerRole::Mrms,
        WorkerRole::Nowcast,
        WorkerRole::Hrrr,
        WorkerRole::Aux,
        WorkerRole::Alerts,
        WorkerRole::OpenMeteo,
    ];

    fn set(names: &[&'static str]) -> BTreeSet<&'static str> {
        names.iter().copied().collect()
    }

    fn all_but_open_meteo() -> BTreeSet<&'static str> {
        wire::ALL_ACTIVITY_NAMES
            .iter()
            .copied()
            .filter(|n| *n != wire::ACTIVITY_OPEN_METEO_SYNC)
            .collect()
    }

    #[test]
    fn legacy_and_all_register_the_full_registry_minus_open_meteo() {
        for role in [WorkerRole::Legacy, WorkerRole::All] {
            let reg = registration(role);
            assert_eq!(reg.workflows.len(), 14, "{role:?} workflow count");
            assert_eq!(
                set(&reg.workflows),
                set(&wire::ALL_WORKFLOW_TYPES),
                "{role:?} workflows"
            );
            assert_eq!(reg.activities.len(), 34, "{role:?} activity count");
            assert_eq!(
                set(&reg.activities),
                all_but_open_meteo(),
                "{role:?} activities"
            );
            assert!(!reg.activities.contains(&wire::ACTIVITY_OPEN_METEO_SYNC));
        }
        assert_eq!(
            registration(WorkerRole::Legacy),
            registration(WorkerRole::All)
        );
    }

    #[test]
    fn every_registered_name_is_a_wire_constant() {
        for role in POOLS {
            let reg = registration(role);
            for name in &reg.workflows {
                assert!(
                    wire::ALL_WORKFLOW_TYPES.contains(name),
                    "{role:?} unknown workflow {name}"
                );
            }
            for name in &reg.activities {
                assert!(
                    wire::ALL_ACTIVITY_NAMES.contains(name),
                    "{role:?} unknown activity {name}"
                );
            }
        }
    }

    #[test]
    fn pools_cover_every_activity_exactly_once_apart_from_smoke() {
        let mut owners: BTreeMap<&str, usize> = BTreeMap::new();
        for role in POOLS {
            for name in registration(role).activities {
                *owners.entry(name).or_default() += 1;
            }
        }
        for name in wire::ALL_ACTIVITY_NAMES {
            let expected = match name {
                wire::ACTIVITY_RADAR_DEPLOYMENT_SMOKE => 5,
                _ => 1,
            };
            assert_eq!(
                owners.get(name).copied().unwrap_or(0),
                expected,
                "owners of {name}"
            );
        }
    }

    #[test]
    fn pools_cover_every_workflow_exactly_once_apart_from_smoke() {
        let mut owners: BTreeMap<&str, usize> = BTreeMap::new();
        for role in POOLS {
            for name in registration(role).workflows {
                *owners.entry(name).or_default() += 1;
            }
        }
        for name in wire::ALL_WORKFLOW_TYPES {
            let expected = match name {
                wire::WORKFLOW_DEPLOYMENT_SMOKE => 5,
                _ => 1,
            };
            assert_eq!(
                owners.get(name).copied().unwrap_or(0),
                expected,
                "owners of {name}"
            );
        }
    }

    #[test]
    fn deployment_smoke_is_on_every_role_except_legacy_all_and_open_meteo() {
        for role in POOLS {
            let reg = registration(role);
            let has_workflow = reg.workflows.contains(&wire::WORKFLOW_DEPLOYMENT_SMOKE);
            let has_activity = reg
                .activities
                .contains(&wire::ACTIVITY_RADAR_DEPLOYMENT_SMOKE);
            let expected = role != WorkerRole::OpenMeteo;
            assert_eq!(has_workflow, expected, "{role:?} smoke workflow");
            assert_eq!(has_activity, expected, "{role:?} smoke activity");
        }
    }

    #[test]
    fn pool_tables_match_spec_3_2() {
        let smoke_wf = wire::WORKFLOW_DEPLOYMENT_SMOKE;
        let smoke_act = wire::ACTIVITY_RADAR_DEPLOYMENT_SMOKE;

        let mrms = registration(WorkerRole::Mrms);
        assert_eq!(
            set(&mrms.workflows),
            set(&[wire::WORKFLOW_INGEST_MRMS, smoke_wf])
        );
        assert_eq!(
            set(&mrms.activities),
            set(&[
                wire::ACTIVITY_MRMS_LIST_UNPROCESSED_KEYS,
                wire::ACTIVITY_MRMS_PROCESS_FRAME,
                wire::ACTIVITY_MRMS_MARK_PROCESSED,
                wire::ACTIVITY_MRMS_CLEANUP,
                smoke_act,
            ])
        );

        let nowcast = registration(WorkerRole::Nowcast);
        assert_eq!(
            set(&nowcast.workflows),
            set(&[wire::WORKFLOW_NOWCAST, smoke_wf])
        );
        assert_eq!(
            set(&nowcast.activities),
            set(&[wire::ACTIVITY_NOWCAST_RUN, smoke_act])
        );

        let hrrr = registration(WorkerRole::Hrrr);
        assert_eq!(
            set(&hrrr.workflows),
            set(&[
                wire::WORKFLOW_INGEST_HRRR,
                wire::WORKFLOW_INGEST_GLOBAL_MODEL,
                smoke_wf
            ])
        );
        assert_eq!(hrrr.activities.len(), 12);

        let aux = registration(WorkerRole::Aux);
        assert_eq!(
            set(&aux.workflows),
            set(&[
                wire::WORKFLOW_INGEST_LIGHTNING,
                wire::WORKFLOW_INGEST_TROPICAL,
                wire::WORKFLOW_INGEST_AIRQUALITY,
                wire::WORKFLOW_TILE_CLEANUP,
                wire::WORKFLOW_OPEN_METEO_SYNC,
                smoke_wf,
            ])
        );
        assert_eq!(aux.activities.len(), 9);
        assert!(!aux.activities.contains(&wire::ACTIVITY_OPEN_METEO_SYNC));

        let alerts = registration(WorkerRole::Alerts);
        assert_eq!(
            set(&alerts.workflows),
            set(&[
                wire::WORKFLOW_POLL_ALERTS,
                wire::WORKFLOW_WATCH_STORM,
                wire::WORKFLOW_REGISTER_PUSH_TOKEN,
                wire::WORKFLOW_DELETE_PUSH_TOKEN,
                smoke_wf,
            ])
        );
        assert_eq!(alerts.activities.len(), 10);

        let open_meteo = registration(WorkerRole::OpenMeteo);
        assert!(open_meteo.workflows.is_empty());
        assert_eq!(open_meteo.activities, vec![wire::ACTIVITY_OPEN_METEO_SYNC]);
    }

    #[test]
    fn deployment_options_opt_in_only_with_both_versioning_names() {
        let mut cfg = WorkerConfig::from_lookup(&|_| None).expect("defaults are valid");
        let off = deployment_options(&cfg);
        assert!(!off.use_worker_versioning);
        assert_eq!(off.default_versioning_behavior, None);

        cfg.versioning = Some(crate::config::Versioning {
            deployment_name: "radar-ng".to_owned(),
            build_id: "2026-10-14a".to_owned(),
        });
        let on = deployment_options(&cfg);
        assert!(on.use_worker_versioning);
        assert_eq!(
            on.default_versioning_behavior,
            Some(VersioningBehavior::Pinned)
        );
        assert_eq!(on.version.build_id, "2026-10-14a");
        assert_eq!(on.version.deployment_name, "radar-ng");
    }
}
