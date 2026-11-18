//! Sky payloads (`IngestSkyWorkflow` and `sky_fetch_and_publish`).

use serde::{Deserialize, Serialize};

/// `SkyResult`: output of `sky_fetch_and_publish` and `IngestSkyWorkflow`.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct SkyResult {
    /// The product run (`iss`).
    pub product: String,
    /// `published`, `skipped-slot`, `http-error`, `transport-error` or `invalid`.
    pub outcome: String,
    /// Upstream HTTP status, when a response arrived.
    #[serde(default)]
    pub http_status: Option<i64>,
    /// Epoch of the published elements (RFC 3339), when published.
    #[serde(default)]
    pub epoch: Option<String>,
}
