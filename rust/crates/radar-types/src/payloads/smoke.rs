//! Deployment smoke payloads (`RadarDeploymentSmokeWorkflow` and `radar_deployment_smoke`).

use serde::{Deserialize, Serialize};

/// `{"ok": true, "role": "<role>"}`: the smoke gate's result.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DeploymentSmokeResult {
    /// Always `true` on success (failures raise instead).
    pub ok: bool,
    /// The `WORKER_ROLE` the check ran for.
    pub role: String,
}
