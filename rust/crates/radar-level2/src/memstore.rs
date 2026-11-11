//! In-memory `ViewStore` and `SweepIndex`: the reference semantics, used by tests and fixtures.

use crate::{health::Health, viewport::*};
use anyhow::{Result, bail};
use async_trait::async_trait;
use chrono::{DateTime, TimeDelta, Utc};
use std::{
    collections::{BTreeMap, HashMap},
    sync::{
        Mutex,
        atomic::{AtomicBool, Ordering},
    },
};

#[derive(Default)]
struct Inner {
    stations: HashMap<String, DateTime<Utc>>,
    views: BTreeMap<String, ViewRecord>,
    revisions: HashMap<(String, String), Revision>,
    pins: Vec<(String, String, DateTime<Utc>)>,
    tiles: HashMap<String, (TileMeta, bool)>,
    sweeps: HashMap<String, Vec<SweepRef>>,
    health: HashMap<String, Health>,
}

#[derive(Default)]
pub struct MemoryStore {
    inner: Mutex<Inner>,
    /// Makes `publish` fail as a database commit failure would.
    pub fail_publish: AtomicBool,
    /// Makes every store call fail as an unreachable database would.
    pub fail_all: AtomicBool,
}

impl MemoryStore {
    fn guard(&self) -> Result<std::sync::MutexGuard<'_, Inner>> {
        if self.fail_all.load(Ordering::SeqCst) {
            bail!("database unavailable");
        }
        Ok(self.inner.lock().expect("store lock"))
    }

    pub fn add_sweep(&self, station: &str, sweep: SweepRef) {
        let mut g = self.inner.lock().expect("store lock");
        let list = g.sweeps.entry(station.into()).or_default();
        list.push(sweep);
        list.sort_by_key(|s| std::cmp::Reverse(s.scan_end));
    }

    pub fn set_health(&self, station: &str, health: Health) {
        self.inner
            .lock()
            .expect("store lock")
            .health
            .insert(station.into(), health);
    }

    pub fn put_tile(&self, key: &str, meta: TileMeta) {
        self.inner
            .lock()
            .expect("store lock")
            .tiles
            .insert(key.into(), (meta, false));
    }

    pub fn pin_count(&self) -> usize {
        self.inner.lock().expect("store lock").pins.len()
    }

    pub fn revision_count(&self) -> usize {
        self.inner.lock().expect("store lock").revisions.len()
    }

    pub fn put_view(&self, view: ViewRecord) {
        self.inner
            .lock()
            .expect("store lock")
            .views
            .insert(view.id.clone(), view);
    }
}

#[async_trait]
impl SweepIndex for MemoryStore {
    async fn recent(&self, station: &str, n: usize) -> Result<Vec<SweepRef>> {
        Ok(self
            .guard()?
            .sweeps
            .get(station)
            .map(|l| l.iter().take(n).cloned().collect())
            .unwrap_or_default())
    }
    async fn health(&self, station: &str) -> Result<Health> {
        Ok(self
            .guard()?
            .health
            .get(station)
            .cloned()
            .unwrap_or_default())
    }
}

#[async_trait]
impl ViewStore for MemoryStore {
    async fn touch_station(&self, station: &str, now: DateTime<Utc>) -> Result<bool> {
        let mut g = self.guard()?;
        let until = now + TimeDelta::seconds(600);
        let active = g.stations.values().filter(|u| **u >= now).count();
        let known = g.stations.get(station).is_some_and(|u| *u >= now);
        if !known && active >= 8 {
            return Ok(false);
        }
        g.stations.insert(station.into(), until);
        Ok(true)
    }

    async fn submit(
        &self,
        scope: &ViewScope,
        centre: [f64; 2],
        now: DateTime<Utc>,
    ) -> Result<Submitted> {
        let mut g = self.guard()?;
        let id = scope.id();
        let pending = g
            .views
            .values()
            .filter(|v| v.build.is_some() && v.lease_until > now)
            .count();
        let existing = g.views.get(&id).cloned();
        let Some(view) = apply_submit(existing, scope, centre, now, pending) else {
            return Ok(Submitted::Saturated);
        };
        g.views.insert(id, view.clone());
        Ok(Submitted::Record(Box::new(view)))
    }

    async fn get(&self, id: &str) -> Result<Option<ViewRecord>> {
        Ok(self.guard()?.views.get(id).cloned())
    }

    async fn claim(&self, now: DateTime<Utc>, ttl: TimeDelta) -> Result<Option<ViewRecord>> {
        let mut g = self.guard()?;
        let pick = g
            .views
            .values()
            .filter(|v| v.lease_until > now)
            .filter(|v| {
                v.build
                    .as_ref()
                    .is_some_and(|b| b.deadline >= now && b.claimed_until.is_none_or(|c| c < now))
            })
            .min_by_key(|v| v.build.as_ref().map(|b| b.since))
            .map(|v| v.id.clone());
        let Some(id) = pick else { return Ok(None) };
        let view = g.views.get_mut(&id).expect("picked");
        if let Some(b) = view.build.as_mut() {
            b.claimed_until = Some(now + ttl);
        }
        Ok(Some(view.clone()))
    }

