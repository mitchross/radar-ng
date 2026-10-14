#![allow(clippy::needless_range_loop, clippy::type_complexity)] // numerical kernels index several buffers in lockstep
//! # radar-nowcast
//!
//! The radar-ng nowcast: the numerical core (pyramidal Lucas-Kanade motion, semi-Lagrangian advection, FFT
//! cascade, AR(2), CDF probability matching, S-PROG, verification scores) and the activity glue
//! (`nowcast_run`: input selection, grid I/O, tiles, manifest swap, status, skill log).
//!
//! The entry point for the worker is [`nowcast_run`]: a plain async function with no Temporal types. The
//! benchmark entry point is [`bench_nowcast`]. The algorithm fidelity and every deviation from the Python
//! originals are in `README.md` in this crate.

pub mod advection;
pub mod ar;
pub mod bench;
pub mod cascade;
pub mod config;
pub mod error;
pub mod field;
pub mod inputs;
pub mod motion;
pub mod probmatch;
pub mod render;
pub mod run;
pub mod score;
pub mod select;
pub mod skill;
pub mod sprog;
pub mod status;

pub use bench::{BenchReport, bench_nowcast};
pub use config::NowcastConfig;
pub use error::NowcastError;
pub use field::Field2;
pub use motion::{MotionParams, Velocity, dense_lucaskanade, estimate_motion};
pub use run::{NowcastOutput, nowcast_run};
pub use sprog::{Forecast, SprogParams, forecast};
