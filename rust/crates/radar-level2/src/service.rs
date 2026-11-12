//! The read side of the Level II viewport API: sites, view submit/status, manifests and tiles.
//! Handlers in `radar-api` only translate these results to HTTP. Nothing here decodes or renders.

use crate::{
    catalogue::Catalogue,
    health::State,
    publish::{Clock, TILE_CAP},
    tile::PALETTES,
    viewport::*,
};
use anyhow::{Context, Result, ensure};
use chrono::{DateTime, TimeDelta, Utc};

use futures_util::StreamExt;
use object_store::{ObjectStore, path::Path};
use serde_json::{Value, json};
use std::sync::{Arc, Mutex};

const SITES_TTL_S: i64 = 15;
const VERIFY_EVERY_S: i64 = 15;

pub struct Level2Service {
    pub store: Arc<dyn ViewStore>,
    pub index: Arc<dyn SweepIndex>,
    pub objects: Arc<dyn ObjectStore>,
    pub catalogue: Arc<Catalogue>,
    pub enabled: Vec<String>,
    pub namespace: String,
    pub clock: Clock,
    sites: Mutex<Option<(DateTime<Utc>, Arc<Body>)>>,
}

/// An encoded JSON body and its strong ETag.
#[derive(Debug)]
pub struct Body {
    pub bytes: Vec<u8>,
    pub etag: String,
}

impl Body {
    pub fn json(value: &Value) -> Self {
        let bytes = serde_json::to_vec(value).unwrap_or_default();
        let etag = format!("\"{}\"", &crate::checksum(&bytes)[..24]);
        Self { bytes, etag }
    }
}

pub enum Submit {
    Invalid(&'static str),
    Reply { status: u16, body: Body },
}

pub enum TileReply {
    Ok { bytes: Vec<u8>, sha256: String },
    NotFound,
    Gone,
    Unavailable(&'static str),
}

pub enum ManifestReply {
    Ok(Box<Revision>),
    NotFound,
    Gone,
    Unavailable,
}

fn hex_len(s: &str, n: usize) -> bool {
    s.len() == n
        && s.bytes()
            .all(|b| b.is_ascii_hexdigit() && !b.is_ascii_uppercase())
}

pub fn valid_view_id(s: &str) -> bool {
    hex_len(s, 32)
}

pub fn valid_revision(s: &str) -> bool {
    hex_len(s, 16)
}

fn fallback(reason: &str, station: Option<&str>) -> Body {
    Body::json(&json!({
        "schema_version": SCHEMA_VERSION,
        "view_id": null,
        "state": "fallback",
        "reason": reason,
        "station_id": station,
        "poll_after_s": POLL_READY_S,
    }))
}

impl Level2Service {
    pub fn new(
        store: Arc<dyn ViewStore>,
        index: Arc<dyn SweepIndex>,
        objects: Arc<dyn ObjectStore>,
        catalogue: Arc<Catalogue>,
        enabled: Vec<String>,
        namespace: String,
        clock: Clock,
    ) -> Result<Self> {
        ensure!(
            enabled
                .iter()
                .all(|id| crate::catalogue::FEED_VERIFIED.contains(&id.as_str())
                    && catalogue.sites.contains_key(id)),
            "unverified Level II station in the enabled list"
        );
        ensure!(
            namespace.starts_with("level2/"),
            "invalid Level II namespace"
        );
        Ok(Self {
            store,
            index,
            objects,
            catalogue,
            enabled,
            namespace,
            clock,
            sites: Mutex::new(None),
        })
    }

    pub fn now(&self) -> DateTime<Utc> {
        (self.clock)()
    }

    /// The catalogue, enabled regions and per-station health clocks. One snapshot per 15 s.
    pub async fn sites(&self) -> Result<Arc<Body>> {
        let now = self.now();
        if let Some((at, body)) = self.sites.lock().expect("sites lock").as_ref()
            && now - *at < TimeDelta::seconds(SITES_TTL_S)
            && now >= *at
        {
            return Ok(body.clone());
        }
        let mut sites = Vec::new();
        for (id, site) in &self.catalogue.sites {
            let enabled = self.enabled.contains(id);
            let health = if enabled {
                let h = self.index.health(id).await?;
                let r = h.report(now);
                json!({
                    "state": r.state,
                    "eligible": r.eligible,
                    "feed_late": r.feed_late,
                    "source_unreachable": r.source_unreachable,
                    "processing_failure": r.processing_failure,
                    "low_sweep_end": h.low_sweep_end,
                    "newest_chunk_modified": h.newest_chunk_modified,
                })
            } else {
                Value::Null
            };
            sites.push(json!({"id": id, "name": site.name, "lat": site.lat, "lon": site.lon, "enabled": enabled, "health": health}));
        }
        let body = Arc::new(Body::json(&json!({
            "schema_version": SCHEMA_VERSION,
            "generated_at": now,
            "enabled_regions": self.enabled,
            "max_ground_range_m": MAX_GROUND_RANGE_M,
            "max_age_s": MAX_AGE_S,
            "catalogue": {
                "source": self.catalogue.source,
                "fetched_at": self.catalogue.fetched_at,
                "sha256": self.catalogue.sha256,
                "count": self.catalogue.sites.len(),
            },
            "sites": sites,
        })));
        *self.sites.lock().expect("sites lock") = Some((now, body.clone()));
        Ok(body)
    }

