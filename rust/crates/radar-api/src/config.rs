//! Process configuration, read once at startup from the environment.
//!
//! Names and defaults follow `spec-api.md` section 14 (the **code** defaults, where the docs
//! disagree). Python reads most of these at import time; the port reads them once in
//! [`Config::from_env`]. Numeric values use the strict `radar_core::env` parsers: a value that
//! does not parse is a startup error, not a silent default.

use std::net::SocketAddr;
use std::path::PathBuf;

use anyhow::{Context, Result};
use radar_core::env;

const MIB: i64 = 1024 * 1024;

/// Every setting the API reads. Fields are public so tests and the `radar` binary can override them.
#[derive(Debug, Clone)]
pub struct Config {
    /// Listen address. `LISTEN_ADDR`, default `0.0.0.0:8080` (the Caddy port this replaces).
    pub listen_addr: SocketAddr,
    /// `TILE_DIR`, served at `/tiles/*`. Default `/data/tiles`.
    pub tile_dir: PathBuf,
    /// `GRID_DIR`, read by the inspector, nowcast and wind routes. Default `/data/grids`.
    pub grid_dir: PathBuf,
    /// `STATE_DIR`, holds manifest, storms, lightning, tropical, nowcast files and push tokens.
    pub state_dir: PathBuf,
    /// `BASEMAP_ROOT`, the static root for `/basemap/styles/*` and `/basemap/fonts/*`
    /// (Caddy's `root * /srv/basemap`). Default `/srv/basemap`.
    pub basemap_root: PathBuf,
    /// `BASEMAP_PMTILES`, the archive served at `/basemap/tiles/{z}/{x}/{y}.mvt`.
    /// Default `/data/basemap.pmtiles` (where the image copies it).
    pub pmtiles_path: PathBuf,
    /// `BASEMAP_PMTILES_RECHECK_S`. Seconds between identity checks of the archive file (F6). A replaced
    /// file is reopened at the next check after the interval. Default 10. Zero checks on every request.
    pub pmtiles_recheck_s: u64,
    /// `OPEN_METEO_BASE`. Default `https://api.open-meteo.com/v1/forecast`.
    pub open_meteo_base: String,
    /// `MRMS_MAX_AGE_S`, staleness threshold for health. Default 600.
    pub mrms_max_age_s: i64,
    /// `FORECAST_TTL_S`. Default 900 (code), not the 300 in the docs.
    pub forecast_ttl_s: u64,
    /// `FORECAST_CACHE_MAX_BYTES` (new; replaces `FORECAST_CACHE_MAX_ENTRIES`). Default 64 MiB.
    pub forecast_cache_max_bytes: u64,
    /// `WIND_CACHE_MAX_BYTES` (new; replaces `WIND_CACHE_MAX_ENTRIES`). Default 32 MiB.
    pub wind_cache_max_bytes: u64,
    /// `NOWCAST_POINT_CACHE_MAX_BYTES` (new; replaces `NOWCAST_POINT_CACHE_MAX_ENTRIES`). Default 16 MiB.
    pub nowcast_point_cache_max_bytes: u64,
    /// `API_RATE_LIMIT_RPS`. Default 20.
    pub rate_limit_rps: f64,
    /// `API_RATE_LIMIT_BURST`. Default 60.
    pub rate_limit_burst: f64,
    /// `DISABLE_WORKFLOW_ROUTES`: only the exact value `1` disables the `/v1` routes.
    pub workflow_routes_enabled: bool,
    /// `WORKFLOW_AUTH_SIGNING_KEY`, falling back to `WORKFLOW_API_SIGNING_KEY`. Empty means unconfigured.
    pub workflow_signing_key: Vec<u8>,
    /// `TEMPORAL_ADDRESS`. Default `localhost:7233` (code).
    pub temporal_address: String,
    /// `TEMPORAL_NAMESPACE`. Default `default`.
    pub temporal_namespace: String,
    /// `TEMPORAL_ALERTS_TASK_QUEUE`. Default `radar-ng-alerts`.
    pub temporal_alerts_task_queue: String,
    /// `LLM_BASE_URL` without a trailing slash. Empty disables the LLM routes.
    pub llm_base_url: String,
    /// `LLM_API_KEY`.
    pub llm_api_key: String,
    /// `LLM_MODEL`. Default `qwen3.8-27b`.
    pub llm_model: String,
    /// `LLM_TIMEOUT_S`. Default 20.
    pub llm_timeout_s: f64,
    /// `LLM_MAX_CONCURRENCY`. Default 2.
    pub llm_max_concurrency: usize,
    /// `BRIEFING_TTL_S`, also the explainer cache TTL. Default 600.
    pub briefing_ttl_s: u64,
    /// `LLM_BREAKER_FAILURES`. Default 3.
    pub llm_breaker_failures: u32,
    /// `LLM_BREAKER_COOLDOWN_S`. Default 300.
    pub llm_breaker_cooldown_s: u64,
    /// `NWS_POINT_ALERTS_URL`. Default `https://api.weather.gov/alerts/active`.
    pub nws_point_alerts_url: String,
    /// `NWS_ALERT_DETAIL_BASE` (new, defect D14). Default `https://api.weather.gov/alerts`.
    pub nws_alert_detail_base: String,
    /// `NWS_USER_AGENT`. Default `(radar-ng, admin@example.com)` as in the code.
    pub nws_user_agent: String,
    /// `SPC_OUTLOOK_BASE`. Default `https://www.spc.noaa.gov/products/outlook`.
    pub spc_outlook_base: String,
    /// `ALERTS_TTL_S`. Default 60.
    pub alerts_ttl_s: u64,
    /// `ALERTS_CACHE_MAX_BYTES` (new; replaces `ALERTS_CACHE_MAX_ENTRIES`). Budget for the point
    /// fallback bodies and, separately, for the point-to-zone lookups. Default 16 MiB.
    pub alerts_cache_max_bytes: u64,
    /// `NWS_POINTS_URL` (new). Default `https://api.weather.gov/points`.
    pub nws_points_url: String,
    /// `GEOCODER_URL`, wins over Photon when set.
    pub geocoder_url: String,
    /// `PHOTON_URL`.
    pub photon_url: String,
    /// `GEOCODE_TTL_S`. Default 86400.
    pub geocode_ttl_s: u64,
    /// `GEOCODE_CACHE_MAX_BYTES` (new; replaces `GEOCODE_CACHE_MAX_ENTRIES`). Default 8 MiB each for
    /// the forward and reverse caches.
    pub geocode_cache_max_bytes: u64,
    /// `PLACES_UPSTREAM_TIMEOUT_S`. Default 6.
    pub places_timeout_s: f64,
    /// `USGS_IMAGERY_BASE` (new). Default `https://basemap.nationalmap.gov`.
    pub usgs_imagery_base: String,
    /// `BASEMAP_TILES_UPSTREAM`: a go-pmtiles server to proxy `/basemap/tiles/*` to when the local
    /// `BASEMAP_PMTILES` archive is absent (Caddy's `reverse_proxy basemap:8081`). Default
    /// `http://basemap:8081`, so the image drops into a cluster that still runs the basemap service.
    /// Empty disables the fallback.
    pub basemap_tiles_upstream: String,
    /// `IMAGERY_CACHE_MAX_BYTES` (new, moka). Default 128 MiB. Zero disables the cache.
    pub imagery_cache_max_bytes: u64,
    /// `GIBS_WMTS_BASE`: NASA GIBS WMTS root for the satellite layer (tiles and DescribeDomains).
    /// Default `https://gibs.earthdata.nasa.gov/wmts/epsg3857/nrt`. Empty turns the satellite layer off.
    pub gibs_wmts_base: String,
    /// `SATELLITE_CACHE_MAX_BYTES`: moka cache for satellite tiles. Default 128 MiB. Zero disables it.
    pub satellite_cache_max_bytes: u64,
    /// `TRUSTED_PROXIES` (new): extra comma-separated IPs or CIDRs trusted for `X-Forwarded-For`,
    /// in addition to the private ranges and loopback.
    pub trusted_proxies_extra: String,
}

