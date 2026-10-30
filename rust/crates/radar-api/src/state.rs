//! Shared application state: configuration, caches, the rate limiter, metrics, the upstream HTTP
//! client, the breaker and the lazily connected Temporal client.
//!
//! Every cache is in-process and per replica, as in Python (spec 11). TTL caches use `moka` with a
//! cache-wide time-to-live (entries expire `ttl` after insertion, and a hit does not extend it,
//! matching the Python `(insert_time, body)` tuples). Response caches hold the encoded body and
//! are bounded by bytes. Caches filled from an upstream are `moka::future` caches, so concurrent
//! misses for one key make one upstream call. The manifest and skill snapshots are single entries
//! behind a `Mutex`, and no lock is ever held across an `.await`.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use anyhow::{Context, Result};
use bytes::Bytes;
use moka::future::Cache as AsyncCache;
use moka::sync::Cache;
use radar_core::nowcast_skill::SkillConfig;
use radar_core::push_tokens::PushTokenStore;
use radar_storms::StormConfig;
use serde_json::Value;
use tokio::sync::{Mutex as AsyncMutex, Semaphore, watch};

use crate::alerts::{AlertFeed, PointZones, ZONES_TTL};
use crate::client_ip::TrustedProxies;
use crate::config::Config;
use crate::metrics::Metrics;
use crate::ratelimit::RateLimiter;

/// A parsed manifest plus its exact encoded bytes and ETag (spec 5.1).
#[derive(Debug)]
pub struct ManifestSnapshot {
    /// The normalized manifest as JSON (what the readers walk).
    pub value: Arc<Value>,
    /// Starlette-compact bytes served on 200.
    pub encoded: Bytes,
    /// `"<blake2b-96 hex>"` over `encoded`.
    pub etag: String,
}

/// One cached imagery tile: the JPEG bytes and the upstream headers the proxy forwards.
#[derive(Debug, Clone)]
pub struct CachedImagery {
    /// The tile bytes.
    pub body: Bytes,
    /// The forwarded upstream headers (content type, `ETag`, `Last-Modified`).
    pub headers: reqwest::header::HeaderMap,
}

/// The LLM breaker (spec 7.3): stop calling after N consecutive failures for a cooldown.
#[derive(Debug)]
pub struct Breaker {
    failures: u32,
    open_until: Option<Instant>,
    threshold: u32,
    cooldown: Duration,
}

impl Breaker {
    /// A closed breaker.
    #[must_use]
    pub fn new(threshold: u32, cooldown: Duration) -> Self {
        Self {
            failures: 0,
            open_until: None,
            threshold,
            cooldown,
        }
    }

    /// True when a call may be made now. There is no half-open probe (as in Python).
    #[must_use]
    pub fn allow(&self, now: Instant) -> bool {
        self.open_until.is_none_or(|t| now >= t)
    }

    /// Record one outcome. Success resets the count. The threshold-th failure opens the breaker and
    /// resets the count to zero.
    pub fn record(&mut self, ok: bool, now: Instant) {
        if ok {
            self.failures = 0;
            return;
        }
        self.failures += 1;
        if self.failures >= self.threshold {
            self.open_until = Some(now + self.cooldown);
            self.failures = 0;
        }
    }
}

/// Everything the handlers share. Cloning is cheap (an `Arc`).
#[derive(Clone)]
pub struct AppState(Arc<Inner>);

impl std::ops::Deref for AppState {
    type Target = Inner;
    fn deref(&self) -> &Inner {
        &self.0
    }
}