    async fn expire_builds(&self, now: DateTime<Utc>) -> Result<usize> {
        let mut g = self.guard()?;
        let mut n = 0;
        for v in g.views.values_mut() {
            if v.build.as_ref().is_some_and(|b| now > b.deadline) {
                v.settle(now);
                n += 1;
            }
        }
        Ok(n)
    }

    async fn start_refresh(&self, id: &str, now: DateTime<Utc>) -> Result<()> {
        let mut g = self.guard()?;
        if let Some(v) = g.views.get_mut(id) {
            v.start_build(now);
        }
        Ok(())
    }

    async fn stage_pins(&self, owner: &str, keys: &[String], until: DateTime<Utc>) -> Result<Pin> {
        let mut g = self.guard()?;
        if keys.iter().any(|k| g.tiles.get(k).is_some_and(|t| t.1)) {
            return Ok(Pin::Collecting);
        }
        g.pins.retain(|p| p.1 != owner);
        for k in keys {
            g.pins.push((k.clone(), owner.into(), until));
        }
        Ok(Pin::Staged)
    }

    async fn publish(&self, rev: &Revision, owner: &str, now: DateTime<Utc>) -> Result<()> {
        if self.fail_publish.load(Ordering::SeqCst) {
            bail!("commit failed");
        }
        let mut g = self.guard()?;
        if !g.views.contains_key(&rev.view_id) {
            bail!("unknown view");
        }
        g.pins.retain(|p| p.1 != owner);
        for k in &rev.tile_keys {
            g.pins.push((
                k.clone(),
                format!("rev:{}:{}", rev.view_id, rev.revision),
                rev.read_until,
            ));
        }
        g.revisions
            .insert((rev.view_id.clone(), rev.revision.clone()), rev.clone());
        g.views
            .get_mut(&rev.view_id)
            .expect("checked")
            .mark_published(rev, now);
        Ok(())
    }

    async fn fail(&self, id: &str, reason: &str, owner: &str, now: DateTime<Utc>) -> Result<()> {
        let mut g = self.guard()?;
        g.pins.retain(|p| p.1 != owner);
        if let Some(v) = g.views.get_mut(id) {
            v.mark_failed(reason, now);
        }
        Ok(())
    }

    async fn revoke(&self, id: &str, _reason: &str, now: DateTime<Utc>) -> Result<()> {
        if let Some(v) = self.guard()?.views.get_mut(id) {
            v.revoke(now);
        }
        Ok(())
    }

    async fn renew(
        &self,
        id: &str,
        revision: &str,
        expires_at: DateTime<Utc>,
        now: DateTime<Utc>,
    ) -> Result<bool> {
        let mut g = self.guard()?;
        let key = (id.to_owned(), revision.to_owned());
        let Some(view) = g.views.get_mut(id) else {
            return Ok(false);
        };
        if !view.mark_renewed(revision, expires_at, now) {
            return Ok(false);
        }
        let read_until = expires_at + TimeDelta::seconds(READ_GRACE_S);
        let Some(rev) = g.revisions.get_mut(&key) else {
            return Ok(false);
        };
        rev.expires_at = expires_at;
        rev.read_until = read_until;
        let owner = format!("rev:{id}:{revision}");
        for p in g.pins.iter_mut().filter(|p| p.1 == owner) {
            p.2 = read_until;
        }
        Ok(true)
    }

    async fn revision(&self, id: &str, revision: &str) -> Result<Option<Revision>> {
        Ok(self
            .guard()?
            .revisions
            .get(&(id.to_owned(), revision.to_owned()))
            .cloned())
    }

    async fn tile_meta(&self, key: &str) -> Result<Option<TileMeta>> {
        Ok(self
            .guard()?
            .tiles
            .get(key)
            .filter(|t| !t.1)
            .map(|t| t.0.clone()))
    }

    async fn tile_pin(&self, key: &str) -> Result<Option<DateTime<Utc>>> {
        let g = self.guard()?;
        if g.tiles.get(key).is_some_and(|t| t.1) {
            return Ok(None);
        }
        Ok(g.pins.iter().filter(|p| p.0 == key).map(|p| p.2).max())
    }

    async fn collect(&self, now: DateTime<Utc>) -> Result<Vec<String>> {
        let mut g = self.guard()?;
        g.revisions.retain(|_, r| r.read_until >= now);
        let live: std::collections::HashSet<String> = g
            .pins
            .iter()
            .filter(|p| p.2 >= now)
            .map(|p| p.0.clone())
            .collect();
        let dead: Vec<String> = g
            .pins
            .iter()
            .filter(|p| p.2 < now && !live.contains(&p.0))
            .map(|p| p.0.clone())
            .collect::<std::collections::BTreeSet<_>>()
            .into_iter()
            .collect();
        for k in &dead {
            if let Some(t) = g.tiles.get_mut(k) {
                t.1 = true;
            }
        }
        g.pins.retain(|p| p.2 >= now || live.contains(&p.0));
        g.views.retain(|_, v| {
            v.lease_until >= now || v.ready.as_ref().is_some_and(|r| r.expires_at >= now)
        });
        Ok(dead)
    }

    async fn collected(&self, keys: &[String]) -> Result<()> {
        let mut g = self.guard()?;
        for k in keys {
            g.tiles.remove(k);
        }
        Ok(())
    }
}
