//! Level II shadow recorder (#293). A small process that holds demand for the pilot stations
//! through the same HTTP path a client uses, and records what it saw at the edge. The pool writes
//! the upstream clocks ([`crate::clocks`]); this joins them into one row per lowest sweep.
//!
//! Docs: `docs/develop/level2-shadow.md` (field names, valid days, how to read the report).

use crate::{clocks::SweepClock, leases::Pool};
use anyhow::{Context, Result, bail, ensure};
use async_trait::async_trait;
use chrono::{DateTime, NaiveDate, TimeDelta, Utc};
use serde::Serialize;
use serde_json::{Value, json};
use std::{
    collections::{BTreeMap, HashSet},
    sync::Arc,
    time::{Duration, Instant},
};

pub const ENABLED_ENV: &str = "LEVEL2_SHADOW_ENABLED";
/// A day counts when the recorder observed every pilot station for this share of it.
pub const VALID_UPTIME: f64 = 0.95;
/// Spacing between indexed lowest sweeps above this is a feed gap (default 15 minutes).
pub const GAP_S_DEFAULT: i64 = 900;
const DAY_S: f64 = 86_400.0;
const SEEN_MEMORY: usize = 64;

#[derive(clap::Args)]
pub struct ShadowArgs {
    #[command(subcommand)]
    pub cmd: ShadowCmd,
}

#[derive(clap::Subcommand)]
pub enum ShadowCmd {
    /// Run the recorder until stopped (needs LEVEL2_SHADOW_ENABLED=true).
    Record,
    /// Store and print the summary for one UTC day, Prometheus figures included (default: yesterday).
    Summary {
        #[arg(long)]
        day: Option<NaiveDate>,
    },
    /// Print the pilot report over the last N UTC days.
    Report {
        #[arg(long, default_value_t = 30)]
        days: u32,
        /// Last day included (default: yesterday).
        #[arg(long)]
        until: Option<NaiveDate>,
        #[arg(long)]
        json: bool,
    },
    /// Leave a UTC day out of the report (drills, known outages). The rows stay stored.
    Exclude {
        #[arg(long)]
        day: NaiveDate,
        #[arg(long)]
        reason: String,
    },
}

fn required(name: &str) -> Result<String> {
    std::env::var(name).map_err(|_| anyhow::anyhow!("{name} required"))
}

/// The explicit pilot list. There is no default: an unset list is an error, never KGRR.
pub fn stations_from(raw: &str) -> Result<Vec<String>> {
    let list: Vec<String> = raw
        .split(',')
        .map(|s| s.trim().to_ascii_uppercase())
        .filter(|s| !s.is_empty())
        .collect();
    ensure!(!list.is_empty(), "LEVEL2_ENABLED_STATIONS is empty");
    ensure!(
        list.iter()
            .all(|s| s.len() == 4 && s.bytes().all(|b| b.is_ascii_alphanumeric())),
        "LEVEL2_ENABLED_STATIONS must be ICAO ids"
    );
    Ok(list)
}

#[derive(Debug, Clone)]
pub struct Config {
    pub stations: Vec<String>,
    pub api: String,
    pub edge: String,
    pub zoom: u32,
    pub palette: String,
    pub cold_every: Option<Duration>,
    pub gap_s: i64,
    pub prom: Option<String>,
    pub prom_namespace: String,
    pub prom_pods: String,
}

impl Config {
    pub fn from_env() -> Result<Self> {
        let api = required("LEVEL2_SHADOW_API_URL")?
            .trim_end_matches('/')
            .to_owned();
        let edge = std::env::var("LEVEL2_SHADOW_EDGE_URL")
            .map(|s| s.trim_end_matches('/').to_owned())
            .unwrap_or_else(|_| api.clone());
        let cold = radar_core::env::int_var("LEVEL2_SHADOW_COLD_EVERY_S", 1800, 0, 86_400)?;
        Ok(Self {
            stations: stations_from(&required("LEVEL2_ENABLED_STATIONS")?)?,
            api,
            edge,
            zoom: radar_core::env::int_var("LEVEL2_SHADOW_ZOOM", 9, 8, 13)? as u32,
            palette: std::env::var("LEVEL2_SHADOW_PALETTE").unwrap_or_else(|_| "classic".into()),
            cold_every: (cold > 0).then(|| Duration::from_secs(cold as u64)),
            gap_s: radar_core::env::int_var("LEVEL2_SHADOW_GAP_S", GAP_S_DEFAULT, 60, 86_400)?,
            prom: std::env::var("LEVEL2_SHADOW_PROM_URL")
                .ok()
                .filter(|s| !s.is_empty()),
            prom_namespace: std::env::var("LEVEL2_SHADOW_PROM_NAMESPACE")
                .unwrap_or_else(|_| "radar-ng".into()),
            prom_pods: std::env::var("LEVEL2_SHADOW_PROM_PODS")
                .unwrap_or_else(|_| "radar-ng-worker-level2.*|level2-.*".into()),
        })
    }

    /// Seconds between view submits per station, kept under the API's 0.5/s per-client budget.
    pub fn submit_every(&self) -> Duration {
        Duration::from_secs_f64((self.stations.len() as f64 * 2.5).max(15.0))
    }
}

/// What the recorder saw at the edge for one lowest sweep.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct SeenSweep {
    pub station: String,
    pub sweep_id: String,
    pub scan_end: DateTime<Utc>,
    pub view_id: String,
    pub revision: String,
    /// `issued_at` of the first revision that carried this sweep as its newest frame.
    pub manifest_ready_at: DateTime<Utc>,
    /// Edge fetch time of that manifest.
    pub manifest_ms: f64,
    /// When the first tile of the sweep finished downloading through the edge.
    pub edge_display_at: DateTime<Utc>,
    pub tile_cold_ms: f64,
    pub tile_cached_ms: Option<f64>,
    pub tile_status: i32,
    /// Edge cache header of the second fetch (`cf-cache-status`, `x-cache` or `age`), if any.
    pub tile_cache: Option<String>,
}

/// A discrete happening: fallback onset, cold view result, timeout, error, rate limit.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Event {
    pub at: DateTime<Utc>,
    pub station: String,
    /// `fallback`, `cold_ready`, `cold_timeout`, `cold_fallback`, `error` or `rate_limited`.
    pub kind: String,
    pub reason: Option<String>,
    pub seconds: Option<f64>,
    pub tile_ms: Option<f64>,
}

