//! Alert, lightning and push activities for the radar-ng Rust port.
//!
//! These are plain async functions with no Temporal types in their signatures. The worker crate wraps each one
//! in an `#[activity]` with the same name, retry policy and timeouts (spec-temporal section 4, spec-ingest-aux
//! sections 6, 10, spec-shared sections 8 and 9). Sources of truth: `specs/spec-ingest-aux.md` (sections 6 and
//! 10), `specs/spec-temporal.md` (sections 4.10 to 4.12 and 8), and `specs/spec-shared.md` (sections 8 and 9).
//!
//! Modules:
//!
//! - [`lightning`]: the Blitzortung websocket consumer (`lightning_consume_stream`).
//! - [`alerts`]: NWS active alerts (`fetch_nws_active_alerts`, `mark_alerts_seen`,
//!   `signal_matching_storm_watches`) and the [`WatchSignaler`] trait that the worker implements.
//! - [`storm_watch`]: push-token persistence, frame comparison, change detection and push fan-out.
//! - [`push`]: the APNs (HTTP/2 + ES256 JWT) and FCM (HTTP v1 + service-account OAuth) senders.

#![forbid(unsafe_code)]

pub mod alerts;
pub mod lightning;
pub mod push;
pub mod storm_watch;

pub use alerts::{WatchCenter, WatchSignaler};
pub use lightning::{LightningConfig, LightningError};
pub use push::{PushConfig, PushError, PushSender};

/// Wall-clock seconds since the Unix epoch, as Python's `time.time()` returns them.
pub(crate) fn now_epoch() -> f64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs_f64())
        .unwrap_or(0.0)
}

/// Uniform random value in `[0, 1)`, used for reconnect jitter (Python `random.random()`). Built from the
/// standard library hasher so the crate needs no RNG dependency.
pub(crate) fn unit_random() -> f64 {
    use std::hash::{BuildHasher, Hasher};
    let mut hasher = std::collections::hash_map::RandomState::new().build_hasher();
    hasher.write_u128(
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0),
    );
    (hasher.finish() >> 11) as f64 / (1u64 << 53) as f64
}