    /// Tile demand: pick the nearest usable station, then return pending, ready or a fallback reason.
    pub async fn submit(&self, raw: RawView) -> Result<Submit> {
        let now = self.now();
        let v = match validate(raw) {
            Ok(v) => v,
            Err(reason) => return Ok(Submit::Invalid(reason)),
        };
        let candidates = self
            .catalogue
            .candidates(v.centre[0], v.centre[1], &self.enabled)?;
        if candidates.is_empty() {
            return Ok(reply(200, fallback("out_of_range", None)));
        }
        let mut reason = "late";
        let mut chosen = None;
        for site in candidates {
            let health = self.index.health(&site.id).await?;
            let r = health.report(now);
            match r.state {
                State::Healthy | State::Unknown | State::Warming => {
                    if r.source_unreachable && !r.eligible {
                        reason = "source_unreachable";
                        continue;
                    }
                    chosen = Some(site.id.clone());
                    break;
                }
                State::FutureClock => reason = "future_clock",
                State::Late => {
                    reason = if r.source_unreachable {
                        "source_unreachable"
                    } else {
                        "late"
                    }
                }
            }
        }
        let Some(station) = chosen else {
            return Ok(reply(200, fallback(reason, None)));
        };
        if !self.store.touch_station(&station, now).await? {
            return Ok(reply(200, fallback("saturated", Some(&station))));
        }
        let scope = ViewScope {
            station: station.clone(),
            zoom: v.zoom,
            tiles: v.tiles,
            palette: v.palette,
        };
        let record = match self.store.submit(&scope, v.centre, now).await? {
            Submitted::Saturated => return Ok(reply(200, fallback("saturated", Some(&station)))),
            Submitted::Record(r) => *r,
        };
        let record = self.keep_fresh(record, now).await?;
        let status = if matches!(record.status(now), Status::Pending { .. }) {
            202
        } else {
            200
        };
        Ok(reply(status, Body::json(&record.status_json(now))))
    }

    /// Renews a still-valid revision only while every referenced object exists, and starts a
    /// rebuild when a newer sweep is indexed.
    async fn keep_fresh(&self, record: ViewRecord, now: DateTime<Utc>) -> Result<ViewRecord> {
        let Some(ready) = record.ready_valid(now).cloned() else {
            return Ok(record);
        };
        let latest = self.index.recent(&record.scope.station, 1).await?;
        if latest
            .first()
            .is_some_and(|s| s.scan_end > ready.newest_scan_end)
            && record.build.is_none()
        {
            self.store.start_refresh(&record.id, now).await?;
        }
        if now - ready.verified_at >= TimeDelta::seconds(VERIFY_EVERY_S) {
            let revision = self.store.revision(&record.id, &ready.revision).await?;
            let intact = match &revision {
                Some(rev) => self.all_exist(&rev.tile_keys).await?,
                None => false,
            };
            let expires = (now + TimeDelta::seconds(VIEW_LEASE_S))
                .min(ready.newest_scan_end + TimeDelta::seconds(MAX_AGE_S));
            if !intact
                || expires <= now
                || !self
                    .store
                    .renew(&record.id, &ready.revision, expires, now)
                    .await?
            {
                self.store.revoke(&record.id, "tile_missing", now).await?;
            }
        }
        Ok(self.store.get(&record.id).await?.unwrap_or(record))
    }

    async fn all_exist(&self, keys: &[String]) -> Result<bool> {
        let store = self.objects.as_ref();
        for chunk in keys.chunks(16) {
            let found =
                futures_util::future::try_join_all(chunk.iter().map(|k| exists(store, k))).await?;
            if found.contains(&false) {
                return Ok(false);
            }
        }
        Ok(true)
    }

