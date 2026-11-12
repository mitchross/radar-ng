//! Ready-viewport contract (slice C): request validation, view records and the store traits.
//! Nothing here decodes or renders; the API only reads what the publisher committed.

use crate::{health::Health, polar::ground_to_slant, tile::PALETTES};
use anyhow::Result;
use async_trait::async_trait;
use chrono::{DateTime, TimeDelta, Utc};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::collections::BTreeSet;

pub const SCHEMA_VERSION: u64 = 1;
pub const MAX_TILES: usize = 64;
pub const MAX_FRAMES: usize = 6;
pub const MAX_SPAN_TILES: u32 = 8;
pub const PENDING_DEADLINE_S: i64 = 30;
pub const VIEW_LEASE_S: i64 = 60;
pub const READ_GRACE_S: i64 = 120;
pub const MAX_AGE_S: i64 = 600;
pub const FALLBACK_RETRY_S: i64 = 15;
pub const MAX_PENDING_VIEWS: usize = 64;
pub const MAX_GROUND_RANGE_M: f64 = 150_000.0;
pub const POLL_PENDING_S: i64 = 5;
pub const POLL_READY_S: i64 = 15;
pub const TILE_TEMPLATE: &str = "/level2/tiles/{station}/{path}/{palette}/{z}/{x}/{y}.png";
const EARTH_RADIUS_M: f64 = 6_371_000.0;

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Centre {
    pub lat: f64,
    pub lon: f64,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RawView {
    pub centre: Centre,
    pub zoom: u32,
    pub tiles: Vec<[u32; 2]>,
    pub palette: String,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Validated {
    pub centre: [f64; 2],
    pub zoom: u32,
    pub tiles: Vec<[u32; 2]>,
    pub palette: String,
}

/// What a view promises: one station, one source zoom, an exact tile set and one palette.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ViewScope {
    pub station: String,
    pub zoom: u32,
    pub tiles: Vec<[u32; 2]>,
    pub palette: String,
}

impl ViewScope {
    /// Shared by every visitor asking for the same station, zoom, tile set and palette.
    pub fn id(&self) -> String {
        let tiles: Vec<String> = self
            .tiles
            .iter()
            .map(|t| format!("{},{}", t[0], t[1]))
            .collect();
        let text = format!(
            "level2-view-v1|{}|{}|{}|{}",
            self.station,
            self.zoom,
            tiles.join(";"),
            self.palette
        );
        crate::checksum(text.as_bytes())[..32].to_owned()
    }
}

pub fn tile_x(lon: f64, z: u32) -> f64 {
    (lon + 180.0) / 360.0 * f64::from(1u32 << z)
}

pub fn tile_y(lat: f64, z: u32) -> f64 {
    let r = lat.to_radians();
    (1.0 - (r.tan() + 1.0 / r.cos()).ln() / std::f64::consts::PI) / 2.0 * f64::from(1u32 << z)
}

/// Checks bounds, the exact tile set and the centre. The reason is the public fallback code.
pub fn validate(raw: RawView) -> Result<Validated, &'static str> {
    let (lat, lon) = (raw.centre.lat, raw.centre.lon);
    if !(lat.is_finite() && lon.is_finite() && lat.abs() <= 85.0 && lon.abs() <= 180.0) {
        return Err("invalid_centre");
    }
    if !(8..=13).contains(&raw.zoom) {
        return Err("invalid_zoom");
    }
    if !PALETTES.contains(&raw.palette.as_str()) {
        return Err("invalid_palette");
    }
    if raw.tiles.is_empty() {
        return Err("no_tiles");
    }
    if raw.tiles.len() > MAX_TILES {
        return Err("too_many_tiles");
    }
    let n = 1u32 << raw.zoom;
    if raw.tiles.iter().any(|t| t[0] >= n || t[1] >= n) {
        return Err("invalid_tile");
    }
    let set: BTreeSet<[u32; 2]> = raw.tiles.iter().copied().collect();
    if set.len() != raw.tiles.len() {
        return Err("duplicate_tile");
    }
    let tiles: Vec<[u32; 2]> = set.into_iter().collect();
    let span = |i: usize| {
        let (lo, hi) = tiles
            .iter()
            .fold((u32::MAX, 0), |(lo, hi), t| (lo.min(t[i]), hi.max(t[i])));
        (lo, hi)
    };
    let (x0, x1) = span(0);
    let (y0, y1) = span(1);
    if x1 - x0 >= MAX_SPAN_TILES || y1 - y0 >= MAX_SPAN_TILES {
        return Err("excessive_area");
    }
    let (cx, cy) = (tile_x(lon, raw.zoom), tile_y(lat, raw.zoom));
    let inside = cx >= f64::from(x0)
        && cx <= f64::from(x1 + 1)
        && cy >= f64::from(y0)
        && cy <= f64::from(y1 + 1);
    if !inside {
        return Err("centre_outside_tiles");
    }
    Ok(Validated {
        centre: [lat, lon],
        zoom: raw.zoom,
        tiles,
        palette: raw.palette,
    })
}