/// The state behind [`AppState`]. Fields are public within the crate.
pub struct Inner {
    /// Process configuration.
    pub config: Config,
    /// Shared outbound HTTP client (Open-Meteo, NWS, geocoder, Photon, LLM, USGS).
    pub http: reqwest::Client,
    /// Trusted proxies for `X-Forwarded-For`.
    pub trusted: TrustedProxies,
    /// Per-client token bucket.
    pub limiter: Mutex<RateLimiter>,
    /// Counters and the HTTP histogram.
    pub metrics: Metrics,
    /// Storm detection and prefetch settings.
    pub storm_cfg: StormConfig,
    /// Nowcast skill-log settings.
    pub skill_cfg: SkillConfig,
    /// Single-entry manifest snapshot with its expiry.
    pub manifest: Mutex<Option<(Instant, Arc<ManifestSnapshot>)>>,
    /// Last successfully read manifest, served when a read fails.
    pub manifest_last_good: Mutex<Option<Arc<Value>>>,
    /// Single-entry skill summary, re-read only when the skill log changes.
    pub skill: Mutex<Option<crate::nowcast::SkillCache>>,
    /// Open-Meteo bodies by rounded cell.
    pub forecast_cache: AsyncCache<String, Bytes>,
    /// Wind-field responses by resolved timestamp (TTL 300 s).
    pub wind_cache: Cache<String, Bytes>,
    /// Nowcast point bodies (without `skill`) by `(run_id, lat3, lon3)`. No TTL.
    pub nowcast_cache: Cache<(String, String, String), Bytes>,
    /// The national NWS alert feed, one entry (TTL `ALERTS_TTL_S`).
    pub alert_feed: AsyncCache<(), Arc<AlertFeed>>,
    /// NWS zones by 3-dp point key (TTL one day).
    pub alert_zones: AsyncCache<String, Arc<PointZones>>,
    /// Direct `?point=` alert bodies by 3-dp point key, for points without zones.
    pub alerts_cache: AsyncCache<String, Bytes>,
    /// Encoded SPC outlook responses by day (1..=3).
    pub outlook_cache: Cache<i64, Bytes>,
    /// Geocoder search results by `query|limit`.
    pub geocode_cache: Cache<String, Bytes>,
    /// Reverse-geocode results by 3-dp point.
    pub reverse_cache: Cache<String, Bytes>,
    /// Briefing and explainer bodies (TTL `BRIEFING_TTL_S`, max 512).
    pub briefing_cache: Cache<String, Arc<Value>>,
    /// The LLM breaker.
    pub breaker: Mutex<Breaker>,
    /// LLM concurrency guard. Acquisition is non-blocking, so callers never queue.
    pub llm_slots: Semaphore,
    /// Single-flight map for briefings: followers wait on the leader's watch channel.
    pub inflight: Mutex<HashMap<String, watch::Receiver<Option<Arc<Value>>>>>,
    /// USGS imagery tiles by `(z, x, y)`, weighted by body length. Each entry keeps the forwarded
    /// upstream headers (`ETag`, `Last-Modified`) so a hit answers exactly like a miss.
    pub imagery_cache: Cache<(u8, u64, u64), CachedImagery>,
    /// Satellite tiles by `(epoch minute, z, x, y)`, weighted by body length. `try_get_with` makes
    /// concurrent misses for one tile share one GIBS fetch.
    pub satellite_tiles: moka::future::Cache<crate::satellite::TileKey, Bytes>,
    /// GIBS frame times for the `satellite` manifest layer.
    pub satellite_domain: crate::satellite::DomainCache,
    /// Lazily connected Temporal client. Reset to `None` after an RPC failure.
    pub temporal: AsyncMutex<Option<temporalio_client::Client>>,
    /// The basemap archive, opened on first use and reopened when the file is replaced (F6).
    pub pmtiles: AsyncMutex<Option<crate::basemap::OpenArchive>>,
    /// When the archive file was last found missing while an upstream is set. Requests proxy without
    /// retrying `open` until the recheck interval passes. `None` once the archive opens.
    pub pmtiles_missing: Mutex<Option<Instant>>,
    /// SQLite push-token store under `STATE_DIR`.
    pub push_tokens: PushTokenStore,
}

/// A moka weight in bytes, saturating at `u32::MAX`.
#[must_use]
pub fn weight(bytes: usize) -> u32 {
    u32::try_from(bytes).unwrap_or(u32::MAX)
}

impl AppState {
    /// State from the process environment (`Config::from_env`).
    ///
    /// # Errors
    /// Fails on an invalid environment value or an invalid `TRUSTED_PROXIES` entry.
    pub fn from_env() -> Result<Self> {
        Self::new(Config::from_env()?)
    }