/// Where recorder output goes. Postgres in production; a vector in tests.
#[async_trait]
pub trait Sink: Send + Sync {
    async fn seen(&self, row: &SeenSweep) -> Result<()>;
    async fn event(&self, e: &Event) -> Result<()>;
    /// Adds `seconds` of observed `state` (`ready`, `pending`, `fallback:<reason>`, `error`).
    async fn state(&self, day: NaiveDate, station: &str, state: &str, seconds: f64) -> Result<()>;
}

pub struct Recorder {
    pub cfg: Config,
    pub http: reqwest::Client,
    pub sink: Arc<dyn Sink>,
}

struct Site {
    id: String,
    lat: f64,
    lon: f64,
}

#[derive(Debug)]
enum Reply {
    Status(Value),
    RateLimited(u64),
    Failed(String),
}

fn tile_of(lat: f64, lon: f64, z: u32) -> [u32; 2] {
    let n = f64::from(1u32 << z);
    let x = crate::viewport::tile_x(lon, z).clamp(0.0, n - 1.0) as u32;
    let y = crate::viewport::tile_y(lat, z).clamp(0.0, n - 1.0) as u32;
    [x, y]
}

fn ms(d: Duration) -> f64 {
    d.as_secs_f64() * 1000.0
}

fn cache_header(h: &reqwest::header::HeaderMap) -> Option<String> {
    ["cf-cache-status", "x-cache", "age"]
        .iter()
        .find_map(|k| h.get(*k).and_then(|v| v.to_str().ok()))
        .map(|v| v.chars().take(32).collect())
}

/// The newest frame of a viewport envelope: (sweep id, scan end, first tile path).
fn newest_frame(
    env: &Value,
    station: &str,
    palette: &str,
) -> Result<(String, DateTime<Utc>, String)> {
    let layer = &env["manifest"]["layers"][format!("level2-ref-{station}")];
    let frame = layer["frames"]
        .as_array()
        .and_then(|f| f.last())
        .context("manifest without frames")?;
    let path = frame["path"].as_str().context("frame path")?;
    let sweep = frame["sweep_id"]
        .as_str()
        .or_else(|| path.split('/').next())
        .context("frame sweep_id")?;
    let end: DateTime<Utc> = frame["scan_end"]
        .as_str()
        .or_else(|| frame["timestamp"].as_str())
        .context("frame scan_end")?
        .parse()?;
    let t = env["required_tiles"]
        .as_array()
        .and_then(|t| t.first())
        .context("no required tiles")?;
    let url = env["tile_url_template"]
        .as_str()
        .context("tile template")?
        .replace("{station}", station)
        .replace("{path}", path)
        .replace("{palette}", palette)
        .replace("{z}", &t["z"].to_string())
        .replace("{x}", &t["x"].to_string())
        .replace("{y}", &t["y"].to_string());
    Ok((sweep.to_owned(), end, url))
}

impl Recorder {
    pub fn new(cfg: Config, sink: Arc<dyn Sink>) -> Result<Self> {
        Ok(Self {
            cfg,
            http: reqwest::Client::builder()
                .timeout(Duration::from_secs(30))
                .redirect(reqwest::redirect::Policy::none())
                .user_agent("radar-ng-level2-shadow")
                .build()?,
            sink,
        })
    }

    async fn sites(&self) -> Result<Vec<Site>> {
        let body = self
            .http
            .get(format!("{}/api/level2/sites", self.cfg.api))
            .send()
            .await?
            .error_for_status()
            .context("GET /api/level2/sites (is the Level II gate on for this API?)")?
            .bytes()
            .await?;
        let v: Value = serde_json::from_slice(&body)?;
        let mut out = vec![];
        for id in &self.cfg.stations {
            let site = v["sites"]
                .as_array()
                .and_then(|s| s.iter().find(|s| s["id"] == id.as_str()))
                .with_context(|| format!("{id} not in /api/level2/sites"))?;
            ensure!(
                site["enabled"] == true,
                "{id} is not enabled on {}: check LEVEL2_ENABLED_STATIONS there",
                self.cfg.api
            );
            out.push(Site {
                id: id.clone(),
                lat: site["lat"].as_f64().context("lat")?,
                lon: site["lon"].as_f64().context("lon")?,
            });
        }
        Ok(out)
    }

    async fn submit(&self, site: &Site, zoom: u32) -> Reply {
        let body = json!({
            "centre": {"lat": site.lat, "lon": site.lon},
            "zoom": zoom,
            "tiles": [tile_of(site.lat, site.lon, zoom)],
            "palette": self.cfg.palette,
        });
        let sent = self
            .http
            .post(format!("{}/api/level2/views", self.cfg.api))
            .header("content-type", "application/json")
            .body(body.to_string())
            .send()
            .await;
        read_reply(sent).await
    }

    async fn status(&self, view: &str) -> Reply {
        read_reply(
            self.http
                .get(format!("{}/api/level2/views/{view}", self.cfg.api))
                .send()
                .await,
        )
        .await
    }

    /// GETs `path` through the edge. Returns (status, elapsed, cache header, bytes).
    async fn edge_get(&self, path: &str) -> Result<(u16, Duration, Option<String>, Vec<u8>)> {
        let started = Instant::now();
        let r = self
            .http
            .get(format!("{}{path}", self.cfg.edge))
            .send()
            .await?;
        let status = r.status().as_u16();
        let cache = cache_header(r.headers());
        let bytes = r.bytes().await?.to_vec();
        Ok((status, started.elapsed(), cache, bytes))
    }

    async fn event(
        &self,
        station: &str,
        kind: &str,
        reason: Option<&str>,
        seconds: Option<f64>,
        tile_ms: Option<f64>,
    ) {
        let e = Event {
            at: Utc::now(),
            station: station.into(),
            kind: kind.into(),
            reason: reason.map(Into::into),
            seconds,
            tile_ms,
        };
        if let Err(err) = self.sink.event(&e).await {
            eprintln!("level2_shadow_sink_error={err}");
        }
    }

