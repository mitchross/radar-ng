//! radar-ingest: MRMS and HRRR ingest activity bodies as plain async functions (no Temporal types).
//!
//! The worker crate wraps these. See README.md for the public API and deviations from the Python.

pub mod bench;
pub mod common;
pub mod config;
pub mod ctx;
pub mod heartbeat;
pub mod hrrr;
pub mod mrms;
pub mod packed;
pub mod scratch;

pub use config::IngestConfig;
pub use ctx::{IngestCtx, NowcastKicker};
pub use heartbeat::Heartbeat;

pub use bench::{BenchReport, bench_render_once};
pub use scratch::ActivityInfo;
