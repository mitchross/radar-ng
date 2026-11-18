//! Static-data payloads (`EnsureStaticDataWorkflow` and `static_data_ensure`).

use serde::{Deserialize, Serialize};

/// `StaticDataResult`: output of `static_data_ensure` and `EnsureStaticDataWorkflow`.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct StaticDataResult {
    /// The product (`normals`).
    pub product: String,
    /// `present` (marker found, nothing done) or `built`.
    pub outcome: String,
    /// Bytes of the product on the state volume.
    #[serde(default)]
    pub bytes: Option<i64>,
}
