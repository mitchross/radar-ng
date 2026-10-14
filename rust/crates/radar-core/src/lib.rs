//! radar-core: Rust port of radar-ng's `backend/shared/` data-plane library.
//!
//! Covers the modules of `spec-shared.md` that are not the tiler, palettes/PNG (radar-tiler) or
//! storms / storm prefetch / storm watch (radar-storms):
//!
//! - [`fsutil`]: `flock(2)` guards, atomic tmp+rename writes, directory fsync helpers.
//! - [`env`]: typed environment readers and the [`env::Paths`] layout (`STATE_DIR`, `GRID_DIR`, `TILE_DIR`).
//! - [`logging`]: JSON log lines matching spec §11, plus a `retry` helper.
//! - [`manifest`]: manifest v2 read/add/remove/replace under `manifest.lock`.
//! - [`processed_set`]: bounded on-disk `ProcessedSet`.
//! - [`grid_dump`]: byte-exact grid writer, generation marker, retention; [`grid_read`]: reader and
//!   bilinear point sampling.
//! - [`push_tokens`]: SQLite push-token store.
//! - [`nowcast_skill`]: verification log, scores, and Prometheus text.
//! - [`heartbeat`]: `run_blocking_with_heartbeat` for Temporal-style liveness beats.
//! - [`time`]: Python-compatible `isoformat`, timestamp parsing, and round-half-even helpers.
//! - [`pycompat`]: Python-compatible JSON writer, float `repr`, and `int()`/`str()` coercions.
//!
//! See `README.md` in this crate for the public API summary and every deliberate deviation from the
//! Python originals.

pub mod env;
pub mod fsutil;
pub mod grid_dump;
pub mod grid_read;
pub mod heartbeat;
pub mod logging;
pub mod manifest;
pub mod nowcast_skill;
pub mod processed_set;
pub mod push_tokens;
pub mod pycompat;
pub mod time;
