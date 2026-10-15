//! radar-grib: GRIB2 decoding and NOAA data access for the radar-ng Rust port.
//!
//! Replaces the Python `pygrib` + `pyproj` + `boto`/`httpx` usage in `backend/ingest_mrms`,
//! `backend/ingest_hrrr` and `backend/ingest_airquality`.
//!
//! Modules:
//! - [`decode`]: GRIB2 message selection and decoding to [`radar_geo::Grid`].
//! - [`units`]: the unit conversions and the precip-type category grid used by HRRR.
//! - [`mrms`]: MRMS object keys, gunzip, and south-up orientation.
//! - [`idx`]: HRRR/NAQFC `.idx` parsing and byte-range planning.
//! - [`s3`]: anonymous S3 ListObjectsV2 and ranged GET.
//! - [`hrrr`]: HRRR file keys, latest-run finder and subset fetch.
//! - [`gfs`]: GFS 0.25 degree keys, latest-run finder and REFC fetch (long-range radar).
//! - [`uvi`]: NOAA/CPC UV Index forecast keys, latest-run finder and fetch.
//! - [`naqfc`]: NAQFC (AQM) file URLs, latest-run finder and chunk download.
//! - [`http`]: the shared reqwest client settings.
//!
//! The crate has no Temporal types and no filesystem state. Callers own temp directories,
//! ProcessedSet files and retry policy.

pub mod decode;
pub mod error;
pub mod gfs;
pub mod hrrr;
pub mod http;
pub mod idx;
pub mod mrms;
pub mod naqfc;
mod packing;
pub mod s3;
pub mod units;
pub mod uvi;

pub use decode::{
    DecodedMessage, MessageInfo, MessageSelector, decode_first_message, decode_message,
    find_message, list_messages,
};
pub use error::{GribError, Result};
