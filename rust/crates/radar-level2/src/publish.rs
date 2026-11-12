//! Publishes a ready viewport: renders the finite tile set for the retained sweeps, verifies every
//! object, then commits one revision. The API never calls this; a worker drives it.

use crate::{
    artifact,
    catalogue::{Catalogue, Site, distance_m},
    tile::{Outcome, TileRequest, TileService},
    viewport::*,
};
use anyhow::{Context, Result, ensure};
use async_trait::async_trait;
use chrono::{DateTime, TimeDelta, Utc};
use futures_util::{StreamExt, stream};
use object_store::{ObjectStore, path::Path};
use radar_core::{manifest::parse_manifest_value, time::isoformat_utc};
use serde_json::{Value, json};
use std::{sync::Arc, time::Duration};

pub const TILE_CAP: usize = 1024 * 1024;
const ZATTRS_CAP: usize = 8 * 1024 * 1024;
const CLAIM_TTL_S: i64 = 10;
const PNG_MAGIC: &[u8] = b"\x89PNG\r\n\x1a\n";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TileDone {
    pub key: String,
    pub sha256: String,
    pub bytes: usize,
    pub transparent: bool,
}

/// Where tiles come from. `TileService` is the real one; tests substitute a fake.
#[async_trait]
pub trait TileSource: Send + Sync {
    fn key(&self, commit_key: &str, palette: &str, z: u32, x: u32, y: u32) -> Option<String>;
    fn version(&self) -> String;
    async fn tile(
        &self,
        commit_key: &str,
        palette: &str,
        z: u32,
        x: u32,
        y: u32,
        deadline: Duration,
    ) -> Result<TileDone, String>;
}

#[async_trait]
impl TileSource for Arc<TileService> {
    fn key(&self, commit_key: &str, palette: &str, z: u32, x: u32, y: u32) -> Option<String> {
        self.tile_key(&TileRequest {
            commit_key: commit_key.into(),
            palette: palette.into(),
            z,
            x,
            y,
        })
    }
    fn version(&self) -> String {
        TileService::version(self).to_owned()
    }
    async fn tile(
        &self,
        commit_key: &str,
        palette: &str,
        z: u32,
        x: u32,
        y: u32,
        deadline: Duration,
    ) -> Result<TileDone, String> {
        let req = TileRequest {
            commit_key: commit_key.into(),
            palette: palette.into(),
            z,
            x,
            y,
        };
        match self.render(req, deadline).await {
            Outcome::Ready(r) => Ok(TileDone {
                key: r.key,
                sha256: r.sha256,
                bytes: r.png.len(),
                transparent: r.transparent,
            }),
            Outcome::Fallback(reason) => Err(reason.to_owned()),
        }
    }
}

#[derive(Debug, PartialEq, Eq)]
pub enum Done {
    Idle,
    Published(String),
    Waiting(String),
    Failed(String),
}

pub type Clock = Arc<dyn Fn() -> DateTime<Utc> + Send + Sync>;

pub struct Publisher {
    pub store: Arc<dyn ViewStore>,
    pub index: Arc<dyn SweepIndex>,
    pub tiles: Arc<dyn TileSource>,
    pub objects: Arc<dyn ObjectStore>,
    pub catalogue: Arc<Catalogue>,
    pub clock: Clock,
    pub instance: String,
}

fn newest_age(end: DateTime<Utc>, now: DateTime<Utc>) -> TimeDelta {
    now - end
}

impl Publisher {
    fn now(&self) -> DateTime<Utc> {
        (self.clock)()
    }

    /// Claims one pending view and works it. `Idle` when nothing is waiting.
    pub async fn run_once(&self) -> Result<Done> {
        let now = self.now();
        self.store.expire_builds(now).await?;
        let Some(view) = self
            .store
            .claim(now, TimeDelta::seconds(CLAIM_TTL_S))
            .await?
        else {
            return Ok(Done::Idle);
        };
        Ok(self.process(view).await)
    }

    /// Drops expired revisions and pins, then deletes tiles nothing pins any more.
    pub async fn collect(&self) -> Result<usize> {
        let keys = self.store.collect(self.now()).await?;
        for key in &keys {
            match self.objects.delete(&Path::from(key.as_str())).await {
                Ok(()) | Err(object_store::Error::NotFound { .. }) => {}
                Err(e) => return Err(e.into()),
            }
        }
        self.store.collected(&keys).await?;
        Ok(keys.len())
    }

