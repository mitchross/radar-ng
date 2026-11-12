//! PostgreSQL `ViewStore` and `SweepIndex`. Every mutation runs in one transaction under one advisory lock.

use crate::{health::Health, leases::Pool, viewport::*};
use anyhow::Result;
use async_trait::async_trait;
use chrono::{DateTime, TimeDelta, Utc};
use tokio::sync::Mutex;
use tokio_postgres::Transaction;

pub struct PgStore {
    pool: Mutex<Pool>,
    schema: String,
}

async fn lock(tx: &Transaction<'_>) -> Result<()> {
    tx.query_one("SELECT pg_advisory_xact_lock(290,1)", &[])
        .await?;
    Ok(())
}

fn doc(v: &ViewRecord) -> Result<String> {
    Ok(serde_json::to_string(v)?)
}

impl PgStore {
    pub async fn new(pool: Pool) -> Result<Self> {
        let schema = pool.schema().to_owned();
        let s = &schema;
        pool.client
            .batch_execute(&format!(
                "
            ALTER TABLE {s}.tiles ADD COLUMN IF NOT EXISTS collecting boolean NOT NULL DEFAULT false;
            CREATE TABLE IF NOT EXISTS {s}.views(id text PRIMARY KEY,doc text NOT NULL,lease_until timestamptz NOT NULL,building boolean NOT NULL,build_since timestamptz,build_deadline timestamptz,claimed_until timestamptz);
            CREATE TABLE IF NOT EXISTS {s}.revisions(view_id text NOT NULL,revision text NOT NULL,doc text NOT NULL,expires_at timestamptz NOT NULL,read_until timestamptz NOT NULL,PRIMARY KEY(view_id,revision));
            CREATE TABLE IF NOT EXISTS {s}.pins(key text NOT NULL,owner text NOT NULL,until timestamptz NOT NULL,PRIMARY KEY(key,owner));
            CREATE INDEX IF NOT EXISTS pins_key ON {s}.pins(key);
        "
            ))
            .await?;
        Ok(Self {
            pool: Mutex::new(pool),
            schema,
        })
    }

    async fn with<T, F>(&self, f: F) -> Result<T>
    where
        F: for<'a> FnOnce(
            &'a Transaction<'a>,
            &'a str,
        ) -> std::pin::Pin<
            Box<dyn std::future::Future<Output = Result<T>> + Send + 'a>,
        >,
    {
        let mut pool = self.pool.lock().await;
        let tx = pool.client.transaction().await?;
        lock(&tx).await?;
        let out = f(&tx, &self.schema).await?;
        tx.commit().await?;
        Ok(out)
    }
}

async fn load(tx: &Transaction<'_>, s: &str, id: &str) -> Result<Option<ViewRecord>> {
    tx.query_opt(&format!("SELECT doc FROM {s}.views WHERE id=$1"), &[&id])
        .await?
        .map(|r| Ok(serde_json::from_str(&r.get::<_, String>(0))?))
        .transpose()
}

async fn save(tx: &Transaction<'_>, s: &str, v: &ViewRecord) -> Result<()> {
    let (building, since, deadline, claimed) = match &v.build {
        Some(b) => (true, Some(b.since), Some(b.deadline), b.claimed_until),
        None => (false, None, None, None),
    };
    tx.execute(
        &format!(
            "INSERT INTO {s}.views(id,doc,lease_until,building,build_since,build_deadline,claimed_until) VALUES($1,$2,$3,$4,$5,$6,$7)
             ON CONFLICT(id) DO UPDATE SET doc=excluded.doc,lease_until=excluded.lease_until,building=excluded.building,build_since=excluded.build_since,build_deadline=excluded.build_deadline,claimed_until=excluded.claimed_until"
        ),
        &[&v.id, &doc(v)?, &v.lease_until, &building, &since, &deadline, &claimed],
    )
    .await?;
    Ok(())
}

#[async_trait]
impl SweepIndex for PgStore {
    async fn recent(&self, station: &str, n: usize) -> Result<Vec<SweepRef>> {
        let pool = self.pool.lock().await;
        let limit = n as i64;
        let rows = pool
            .client
            .query(
                &format!("SELECT scan_end,identity,commit_key FROM {}.sweeps WHERE station=$1 AND indexed ORDER BY scan_end DESC LIMIT $2", self.schema),
                &[&station, &limit],
            )
            .await?;
        Ok(rows
            .iter()
            .map(|r| SweepRef {
                scan_end: r.get(0),
                identity: r.get(1),
                commit_key: r.get(2),
            })
            .collect())
    }