    /// Handles a ready status: on a new revision, fetch its manifest and the newest frame's tile.
    async fn on_ready(&self, site: &Site, status: &Value, seen: &mut Vec<String>) -> Result<()> {
        let url = status["manifest_url"].as_str().context("manifest_url")?;
        let (code, took, _, body) = self.edge_get(url).await?;
        ensure!(code == 200, "manifest HTTP {code}");
        let env: Value = serde_json::from_slice(&body)?;
        let (sweep, end, tile) = newest_frame(&env, &site.id, &self.cfg.palette)?;
        if seen.contains(&sweep) {
            return Ok(());
        }
        let first = seen.is_empty();
        seen.push(sweep.clone());
        if seen.len() > SEEN_MEMORY {
            seen.remove(0);
        }
        if first {
            // Started mid-sweep: this frame was ready before we looked, so its clocks would lie.
            return Ok(());
        }
        let issued: DateTime<Utc> = env["issued_at"].as_str().context("issued_at")?.parse()?;
        let (tile_code, cold, _, png) = self.edge_get(&tile).await?;
        let displayed = Utc::now();
        let ok = tile_code == 200 && png.starts_with(b"\x89PNG");
        let (cached, cache) = if ok {
            let (_, t, c, _) = self.edge_get(&tile).await?;
            (Some(ms(t)), c)
        } else {
            (None, None)
        };
        self.sink
            .seen(&SeenSweep {
                station: site.id.clone(),
                sweep_id: sweep,
                scan_end: end,
                view_id: env["view_id"].as_str().unwrap_or_default().into(),
                revision: env["revision"].as_str().unwrap_or_default().into(),
                manifest_ready_at: issued,
                manifest_ms: ms(took),
                edge_display_at: displayed,
                tile_cold_ms: ms(cold),
                tile_cached_ms: cached,
                tile_status: i32::from(tile_code),
                tile_cache: cache,
            })
            .await
    }

    /// One steady observation of one station. Returns the state label for the uptime ledger.
    async fn observe(
        &self,
        site: &Site,
        seen: &mut Vec<String>,
        last: &mut String,
    ) -> (String, Duration) {
        let mut reply = self.submit(site, self.cfg.zoom).await;
        let deadline = Instant::now() + Duration::from_secs(40);
        loop {
            match &reply {
                Reply::Status(s) if s["state"] == "pending" && Instant::now() < deadline => {
                    tokio::time::sleep(Duration::from_secs(
                        s["poll_after_s"].as_u64().unwrap_or(5).clamp(1, 15),
                    ))
                    .await;
                    let Some(id) = s["view_id"].as_str() else {
                        break;
                    };
                    reply = self.status(id).await;
                }
                _ => break,
            }
        }
        let (state, wait) = match reply {
            Reply::Status(s) => match s["state"].as_str().unwrap_or("unknown") {
                "ready" => {
                    if let Err(e) = self.on_ready(site, &s, seen).await {
                        self.event(&site.id, "error", Some(&format!("{e:#}")), None, None)
                            .await;
                    }
                    ("ready".to_owned(), self.cfg.submit_every())
                }
                "pending" => {
                    self.event(&site.id, "fallback", Some("pending_timeout"), None, None)
                        .await;
                    (
                        "fallback:pending_timeout".to_owned(),
                        self.cfg.submit_every(),
                    )
                }
                other => {
                    let reason = s["reason"].as_str().unwrap_or(other).to_owned();
                    let label = format!("fallback:{reason}");
                    if *last != label {
                        self.event(&site.id, "fallback", Some(&reason), None, None)
                            .await;
                    }
                    (label, self.cfg.submit_every())
                }
            },
            Reply::RateLimited(after) => {
                self.event(&site.id, "rate_limited", None, None, None).await;
                ("error".to_owned(), Duration::from_secs(after.max(2)))
            }
            Reply::Failed(e) => {
                if !last.starts_with("error") {
                    self.event(&site.id, "error", Some(&e), None, None).await;
                }
                ("error".to_owned(), self.cfg.submit_every())
            }
        };
        last.clone_from(&state);
        (state, wait)
    }

    /// A view nobody has asked for: measures cold readiness separately from the steady view.
    async fn cold_probe(&self, site: &Site, n: u32) {
        let zoom = 10 + n % 4;
        let asked = Utc::now();
        let mut reply = self.submit(site, zoom).await;
        if let Reply::Status(s) = &reply
            && s["state"] == "ready"
        {
            self.event(
                &site.id,
                "cold_ready",
                Some("already_ready"),
                Some(0.0),
                None,
            )
            .await;
            return;
        }
        let deadline = Instant::now() + Duration::from_secs(45);
        while let Reply::Status(s) = &reply {
            if s["state"] != "pending" || Instant::now() >= deadline {
                break;
            }
            tokio::time::sleep(Duration::from_secs(
                s["poll_after_s"].as_u64().unwrap_or(5).clamp(1, 15),
            ))
            .await;
            let Some(id) = s["view_id"].as_str() else {
                break;
            };
            reply = self.status(id).await;
        }
        match reply {
            Reply::Status(s) if s["state"] == "ready" => {
                let fetched = async {
                    let url = s["manifest_url"].as_str().context("manifest_url")?;
                    let (code, _, _, body) = self.edge_get(url).await?;
                    ensure!(code == 200, "manifest HTTP {code}");
                    let env: Value = serde_json::from_slice(&body)?;
                    let issued: DateTime<Utc> =
                        env["issued_at"].as_str().context("issued_at")?.parse()?;
                    let (_, _, tile) = newest_frame(&env, &site.id, &self.cfg.palette)?;
                    let (_, took, _, _) = self.edge_get(&tile).await?;
                    Ok::<_, anyhow::Error>(((issued - asked).as_seconds_f64().max(0.0), ms(took)))
                }
                .await;
                match fetched {
                    Ok((ready_s, tile_ms)) => {
                        self.event(&site.id, "cold_ready", None, Some(ready_s), Some(tile_ms))
                            .await
                    }
                    Err(e) => {
                        self.event(&site.id, "error", Some(&format!("cold: {e:#}")), None, None)
                            .await
                    }
                }
            }
            Reply::Status(s) if s["state"] == "pending" => {
                self.event(
                    &site.id,
                    "cold_timeout",
                    Some("pending_timeout"),
                    None,
                    None,
                )
                .await
            }
            Reply::Status(s) => {
                let reason = s["reason"].as_str().unwrap_or("unknown").to_owned();
                self.event(&site.id, "cold_fallback", Some(&reason), None, None)
                    .await
            }
            Reply::RateLimited(_) => {
                self.event(&site.id, "rate_limited", Some("cold"), None, None)
                    .await
            }
            Reply::Failed(e) => {
                self.event(&site.id, "error", Some(&format!("cold: {e}")), None, None)
                    .await
            }
        }
    }

