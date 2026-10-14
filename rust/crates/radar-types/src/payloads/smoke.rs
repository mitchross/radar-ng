//! Deployment smoke payloads (`RadarDeploymentSmokeWorkflow` and `radar_deployment_smoke`).
//!
//! Source: `temporal/workflows/deployment_smoke.py`, `temporal/shared/deployment_smoke.py`.
//! The workflow and activity take no input.

use serde::{Deserialize, Serialize};

/// `{"ok": true, "role": "<role>"}`: the smoke gate's result.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DeploymentSmokeResult {
    /// Always `true` on success (failures raise instead).
    pub ok: bool,
    /// The `WORKER_ROLE` the check ran for.
    pub role: String,
}
