//! Shared context for the activities: config, HTTP client, S3 clients, and the nowcast kicker.

use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use radar_grib::s3::S3Client;

use crate::config::IngestConfig;

/// Triggers the nowcast schedule (Python `kick_nowcast`). The worker implements this with its Temporal
/// client, connecting lazily inside `trigger`. This crate has no Temporal types.
#[async_trait]
pub trait NowcastKicker: Send + Sync {
    /// Trigger `schedule_id` with buffer-one overlap. Errors are logged by the caller, never raised.
    async fn trigger(
        &self,
        schedule_id: &str,
        timestamp: &str,
        timeout: Duration,
    ) -> anyhow::Result<()>;
}

/// Everything an activity needs. Cheap to clone.
#[derive(Clone)]
pub struct IngestCtx {
    /// Environment-derived config.
    pub cfg: Arc<IngestConfig>,
    /// Anonymous S3 client for MRMS.
    pub mrms_s3: S3Client,
    /// Anonymous S3 client for HRRR.
    pub hrrr_s3: S3Client,
    /// Anonymous S3 client for GFS.
    pub gfs_s3: S3Client,
    /// Plain HTTPS client for the UV Index product (NOMADS, same GET/HEAD shape).
    pub uv_http: S3Client,
    /// Nowcast kicker. `None` disables the kick regardless of `NOWCAST_KICK_ENABLED`.
    pub nowcast: Option<Arc<dyn NowcastKicker>>,
}

impl IngestCtx {
    /// Build the context with the production HTTP client and the bases from `cfg`.
    pub fn new(cfg: IngestConfig, nowcast: Option<Arc<dyn NowcastKicker>>) -> anyhow::Result<Self> {
        let http = radar_grib::http::client()?;
        Ok(Self {
            mrms_s3: S3Client::new(http.clone(), cfg.mrms_base_url.clone()),
            hrrr_s3: S3Client::new(http.clone(), cfg.hrrr_base_url.clone()),
            gfs_s3: S3Client::new(http.clone(), cfg.gfs_base_url.clone()),
            uv_http: S3Client::new(http, cfg.uv_base_url.clone()),
            cfg: Arc::new(cfg),
            nowcast,
        })
    }
}

/// `kick_nowcast(timestamp)` (spec §12.1.1). Returns true only when the kick succeeded. Never raises.
pub async fn kick_nowcast(ctx: &IngestCtx, timestamp: &str) -> bool {
    if !ctx.cfg.nowcast_kick_enabled {
        return false;
    }
    let Some(kicker) = ctx.nowcast.as_ref() else {
        return false;
    };
    let schedule = ctx.cfg.nowcast_schedule_id.clone();
    let timeout = ctx.cfg.nowcast_kick_timeout;
    let result = tokio::time::timeout(timeout, kicker.trigger(&schedule, timestamp, timeout)).await;
    match result {
        Ok(Ok(())) => {
            tracing::info!(timestamp = %timestamp, schedule_id = %schedule, "nowcast_kicked");
            true
        }
        Ok(Err(err)) => {
            tracing::warn!(timestamp = %timestamp, schedule_id = %schedule, err = %err, "nowcast_kick_failed");
            false
        }
        Err(_) => {
            tracing::warn!(timestamp = %timestamp, schedule_id = %schedule, err = "timed out", "nowcast_kick_failed");
            false
        }
    }
}
