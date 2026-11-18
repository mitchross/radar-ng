//! Sky ingest and pass maths (design `weather-extras.md` section 7).
//!
//! - [`iss`]: the `ingest-iss` product. One CelesTrak OMM request per 6 h slot, written to
//!   `STATE_DIR/iss.json`. A non-200 keeps the last good file and is never retried.
//! - [`passes`]: visible ISS passes for a viewer, computed with the `sgp4` crate.
//! - [`eccc`]: the `ingest-star-seeing` product. ECCC RDPS seeing and transparency, packed per
//!   valid hour under `STATE_DIR/star-seeing/`. Off unless `ECCC_SEEING_ENABLED=true`.
//!
//! [`sky_fetch_and_publish`] is the activity body. It takes a product name so later products
//! (the ECCC seeing fields, #330) can share `IngestSkyWorkflow`.

pub mod eccc;
pub mod iss;
pub mod passes;

use std::path::PathBuf;

use radar_types::payloads::sky::SkyResult;

/// Product name of the ISS elements (`ingest-iss`).
pub const PRODUCT_ISS: &str = "iss";
/// Product name of the ECCC seeing and transparency fields (`ingest-star-seeing`).
pub const PRODUCT_STAR_SEEING: &str = "star-seeing";

/// Errors from a sky ingest run. An upstream failure is not an error: it is reported in the result.
#[derive(Debug, thiserror::Error)]
pub enum SkyError {
    /// The workflow asked for a product this build does not know.
    #[error("unknown sky product {0:?}")]
    UnknownProduct(String),
    /// Writing the state files failed.
    #[error("writing sky state: {0}")]
    Io(#[from] std::io::Error),
    /// The HTTP client could not be built.
    #[error("building the sky HTTP client: {0}")]
    Client(#[from] reqwest::Error),
}

/// Sky ingest configuration.
#[derive(Debug, Clone)]
pub struct SkyConfig {
    /// `STATE_DIR`.
    pub state_dir: PathBuf,
    /// `ECCC_SEEING_ENABLED`: fetch the ECCC seeing fields. Off by default (see [`eccc`]).
    pub seeing_enabled: bool,
}

impl SkyConfig {
    /// Reads `STATE_DIR` and `ECCC_SEEING_ENABLED`. The upstream URLs are fixed on purpose.
    pub fn from_env() -> Self {
        Self {
            state_dir: radar_core::env::path_var("STATE_DIR", "/data/state"),
            seeing_enabled: std::env::var("ECCC_SEEING_ENABLED")
                .is_ok_and(|v| v.trim().eq_ignore_ascii_case("true")),
        }
    }
}

/// `sky_fetch_and_publish`: run one product once.
pub async fn sky_fetch_and_publish(
    config: &SkyConfig,
    product: &str,
) -> Result<SkyResult, SkyError> {
    match product {
        PRODUCT_ISS => {
            let http = iss::client()?;
            iss::ingest(
                &http,
                iss::CELESTRAK_ISS_URL,
                &config.state_dir,
                chrono::Utc::now(),
            )
            .await
        }
        PRODUCT_STAR_SEEING => {
            let http = eccc::client()?;
            eccc::ingest(
                &http,
                eccc::DATAMART_URL,
                &config.state_dir,
                chrono::Utc::now(),
                config.seeing_enabled,
            )
            .await
        }
        other => Err(SkyError::UnknownProduct(other.to_owned())),
    }
}
