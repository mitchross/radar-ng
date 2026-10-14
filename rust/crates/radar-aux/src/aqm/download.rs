//! GRIB2 download for one (run, product), cached on disk so the chunks of a run share one fetch.
//!
//! Decisions (spec-ingest-aux 5.5, DECIDE D-5 and D-6, PLAN defect mrms/hrrr D3):
//! - The AWS mirror is tried first, then NOMADS. Any non-2xx status or transport error on AWS falls
//!   through to NOMADS. The Python fell through on 404 only. NOAA can answer a missing anonymous S3 key
//!   with 403, so 404-only fallback could never reach NOMADS. Redirects are not followed.
//! - One download per (run, product). The file is streamed to `<product>.grib2.part` under an exclusive
//!   `flock` on `.<product>.lock`, then renamed. Chunks that wait on the lock keep heartbeating.
//! - A failed download removes its partial file, so nothing is left behind (no tmp leak).
//! - Each read is bounded by `read_timeout`, which matches httpx's per-operation `timeout=300`.

use std::path::{Path, PathBuf};

use futures_util::StreamExt;
use radar_core::fsutil::FlockGuard;
use serde_json::json;
use tokio::io::AsyncWriteExt;
use tracing::{info, warn};

use super::{AqmConfig, AqmError, LayerSpec, cache_dir};
use crate::beat::{Beat, with_heartbeats};

/// Returns the path of the cached GRIB2 file for `spec.product` of `run_id`, downloading it if needed.
///
/// The lock is held only while this function runs. The returned file is complete (renamed from `.part`).
pub(crate) async fn ensure_product_file(
    cfg: &AqmConfig,
    http: &reqwest::Client,
    run_id: &str,
    spec: &LayerSpec,
    start_msg: i64,
    beat: &Beat,
) -> Result<PathBuf, AqmError> {
    let dir = cache_dir(&cfg.tmp_root, run_id);
    let final_path = dir.join(format!("{}.grib2", spec.product));
    let part_path = dir.join(format!("{}.grib2.part", spec.product));
    let lock_path = dir.join(format!(".{}.lock", spec.product));
    let details = json!({ "phase": "download", "layer": spec.name, "start_msg": start_msg });
    let beat_fn = {
        let beat = beat.clone();
        let details = details.clone();
        move || beat(details.clone())
    };

    let lock_dir = dir.clone();
    let guard: FlockGuard = radar_core::heartbeat::run_blocking_with_heartbeat(
        move || -> std::io::Result<FlockGuard> {
            std::fs::create_dir_all(&lock_dir)?;
            FlockGuard::exclusive(&lock_path)
        },
        cfg.heartbeat_every,
        beat_fn.clone(),
    )
    .await?;

    if final_path.is_file() {
        drop(guard);
        return Ok(final_path);
    }

    let urls = cfg.sources.grib_urls(run_id, spec.product)?;
    let outcome = with_heartbeats(
        download_first_available(http, &urls, &part_path, cfg.read_timeout, run_id),
        cfg.heartbeat_every,
        beat_fn,
    )
    .await;
    let result = match outcome {
        Ok(()) => {
            tokio::fs::rename(&part_path, &final_path).await?;
            Ok(final_path)
        }
        Err(err) => {
            let _ = tokio::fs::remove_file(&part_path).await;
            Err(err)
        }
    };
    drop(guard);
    result
}

/// Tries each URL in order. Any non-2xx status or transport error moves on. The last error is returned.
async fn download_first_available(
    http: &reqwest::Client,
    urls: &[String],
    dest: &Path,
    read_timeout: std::time::Duration,
    run_id: &str,
) -> Result<(), AqmError> {
    let mut last_err: Option<AqmError> = None;
    for url in urls {
        match fetch_to_file(http, url, dest, read_timeout).await {
            Ok(()) => {
                info!(run_id, url = %url, "aqm_source_used");
                return Ok(());
            }
            Err(err) => {
                match &err {
                    AqmError::Status { status: 404, .. } => {
                        info!(run_id, url = %url, "aqm_source_not_found")
                    }
                    other => warn!(run_id, url = %url, error = %other, "aqm_source_failed"),
                }
                last_err = Some(err);
            }
        }
    }
    Err(last_err.unwrap_or_else(|| AqmError::NoSource {
        run_id: run_id.to_string(),
    }))
}

/// Streams one URL into `dest`. Status must be 2xx. Redirects are not followed (client policy).
async fn fetch_to_file(
    http: &reqwest::Client,
    url: &str,
    dest: &Path,
    read_timeout: std::time::Duration,
) -> Result<(), AqmError> {
    let resp = tokio::time::timeout(read_timeout, http.get(url).send())
        .await
        .map_err(|_| AqmError::Timeout {
            url: url.to_string(),
        })?
        .map_err(|source| AqmError::Transport {
            url: url.to_string(),
            source,
        })?;
    let status = resp.status();
    if !status.is_success() {
        return Err(AqmError::Status {
            status: status.as_u16(),
            url: url.to_string(),
        });
    }
    let mut file = tokio::fs::File::create(dest).await?;
    let mut stream = resp.bytes_stream();
    loop {
        match tokio::time::timeout(read_timeout, stream.next()).await {
            Err(_) => {
                return Err(AqmError::Timeout {
                    url: url.to_string(),
                });
            }
            Ok(None) => break,
            Ok(Some(Err(source))) => {
                return Err(AqmError::Transport {
                    url: url.to_string(),
                    source,
                });
            }
            Ok(Some(Ok(chunk))) => file.write_all(&chunk).await?,
        }
    }
    file.flush().await?;
    file.sync_all().await?;
    Ok(())
}
