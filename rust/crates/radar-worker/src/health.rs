//! Health and liveness touch files (spec section 9.1).
//!
//! Every 30 s the loop touches the alive file, then checks Temporal reachability and touches the health file
//! only on success. A successful health RPC does not prove the SDK is polling (drift D14).

use std::fs::OpenOptions;
use std::path::Path;
use std::time::{Duration, SystemTime};

use temporalio_client::Client;
use temporalio_common::protos::grpc::health::v1::{
    HealthCheckRequest, health_check_response::ServingStatus,
};
use tonic::Request;

/// Time between iterations.
pub const HEALTH_PERIOD: Duration = Duration::from_secs(30);
/// Timeout for one health RPC.
pub const HEALTH_RPC_TIMEOUT: Duration = Duration::from_secs(10);
/// Pause before the single retry.
pub const HEALTH_RETRY_DELAY: Duration = Duration::from_secs(1);

/// Update a file's mtime, creating it if needed. Failures are returned so the caller can log them.
pub fn touch(path: &Path) -> std::io::Result<()> {
    let file = OpenOptions::new().create(true).append(true).open(path)?;
    file.set_modified(SystemTime::now())
}

/// One Temporal reachability check: up to two attempts, one second apart. Never raises.
pub async fn check_once(client: &Client) -> bool {
    for attempt in 1..=2u32 {
        let mut health = client.connection().health_service();
        let request = Request::new(HealthCheckRequest {
            service: String::new(),
        });
        match tokio::time::timeout(HEALTH_RPC_TIMEOUT, health.check(request)).await {
            Ok(Ok(response)) if response.get_ref().status == ServingStatus::Serving as i32 => {
                return true;
            }
            Ok(Ok(_)) => tracing::warn!(attempt, "temporal health check not serving"),
            Ok(Err(status)) => {
                tracing::warn!(attempt, code = ?status.code(), "temporal health check failed")
            }
            Err(_) => tracing::warn!(attempt, "temporal health check timed out"),
        }
        if attempt == 1 {
            tokio::time::sleep(HEALTH_RETRY_DELAY).await;
        }
    }
    false
}

/// Run the loop forever. Started only after the poller is running (spec 3.4).
pub async fn health_file_loop(
    client: Client,
    health_file: std::path::PathBuf,
    alive_file: std::path::PathBuf,
) {
    loop {
        if let Err(err) = touch(&alive_file) {
            tracing::warn!(path = %alive_file.display(), error = %err, "could not touch alive file");
        }
        if check_once(&client).await
            && let Err(err) = touch(&health_file)
        {
            tracing::warn!(path = %health_file.display(), error = %err, "could not touch health file");
        }
        tokio::time::sleep(HEALTH_PERIOD).await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn touch_creates_and_updates_the_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("worker-alive");
        touch(&path).unwrap();
        assert!(path.exists());
        let before = std::fs::metadata(&path).unwrap().modified().unwrap();
        std::thread::sleep(Duration::from_millis(20));
        touch(&path).unwrap();
        let after = std::fs::metadata(&path).unwrap().modified().unwrap();
        assert!(after >= before);
    }

    #[test]
    fn touch_reports_errors_instead_of_panicking() {
        let dir = tempfile::tempdir().unwrap();
        let missing = dir.path().join("no-such-dir").join("file");
        assert!(touch(&missing).is_err());
    }
}
