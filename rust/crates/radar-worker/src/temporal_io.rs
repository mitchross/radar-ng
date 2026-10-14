//! Temporal client adapters for the sibling crates' abstractions: the nowcast kicker and the WatchStorm
//! signaler. The sibling crates have no Temporal types, so the worker implements their traits here.

use std::time::Duration;

use anyhow::Context;
use async_trait::async_trait;
use futures::StreamExt;
use radar_alerts::alerts::{WatchCenter, WatchSignaler};
use radar_ingest::NowcastKicker;
use radar_types::payloads::watch_storm::WatchStormState;
use temporalio_client::envconfig::LoadClientConfigProfileOptions;
use temporalio_client::{
    Client, ClientOptions, Connection, WorkflowListOptions, WorkflowQueryOptions,
    WorkflowSignalOptions,
    schedules::{ScheduleOverlapPolicy, TriggerScheduleOptions},
};

use crate::workflows::watch_storm::WatchStormWorkflow;

/// Visibility query for running storm watches (spec 5.2).
pub const RUNNING_WATCHES_QUERY: &str =
    "WorkflowType='WatchStormWorkflow' AND ExecutionStatus='Running'";

/// Connect with the envconfig defaults: `TEMPORAL_ADDRESS` (default `localhost:7233`), `TEMPORAL_NAMESPACE`
/// (default `default`), and optional TLS and API-key variables. Plaintext when TLS is unset (spec 3.7).
pub async fn connect() -> anyhow::Result<Client> {
    let (connection_options, client_options) =
        ClientOptions::load_from_config(LoadClientConfigProfileOptions::default())
            .map_err(|err| anyhow::anyhow!("loading Temporal client configuration: {err}"))?;
    let connection = Connection::connect(connection_options)
        .await
        .context("connecting to Temporal")?;
    Client::new(connection, client_options).context("creating Temporal client")
}

/// Triggers the nowcast schedule with `BUFFER_ONE` overlap (spec 5.3). The caller enforces the timeout.
#[derive(Clone)]
pub struct ScheduleKicker {
    client: Client,
}

impl ScheduleKicker {
    /// Wrap a connected client.
    pub fn new(client: Client) -> Self {
        Self { client }
    }
}

#[async_trait]
impl NowcastKicker for ScheduleKicker {
    async fn trigger(
        &self,
        schedule_id: &str,
        timestamp: &str,
        _timeout: Duration,
    ) -> anyhow::Result<()> {
        tracing::debug!(schedule_id, timestamp, "triggering nowcast schedule");
        self.client
            .get_schedule_handle(schedule_id)
            .trigger(
                ScheduleOverlapPolicy::BufferOne,
                TriggerScheduleOptions::default(),
            )
            .await
            .context("triggering nowcast schedule")?;
        Ok(())
    }
}

/// Lists, queries and signals WatchStorm executions over the Temporal client.
#[derive(Clone)]
pub struct TemporalWatchSignaler {
    client: Client,
}

impl TemporalWatchSignaler {
    /// Wrap a connected client.
    pub fn new(client: Client) -> Self {
        Self { client }
    }
}

#[async_trait]
impl WatchSignaler for TemporalWatchSignaler {
    async fn list_running_watches(&self) -> anyhow::Result<Vec<String>> {
        let mut stream = self
            .client
            .list_workflows(RUNNING_WATCHES_QUERY, WorkflowListOptions::default());
        let mut ids = Vec::new();
        while let Some(execution) = stream.next().await {
            ids.push(
                execution
                    .context("listing running storm watches")?
                    .id()
                    .to_owned(),
            );
        }
        Ok(ids)
    }

    async fn watch_center(&self, workflow_id: &str) -> anyhow::Result<Option<WatchCenter>> {
        let handle = self
            .client
            .get_workflow_handle::<WatchStormWorkflow>(workflow_id);
        let state: WatchStormState = handle
            .query(
                WatchStormWorkflow::get_current_state,
                (),
                WorkflowQueryOptions::default(),
            )
            .await
            .context("querying watch state")?;
        Ok(Some(WatchCenter {
            lat: state.lat,
            lng: state.lng,
        }))
    }

    async fn signal_alert_match(&self, workflow_id: &str, alert_id: &str) -> anyhow::Result<()> {
        let handle = self
            .client
            .get_workflow_handle::<WatchStormWorkflow>(workflow_id);
        handle
            .signal(
                WatchStormWorkflow::alert_match,
                alert_id.to_owned(),
                WorkflowSignalOptions::default(),
            )
            .await
            .context("signalling alert match")?;
        Ok(())
    }
}
