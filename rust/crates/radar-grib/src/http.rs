//! Shared HTTP client settings for NOAA access (spec-ingest-mrms-hrrr §4, §17.2; spec-ingest-aux §5.4).
//!
//! Matches httpx defaults where the Python code relies on them: redirects are not followed, gzip
//! Content-Encoding is decoded, and every call carries an explicit timeout. There are no in-process
//! retries; Temporal's activity retry policy owns retries, as in the Python workers.

use std::time::Duration;

use crate::error::{GribError, Result};

/// User-Agent sent to NOAA. httpx sent its own default; NOAA does not require a specific value.
pub const USER_AGENT: &str = "radar-ng-ingest/0.1 (radar-grib; rust)";

/// Timeout for MRMS and HRRR object listings (`_list_keys_sync`).
pub const LIST_TIMEOUT: Duration = Duration::from_secs(30);
/// Timeout for MRMS object downloads (`_download_and_decode_sync`).
pub const MRMS_GET_TIMEOUT: Duration = Duration::from_secs(60);
/// Timeout for HEAD probes (HRRR latest-run finder, NAQFC latest-run finder).
pub const HEAD_TIMEOUT: Duration = Duration::from_secs(10);
/// Timeout for `.idx` fetches.
pub const IDX_TIMEOUT: Duration = Duration::from_secs(15);
/// Timeout for each ranged GET of an HRRR subset.
pub const RANGE_TIMEOUT: Duration = Duration::from_secs(120);
/// Timeout for whole-object GETs of HRRR and NAQFC files (full-file fallback, chunk download).
pub const FULL_FILE_TIMEOUT: Duration = Duration::from_secs(300);

/// Builds the reqwest client builder with the crate-wide settings.
///
/// Redirects are disabled (httpx default), gzip is accepted, and the User-Agent is [`USER_AGENT`].
pub fn client_builder() -> reqwest::ClientBuilder {
    reqwest::Client::builder()
        .user_agent(USER_AGENT)
        .redirect(reqwest::redirect::Policy::none())
        .gzip(true)
        .pool_max_idle_per_host(20)
}

/// Builds a client from [`client_builder`].
pub fn client() -> Result<reqwest::Client> {
    client_builder()
        .build()
        .map_err(|e| GribError::Parse(format!("building HTTP client: {e}")))
}