    async fn health(&self, station: &str) -> Result<Health> {
        self.pool.lock().await.health(station).await
    }
}

#[async_trait]
impl ViewStore for PgStore {
    async fn touch_station(&self, station: &str, now: DateTime<Utc>) -> Result<bool> {
        let station = station.to_owned();
        self.with(move |tx, s| {
            Box::pin(async move {
                let count: i64 = tx
                    .query_one(&format!("SELECT count(*) FROM {s}.stations WHERE demand_until>=$1 OR busy"), &[&now])
                    .await?
                    .get(0);
                let active = tx
                    .query_opt(&format!("SELECT demand_until>=$2 OR busy FROM {s}.stations WHERE station=$1"), &[&station, &now])
                    .await?
                    .is_some_and(|r| r.get::<_, bool>(0));
                if !active && count >= 8 {
                    return Ok(false);
                }
                let until = now + TimeDelta::seconds(600);
                tx.execute(
                    &format!("INSERT INTO {s}.stations(station,demand_until) VALUES($1,$2) ON CONFLICT(station) DO UPDATE SET demand_until=GREATEST({s}.stations.demand_until,excluded.demand_until)"),
                    &[&station, &until],
                )
                .await?;
                Ok(true)
            })
        })
        .await
    }

    async fn submit(
        &self,
        scope: &ViewScope,
        centre: [f64; 2],
        now: DateTime<Utc>,
    ) -> Result<Submitted> {
        let scope = scope.clone();
        self.with(move |tx, s| {
            Box::pin(async move {
                let id = scope.id();
                let pending: i64 = tx
                    .query_one(
                        &format!(
                            "SELECT count(*) FROM {s}.views WHERE building AND lease_until>$1"
                        ),
                        &[&now],
                    )
                    .await?
                    .get(0);
                let existing = load(tx, s, &id).await?;
                let Some(view) = apply_submit(existing, &scope, centre, now, pending as usize)
                else {
                    return Ok(Submitted::Saturated);
                };
                save(tx, s, &view).await?;
                Ok(Submitted::Record(Box::new(view)))
            })
        })
        .await
    }

    async fn get(&self, id: &str) -> Result<Option<ViewRecord>> {
        let pool = self.pool.lock().await;
        pool.client
            .query_opt(
                &format!("SELECT doc FROM {}.views WHERE id=$1", self.schema),
                &[&id],
            )
            .await?
            .map(|r| Ok(serde_json::from_str(&r.get::<_, String>(0))?))
            .transpose()
    }

    async fn claim(&self, now: DateTime<Utc>, ttl: TimeDelta) -> Result<Option<ViewRecord>> {
        self.with(move |tx, s| {
            Box::pin(async move {
                let Some(row) = tx
                    .query_opt(
                        &format!("SELECT id FROM {s}.views WHERE building AND lease_until>$1 AND build_deadline>=$1 AND (claimed_until IS NULL OR claimed_until<$1) ORDER BY build_since LIMIT 1"),
                        &[&now],
                    )
                    .await?
                else {
                    return Ok(None);
                };
                let id: String = row.get(0);
                let Some(mut v) = load(tx, s, &id).await? else { return Ok(None) };
                if let Some(b) = v.build.as_mut() {
                    b.claimed_until = Some(now + ttl);
                }
                save(tx, s, &v).await?;
                Ok(Some(v))
            })
        })
        .await
    }

    async fn expire_builds(&self, now: DateTime<Utc>) -> Result<usize> {
        self.with(move |tx, s| {
            Box::pin(async move {
                let rows = tx
                    .query(
                        &format!("SELECT id FROM {s}.views WHERE building AND build_deadline<$1"),
                        &[&now],
                    )
                    .await?;
                for r in &rows {
                    let id: String = r.get(0);
                    if let Some(mut v) = load(tx, s, &id).await? {
                        v.settle(now);
                        save(tx, s, &v).await?;
                    }
                }
                Ok(rows.len())
            })
        })
        .await
    }

