//! # radar-storms
//!
//! Rust port of radar-ng's storm-cell detection and tracking, storm prefetch
//! planning and storm-watch change detection (`backend/shared/storms.py`,
//! `storm_prefetch.py`, `storm_watch.py`, and the pure parts of
//! `backend/api/api/storm_watch_activities.py`). Behavior follows
//! `specs/spec-shared.md` sections 6 to 8, with the PLAN.md defect fixes D1, D2
//! and D3 applied. See `README.md` for the full list of deviations.
//!
//! Modules:
//! - [`detect`]: threshold, 4-connected labelling, features (`detect_storms`).
//! - [`storage`]: `storms.json` writer (`storms.lock` flock, stale-frame guard, atomic replace).
//! - [`prefetch`]: three-bbox tile prefetch plan and manifest reader.
//! - [`watch`]: frame sampling, change detection and frame comparison.
//! - [`config`]: environment configuration with the Python defaults.
//!
//! Grids are passed as [`Grid`] (row-major `f32` plus axes), so the crate does not
//! depend on `radar-geo`.

#![forbid(unsafe_code)]

pub mod config;
pub mod detect;
pub mod doc;
pub mod error;
pub mod geo;
pub mod grid;
pub mod label;
pub mod numeric;
pub mod prefetch;
pub mod storage;
pub mod time;
pub mod tracking;
pub mod watch;

pub use config::StormConfig;
pub use detect::{detect_storms, detect_storms_with_clock};
pub use doc::{CellFeature, StormsDoc};
pub use error::StormsError;
pub use grid::Grid;
pub use prefetch::{PrefetchPlan, PrefetchRequest, build_storm_prefetch_plan};
pub use storage::{WriteOutcome, write_storms_json};
pub use watch::{ChangeEvent, ChangeKind, FrameSample, WatchConfig, detect_change, sample_window};
