//! The pool's two jobs: a housekeeping tick and one bounded poll per demanded station.

use crate::{
    feed::Feed,
    leases::Pool,
    metrics,
    pool_ops::{Idle, Reaped},
    tracer,
};
use anyhow::{Result, ensure};
use object_store::{ObjectStore, aws::AmazonS3Builder, path::Path};
use std::{
    sync::Arc,
    time::{Duration, Instant},
};

/// A station poll that fails backs off 10 s, 20 s, 40 s ... up to this many seconds.
pub const BACKOFF_MAX_S: u64 = 120;
/// Seconds between pool passes.
pub const POLL_INTERVAL_S: u64 = 10;
/// Objects deleted per eviction call, so a huge station cannot hold the tick open.
const DELETE_BATCH: usize = 500;

/// Seconds to wait after `failures` consecutive failed polls of one station.
pub fn backoff_s(failures: u32) -> u64 {
    if failures == 0 {
        return POLL_INTERVAL_S;
    }
    (POLL_INTERVAL_S << failures.min(6)).min(BACKOFF_MAX_S)
}

pub fn enabled() -> Result<bool> {
    Ok(radar_core::env::bool_var("LEVEL2_NUMERIC_ENABLED", false)?)
}

#[derive(Debug, Default, PartialEq, Eq)]
pub struct Tick {
    /// Demanded stations, at most eight.
    pub stations: Vec<String>,
}

#[derive(Debug, PartialEq, Eq)]
pub struct PollOutcome {
    /// `numeric_ready`, `fallback` or `skipped`.
    pub status: &'static str,
    pub reason: Option<String>,
}

impl PollOutcome {
    fn skipped(reason: &str) -> Self {
        Self {
            status: "skipped",
            reason: Some(reason.into()),
        }
    }
}

pub struct Poller {
    url: String,
    schema: String,
    store: Arc<dyn ObjectStore>,
    namespace: String,
    feed: Feed,
    instance: String,
    ready: tokio::sync::OnceCell<()>,
}

fn required(name: &str) -> Result<String> {
    std::env::var(name).map_err(|_| anyhow::anyhow!("{name} required"))
}

impl Poller {
    pub fn new(
        url: String,
        schema: String,
        store: Arc<dyn ObjectStore>,
        namespace: String,
    ) -> Result<Self> {
        Ok(Self {
            url,
            schema,
            store,
            namespace,
            feed: Feed::new()?,
            instance: uuid::Uuid::new_v4().simple().to_string(),
            ready: tokio::sync::OnceCell::new(),
        })
    }

    /// Swaps the NOAA client, for failure fixtures.
    pub fn with_feed(mut self, feed: Feed) -> Self {
        self.feed = feed;
        self
    }

    /// Builds the poller from the Level II environment. Fails when the numeric gate is off.
    pub fn from_env() -> Result<Self> {
        ensure!(enabled()?, "Level II numeric gate disabled");
        let endpoint = required("LEVEL2_S3_ENDPOINT")?;
        let store: Arc<dyn ObjectStore> = Arc::new(
            AmazonS3Builder::new()
                .with_endpoint(&endpoint)
                .with_allow_http(endpoint.starts_with("http://"))
                .with_bucket_name(required("LEVEL2_S3_BUCKET")?)
                .with_region("us-east-1")
                .with_access_key_id(required("LEVEL2_S3_ACCESS_KEY")?)
                .with_secret_access_key(required("LEVEL2_S3_SECRET_KEY")?)
                .build()?,
        );
        Self::new(
            required("LEVEL2_PG_URL")?,
            std::env::var("LEVEL2_PG_SCHEMA").unwrap_or_else(|_| "level2_numeric".into()),
            store,
            required("LEVEL2_OBJECT_PREFIX")?,
        )
    }

    async fn pool(&self) -> Result<Pool> {
        let pool = Pool::connect(&self.url, &self.schema).await?;
        // Schema DDL runs once per process; concurrent polls would deadlock on it.
        self.ready.get_or_try_init(|| pool.initialize()).await?;
        Ok(pool)
    }

    /// Reaps dead leases, evicts idle stations, refreshes gauges and returns the demanded stations.
    pub async fn tick(&self) -> Result<Tick> {
        let mut pool = self.pool().await?;
        pool.beat(&self.instance).await?;
        let reaped = pool.reap().await?;
        count_reaped(&reaped);
        self.evict_idle(&mut pool).await?;
        let stations = pool.demanded().await?;
        for station in &stations {
            self.prune(&mut pool, station).await?;
        }
        let usage = pool.usage().await?;
        metrics::gauge("radar_level2_active_stations", usage.active_stations as f64);
        metrics::gauge("radar_level2_busy_stations", usage.busy_stations as f64);
        metrics::gauge("radar_level2_decodes_running", usage.decodes_running as f64);
        metrics::gauge("radar_level2_sweep_cache_bytes", usage.sweep_bytes as f64);
        metrics::gauge("radar_level2_render_cache_bytes", usage.render_bytes as f64);
        metrics::gauge("radar_level2_object_bytes", usage.object_bytes as f64);
        Ok(Tick { stations })
    }