    async fn start_refresh(&self, id: &str, now: DateTime<Utc>) -> Result<()> {
        let id = id.to_owned();
        self.with(move |tx, s| {
            Box::pin(async move {
                if let Some(mut v) = load(tx, s, &id).await? {
                    v.start_build(now);
                    save(tx, s, &v).await?;
                }
                Ok(())
            })
        })
        .await
    }

    async fn stage_pins(&self, owner: &str, keys: &[String], until: DateTime<Utc>) -> Result<Pin> {
        let (owner, keys) = (owner.to_owned(), keys.to_vec());
        self.with(move |tx, s| {
            Box::pin(async move {
                let blocked: i64 = tx
                    .query_one(&format!("SELECT count(*) FROM {s}.tiles WHERE collecting AND key=ANY($1)"), &[&keys])
                    .await?
                    .get(0);
                if blocked > 0 {
                    return Ok(Pin::Collecting);
                }
                tx.execute(&format!("DELETE FROM {s}.pins WHERE owner=$1"), &[&owner]).await?;
                tx.execute(
                    &format!("INSERT INTO {s}.pins(key,owner,until) SELECT k,$2,$3 FROM unnest($1::text[]) k ON CONFLICT(key,owner) DO UPDATE SET until=excluded.until"),
                    &[&keys, &owner, &until],
                )
                .await?;
                Ok(Pin::Staged)
            })
        })
        .await
    }

    async fn publish(&self, rev: &Revision, owner: &str, now: DateTime<Utc>) -> Result<()> {
        let (rev, owner) = (rev.clone(), owner.to_owned());
        self.with(move |tx, s| {
            Box::pin(async move {
                let Some(mut v) = load(tx, s, &rev.view_id).await? else {
                    anyhow::bail!("unknown view");
                };
                tx.execute(&format!("DELETE FROM {s}.pins WHERE owner=$1"), &[&owner]).await?;
                let pin_owner = format!("rev:{}:{}", rev.view_id, rev.revision);
                tx.execute(
                    &format!("INSERT INTO {s}.pins(key,owner,until) SELECT k,$2,$3 FROM unnest($1::text[]) k ON CONFLICT(key,owner) DO UPDATE SET until=excluded.until"),
                    &[&rev.tile_keys, &pin_owner, &rev.read_until],
                )
                .await?;
                tx.execute(
                    &format!("INSERT INTO {s}.revisions(view_id,revision,doc,expires_at,read_until) VALUES($1,$2,$3,$4,$5) ON CONFLICT(view_id,revision) DO UPDATE SET doc=excluded.doc,expires_at=excluded.expires_at,read_until=excluded.read_until"),
                    &[&rev.view_id, &rev.revision, &serde_json::to_string(&rev)?, &rev.expires_at, &rev.read_until],
                )
                .await?;
                v.mark_published(&rev, now);
                save(tx, s, &v).await?;
                Ok(())
            })
        })
        .await
    }

    async fn fail(&self, id: &str, reason: &str, owner: &str, now: DateTime<Utc>) -> Result<()> {
        let (id, reason, owner) = (id.to_owned(), reason.to_owned(), owner.to_owned());
        self.with(move |tx, s| {
            Box::pin(async move {
                tx.execute(&format!("DELETE FROM {s}.pins WHERE owner=$1"), &[&owner])
                    .await?;
                if let Some(mut v) = load(tx, s, &id).await? {
                    v.mark_failed(&reason, now);
                    save(tx, s, &v).await?;
                }
                Ok(())
            })
        })
        .await
    }

    async fn revoke(&self, id: &str, _reason: &str, now: DateTime<Utc>) -> Result<()> {
        let id = id.to_owned();
        self.with(move |tx, s| {
            Box::pin(async move {
                if let Some(mut v) = load(tx, s, &id).await? {
                    v.revoke(now);
                    save(tx, s, &v).await?;
                }
                Ok(())
            })
        })
        .await
    }

