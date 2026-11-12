//! Level II pool workflow payloads (`Level2PoolWorkflow`, `level2_tick`, `level2_poll_station`).

use serde::{Deserialize, Serialize};

/// Result of `level2_tick`: stations with live tile demand, at most eight.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Level2Tick {
    /// Demanded station IDs, sorted.
    #[serde(default)]
    pub stations: Vec<String>,
    /// False while the Level II gates are off. The workflow then idles.
    #[serde(default)]
    pub enabled: bool,
}

/// Input of `level2_poll_station`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Level2PollInput {
    /// Station to poll, for example `KGRR`.
    pub station: String,
}

/// Result of `level2_poll_station`. A fallback is a normal result, not an error.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Level2PollResult {
    /// `numeric_ready`, `fallback` or `skipped`.
    #[serde(default)]
    pub status: String,
    /// Fallback or skip reason.
    #[serde(default)]
    pub reason: Option<String>,
}

/// Workflow input. Only per-station failure counts roll over; all other state lives in Postgres.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Level2PoolInput {
    /// Consecutive failed polls per station, for backoff across continue-as-new.
    #[serde(default)]
    pub failures: std::collections::BTreeMap<String, u32>,
}
