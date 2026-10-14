//! radar-types: Temporal wire contracts for radar-ng.
//!
//! This crate is the single Rust source of truth for everything that crosses the
//! Temporal boundary: workflow / activity / signal / query names, task queues,
//! patch IDs, the JSON payload structs (wire-compatible with the Python workers,
//! `json/plain` payloads, snake_case field names), the schedule table, retry and
//! timeout constants, and workflow ID derivation.
//!
//! Modules:
//! - [`wire`]: names, queues, env var names, worker roles, queue routing.
//! - [`payloads`]: one struct per Python dataclass used as a workflow/activity
//!   input or result, grouped by pipeline.
//! - [`schedules`]: the 12 Temporal Schedule definitions (spec-temporal section 6.2).
//! - [`retry`]: retry policies and per-activity call options.
//! - [`ids`]: deterministic workflow ID derivation.

pub mod ids;
pub mod payloads;
pub mod retry;
pub mod schedules;
pub mod wire;