impl Config {
    /// Read every setting from the process environment.
    ///
    /// # Errors
    /// Fails when a numeric variable is set to a value that does not parse, or when
    /// `LISTEN_ADDR` is not a socket address.
    pub fn from_env() -> Result<Self> {
        let listen = str_var("LISTEN_ADDR", "0.0.0.0:8080");
        let listen_addr: SocketAddr = listen
            .parse()
            .with_context(|| format!("LISTEN_ADDR is not a socket address: {listen}"))?;
        let signing_key = std::env::var("WORKFLOW_AUTH_SIGNING_KEY")
            .or_else(|_| std::env::var("WORKFLOW_API_SIGNING_KEY"))
            .unwrap_or_default();
        Ok(Self {
            listen_addr,
            tile_dir: env::path_var("TILE_DIR", "/data/tiles"),
            grid_dir: env::path_var("GRID_DIR", "/data/grids"),
            state_dir: env::path_var("STATE_DIR", "/data/state"),
            basemap_root: env::path_var("BASEMAP_ROOT", "/srv/basemap"),
            pmtiles_path: env::path_var("BASEMAP_PMTILES", "/data/basemap.pmtiles"),
            pmtiles_recheck_s: pos_u64("BASEMAP_PMTILES_RECHECK_S", 10)?,
            open_meteo_base: str_var("OPEN_METEO_BASE", "https://api.open-meteo.com/v1/forecast"),
            mrms_max_age_s: env::int_var("MRMS_MAX_AGE_S", 600, 0, i64::MAX)?,
            forecast_ttl_s: pos_u64("FORECAST_TTL_S", 900)?,
            forecast_cache_max_bytes: pos_u64("FORECAST_CACHE_MAX_BYTES", 64 * MIB)?,
            wind_cache_max_bytes: pos_u64("WIND_CACHE_MAX_BYTES", 32 * MIB)?,
            nowcast_point_cache_max_bytes: pos_u64("NOWCAST_POINT_CACHE_MAX_BYTES", 16 * MIB)?,
            rate_limit_rps: env::float_var("API_RATE_LIMIT_RPS", 20.0)?,
            rate_limit_burst: env::float_var("API_RATE_LIMIT_BURST", 60.0)?,
            workflow_routes_enabled: std::env::var("DISABLE_WORKFLOW_ROUTES").as_deref() != Ok("1"),
            workflow_signing_key: signing_key.into_bytes(),
            temporal_address: str_var("TEMPORAL_ADDRESS", "localhost:7233"),
            temporal_namespace: str_var("TEMPORAL_NAMESPACE", "default"),
            temporal_alerts_task_queue: str_var("TEMPORAL_ALERTS_TASK_QUEUE", "radar-ng-alerts"),
            llm_base_url: str_var("LLM_BASE_URL", "")
                .trim_end_matches('/')
                .to_string(),
            llm_api_key: str_var("LLM_API_KEY", ""),
            llm_model: str_var("LLM_MODEL", "qwen3.8-27b"),
            llm_timeout_s: env::float_var("LLM_TIMEOUT_S", 20.0)?,
            llm_max_concurrency: pos_u64("LLM_MAX_CONCURRENCY", 2)? as usize,
            briefing_ttl_s: pos_u64("BRIEFING_TTL_S", 600)?,
            llm_breaker_failures: pos_u64("LLM_BREAKER_FAILURES", 3)? as u32,
            llm_breaker_cooldown_s: pos_u64("LLM_BREAKER_COOLDOWN_S", 300)?,
            spc_outlook_base: str_var(
                "SPC_OUTLOOK_BASE",
                "https://www.spc.noaa.gov/products/outlook",
            ),
            nws_point_alerts_url: str_var(
                "NWS_POINT_ALERTS_URL",
                "https://api.weather.gov/alerts/active",
            ),
            nws_alert_detail_base: str_var(
                "NWS_ALERT_DETAIL_BASE",
                "https://api.weather.gov/alerts",
            ),
            nws_user_agent: str_var("NWS_USER_AGENT", "(radar-ng, admin@example.com)"),
            alerts_ttl_s: pos_u64("ALERTS_TTL_S", 60)?,
            alerts_cache_max_bytes: pos_u64("ALERTS_CACHE_MAX_BYTES", 16 * MIB)?,
            nws_points_url: str_var("NWS_POINTS_URL", "https://api.weather.gov/points"),
            geocoder_url: str_var("GEOCODER_URL", "")
                .trim_end_matches('/')
                .to_string(),
            photon_url: str_var("PHOTON_URL", "").trim_end_matches('/').to_string(),
            geocode_ttl_s: pos_u64("GEOCODE_TTL_S", 86_400)?,
            geocode_cache_max_bytes: pos_u64("GEOCODE_CACHE_MAX_BYTES", 8 * MIB)?,
            places_timeout_s: env::float_var("PLACES_UPSTREAM_TIMEOUT_S", 6.0)?,
            usgs_imagery_base: str_var("USGS_IMAGERY_BASE", "https://basemap.nationalmap.gov")
                .trim_end_matches('/')
                .to_string(),
            basemap_tiles_upstream: str_var("BASEMAP_TILES_UPSTREAM", "http://basemap:8081")
                .trim_end_matches('/')
                .to_string(),
            imagery_cache_max_bytes: pos_u64("IMAGERY_CACHE_MAX_BYTES", 128 * MIB)?,
            gibs_wmts_base: str_var(
                "GIBS_WMTS_BASE",
                "https://gibs.earthdata.nasa.gov/wmts/epsg3857/nrt",
            )
            .trim_end_matches('/')
            .to_string(),
            satellite_cache_max_bytes: pos_u64("SATELLITE_CACHE_MAX_BYTES", 128 * MIB)?,
            trusted_proxies_extra: str_var("TRUSTED_PROXIES", ""),
        })
    }