    pub async fn process(&self, view: ViewRecord) -> Done {
        let owner = format!("build:{}:{}", self.instance, view.id);
        let outcome = self.build(&view, &owner).await;
        match &outcome {
            Ok(Done::Published(_)) => {
                let cold = (self.now() - view.created_at).to_std().unwrap_or_default();
                crate::metrics::observe("radar_level2_cold_ready_seconds", cold);
            }
            Err(reason) => crate::metrics::incr(
                "radar_level2_fallback_total",
                &[("reason", reason.as_str()), ("stage", "publish")],
            ),
            _ => {}
        }
        match outcome {
            Ok(Done::Published(r)) => Done::Published(r),
            Ok(Done::Waiting(reason)) => {
                let now = self.now();
                let near = view
                    .build
                    .as_ref()
                    .is_none_or(|b| b.deadline - now <= TimeDelta::seconds(CLAIM_TTL_S));
                if near {
                    crate::metrics::incr(
                        "radar_level2_fallback_total",
                        &[("reason", reason.as_str()), ("stage", "publish")],
                    );
                    let _ = self.store.fail(&view.id, &reason, &owner, now).await;
                    Done::Failed(reason)
                } else {
                    Done::Waiting(reason)
                }
            }
            Ok(other) => other,
            Err(reason) => {
                let _ = self.store.fail(&view.id, &reason, &owner, self.now()).await;
                Done::Failed(reason)
            }
        }
    }

    async fn build(&self, view: &ViewRecord, owner: &str) -> Result<Done, String> {
        let db = |_e: anyhow::Error| "database_unavailable".to_owned();
        let now = self.now();
        let deadline = view.build.as_ref().map_or(now, |b| b.deadline);
        if now > deadline {
            return Err("pending_timeout".into());
        }
        let station = &view.scope.station;
        let site = self.catalogue.sites.get(station).ok_or("unknown_station")?;
        let mut sweeps = self.index.recent(station, MAX_FRAMES).await.map_err(db)?;
        sweeps.sort_by_key(|s| s.scan_end);
        let Some(newest) = sweeps.last() else {
            return Ok(Done::Waiting("warming".into()));
        };
        let age = newest_age(newest.scan_end, now);
        if age < TimeDelta::zero() {
            return Err("future_clock".into());
        }
        if age > TimeDelta::seconds(MAX_AGE_S) {
            return Ok(Done::Waiting("late".into()));
        }
        let mut frames = Vec::new();
        for s in &sweeps {
            frames.push(
                self.frame(station, s)
                    .await
                    .map_err(|_| "artifact_unavailable".to_owned())?,
            );
        }
        let z = view.scope.zoom;
        let mut wanted = Vec::new();
        for f in &frames {
            for t in &view.scope.tiles {
                let key = self
                    .tiles
                    .key(&f.sweep.commit_key, &view.scope.palette, z, t[0], t[1])
                    .ok_or("invalid_tile")?;
                wanted.push((f.sweep.commit_key.clone(), t[0], t[1], key));
            }
        }
        let keys: Vec<String> = wanted.iter().map(|w| w.3.clone()).collect();
        let staged = self
            .store
            .stage_pins(owner, &keys, deadline + TimeDelta::seconds(READ_GRACE_S))
            .await
            .map_err(db)?;
        if staged == Pin::Collecting {
            return Ok(Done::Waiting("gc_in_progress".into()));
        }
        let remaining = (deadline - self.now()).to_std().unwrap_or(Duration::ZERO);
        let rendered = tokio::time::timeout(
            remaining,
            self.render_all(&wanted, &view.scope.palette, z, deadline),
        )
        .await
        .map_err(|_| "pending_timeout".to_owned())??;
        self.verify_all(&rendered).await?;
        let after = self.now();
        let age = newest_age(newest.scan_end, after);
        if age > TimeDelta::seconds(MAX_AGE_S) || age < TimeDelta::zero() {
            return Err("late".into());
        }
        let expires_at = (after + TimeDelta::seconds(VIEW_LEASE_S))
            .min(newest.scan_end + TimeDelta::seconds(MAX_AGE_S));
        let read_until = expires_at + TimeDelta::seconds(READ_GRACE_S);
        let version = self.tiles.version();
        let revision = revision_id(
            &frames
                .iter()
                .map(|f| f.sweep.identity.clone())
                .collect::<Vec<_>>(),
            &version,
            &view.scope,
        );
        let envelope = build_envelope(&EnvelopeInput {
            view,
            site,
            frames: &frames,
            tile_version: &version,
            revision: &revision,
            issued_at: after,
            expires_at,
            read_until,
        })
        .map_err(|_| "manifest_invalid".to_owned())?;
        let rev = Revision {
            view_id: view.id.clone(),
            revision: revision.clone(),
            etag: format!("\"{}\"", &crate::checksum(envelope.as_bytes())[..24]),
            envelope,
            issued_at: after,
            expires_at,
            read_until,
            newest_scan_end: newest.scan_end,
            tile_keys: keys,
        };
        self.store
            .publish(&rev, owner, after)
            .await
            .map_err(|_| "database_unavailable".to_owned())?;
        Ok(Done::Published(revision))
    }

