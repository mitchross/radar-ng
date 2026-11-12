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

/// Workflow input. Carries nothing: state lives in Postgres, so a rollover loses nothing.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Level2PoolInput {}
