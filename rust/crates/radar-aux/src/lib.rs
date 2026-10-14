//! radar-aux: the "aux" ingest activities of the radar-ng Rust port, as plain async functions.
//!
//! Covered pipelines (spec `specs/spec-ingest-aux.md`, sections 5, 7, 8 and 9):
//!
//! - [`aqm`]: NAQFC air quality. `aqm_find_latest_run`, `aqm_render_chunk`, `aqm_publish_run`,
//!   `aqm_mark_processed` and `aqm_cleanup`.
//! - [`tropical`]: NHC tropical cyclones. `tropical_fetch_and_publish`.
//! - [`tile_cleanup`]: tile and grid retention. `tile_cleanup_sweep`.
//! - [`open_meteo`]: Open-Meteo sync. Runs the upstream binary as a child process. `open_meteo_sync`.
//!
//! Nothing here depends on Temporal. The worker crate (`radar-worker`) wraps each function in an
//! `#[activity]` and maps [`beat::Beat`] onto `activity.heartbeat`. Payload types come from
//! `radar-types`. Long-running work uses [`beat`] for heartbeats and `spawn_blocking` for CPU and
//! file work. Each activity takes a config struct built from the environment (`from_env`) or by hand
//! (tests). Deliberate deviations from the Python are listed in `README.md`.
//!
//! Lightning (spec section 6) and NWS alerts (section 10) are ported in other crates.

pub mod aqm;
pub mod beat;
pub mod open_meteo;
pub mod tile_cleanup;
pub mod tropical;

pub use beat::{Beat, no_beat};