    async fn render_all(
        &self,
        wanted: &[(String, u32, u32, String)],
        palette: &str,
        z: u32,
        deadline: DateTime<Utc>,
    ) -> Result<Vec<TileDone>, String> {
        let results: Vec<Result<TileDone, String>> = stream::iter(wanted.iter())
            .map(|(commit, x, y, key)| async move {
                loop {
                    let left = (deadline - self.now()).to_std().unwrap_or(Duration::ZERO);
                    if left.is_zero() {
                        return Err("pending_timeout".to_owned());
                    }
                    match self.tiles.tile(commit, palette, z, *x, *y, left).await {
                        Err(r) if r == "render_saturated" => {
                            tokio::time::sleep(Duration::from_millis(25)).await
                        }
                        Ok(done) if done.key != *key => return Err("tile_key_mismatch".into()),
                        other => return other,
                    }
                }
            })
            .buffer_unordered(crate::RENDER_JOBS)
            .collect()
            .await;
        results.into_iter().collect()
    }

    /// Reads every tile back from the object store and checks it against what the renderer reported.
    async fn verify_all(&self, tiles: &[TileDone]) -> Result<(), String> {
        let results: Vec<Result<(), String>> = stream::iter(tiles.iter())
            .map(|t| async move {
                let bytes = artifact::read_bounded(
                    self.objects.as_ref(),
                    &Path::from(t.key.as_str()),
                    TILE_CAP,
                )
                .await
                .map_err(|_| "tile_missing".to_owned())?;
                let meta = self
                    .store
                    .tile_meta(&t.key)
                    .await
                    .map_err(|_| "database_unavailable".to_owned())?;
                let sha = crate::checksum(&bytes);
                let meta_ok =
                    meta.is_some_and(|m| m.sha256 == sha && m.bytes == bytes.len() as i64);
                if !bytes.starts_with(PNG_MAGIC)
                    || sha != t.sha256
                    || bytes.len() != t.bytes
                    || !meta_ok
                {
                    return Err("tile_verification_failed".to_owned());
                }
                Ok(())
            })
            .buffer_unordered(8)
            .collect()
            .await;
        results.into_iter().collect()
    }

    async fn frame(&self, station: &str, sweep: &SweepRef) -> Result<FrameInput> {
        let (commit, polar) = artifact::load(self.objects.as_ref(), &sweep.commit_key).await?;
        ensure!(
            commit.station == station
                && commit.identity == sweep.identity
                && commit.scan_end == sweep.scan_end,
            "artifact mismatch"
        );
        let attrs: Value = serde_json::from_slice(
            &artifact::read_bounded(
                self.objects.as_ref(),
                &Path::from(format!(
                    "{}.zattrs",
                    sweep.commit_key.trim_end_matches("commit.json")
                )),
                ZATTRS_CAP,
            )
            .await?,
        )?;
        Ok(FrameInput {
            sweep: sweep.clone(),
            attrs,
            elevation_deg: polar.mean_elevation_deg().context("elevation")?,
            sources: serde_json::to_value(&commit.sources)?,
        })
    }
}

pub fn revision_id(sweeps: &[String], tile_version: &str, scope: &ViewScope) -> String {
    let text = format!(
        "level2-revision-v1|{}|{tile_version}|{}",
        sweeps.join(","),
        scope.id()
    );
    crate::checksum(text.as_bytes())[..16].to_owned()
}

/// The facts the envelope carries about one sweep.
#[derive(Debug, Clone)]
pub struct FrameInput {
    pub sweep: SweepRef,
    pub attrs: Value,
    pub elevation_deg: f64,
    pub sources: Value,
}