    /// One bounded poll of a demanded station. A fallback is a normal outcome, never an error.
    pub async fn poll(&self, station: &str) -> Result<PollOutcome> {
        let mut pool = self.pool().await?;
        let asked = Instant::now();
        if !pool.claim_poll(station).await? {
            metrics::incr(
                "radar_level2_poll_skipped_total",
                &[("reason", "not_claimable")],
            );
            return Ok(PollOutcome::skipped("not_claimable"));
        }
        let permit = match pool.decode().await? {
            Some(p) => p,
            None => {
                pool.release_station(station).await?;
                metrics::incr(
                    "radar_level2_poll_skipped_total",
                    &[("reason", "decode_saturated")],
                );
                return Ok(PollOutcome::skipped("decode_saturated"));
            }
        };
        metrics::observe("radar_level2_permit_wait_seconds", asked.elapsed());
        let started = Instant::now();
        let result = tracer::trace(
            &self.feed,
            &mut pool,
            self.store.as_ref(),
            station,
            &self.namespace,
        )
        .await;
        metrics::observe("radar_level2_poll_seconds", started.elapsed());
        pool.release_decode(&permit).await?;
        let health = pool.health(station).await.unwrap_or_default();
        pool.release_station(station).await?;
        let now = chrono::Utc::now();
        if let Some(t) = health.newest_chunk_modified {
            let age = (now - t).num_seconds().max(0) as f64;
            metrics::gauge_family(
                "radar_level2_feed_age_seconds",
                "station",
                &[(station.to_owned(), age)],
            );
        }
        if let Some(t) = health.low_sweep_end {
            let age = (now - t).num_seconds().max(0) as f64;
            metrics::gauge_family(
                "radar_level2_sweep_age_seconds",
                "station",
                &[(station.to_owned(), age)],
            );
        }
        Ok(match result {
            Ok(v) if v["status"] == "numeric_ready" => {
                metrics::incr("radar_level2_poll_total", &[("status", "numeric_ready")]);
                PollOutcome {
                    status: "numeric_ready",
                    reason: None,
                }
            }
            Ok(v) => fallback(v["reason"].as_str().unwrap_or("unknown")),
            Err(_) => fallback("artifact_unavailable"),
        })
    }

    async fn evict_idle(&self, pool: &mut Pool) -> Result<()> {
        let Idle { claimed, .. } = pool.idle_stations().await?;
        for station in claimed {
            let prefixes = [
                format!("{}/sweeps/{station}/", self.namespace),
                format!("{}/tiles/{station}/", self.namespace),
            ];
            let mut failed = false;
            for prefix in &prefixes {
                if delete_prefix(self.store.as_ref(), prefix).await.is_err() {
                    failed = true;
                    break;
                }
            }
            if failed {
                pool.abort_evict(&station).await?;
                metrics::incr("radar_level2_evict_total", &[("result", "failed")]);
            } else {
                pool.finish_evict(&station, &self.namespace).await?;
                metrics::incr("radar_level2_evict_total", &[("result", "evicted")]);
            }
        }
        Ok(())
    }

    async fn prune(&self, pool: &mut Pool, station: &str) -> Result<()> {
        for (end, commit_key) in pool.prunable(station).await? {
            let Some((prefix, _)) = commit_key.rsplit_once('/') else {
                continue;
            };
            let prefix = format!("{prefix}/");
            if delete_prefix(self.store.as_ref(), &prefix).await.is_ok() {
                pool.forget_sweep(station, end, &prefix).await?;
                metrics::incr("radar_level2_pruned_sweeps_total", &[]);
            }
        }
        Ok(())
    }
}

fn fallback(reason: &str) -> PollOutcome {
    metrics::incr("radar_level2_poll_total", &[("status", "fallback")]);
    metrics::incr(
        "radar_level2_fallback_total",
        &[("reason", reason), ("stage", "poll")],
    );
    PollOutcome {
        status: "fallback",
        reason: Some(reason.into()),
    }
}

fn count_reaped(r: &Reaped) {
    for (kind, n) in [
        ("station", r.stations),
        ("decode", r.decodes),
        ("render", r.renders),
        ("reservation", r.reservations),
    ] {
        if n > 0 {
            metrics::add("radar_level2_reaped_total", &[("kind", kind)], n as f64);
        }
    }
}

/// Deletes every object under `prefix`, a bounded batch at a time.
pub async fn delete_prefix(store: &dyn ObjectStore, prefix: &str) -> Result<usize> {
    use futures_util::StreamExt;
    let path = Path::from(prefix.trim_end_matches('/'));
    let mut total = 0;
    loop {
        let mut listed = store.list(Some(&path));
        let mut batch = Vec::new();
        while let Some(meta) = listed.next().await {
            batch.push(meta?.location);
            if batch.len() >= DELETE_BATCH {
                break;
            }
        }
        drop(listed);
        if batch.is_empty() {
            return Ok(total);
        }
        for location in &batch {
            store.delete(location).await?;
        }
        total += batch.len();
        tokio::time::sleep(Duration::ZERO).await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn backoff_doubles_from_the_ten_second_poll_and_caps() {
        let seq: Vec<u64> = (0..8).map(backoff_s).collect();
        assert_eq!(seq, [10, 20, 40, 80, 120, 120, 120, 120]);
    }

    #[tokio::test]
    async fn delete_prefix_removes_only_that_station() -> Result<()> {
        let store = object_store::memory::InMemory::new();
        for key in [
            "l2/sweeps/KGRR/a/commit.json",
            "l2/sweeps/KGRR/b/x",
            "l2/sweeps/KGRRX/a/x",
            "l2/sweeps/KDTX/a/x",
        ] {
            store.put(&Path::from(key), vec![1u8].into()).await?;
        }
        assert_eq!(delete_prefix(&store, "l2/sweeps/KGRR/").await?, 2);
        assert!(store.head(&Path::from("l2/sweeps/KGRRX/a/x")).await.is_ok());
        assert!(store.head(&Path::from("l2/sweeps/KDTX/a/x")).await.is_ok());
        Ok(())
    }
}