    /// Observes one station forever (or `rounds` times, for tests).
    async fn station_loop(self: Arc<Self>, site: Site, offset: Duration, rounds: Option<usize>) {
        tokio::time::sleep(offset).await;
        let mut seen = vec![];
        let mut last = String::new();
        let mut next_cold = self.cfg.cold_every.map(|c| Instant::now() + c / 2 + offset);
        let mut cold_n = 0;
        let mut round = 0;
        loop {
            let started = Instant::now();
            let (state, wait) = self.observe(&site, &mut seen, &mut last).await;
            if let Some(at) = next_cold
                && Instant::now() >= at
            {
                self.cold_probe(&site, cold_n).await;
                cold_n += 1;
                next_cold = self.cfg.cold_every.map(|c| Instant::now() + c);
            }
            let left = wait.saturating_sub(started.elapsed());
            tokio::time::sleep(left).await;
            let secs = started.elapsed().as_secs_f64();
            if let Err(e) = self
                .sink
                .state(Utc::now().date_naive(), &site.id, &state, secs)
                .await
            {
                eprintln!("level2_shadow_sink_error={e}");
            }
            round += 1;
            if rounds.is_some_and(|r| round >= r) {
                return;
            }
        }
    }

    /// Runs every station loop. `rounds` bounds each loop (tests); production passes `None`.
    pub async fn run(self: Arc<Self>, rounds: Option<usize>) -> Result<()> {
        let sites = loop {
            match self.sites().await {
                Ok(s) => break s,
                Err(e) if rounds.is_some() => return Err(e),
                Err(e) => {
                    eprintln!("level2_shadow_sites_error={e:#}");
                    tokio::time::sleep(Duration::from_secs(30)).await;
                }
            }
        };
        let spread = self.cfg.submit_every() / sites.len().max(1) as u32;
        let mut tasks = vec![];
        for (i, site) in sites.into_iter().enumerate() {
            tasks.push(tokio::spawn(self.clone().station_loop(
                site,
                spread * i as u32,
                rounds,
            )));
        }
        for t in tasks {
            t.await?;
        }
        Ok(())
    }
}

async fn read_reply(sent: reqwest::Result<reqwest::Response>) -> Reply {
    let r = match sent {
        Ok(r) => r,
        Err(e) => return Reply::Failed(format!("request: {e}")),
    };
    let code = r.status().as_u16();
    if code == 429 {
        let after = r
            .headers()
            .get("retry-after")
            .and_then(|v| v.to_str().ok())
            .and_then(|v| v.parse().ok())
            .unwrap_or(2);
        return Reply::RateLimited(after);
    }
    match r.bytes().await {
        Ok(b) if code < 300 => match serde_json::from_slice(&b) {
            Ok(v) => Reply::Status(v),
            Err(_) => Reply::Failed(format!("HTTP {code} not JSON")),
        },
        Ok(_) => Reply::Failed(format!("HTTP {code}")),
        Err(e) => Reply::Failed(format!("body: {e}")),
    }
}

// ---------- storage ----------

pub struct PgSink {
    pool: tokio::sync::Mutex<Pool>,
}

const SEEN_COLUMNS: &str = "station,sweep_id,scan_end,view_id,revision,manifest_ready_at,manifest_ms,edge_display_at,tile_cold_ms,tile_cached_ms,tile_status,tile_cache";

impl PgSink {
    pub async fn connect() -> Result<Self> {
        let url = required("LEVEL2_PG_URL")?;
        let schema = std::env::var("LEVEL2_PG_SCHEMA").unwrap_or_else(|_| "level2_numeric".into());
        let pool = Pool::connect(&url, &schema).await?;
        pool.client
            .batch_execute(&format!("CREATE SCHEMA IF NOT EXISTS {schema};"))
            .await?;
        pool.initialize_clocks().await?;
        initialize(&pool).await?;
        Ok(Self {
            pool: tokio::sync::Mutex::new(pool),
        })
    }
}