    /// State from an explicit configuration. Does not touch the network: the Temporal connection
    /// is made on the first `/v1` request that needs it.
    ///
    /// # Errors
    /// Fails on an invalid `TRUSTED_PROXIES` entry, a bad storm or skill setting, or an HTTP client
    /// build error.
    pub fn new(config: Config) -> Result<Self> {
        let trusted = TrustedProxies::with_extra(&config.trusted_proxies_extra)
            .map_err(|e| anyhow::anyhow!("TRUSTED_PROXIES: {e}"))?;
        let http = reqwest::Client::builder()
            .pool_max_idle_per_host(20)
            .build()
            .context("building the upstream HTTP client")?;
        let storm_cfg = StormConfig::from_env().context("storm configuration")?;
        let skill_cfg = SkillConfig::from_env().context("nowcast skill configuration")?;
        let ttl = |secs: u64| Duration::from_secs(secs);
        let imagery_cache = Cache::builder()
            .max_capacity(config.imagery_cache_max_bytes)
            .weigher(|_k: &(u8, u64, u64), v: &CachedImagery| weight(v.body.len()))
            .time_to_live(Duration::from_secs(604_800))
            .build();
        let satellite_tiles = moka::future::Cache::builder()
            .max_capacity(config.satellite_cache_max_bytes)
            .weigher(|_k: &crate::satellite::TileKey, v: &Bytes| {
                u32::try_from(v.len()).unwrap_or(u32::MAX)
            })
            .time_to_live(Duration::from_secs(
                crate::satellite::ACCEPT_WINDOW_S.unsigned_abs(),
            ))
            .build();
        let inner = Inner {
            limiter: Mutex::new(RateLimiter::new(
                config.rate_limit_rps,
                config.rate_limit_burst,
            )),
            trusted,
            http,
            metrics: Metrics::default(),
            storm_cfg,
            skill_cfg,
            manifest: Mutex::new(None),
            manifest_last_good: Mutex::new(None),
            skill: Mutex::new(None),
            forecast_cache: AsyncCache::builder()
                .max_capacity(config.forecast_cache_max_bytes)
                .weigher(|k: &String, v: &Bytes| weight(k.len() + v.len()))
                .time_to_live(ttl(config.forecast_ttl_s))
                .build(),
            wind_cache: Cache::builder()
                .max_capacity(config.wind_cache_max_bytes)
                .weigher(|k: &String, v: &Bytes| weight(k.len() + v.len()))
                .time_to_live(ttl(300))
                .build(),
            nowcast_cache: Cache::builder()
                .max_capacity(config.nowcast_point_cache_max_bytes)
                .weigher(|k: &(String, String, String), v: &Bytes| {
                    weight(k.0.len() + k.1.len() + k.2.len() + v.len())
                })
                .build(),
            alert_feed: AsyncCache::builder()
                .max_capacity(1)
                .time_to_live(ttl(config.alerts_ttl_s))
                .build(),
            alert_zones: AsyncCache::builder()
                .max_capacity(config.alerts_cache_max_bytes)
                .weigher(|k: &String, v: &Arc<PointZones>| weight(k.len() + v.weight()))
                .time_to_live(ZONES_TTL)
                .build(),
            alerts_cache: AsyncCache::builder()
                .max_capacity(config.alerts_cache_max_bytes)
                .weigher(|k: &String, v: &Bytes| weight(k.len() + v.len()))
                .time_to_live(ttl(config.alerts_ttl_s))
                .build(),
            outlook_cache: Cache::builder()
                .max_capacity(3)
                .time_to_live(ttl(crate::outlooks::OUTLOOK_TTL_S))
                .build(),
            geocode_cache: Cache::builder()
                .max_capacity(config.geocode_cache_max_bytes)
                .weigher(|k: &String, v: &Bytes| weight(k.len() + v.len()))
                .time_to_live(ttl(config.geocode_ttl_s))
                .build(),
            reverse_cache: Cache::builder()
                .max_capacity(config.geocode_cache_max_bytes)
                .weigher(|k: &String, v: &Bytes| weight(k.len() + v.len()))
                .time_to_live(ttl(config.geocode_ttl_s))
                .build(),
            briefing_cache: Cache::builder()
                .max_capacity(512)
                .time_to_live(ttl(config.briefing_ttl_s))
                .build(),
            breaker: Mutex::new(Breaker::new(
                config.llm_breaker_failures,
                Duration::from_secs(config.llm_breaker_cooldown_s),
            )),
            llm_slots: Semaphore::new(config.llm_max_concurrency.max(1)),
            inflight: Mutex::new(HashMap::new()),
            imagery_cache,
            satellite_tiles,
            satellite_domain: crate::satellite::DomainCache::default(),
            temporal: AsyncMutex::new(None),
            pmtiles: AsyncMutex::new(None),
            pmtiles_missing: Mutex::new(None),
            push_tokens: PushTokenStore::new(config.state_dir.clone()),
            config,
        };
        Ok(Self(Arc::new(inner)))
    }

    /// Path of the nowcast status file (`STATE_DIR/nowcast-status.json`).
    #[must_use]
    pub fn state_file(&self, name: &str) -> PathBuf {
        self.config.state_dir.join(name)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn response_caches_are_bounded_by_bytes_not_entries() {
        let dir = std::env::temp_dir();
        let mut config = Config::defaults_for(dir.clone(), dir.clone(), dir);
        config.geocode_cache_max_bytes = 10_000;
        let state = AppState::new(config).expect("state");
        for i in 0..40 {
            state
                .geocode_cache
                .insert(format!("q{i}"), Bytes::from(vec![b'x'; 1000]));
        }
        state.geocode_cache.run_pending_tasks();
        assert!(state.geocode_cache.weighted_size() <= 10_000);
        assert!(state.geocode_cache.entry_count() < 10);
        // One body larger than the whole budget is not kept.
        state
            .geocode_cache
            .insert("big".into(), Bytes::from(vec![b'x'; 20_000]));
        state.geocode_cache.run_pending_tasks();
        assert!(state.geocode_cache.get("big").is_none());
    }

    #[test]
    fn breaker_opens_at_threshold_and_resets_count() {
        // Spec 15.7 #48: with threshold 3, the third consecutive failure opens the breaker.
        let t0 = Instant::now();
        let mut b = Breaker::new(3, Duration::from_secs(300));
        b.record(false, t0);
        b.record(false, t0);
        assert!(b.allow(t0));
        b.record(false, t0);
        assert!(!b.allow(t0));
        assert!(!b.allow(t0 + Duration::from_secs(299)));
        assert!(b.allow(t0 + Duration::from_secs(300)));
    }

    #[test]
    fn breaker_success_resets_failures() {
        let t0 = Instant::now();
        let mut b = Breaker::new(2, Duration::from_secs(10));
        b.record(false, t0);
        b.record(true, t0);
        b.record(false, t0);
        assert!(b.allow(t0));
    }
}