    pub async fn status(&self, id: &str) -> Result<Option<Body>> {
        if !valid_view_id(id) {
            return Ok(None);
        }
        let now = self.now();
        Ok(self
            .store
            .get(id)
            .await?
            .map(|r| Body::json(&r.status_json(now))))
    }

    pub async fn manifest(&self, id: &str, revision: &str) -> ManifestReply {
        if !valid_view_id(id) || !valid_revision(revision) {
            return ManifestReply::NotFound;
        }
        match self.store.revision(id, revision).await {
            Err(_) => ManifestReply::Unavailable,
            Ok(None) => ManifestReply::NotFound,
            Ok(Some(rev)) if self.now() >= rev.expires_at => ManifestReply::Gone,
            Ok(Some(rev)) => ManifestReply::Ok(Box::new(rev)),
        }
    }

    #[allow(clippy::too_many_arguments)] // mirrors the URL path segments
    pub fn tile_key(
        &self,
        station: &str,
        sweep: &str,
        version: &str,
        palette: &str,
        z: u32,
        x: u32,
        y: u32,
    ) -> Option<String> {
        let ok = station.len() == 4
            && station
                .bytes()
                .all(|b| b.is_ascii_uppercase() || b.is_ascii_digit())
            && sweep.len() == 64
            && sweep
                .bytes()
                .all(|b| b.is_ascii_hexdigit() && !b.is_ascii_uppercase())
            && !version.is_empty()
            && version.len() <= 64
            && version
                .bytes()
                .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
            && PALETTES.contains(&palette)
            && (8..=13).contains(&z)
            && x < 1 << z
            && y < 1 << z;
        ok.then(|| {
            format!(
                "{}/tiles/{station}/{sweep}/{version}/{palette}/{z}/{x}/{y}.png",
                self.namespace
            )
        })
    }

    /// An immutable tile. Missing is 404/410, a transient failure is 503, never a transparent 200.
    #[allow(clippy::too_many_arguments)]
    pub async fn tile(
        &self,
        station: &str,
        sweep: &str,
        version: &str,
        palette: &str,
        z: u32,
        x: u32,
        y: u32,
    ) -> TileReply {
        let Some(key) = self.tile_key(station, sweep, version, palette, z, x, y) else {
            return TileReply::NotFound;
        };
        let pin = match self.store.tile_pin(&key).await {
            Ok(p) => p,
            Err(_) => return TileReply::Unavailable("database_unavailable"),
        };
        match pin {
            None => return TileReply::NotFound,
            Some(until) if self.now() > until => return TileReply::Gone,
            Some(_) => {}
        }
        let meta = match self.store.tile_meta(&key).await {
            Ok(Some(m)) => m,
            Ok(None) => return TileReply::NotFound,
            Err(_) => return TileReply::Unavailable("database_unavailable"),
        };
        match fetch(self.objects.as_ref(), &key, TILE_CAP).await {
            Ok(Some(bytes)) => {
                let sha = crate::checksum(&bytes);
                if sha != meta.sha256 || bytes.len() as i64 != meta.bytes {
                    return TileReply::Unavailable("tile_corrupt");
                }
                TileReply::Ok { bytes, sha256: sha }
            }
            Ok(None) => TileReply::NotFound,
            Err(_) => TileReply::Unavailable("store_unavailable"),
        }
    }
}

async fn exists(store: &dyn ObjectStore, key: &str) -> Result<bool> {
    match store.head(&Path::from(key)).await {
        Ok(_) => Ok(true),
        Err(object_store::Error::NotFound { .. }) => Ok(false),
        Err(e) => Err(e.into()),
    }
}

fn reply(status: u16, body: Body) -> Submit {
    Submit::Reply { status, body }
}

/// Reads one object up to `cap` bytes. `None` when it does not exist.
pub async fn fetch(store: &dyn ObjectStore, key: &str, cap: usize) -> Result<Option<Vec<u8>>> {
    let got = match store.get(&Path::from(key)).await {
        Ok(g) => g,
        Err(object_store::Error::NotFound { .. }) => return Ok(None),
        Err(e) => return Err(e.into()),
    };
    ensure!(got.meta.size <= cap as u64, "object over size cap");
    let mut stream = got.into_stream();
    let mut bytes = Vec::new();
    while let Some(chunk) = stream.next().await {
        let chunk = chunk.context("object read")?;
        ensure!(bytes.len() + chunk.len() <= cap, "object over size cap");
        bytes.extend_from_slice(&chunk);
    }
    Ok(Some(bytes))
}