    async fn renew(
        &self,
        id: &str,
        revision: &str,
        expires_at: DateTime<Utc>,
        now: DateTime<Utc>,
    ) -> Result<bool> {
        let (id, revision) = (id.to_owned(), revision.to_owned());
        self.with(move |tx, s| {
            Box::pin(async move {
                let Some(mut v) = load(tx, s, &id).await? else { return Ok(false) };
                if !v.mark_renewed(&revision, expires_at, now) {
                    return Ok(false);
                }
                let Some(row) = tx
                    .query_opt(&format!("SELECT doc FROM {s}.revisions WHERE view_id=$1 AND revision=$2"), &[&id, &revision])
                    .await?
                else {
                    return Ok(false);
                };
                let mut rev: Revision = serde_json::from_str(&row.get::<_, String>(0))?;
                rev.expires_at = expires_at;
                rev.read_until = expires_at + TimeDelta::seconds(READ_GRACE_S);
                tx.execute(
                    &format!("UPDATE {s}.revisions SET doc=$3,expires_at=$4,read_until=$5 WHERE view_id=$1 AND revision=$2"),
                    &[&id, &revision, &serde_json::to_string(&rev)?, &rev.expires_at, &rev.read_until],
                )
                .await?;
                let owner = format!("rev:{id}:{revision}");
                tx.execute(&format!("UPDATE {s}.pins SET until=$2 WHERE owner=$1"), &[&owner, &rev.read_until]).await?;
                save(tx, s, &v).await?;
                Ok(true)
            })
        })
        .await
    }

    async fn revision(&self, id: &str, revision: &str) -> Result<Option<Revision>> {
        let pool = self.pool.lock().await;
        pool.client
            .query_opt(
                &format!(
                    "SELECT doc FROM {}.revisions WHERE view_id=$1 AND revision=$2",
                    self.schema
                ),
                &[&id, &revision],
            )
            .await?
            .map(|r| Ok(serde_json::from_str(&r.get::<_, String>(0))?))
            .transpose()
    }

    async fn tile_meta(&self, key: &str) -> Result<Option<TileMeta>> {
        let pool = self.pool.lock().await;
        Ok(pool
            .client
            .query_opt(
                &format!(
                    "SELECT sha256,bytes,transparent FROM {}.tiles WHERE key=$1 AND NOT collecting",
                    self.schema
                ),
                &[&key],
            )
            .await?
            .map(|r| TileMeta {
                sha256: r.get(0),
                bytes: r.get(1),
                transparent: r.get(2),
            }))
    }

    async fn tile_pin(&self, key: &str) -> Result<Option<DateTime<Utc>>> {
        let pool = self.pool.lock().await;
        let row = pool
            .client
            .query_one(
                &format!("SELECT max(until),bool_or(EXISTS(SELECT 1 FROM {s}.tiles t WHERE t.key=$1 AND t.collecting)) FROM {s}.pins WHERE key=$1", s = self.schema),
                &[&key],
            )
            .await?;
        let collecting: bool = row.get::<_, Option<bool>>(1).unwrap_or(false);
        Ok(if collecting { None } else { row.get(0) })
    }

    async fn collect(&self, now: DateTime<Utc>) -> Result<Vec<String>> {
        self.with(move |tx, s| {
            Box::pin(async move {
                tx.execute(&format!("DELETE FROM {s}.revisions WHERE read_until<$1"), &[&now]).await?;
                tx.execute(
                    &format!("DELETE FROM {s}.views v WHERE lease_until<$1 AND NOT EXISTS(SELECT 1 FROM {s}.revisions r WHERE r.view_id=v.id AND r.expires_at>=$1)"),
                    &[&now],
                )
                .await?;
                let dead = tx
                    .query(
                        &format!("SELECT DISTINCT p.key FROM {s}.pins p WHERE p.until<$1 AND NOT EXISTS(SELECT 1 FROM {s}.pins q WHERE q.key=p.key AND q.until>=$1)"),
                        &[&now],
                    )
                    .await?;
                let keys: Vec<String> = dead.iter().map(|r| r.get(0)).collect();
                tx.execute(&format!("UPDATE {s}.tiles SET collecting=true WHERE key=ANY($1)"), &[&keys]).await?;
                tx.execute(&format!("DELETE FROM {s}.pins WHERE key=ANY($1)"), &[&keys]).await?;
                Ok(keys)
            })
        })
        .await
    }

    async fn collected(&self, keys: &[String]) -> Result<()> {
        let keys = keys.to_vec();
        self.with(move |tx, s| {
            Box::pin(async move {
                tx.execute(
                    &format!("DELETE FROM {s}.tiles WHERE collecting AND key=ANY($1)"),
                    &[&keys],
                )
                .await?;
                tx.execute(
                    &format!("DELETE FROM {s}.objects WHERE id=ANY($1)"),
                    &[&keys],
                )
                .await?;
                Ok(())
            })
        })
        .await
    }
}