async fn initialize(pool: &Pool) -> Result<()> {
    let s = pool.schema();
    pool.client.batch_execute(&format!("
        CREATE TABLE IF NOT EXISTS {s}.shadow_sweeps(station text NOT NULL,sweep_id text NOT NULL,scan_end timestamptz NOT NULL,view_id text NOT NULL,revision text NOT NULL,manifest_ready_at timestamptz NOT NULL,manifest_ms double precision NOT NULL,edge_display_at timestamptz NOT NULL,tile_cold_ms double precision NOT NULL,tile_cached_ms double precision,tile_status integer NOT NULL,tile_cache text,PRIMARY KEY(station,sweep_id));
        CREATE INDEX IF NOT EXISTS shadow_sweeps_end ON {s}.shadow_sweeps(scan_end);
        CREATE TABLE IF NOT EXISTS {s}.shadow_events(at timestamptz NOT NULL,station text NOT NULL,kind text NOT NULL,reason text,seconds double precision,tile_ms double precision);
        CREATE INDEX IF NOT EXISTS shadow_events_at ON {s}.shadow_events(at);
        CREATE TABLE IF NOT EXISTS {s}.shadow_states(day date NOT NULL,station text NOT NULL,state text NOT NULL,seconds double precision NOT NULL,PRIMARY KEY(day,station,state));
        CREATE TABLE IF NOT EXISTS {s}.shadow_daily(day date PRIMARY KEY,doc text NOT NULL);
        CREATE TABLE IF NOT EXISTS {s}.shadow_excluded(day date PRIMARY KEY,reason text NOT NULL);
    ")).await?;
    Ok(())
}

#[async_trait]
impl Sink for PgSink {
    async fn seen(&self, r: &SeenSweep) -> Result<()> {
        let pool = self.pool.lock().await;
        let s = pool.schema();
        pool.client
            .execute(
                &format!("INSERT INTO {s}.shadow_sweeps({SEEN_COLUMNS}) VALUES($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12) ON CONFLICT DO NOTHING"),
                &[&r.station, &r.sweep_id, &r.scan_end, &r.view_id, &r.revision, &r.manifest_ready_at, &r.manifest_ms, &r.edge_display_at, &r.tile_cold_ms, &r.tile_cached_ms, &r.tile_status, &r.tile_cache],
            )
            .await?;
        Ok(())
    }
    async fn event(&self, e: &Event) -> Result<()> {
        let pool = self.pool.lock().await;
        let s = pool.schema();
        pool.client
            .execute(
                &format!("INSERT INTO {s}.shadow_events(at,station,kind,reason,seconds,tile_ms) VALUES($1,$2,$3,$4,$5,$6)"),
                &[&e.at, &e.station, &e.kind, &e.reason, &e.seconds, &e.tile_ms],
            )
            .await?;
        Ok(())
    }
    async fn state(&self, day: NaiveDate, station: &str, state: &str, seconds: f64) -> Result<()> {
        let pool = self.pool.lock().await;
        let s = pool.schema();
        pool.client
            .execute(
                &format!("INSERT INTO {s}.shadow_states(day,station,state,seconds) VALUES($1,$2,$3,$4) ON CONFLICT(day,station,state) DO UPDATE SET seconds={s}.shadow_states.seconds+excluded.seconds"),
                &[&day, &station, &state, &seconds],
            )
            .await?;
        Ok(())
    }
}

/// Everything the report reads, for one window.
#[derive(Debug, Default)]
pub struct Inputs {
    pub clocks: Vec<SweepClock>,
    pub seen: Vec<SeenSweep>,
    pub events: Vec<Event>,
    /// (day, station, state, seconds)
    pub states: Vec<(NaiveDate, String, String, f64)>,
    pub daily: BTreeMap<NaiveDate, Value>,
    pub excluded: BTreeMap<NaiveDate, String>,
}

fn day_start(d: NaiveDate) -> DateTime<Utc> {
    d.and_hms_opt(0, 0, 0).expect("midnight").and_utc()
}

impl PgSink {
    pub async fn load(&self, from: NaiveDate, to: NaiveDate) -> Result<Inputs> {
        let pool = self.pool.lock().await;
        let s = pool.schema();
        let (a, b) = (day_start(from), day_start(to));
        let clocks = pool.clocks(a, b).await?;
        let seen = pool
            .client
            .query(&format!("SELECT {SEEN_COLUMNS} FROM {s}.shadow_sweeps WHERE scan_end>=$1 AND scan_end<$2"), &[&a, &b])
            .await?
            .iter()
            .map(|r| SeenSweep {
                station: r.get(0),
                sweep_id: r.get(1),
                scan_end: r.get(2),
                view_id: r.get(3),
                revision: r.get(4),
                manifest_ready_at: r.get(5),
                manifest_ms: r.get(6),
                edge_display_at: r.get(7),
                tile_cold_ms: r.get(8),
                tile_cached_ms: r.get(9),
                tile_status: r.get(10),
                tile_cache: r.get(11),
            })
            .collect();
        let events = pool
            .client
            .query(&format!("SELECT at,station,kind,reason,seconds,tile_ms FROM {s}.shadow_events WHERE at>=$1 AND at<$2"), &[&a, &b])
            .await?
            .iter()
            .map(|r| Event {
                at: r.get(0),
                station: r.get(1),
                kind: r.get(2),
                reason: r.get(3),
                seconds: r.get(4),
                tile_ms: r.get(5),
            })
            .collect();
        let states = pool
            .client
            .query(&format!("SELECT day,station,state,seconds FROM {s}.shadow_states WHERE day>=$1 AND day<$2"), &[&from, &to])
            .await?
            .iter()
            .map(|r| (r.get(0), r.get(1), r.get(2), r.get(3)))
            .collect();
        let daily = pool
            .client
            .query(
                &format!("SELECT day,doc FROM {s}.shadow_daily WHERE day>=$1 AND day<$2"),
                &[&from, &to],
            )
            .await?
            .iter()
            .map(|r| Ok((r.get(0), serde_json::from_str(&r.get::<_, String>(1))?)))
            .collect::<Result<_>>()?;
        let excluded = pool
            .client
            .query(
                &format!("SELECT day,reason FROM {s}.shadow_excluded WHERE day>=$1 AND day<$2"),
                &[&from, &to],
            )
            .await?
            .iter()
            .map(|r| (r.get(0), r.get(1)))
            .collect();
        Ok(Inputs {
            clocks,
            seen,
            events,
            states,
            daily,
            excluded,
        })
    }

    async fn save_daily(&self, day: NaiveDate, doc: &Value) -> Result<()> {
        let pool = self.pool.lock().await;
        let s = pool.schema();
        pool.client
            .execute(&format!("INSERT INTO {s}.shadow_daily(day,doc) VALUES($1,$2) ON CONFLICT(day) DO UPDATE SET doc=excluded.doc"), &[&day, &doc.to_string()])
            .await?;
        Ok(())
    }

    async fn exclude(&self, day: NaiveDate, reason: &str) -> Result<()> {
        let pool = self.pool.lock().await;
        let s = pool.schema();
        pool.client
            .execute(&format!("INSERT INTO {s}.shadow_excluded(day,reason) VALUES($1,$2) ON CONFLICT(day) DO UPDATE SET reason=excluded.reason"), &[&day, &reason])
            .await?;
        Ok(())
    }

    async fn object_bytes(&self) -> Result<i64> {
        let pool = self.pool.lock().await;
        let s = pool.schema();
        let has = pool
            .client
            .query_one(
                "SELECT to_regclass($1) IS NOT NULL",
                &[&format!("{s}.objects")],
            )
            .await?
            .get::<_, bool>(0);
        if !has {
            return Ok(0);
        }
        Ok(pool
            .client
            .query_one(
                &format!("SELECT COALESCE(sum(bytes),0)::bigint FROM {s}.objects"),
                &[],
            )
            .await?
            .get(0))
    }
}

// ---------- report ----------

/// Nearest-rank percentile of `v` (sorted in place). `None` when empty.
pub fn percentile(v: &mut [f64], p: f64) -> Option<f64> {
    if v.is_empty() {
        return None;
    }
    v.sort_by(f64::total_cmp);
    let rank = ((p / 100.0) * v.len() as f64).ceil().max(1.0) as usize;
    Some(v[rank.min(v.len()) - 1])
}

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct Stat {
    pub n: usize,
    pub p50: Option<f64>,
    pub p95: Option<f64>,
    pub p99: Option<f64>,
    pub max: Option<f64>,
}

pub fn stat(mut v: Vec<f64>) -> Stat {
    v.retain(|x| x.is_finite());
    Stat {
        n: v.len(),
        p50: percentile(&mut v, 50.0),
        p95: percentile(&mut v, 95.0),
        p99: percentile(&mut v, 99.0),
        max: v.last().copied(),
    }
}

fn secs(a: DateTime<Utc>, b: DateTime<Utc>) -> f64 {
    (b - a).as_seconds_f64()
}

/// Days in `[from, to)` that count: recorder uptime for every station and not excluded.
pub fn valid_days(
    i: &Inputs,
    stations: &[String],
    from: NaiveDate,
    to: NaiveDate,
) -> (Vec<NaiveDate>, Vec<Value>) {
    let mut valid = vec![];
    let mut detail = vec![];
    let mut d = from;
    while d < to {
        let mut uptime = BTreeMap::new();
        for st in stations {
            let s: f64 = i
                .states
                .iter()
                .filter(|(day, station, _, _)| *day == d && station == st)
                .map(|r| r.3)
                .sum();
            uptime.insert(st.clone(), (s / DAY_S).min(1.0));
        }
        let excluded = i.excluded.get(&d).cloned();
        let ok = excluded.is_none()
            && !stations.is_empty()
            && uptime.values().all(|u| *u >= VALID_UPTIME);
        if ok {
            valid.push(d);
        }
        detail.push(json!({"day": d, "valid": ok, "excluded": excluded, "uptime": uptime}));
        d = d.succ_opt().expect("date");
    }
    (valid, detail)
}

/// Feed gaps: consecutive indexed lowest sweeps of one station further apart than `gap_s`.
pub fn gaps(clocks: &[SweepClock], gap_s: i64) -> Vec<Value> {
    let mut by: BTreeMap<&str, Vec<DateTime<Utc>>> = BTreeMap::new();
    for c in clocks {
        by.entry(&c.station).or_default().push(c.scan_end);
    }
    let mut out = vec![];
    for (station, mut ends) in by {
        ends.sort();
        for w in ends.windows(2) {
            let s = (w[1] - w[0]).num_seconds();
            if s > gap_s {
                out.push(json!({"station": station, "after": w[0], "seconds": s}));
            }
        }
    }
    out
}

/// The 30-day report: every #293 percentile, cold and cached split, per-day validity.
pub fn build_report(
    i: &Inputs,
    stations: &[String],
    from: NaiveDate,
    to: NaiveDate,
    gap_s: i64,
) -> Value {
    let (valid, days) = valid_days(i, stations, from, to);
    let ok = |t: DateTime<Utc>| valid.contains(&t.date_naive());
    let clocks: Vec<&SweepClock> = i.clocks.iter().filter(|c| ok(c.scan_end)).collect();
    let seen: Vec<&SeenSweep> = i.seen.iter().filter(|s| ok(s.scan_end)).collect();
    let by_id: BTreeMap<(&str, &str), &SweepClock> = clocks
        .iter()
        .map(|c| ((c.station.as_str(), c.sweep_id.as_str()), *c))
        .collect();
    let mut source_lag = vec![];
    let mut processing = vec![];
    let mut availability = vec![];
    let mut manifest_wait = vec![];
    let mut edge_wait = vec![];
    let mut end_to_end = vec![];
    for c in &clocks {
        if let Some(a) = c.source_arrival_at {
            source_lag.push(secs(c.scan_end, a));
            processing.push(secs(a, c.first_available_at));
        }
        availability.push(secs(c.scan_end, c.first_available_at));
    }
    for s in &seen {
        end_to_end.push(secs(s.scan_end, s.edge_display_at));
        edge_wait.push(secs(s.manifest_ready_at, s.edge_display_at));
        if let Some(c) = by_id.get(&(s.station.as_str(), s.sweep_id.as_str())) {
            manifest_wait.push(secs(c.first_available_at, s.manifest_ready_at));
        }
    }
    let displayed: HashSet<(&str, &str)> = seen
        .iter()
        .map(|s| (s.station.as_str(), s.sweep_id.as_str()))
        .collect();
    let undisplayed = clocks
        .iter()
        .filter(|c| !displayed.contains(&(c.station.as_str(), c.sweep_id.as_str())))
        .count();
    let events: Vec<&Event> = i.events.iter().filter(|e| ok(e.at)).collect();
    let cold_ready: Vec<f64> = events
        .iter()
        .filter(|e| e.kind == "cold_ready" && e.reason.is_none())
        .filter_map(|e| e.seconds)
        .collect();
    let cold_tile: Vec<f64> = events
        .iter()
        .filter(|e| e.kind == "cold_ready")
        .filter_map(|e| e.tile_ms)
        .collect();
    let mut counts: BTreeMap<String, usize> = BTreeMap::new();
    for e in &events {
        *counts
            .entry(match &e.reason {
                Some(r) if e.kind != "error" => format!("{}:{r}", e.kind),
                _ => e.kind.clone(),
            })
            .or_default() += 1;
    }
    let mut state_s: BTreeMap<String, f64> = BTreeMap::new();
    for (d, _, state, s) in &i.states {
        if valid.contains(d) {
            *state_s.entry(state.clone()).or_default() += s;
        }
    }
    let mut vcps: BTreeMap<String, usize> = BTreeMap::new();
    for c in &clocks {
        *vcps
            .entry(c.vcp.map_or("unknown".into(), |v| v.to_string()))
            .or_default() += 1;
    }
    let qc = stat(clocks.iter().map(|c| c.qc_ms).collect());
    let manifest = stat(seen.iter().map(|s| s.manifest_ms).collect());
    let cached = stat(seen.iter().filter_map(|s| s.tile_cached_ms).collect());
    let gate = |s: &Stat, limit: f64, strict: bool| match s.p95 {
        None => "no_data",
        Some(v) if (strict && v < limit) || (!strict && v <= limit) => "pass",
        Some(_) => "fail",
    };
    let feed_gaps: Vec<Value> = gaps(
        &i.clocks
            .iter()
            .filter(|c| ok(c.scan_end))
            .cloned()
            .collect::<Vec<_>>(),
        gap_s,
    );
    let mut peak_rss: Option<f64> = None;
    let mut cpu = 0.0;
    let mut object_peak: Option<f64> = None;
    for (d, doc) in &i.daily {
        if !valid.contains(d) {
            continue;
        }
        if let Some(v) = doc["prometheus"]["peak_rss_bytes_max"].as_f64() {
            peak_rss = Some(peak_rss.map_or(v, |p: f64| p.max(v)));
        }
        cpu += doc["prometheus"]["cpu_seconds_total"]
            .as_f64()
            .unwrap_or(0.0);
        if let Some(v) = doc["object_bytes"].as_f64() {
            object_peak = Some(object_peak.map_or(v, |p: f64| p.max(v)));
        }
    }
    json!({
        "window": {"from": from, "to_exclusive": to},
        "stations": stations,
        "valid_days": valid.len(),
        "days": days,
        "gates": {
            "qc_ms_per_lowest_sweep": {"stat": qc, "p95_limit": 25.0, "result": gate(&qc, 25.0, false)},
            "manifest_ms_edge": {"stat": manifest, "p95_limit": 500.0, "result": gate(&manifest, 500.0, true)},
            "tile_cached_ms_edge": {"stat": cached, "p95_limit": 800.0, "result": gate(&cached, 800.0, true)},
        },
        "clocks_s": {
            "source_lag": stat(source_lag),
            "processing": stat(processing),
            "first_availability": stat(availability),
            "manifest_readiness": stat(manifest_wait),
            "edge_display": stat(edge_wait),
            "scan_end_to_edge_display": stat(end_to_end),
        },
        "cold": {
            "view_ready_s": stat(cold_ready),
            "tile_ms": stat(cold_tile),
            "first_tile_ms_steady": stat(seen.iter().map(|s| s.tile_cold_ms).collect()),
        },
        "gaps": {"feed_gaps": feed_gaps.len(), "feed_gap_list": feed_gaps, "undisplayed_sweeps": undisplayed, "missing_sectors": clocks.iter().map(|c| c.missing_sectors as i64).sum::<i64>()},
        "events": counts,
        "state_seconds": state_s,
        "vcp": vcps,
        "sweeps": {"indexed": clocks.len(), "displayed": seen.len()},
        "cost": {"peak_rss_bytes_max": peak_rss, "cpu_seconds_total": cpu, "object_bytes_peak": object_peak},
    })
}

fn fmt_opt(v: Option<f64>) -> String {
    v.map_or("-".into(), |v| format!("{v:.1}"))
}

/// Plain-text rendering of [`build_report`].
pub fn render_report(r: &Value) -> String {
    let mut out = String::new();
    let line = |out: &mut String, name: &str, s: &Value| {
        let g = |k: &str| fmt_opt(s[k].as_f64());
        out.push_str(&format!(
            "  {name:<28} n={:<6} p50={:<8} p95={:<8} p99={:<8} max={}\n",
            s["n"],
            g("p50"),
            g("p95"),
            g("p99"),
            g("max")
        ));
    };
    out.push_str(&format!(
        "Level II shadow report {}..{} (exclusive), stations {}\nvalid days: {}\n\nGates (p95)\n",
        r["window"]["from"].as_str().unwrap_or(""),
        r["window"]["to_exclusive"].as_str().unwrap_or(""),
        r["stations"],
        r["valid_days"],
    ));
    for (k, g) in r["gates"].as_object().into_iter().flatten() {
        line(&mut out, k, &g["stat"]);
        out.push_str(&format!(
            "  {:<28} limit {} -> {}\n",
            "",
            g["p95_limit"],
            g["result"].as_str().unwrap_or("")
        ));
    }
    out.push_str("\nClocks (seconds, valid days only)\n");
    for (k, s) in r["clocks_s"].as_object().into_iter().flatten() {
        line(&mut out, k, s);
    }
    out.push_str("\nCold views (not cached performance)\n");
    for (k, s) in r["cold"].as_object().into_iter().flatten() {
        line(&mut out, k, s);
    }
    out.push_str(&format!(
        "\nGaps: feed {} | undisplayed sweeps {} | missing sectors {}\nSweeps: {}\nVCP: {}\nEvents: {}\nState seconds: {}\nCost: {}\n",
        r["gaps"]["feed_gaps"], r["gaps"]["undisplayed_sweeps"], r["gaps"]["missing_sectors"], r["sweeps"], r["vcp"], r["events"], r["state_seconds"], r["cost"],
    ));
    out
}

// ---------- daily summary and Prometheus capture ----------

async fn prom_query(
    http: &reqwest::Client,
    base: &str,
    q: &str,
    at: DateTime<Utc>,
) -> Result<Vec<(String, f64)>> {
    let body = http
        .get(format!("{}/api/v1/query", base.trim_end_matches('/')))
        .query(&[("query", q), ("time", &at.timestamp().to_string())])
        .send()
        .await?
        .error_for_status()?
        .bytes()
        .await?;
    let v: Value = serde_json::from_slice(&body)?;
    ensure!(v["status"] == "success", "prometheus query failed");
    Ok(v["data"]["result"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|r| {
            let pod = r["metric"]["pod"].as_str().unwrap_or("all").to_owned();
            let val = r["value"][1].as_str()?.parse().ok()?;
            Some((pod, val))
        })
        .collect())
}

/// Peak RSS and CPU seconds per Level II pod over one UTC day, read before Prometheus drops them.
pub async fn capture_prometheus(cfg: &Config, day: NaiveDate) -> Value {
    let Some(base) = &cfg.prom else {
        return json!({"status": "not_configured"});
    };
    let http = reqwest::Client::builder()
        .timeout(Duration::from_secs(30))
        .build();
    let Ok(http) = http else {
        return json!({"status": "error"});
    };
    let at = day_start(day.succ_opt().unwrap_or(day));
    let sel = format!(
        "namespace=\"{}\",container!=\"\",container!=\"POD\",pod=~\"{}\"",
        cfg.prom_namespace, cfg.prom_pods
    );
    let rss = prom_query(
        &http,
        base,
        &format!("max by (pod) (max_over_time(container_memory_rss{{{sel}}}[1d]))"),
        at,
    )
    .await;
    let cpu = prom_query(
        &http,
        base,
        &format!("sum by (pod) (increase(container_cpu_usage_seconds_total{{{sel}}}[1d]))"),
        at,
    )
    .await;
    let obj = prom_query(
        &http,
        base,
        "max(max_over_time(radar_level2_object_bytes[1d]))",
        at,
    )
    .await;
    match (rss, cpu) {
        (Ok(rss), Ok(cpu)) => json!({
            "status": "ok",
            "peak_rss_bytes": rss.iter().cloned().collect::<BTreeMap<_, _>>(),
            "peak_rss_bytes_max": rss.iter().map(|r| r.1).fold(None, |m: Option<f64>, v| Some(m.map_or(v, |m| m.max(v)))),
            "cpu_seconds": cpu.iter().cloned().collect::<BTreeMap<_, _>>(),
            "cpu_seconds_total": cpu.iter().map(|r| r.1).sum::<f64>(),
            "object_bytes_metric_max": obj.ok().and_then(|o| o.first().map(|r| r.1)),
        }),
        (r, c) => json!({"status": "error", "error": format!("{:?} {:?}", r.err(), c.err())}),
    }
}

fn summarize(i: &Inputs, stations: &[String], day: NaiveDate, gap_s: i64) -> Value {
    let next = day.succ_opt().expect("date");
    let mut r = build_report(i, stations, day, next, gap_s);
    // A summary describes the day even when it is not valid; keep the raw counts too.
    r["raw"] = json!({
        "indexed": i.clocks.len(),
        "displayed": i.seen.len(),
        "events": i.events.len(),
        "artifact_bytes": i.clocks.iter().map(|c| c.artifact_bytes).sum::<i64>(),
        "source_bytes": i.clocks.iter().map(|c| c.source_bytes).sum::<i64>(),
    });
    r
}

async fn summary(cfg: &Config, sink: &PgSink, day: NaiveDate) -> Result<Value> {
    let next = day.succ_opt().context("date")?;
    let inputs = sink.load(day, next).await?;
    let mut doc = summarize(&inputs, &cfg.stations, day, cfg.gap_s);
    doc["prometheus"] = capture_prometheus(cfg, day).await;
    doc["object_bytes"] = json!(sink.object_bytes().await?);
    doc["edge"] = json!(cfg.edge);
    sink.save_daily(day, &doc).await?;
    Ok(doc)
}

fn yesterday() -> NaiveDate {
    (Utc::now() - TimeDelta::days(1)).date_naive()
}

/// Entry point for `radar level2-shadow`.
pub async fn run(args: ShadowArgs) -> Result<()> {
    match args.cmd {
        ShadowCmd::Record => {
            if !radar_core::env::bool_var(ENABLED_ENV, false)? {
                bail!("{ENABLED_ENV} is not true; the shadow recorder stays off");
            }
            let cfg = Config::from_env()?;
            let sink = Arc::new(PgSink::connect().await?);
            let addr =
                std::env::var("LEVEL2_METRICS_BIND").unwrap_or_else(|_| "0.0.0.0:9466".into());
            tokio::spawn(crate::metrics::serve(addr.parse()?));
            eprintln!(
                "level2_shadow_start stations={:?} api={} edge={}",
                cfg.stations, cfg.api, cfg.edge
            );
            let daily_cfg = cfg.clone();
            let daily_sink = sink.clone();
            tokio::spawn(async move {
                let mut done: Option<NaiveDate> = None;
                loop {
                    let now = Utc::now();
                    let day = yesterday();
                    if done != Some(day)
                        && now - day_start(now.date_naive()) >= TimeDelta::minutes(10)
                    {
                        match summary(&daily_cfg, &daily_sink, day).await {
                            Ok(_) => {
                                eprintln!("level2_shadow_daily={day}");
                                done = Some(day);
                            }
                            Err(e) => eprintln!("level2_shadow_daily_error={e:#}"),
                        }
                    }
                    tokio::time::sleep(Duration::from_secs(300)).await;
                }
            });
            let recorder = Arc::new(Recorder::new(cfg, sink)?);
            recorder.run(None).await
        }
        ShadowCmd::Summary { day } => {
            let cfg = report_config()?;
            let sink = PgSink::connect().await?;
            let doc = summary(&cfg, &sink, day.unwrap_or_else(yesterday)).await?;
            println!("{}", serde_json::to_string_pretty(&doc)?);
            Ok(())
        }
        ShadowCmd::Report { days, until, json } => {
            ensure!((1..=400).contains(&days), "--days must be 1..=400");
            let cfg = report_config()?;
            let sink = PgSink::connect().await?;
            let to = until.unwrap_or_else(yesterday).succ_opt().context("date")?;
            let from = to - TimeDelta::days(i64::from(days));
            let inputs = sink.load(from, to).await?;
            let r = build_report(&inputs, &cfg.stations, from, to, cfg.gap_s);
            if json {
                println!("{}", serde_json::to_string_pretty(&r)?);
            } else {
                print!("{}", render_report(&r));
            }
            Ok(())
        }
        ShadowCmd::Exclude { day, reason } => {
            ensure!(!reason.trim().is_empty(), "--reason required");
            PgSink::connect().await?.exclude(day, reason.trim()).await?;
            println!("excluded {day}: {reason}");
            Ok(())
        }
    }
}

/// Summary and report need the pilot list but not the API URL.
fn report_config() -> Result<Config> {
    Ok(Config {
        stations: stations_from(&required("LEVEL2_ENABLED_STATIONS")?)?,
        api: std::env::var("LEVEL2_SHADOW_API_URL").unwrap_or_default(),
        edge: std::env::var("LEVEL2_SHADOW_EDGE_URL")
            .or_else(|_| std::env::var("LEVEL2_SHADOW_API_URL"))
            .unwrap_or_default(),
        zoom: 9,
        palette: "classic".into(),
        cold_every: None,
        gap_s: radar_core::env::int_var("LEVEL2_SHADOW_GAP_S", GAP_S_DEFAULT, 60, 86_400)?,
        prom: std::env::var("LEVEL2_SHADOW_PROM_URL")
            .ok()
            .filter(|s| !s.is_empty()),
        prom_namespace: std::env::var("LEVEL2_SHADOW_PROM_NAMESPACE")
            .unwrap_or_else(|_| "radar-ng".into()),
        prom_pods: std::env::var("LEVEL2_SHADOW_PROM_PODS")
            .unwrap_or_else(|_| "radar-ng-worker-level2.*|level2-.*".into()),
    })
}

#[cfg(test)]
#[path = "shadow_tests.rs"]
mod tests;
