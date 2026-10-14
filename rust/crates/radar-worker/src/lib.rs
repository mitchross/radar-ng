//! Temporal control plane for radar-ng: workflows, activities, schedule seeding, the watchdog and the worker.
//!
//! Start with [`run_worker`], which reads [`WorkerConfig::from_env`] and serves the role's queue. The
//! wire contract (names, payloads, retry constants, schedule definitions) lives in `radar-types`. Behaviour
//! and deviations are documented in this crate's README.

pub mod activities;
pub mod config;
pub mod health;
pub mod seed;
pub mod support;
pub mod temporal_io;
pub mod watchdog;
pub mod worker;
pub mod workflows;

pub use config::WorkerConfig;
pub use seed::{Outcome, SeedError};
pub use worker::{Registration, registration, run_watchdog, run_worker, seed_schedules};
