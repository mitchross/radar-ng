//! JSON payload structs for every Temporal workflow and activity input or result.
//!
//! Each struct corresponds to a Python dataclass (or a positional argument list
//! in the Python code, where noted). Encoding rules (spec-temporal section 1.2):
//!
//! - Field names are exactly the Python field names (snake_case, no renames).
//! - Encoding emits every field, with `null` for unset `Option`s.
//! - Decoding accepts missing fields that have a Python default (`#[serde(default)]`
//!   with the same default value), ignores unknown keys, and accepts JSON integers
//!   for `f64` fields.
//! - Non-finite floats are rejected on encode (Python would write `NaN`, which
//!   is not valid JSON and is never produced by the current code).
//!
//! Fields without a Python default are required on decode, except `Option`
//! fields, which serde treats as `None` when absent.

pub mod airquality;
pub mod alerts;
pub mod cleanup;
pub mod global_model;
pub mod hrrr;
pub mod lightning;
pub mod mrms;
pub mod nowcast;
pub mod open_meteo;
pub mod push;
pub mod smoke;
pub mod tropical;
pub mod watch_storm;

mod codec;