/// Height of the beam centre above the radar on the 4/3 effective Earth (not above terrain).
pub fn beam_height_m(ground_m: f64, elevation_deg: f64) -> Option<f64> {
    let k = EARTH_RADIUS_M * 4.0 / 3.0;
    let r = ground_to_slant(ground_m, elevation_deg)?;
    Some((r * r + k * k + 2.0 * r * k * elevation_deg.to_radians().sin()).sqrt() - k)
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Build {
    pub since: DateTime<Utc>,
    pub deadline: DateTime<Utc>,
    pub claimed_until: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReadyInfo {
    pub revision: String,
    pub issued_at: DateTime<Utc>,
    pub expires_at: DateTime<Utc>,
    pub newest_scan_end: DateTime<Utc>,
    pub verified_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FallbackInfo {
    pub reason: String,
    pub at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ViewRecord {
    pub id: String,
    pub scope: ViewScope,
    pub centre: [f64; 2],
    pub created_at: DateTime<Utc>,
    pub lease_until: DateTime<Utc>,
    pub build: Option<Build>,
    pub ready: Option<ReadyInfo>,
    pub fallback: Option<FallbackInfo>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Status {
    Pending {
        deadline: DateTime<Utc>,
    },
    Ready {
        revision: String,
        expires_at: DateTime<Utc>,
    },
    Fallback {
        reason: String,
    },
    Expired,
}

impl ViewRecord {
    pub fn new(scope: ViewScope, centre: [f64; 2], now: DateTime<Utc>) -> Self {
        Self {
            id: scope.id(),
            scope,
            centre,
            created_at: now,
            lease_until: now + TimeDelta::seconds(VIEW_LEASE_S),
            build: Some(Build {
                since: now,
                deadline: now + TimeDelta::seconds(PENDING_DEADLINE_S),
                claimed_until: None,
            }),
            ready: None,
            fallback: None,
        }
    }

    pub fn ready_valid(&self, now: DateTime<Utc>) -> Option<&ReadyInfo> {
        self.ready.as_ref().filter(|r| now < r.expires_at)
    }

    /// The honest state at `now`. A ready revision is promised only until its expiry.
    pub fn status(&self, now: DateTime<Utc>) -> Status {
        if let Some(r) = self.ready_valid(now) {
            return Status::Ready {
                revision: r.revision.clone(),
                expires_at: r.expires_at,
            };
        }
        if let Some(b) = self.build.as_ref().filter(|b| now <= b.deadline) {
            return Status::Pending {
                deadline: b.deadline,
            };
        }
        if let Some(f) = &self.fallback {
            return Status::Fallback {
                reason: f.reason.clone(),
            };
        }
        if self.build.is_some() {
            return Status::Fallback {
                reason: "pending_timeout".into(),
            };
        }
        if self.ready.is_some() {
            return Status::Expired;
        }
        Status::Fallback {
            reason: "unavailable".into(),
        }
    }

    pub fn status_json(&self, now: DateTime<Utc>) -> Value {
        let mut v = json!({
            "schema_version": SCHEMA_VERSION,
            "view_id": self.id,
            "station_id": self.scope.station,
            "source_zoom": self.scope.zoom,
            "palette": self.scope.palette,
        });
        let o = v.as_object_mut().expect("object");
        match self.status(now) {
            Status::Pending { deadline } => {
                o.insert("state".into(), json!("pending"));
                o.insert("reason".into(), json!("warming"));
                o.insert("deadline".into(), json!(deadline));
                o.insert("poll_after_s".into(), json!(POLL_PENDING_S));
            }
            Status::Ready {
                revision,
                expires_at,
            } => {
                o.insert("state".into(), json!("ready"));
                o.insert("expires_at".into(), json!(expires_at));
                o.insert("poll_after_s".into(), json!(POLL_READY_S));
                o.insert(
                    "manifest_url".into(),
                    json!(format!(
                        "/api/level2/views/{}/{revision}/manifest.json",
                        self.id
                    )),
                );
            }
            Status::Fallback { reason } => {
                o.insert("state".into(), json!("fallback"));
                o.insert("reason".into(), json!(reason));
                o.insert("poll_after_s".into(), json!(POLL_READY_S));
            }
            Status::Expired => {
                o.insert("state".into(), json!("expired"));
                o.insert("reason".into(), json!("view_expired"));
                o.insert("poll_after_s".into(), json!(POLL_READY_S));
            }
        }
        v
    }
}

impl ViewRecord {
    /// Turns an overdue build into a fallback unless a ready revision is still valid.
    pub fn settle(&mut self, now: DateTime<Utc>) {
        if self.build.as_ref().is_some_and(|b| now > b.deadline) {
            let at = self.build.take().map_or(now, |b| b.deadline);
            if self.ready_valid(now).is_none() {
                self.fallback = Some(FallbackInfo {
                    reason: "pending_timeout".into(),
                    at,
                });
            }
        }
    }

    pub fn start_build(&mut self, now: DateTime<Utc>) {
        if self.build.is_none() {
            self.fallback = None;
            self.build = Some(Build {
                since: now,
                deadline: now + TimeDelta::seconds(PENDING_DEADLINE_S),
                claimed_until: None,
            });
        }
    }

    pub fn mark_published(&mut self, rev: &Revision, now: DateTime<Utc>) {
        self.build = None;
        self.fallback = None;
        self.ready = Some(ReadyInfo {
            revision: rev.revision.clone(),
            issued_at: rev.issued_at,
            expires_at: rev.expires_at,
            newest_scan_end: rev.newest_scan_end,
            verified_at: now,
        });
    }

    pub fn mark_failed(&mut self, reason: &str, now: DateTime<Utc>) {
        self.build = None;
        if self.ready_valid(now).is_none() {
            self.fallback = Some(FallbackInfo {
                reason: reason.into(),
                at: now,
            });
        }
    }

    /// True when `revision` was still the valid one and now lasts until `expires_at`.
    pub fn mark_renewed(
        &mut self,
        revision: &str,
        expires_at: DateTime<Utc>,
        now: DateTime<Utc>,
    ) -> bool {
        match self.ready.as_mut() {
            Some(r) if r.revision == revision && now < r.expires_at => {
                r.expires_at = expires_at;
                r.verified_at = now;
                true
            }
            _ => false,
        }
    }

    pub fn revoke(&mut self, now: DateTime<Utc>) {
        if let Some(r) = self.ready.as_mut() {
            r.expires_at = r.expires_at.min(now);
        }
        self.start_build(now);
    }
}

/// The shared submit rule: create, share or renew a view. `None` means the pending cap is full.
pub fn apply_submit(
    existing: Option<ViewRecord>,
    scope: &ViewScope,
    centre: [f64; 2],
    now: DateTime<Utc>,
    pending: usize,
) -> Option<ViewRecord> {
    let Some(mut view) = existing else {
        return (pending < MAX_PENDING_VIEWS).then(|| ViewRecord::new(scope.clone(), centre, now));
    };
    view.settle(now);
    view.lease_until = now + TimeDelta::seconds(VIEW_LEASE_S);
    let retry_ok = view
        .fallback
        .as_ref()
        .is_none_or(|f| now - f.at >= TimeDelta::seconds(FALLBACK_RETRY_S));
    if view.build.is_none() && view.ready_valid(now).is_none() && retry_ok {
        if pending >= MAX_PENDING_VIEWS {
            return None;
        }
        view.start_build(now);
    }
    Some(view)
}

/// One committed, verified revision of a view. `envelope` is the exact manifest body.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Revision {
    pub view_id: String,
    pub revision: String,
    pub envelope: String,
    pub etag: String,
    pub issued_at: DateTime<Utc>,
    pub expires_at: DateTime<Utc>,
    pub read_until: DateTime<Utc>,
    pub newest_scan_end: DateTime<Utc>,
    pub tile_keys: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TileMeta {
    pub sha256: String,
    pub bytes: i64,
    pub transparent: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SweepRef {
    pub scan_end: DateTime<Utc>,
    pub identity: String,
    pub commit_key: String,
}

pub enum Submitted {
    Record(Box<ViewRecord>),
    Saturated,
}

#[derive(Debug, PartialEq, Eq)]
pub enum Pin {
    Staged,
    Collecting,
}

#[async_trait]
pub trait SweepIndex: Send + Sync {
    /// Newest indexed sweeps first, at most `n`.
    async fn recent(&self, station: &str, n: usize) -> Result<Vec<SweepRef>>;
    async fn health(&self, station: &str) -> Result<Health>;
}

#[async_trait]
pub trait ViewStore: Send + Sync {
    /// Records demand for a station. False when eight other stations are already active.
    async fn touch_station(&self, station: &str, now: DateTime<Utc>) -> Result<bool>;
    /// Creates, shares or renews a view and decides whether a new build starts.
    async fn submit(
        &self,
        scope: &ViewScope,
        centre: [f64; 2],
        now: DateTime<Utc>,
    ) -> Result<Submitted>;
    async fn get(&self, id: &str) -> Result<Option<ViewRecord>>;
    /// Takes the oldest unclaimed build whose lease and deadline are still open.
    async fn claim(&self, now: DateTime<Utc>, ttl: TimeDelta) -> Result<Option<ViewRecord>>;
    /// Turns overdue builds into `pending_timeout` fallbacks. Returns how many.
    async fn expire_builds(&self, now: DateTime<Utc>) -> Result<usize>;
    /// Starts a build for a ready view whose newest sweep is no longer the newest indexed one.
    async fn start_refresh(&self, id: &str, now: DateTime<Utc>) -> Result<()>;
    /// Pins tile keys for a build so collection cannot remove them while it verifies.
    async fn stage_pins(&self, owner: &str, keys: &[String], until: DateTime<Utc>) -> Result<Pin>;
    /// One transaction: store the revision, pin its tiles until `read_until`, mark the view ready.
    async fn publish(&self, rev: &Revision, owner: &str, now: DateTime<Utc>) -> Result<()>;
    /// Ends a build. A still-valid ready revision stays readable; otherwise the view falls back.
    async fn fail(&self, id: &str, reason: &str, owner: &str, now: DateTime<Utc>) -> Result<()>;
    /// Ends the promise of a ready revision (a referenced object went missing) and starts a rebuild.
    async fn revoke(&self, id: &str, reason: &str, now: DateTime<Utc>) -> Result<()>;
    /// Extends a still-valid ready revision and its pins.
    async fn renew(
        &self,
        id: &str,
        revision: &str,
        expires_at: DateTime<Utc>,
        now: DateTime<Utc>,
    ) -> Result<bool>;
    async fn revision(&self, id: &str, revision: &str) -> Result<Option<Revision>>;
    async fn tile_meta(&self, key: &str) -> Result<Option<TileMeta>>;
    /// Latest time a tile key is promised readable. None when never pinned or already collected.
    async fn tile_pin(&self, key: &str) -> Result<Option<DateTime<Utc>>>;
    /// Drops expired pins and revisions, then marks tiles nothing pins. Returns the keys to delete.
    async fn collect(&self, now: DateTime<Utc>) -> Result<Vec<String>>;
    async fn collected(&self, keys: &[String]) -> Result<()>;
}