    /// A configuration with every upstream and directory at its default, for tests.
    /// Callers overwrite the fields they need.
    #[must_use]
    pub fn defaults_for(state_dir: PathBuf, grid_dir: PathBuf, tile_dir: PathBuf) -> Self {
        Self {
            listen_addr: "127.0.0.1:8080".parse().expect("static address"),
            tile_dir,
            grid_dir,
            state_dir,
            basemap_root: PathBuf::from("/srv/basemap"),
            pmtiles_path: PathBuf::from("/data/basemap.pmtiles"),
            pmtiles_recheck_s: 10,
            open_meteo_base: "https://api.open-meteo.com/v1/forecast".into(),
            mrms_max_age_s: 600,
            forecast_ttl_s: 900,
            forecast_cache_max_bytes: 64 << 20,
            wind_cache_max_bytes: 32 << 20,
            nowcast_point_cache_max_bytes: 16 << 20,
            rate_limit_rps: 20.0,
            rate_limit_burst: 60.0,
            workflow_routes_enabled: true,
            workflow_signing_key: Vec::new(),
            temporal_address: "localhost:7233".into(),
            temporal_namespace: "default".into(),
            temporal_alerts_task_queue: "radar-ng-alerts".into(),
            llm_base_url: String::new(),
            llm_api_key: String::new(),
            llm_model: "qwen3.8-27b".into(),
            llm_timeout_s: 20.0,
            llm_max_concurrency: 2,
            briefing_ttl_s: 600,
            llm_breaker_failures: 3,
            llm_breaker_cooldown_s: 300,
            nws_point_alerts_url: "https://api.weather.gov/alerts/active".into(),
            spc_outlook_base: "https://www.spc.noaa.gov/products/outlook".into(),
            nws_alert_detail_base: "https://api.weather.gov/alerts".into(),
            nws_user_agent: "(radar-ng, admin@example.com)".into(),
            alerts_ttl_s: 60,
            alerts_cache_max_bytes: 16 << 20,
            nws_points_url: "https://api.weather.gov/points".into(),
            geocoder_url: String::new(),
            photon_url: String::new(),
            geocode_ttl_s: 86_400,
            geocode_cache_max_bytes: 8 << 20,
            places_timeout_s: 6.0,
            usgs_imagery_base: "https://basemap.nationalmap.gov".into(),
            basemap_tiles_upstream: String::new(),
            imagery_cache_max_bytes: 0,
            gibs_wmts_base: String::new(),
            satellite_cache_max_bytes: 0,
            trusted_proxies_extra: String::new(),
        }
    }
}

/// A string variable that keeps an empty value (Python `os.environ.get(name, default)` semantics).
fn str_var(name: &str, default: &str) -> String {
    std::env::var(name).unwrap_or_else(|_| default.to_string())
}

/// A non-negative integer variable with the strict `radar_core::env` parser.
fn pos_u64(name: &str, default: i64) -> Result<u64> {
    let v = env::int_var(name, default, 0, i64::MAX)?;
    u64::try_from(v).context("negative value")
}
