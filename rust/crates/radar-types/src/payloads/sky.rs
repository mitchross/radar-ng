//! Sky payloads (`IngestSkyWorkflow` and `sky_fetch_and_publish`).

use serde::{Deserialize, Serialize};

/// `SkyResult`: output of `sky_fetch_and_publish` and `IngestSkyWorkflow`.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct SkyResult {
    /// The product run (`iss` or `star-seeing`).
    pub product: String,
    /// `published`, `skipped-slot`, `http-error`, `transport-error` or `invalid`; `star-seeing` adds
    /// `disabled`, `pending`, `partial` and `up-to-date`.
    pub outcome: String,
    /// Upstream HTTP status, when a response arrived.
    #[serde(default)]
    pub http_status: Option<i64>,
    /// Epoch of the published elements, or the RDPS run for `star-seeing` (RFC 3339).
    #[serde(default)]
    pub epoch: Option<String>,
}
