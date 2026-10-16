//! Activity implementers, one per pool, so each role registers exactly the activity names in spec 3.2.
//!
//! Every method is `#[activity(name = ...)]` with the Python activity name from `radar_types::wire`.
//! Retry and timeout policy lives with the calling workflow (`radar_types::retry`), not here.

pub mod alerts;
pub mod aqm;
pub mod aux;
pub mod global_model;
pub mod hrrr;
pub mod mrms;
pub mod nowcast;
pub mod open_meteo;
pub mod smoke;