pub struct EnvelopeInput<'a> {
    pub view: &'a ViewRecord,
    pub site: &'a Site,
    pub frames: &'a [FrameInput],
    pub tile_version: &'a str,
    pub revision: &'a str,
    pub issued_at: DateTime<Utc>,
    pub expires_at: DateTime<Utc>,
    pub read_until: DateTime<Utc>,
}

fn dual_pol(attrs: &Value, index: usize) -> &'static str {
    let present = attrs["rays"].as_array().is_some_and(|r| {
        r.iter()
            .any(|ray| ray["moments"].get(index).is_some_and(|m| !m.is_null()))
    });
    if present { "available" } else { "absent" }
}

/// The versioned viewport envelope: manifest v2 with one station-scoped layer, plus exact tiles.
pub fn build_envelope(i: &EnvelopeInput) -> Result<String> {
    let station = &i.view.scope.station;
    let palette = &i.view.scope.palette;
    let [lat, lon] = i.view.centre;
    let ground = distance_m(lat, lon, i.site.lat, i.site.lon);
    let frames: Vec<Value> = i
        .frames
        .iter()
        .map(|f| {
            let beam = beam_height_m(ground, f.elevation_deg);
            let a = &f.attrs;
            let end = isoformat_utc(&f.sweep.scan_end);
            json!({
                "timestamp": end,
                "path": format!("{}/{}", f.sweep.identity, i.tile_version),
                "palettes": [palette],
                "kind": "observed",
                "source": "level2",
                "issued_at": isoformat_utc(&i.issued_at),
                "units": "dBZ",
                "station_id": station,
                "station_name": i.site.name,
                "station_lat": i.site.lat,
                "station_lon": i.site.lon,
                "source_sites": [station],
                "volume_id": format!("{}#{}", a["volume_start"].as_str().unwrap_or(""), a["volume_number"].as_str().unwrap_or("")),
                "sweep_id": f.sweep.identity,
                "tile_revision": i.tile_version,
                "scan_start": a["scan_start"],
                "scan_end": end,
                "elevation_deg": f.elevation_deg,
                "geometry_digest": a["geometry_digest"],
                "max_ground_range_m": MAX_GROUND_RANGE_M,
                "ray_count": a["rays"].as_array().map_or(0, Vec::len),
                "missing_sectors_deg": a["missing_sectors_deg"],
                "antenna_altitude_m_msl": a["site"]["site_height_m_raw"],
                "beam_model": "effective-earth-4/3",
                "ground_range_m": ground,
                "beam_height_m_above_radar": beam,
                "qc_profile": a["qc_profile"],
                "decoder": a["decoder"],
                "dual_pol": {"rho": dual_pol(a, 1), "zdr": dual_pol(a, 2)},
                "sources": f.sources,
                "sampler_version": crate::polar::SAMPLER_VERSION,
            })
        })
        .collect();
    let timestamps: Vec<Value> = frames.iter().map(|f| f["timestamp"].clone()).collect();
    let latest = timestamps.last().cloned();
    let layer = format!("level2-ref-{station}");
    let mut layer_value = json!({
        "title": format!("Single-site observed reflectivity · {station}"),
        "product": "level2-ref",
        "station_id": station,
        "kind": "observed",
        "units": "dBZ",
        "delivery": "viewport",
        "frames": frames,
        "timestamps": timestamps,
        "palettes": [palette],
    });
    if let Some(latest) = latest {
        layer_value["latest"] = latest;
    }
    let raw = json!({
        "schema_version": 2,
        "tile_url_template": TILE_TEMPLATE,
        "updated_at": isoformat_utc(&i.issued_at),
        "layers": { layer: layer_value },
    });
    let manifest = parse_manifest_value(raw, i.issued_at)?;
    let required: Vec<Value> = i
        .view
        .scope
        .tiles
        .iter()
        .map(|t| json!({"z": i.view.scope.zoom, "x": t[0], "y": t[1]}))
        .collect();
    let envelope = json!({
        "schema_version": SCHEMA_VERSION,
        "view_id": i.view.id,
        "revision": i.revision,
        "station": {"id": station, "name": i.site.name, "lat": i.site.lat, "lon": i.site.lon},
        "centre": {"lat": lat, "lon": lon, "ground_range_m": ground},
        "source_zoom": i.view.scope.zoom,
        "palettes": [palette],
        "required_tiles": required,
        "tile_url_template": TILE_TEMPLATE,
        "issued_at": i.issued_at,
        "expires_at": i.expires_at,
        "read_until": i.read_until,
        "manifest": manifest,
    });
    Ok(serde_json::to_string(&envelope)?)
}
