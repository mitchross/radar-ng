use crate::{HOT_LIMIT, JOB_RESERVATION, OBJECT_LIMIT, RENDER_JOBS};
use anyhow::{Result, ensure};
use tokio_postgres::{Client, NoTls};

#[derive(Debug, PartialEq, Eq)]
pub enum Reservation {
    Granted,
    Full,
    Duplicate,
}

pub struct Pool {
    pub client: Client,
    schema: String,
}

impl Pool {
    pub async fn connect(url: &str, schema: &str) -> Result<Self> {
        ensure!(
            schema.starts_with("level2_")
                && schema.len() <= 63
                && schema
                    .bytes()
                    .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_'),
            "invalid Level II schema"
        );
        let (client, connection) = tokio_postgres::connect(url, NoTls)
            .await
            .map_err(|_| anyhow::anyhow!("Level II database connection failed"))?;
        tokio::spawn(async move {
            let _ = connection.await;
        });
        Ok(Self {
            client,
            schema: schema.into(),
        })
    }
    pub fn schema(&self) -> &str {
        &self.schema
    }
    pub async fn initialize(&self) -> Result<()> {
        let s = &self.schema;
        self.client.batch_execute(&format!("
            CREATE SCHEMA IF NOT EXISTS {s};
            CREATE TABLE IF NOT EXISTS {s}.stations(station text PRIMARY KEY,demand_until timestamptz NOT NULL,busy boolean NOT NULL DEFAULT false);
            CREATE TABLE IF NOT EXISTS {s}.decodes(slot integer PRIMARY KEY CHECK(slot BETWEEN 1 AND 2),owner text,bytes bigint NOT NULL DEFAULT 0);
            INSERT INTO {s}.decodes(slot) VALUES(1),(2) ON CONFLICT DO NOTHING;
            CREATE TABLE IF NOT EXISTS {s}.objects(id text PRIMARY KEY,bytes bigint NOT NULL CHECK(bytes>0));
            CREATE TABLE IF NOT EXISTS {s}.sweeps(station text NOT NULL,scan_end timestamptz NOT NULL,identity text NOT NULL,commit_key text NOT NULL,indexed boolean NOT NULL DEFAULT true,PRIMARY KEY(station,scan_end));
            CREATE TABLE IF NOT EXISTS {s}.reservations(id text PRIMARY KEY,kind text NOT NULL CHECK(kind IN('sweep','render')),bytes bigint NOT NULL CHECK(bytes>0));
            CREATE TABLE IF NOT EXISTS {s}.renders(slot integer PRIMARY KEY CHECK(slot BETWEEN 1 AND 2),owner text);
            INSERT INTO {s}.renders(slot) VALUES(1),(2) ON CONFLICT DO NOTHING;
            CREATE TABLE IF NOT EXISTS {s}.tiles(key text PRIMARY KEY,sha256 text NOT NULL,bytes bigint NOT NULL,transparent boolean NOT NULL);
            CREATE TABLE IF NOT EXISTS {s}.health(station text PRIMARY KEY,document text NOT NULL);
            CREATE TABLE IF NOT EXISTS {s}.catalogue(id boolean PRIMARY KEY DEFAULT true CHECK(id),document text NOT NULL);
        ")).await?;
        Ok(())
    }
    pub async fn station(&mut self, station: &str) -> Result<bool> {
        let s = &self.schema;
        let tx = self.client.transaction().await?;
        tx.query_one("SELECT pg_advisory_xact_lock(288,1)", &[])
            .await?;
        let count:i64=tx.query_one(&format!("SELECT count(*) FROM {s}.stations WHERE demand_until>=clock_timestamp() OR busy"),&[]).await?.get(0);
        let existing = tx
            .query_opt(
                &format!(
                    "SELECT busy,demand_until>=clock_timestamp() FROM {s}.stations WHERE station=$1"
                ),
                &[&station],
            )
            .await?;
        if existing.as_ref().is_some_and(|r| r.get::<_, bool>(0)) {
            return Ok(false);
        }
        let active = existing.is_some_and(|r| r.get::<_, bool>(1));
        if !active && count >= 8 {
            return Ok(false);
        }
        tx.execute(&format!("INSERT INTO {s}.stations(station,demand_until,busy) VALUES($1,clock_timestamp()+interval '600 seconds',true) ON CONFLICT(station) DO UPDATE SET demand_until=excluded.demand_until,busy=true"),&[&station]).await?;
        tx.commit().await?;
        Ok(true)
    }
    pub async fn release_station(&self, station: &str) -> Result<()> {
        self.client
            .execute(
                &format!(
                    "UPDATE {}.stations SET busy=false WHERE station=$1",
                    self.schema
                ),
                &[&station],
            )
            .await?;
        Ok(())
    }
    /// Claims one of two decode slots if the shared 1 GiB sweep budget has room.
    pub async fn decode(&mut self) -> Result<Option<String>> {
        let owner = uuid::Uuid::new_v4().to_string();
        let s = &self.schema;
        let tx = self.client.transaction().await?;
        tx.query_one("SELECT pg_advisory_xact_lock(288,4)", &[])
            .await?;
        let used = hot_used(&tx, s).await?;
        if used + JOB_RESERVATION as i64 > HOT_LIMIT as i64 {
            return Ok(None);
        }
        let row=tx.query_opt(&format!("UPDATE {s}.decodes SET owner=$1,bytes=$2 WHERE slot=(SELECT slot FROM {s}.decodes WHERE owner IS NULL ORDER BY slot FOR UPDATE SKIP LOCKED LIMIT 1) RETURNING slot"),&[&owner,&(JOB_RESERVATION as i64)]).await?;
        tx.commit().await?;
        Ok(row.map(|_| owner))
    }
    pub async fn release_decode(&self, owner: &str) -> Result<()> {
        self.client
            .execute(
                &format!(
                    "UPDATE {}.decodes SET owner=NULL,bytes=0 WHERE owner=$1",
                    self.schema
                ),
                &[&owner],
            )
            .await?;
        Ok(())
    }
    /// Reserves `bytes` of the pool-wide sweep (1 GiB, shared with decodes) or render (512 MiB) budget.
    pub async fn reserve(
        &mut self,
        kind: &str,
        id: &str,
        bytes: i64,
        cap: i64,
    ) -> Result<Reservation> {
        ensure!(
            bytes > 0 && matches!(kind, "sweep" | "render"),
            "invalid reservation"
        );
        let s = &self.schema;
        let tx = self.client.transaction().await?;
        tx.query_one("SELECT pg_advisory_xact_lock(288,4)", &[])
            .await?;
        let used = if kind == "sweep" {
            hot_used(&tx, s).await?
        } else {
            tx.query_one(&format!("SELECT COALESCE(sum(bytes),0)::bigint FROM {s}.reservations WHERE kind='render'"), &[]).await?.get(0)
        };
        if used.checked_add(bytes).is_none_or(|n| n > cap) {
            return Ok(Reservation::Full);
        }
        let inserted = tx.execute(&format!("INSERT INTO {s}.reservations(id,kind,bytes) VALUES($1,$2,$3) ON CONFLICT DO NOTHING"), &[&id, &kind, &bytes]).await?;
        tx.commit().await?;
        Ok(if inserted == 1 {
            Reservation::Granted
        } else {
            Reservation::Duplicate
        })
    }
    /// Shrinks a reservation; growth must go through a new `reserve`.
    pub async fn shrink(&self, id: &str, bytes: i64) -> Result<()> {
        let n = self
            .client
            .execute(
                &format!(
                    "UPDATE {}.reservations SET bytes=$2 WHERE id=$1 AND bytes>=$2 AND $2>0",
                    self.schema
                ),
                &[&id, &bytes],
            )
            .await?;
        ensure!(n == 1, "reservation shrink");
        Ok(())
    }
    pub async fn release(&self, id: &str) -> Result<()> {
        self.client
            .execute(
                &format!("DELETE FROM {}.reservations WHERE id=$1", self.schema),
                &[&id],
            )
            .await?;
        Ok(())
    }
    pub async fn render_permit(&self) -> Result<Option<String>> {
        let owner = uuid::Uuid::new_v4().to_string();
        let s = &self.schema;
        let row=self.client.query_opt(&format!("UPDATE {s}.renders SET owner=$1 WHERE slot=(SELECT slot FROM {s}.renders WHERE owner IS NULL AND slot<=$2 ORDER BY slot FOR UPDATE SKIP LOCKED LIMIT 1) RETURNING slot"),&[&owner,&(RENDER_JOBS as i32)]).await?;
        Ok(row.map(|_| owner))
    }
    pub async fn release_render(&self, owner: &str) -> Result<()> {
        self.client
            .execute(
                &format!(
                    "UPDATE {}.renders SET owner=NULL WHERE owner=$1",
                    self.schema
                ),
                &[&owner],
            )
            .await?;
        Ok(())
    }
    pub async fn put_tile(
        &self,
        key: &str,
        sha256: &str,
        bytes: i64,
        transparent: bool,
    ) -> Result<()> {
        self.client
            .execute(
                &format!(
                    "INSERT INTO {}.tiles VALUES($1,$2,$3,$4) ON CONFLICT DO NOTHING",
                    self.schema
                ),
                &[&key, &sha256, &bytes, &transparent],
            )
            .await?;
        Ok(())
    }
    pub async fn tile(&self, key: &str) -> Result<Option<(String, i64, bool)>> {
        Ok(self
            .client
            .query_opt(
                &format!(
                    "SELECT sha256,bytes,transparent FROM {}.tiles WHERE key=$1",
                    self.schema
                ),
                &[&key],
            )
            .await?
            .map(|r| (r.get(0), r.get(1), r.get(2))))
    }
    pub async fn reserve_objects(&mut self, id: &str, bytes: i64) -> Result<()> {
        ensure!(bytes > 0 && bytes <= OBJECT_LIMIT, "object quota");
        let tx = self.client.transaction().await?;
        tx.query_one("SELECT pg_advisory_xact_lock(288,2)", &[])
            .await?;
        let used: i64 = tx
            .query_one(
                &format!(
                    "SELECT COALESCE(sum(bytes),0)::bigint FROM {}.objects",
                    self.schema
                ),
                &[],
            )
            .await?
            .get(0);
        ensure!(
            used.checked_add(bytes).is_some_and(|n| n <= OBJECT_LIMIT),
            "Level II object quota exhausted"
        );
        tx.execute(
            &format!(
                "INSERT INTO {}.objects(id,bytes) VALUES($1,$2) ON CONFLICT(id) DO NOTHING",
                self.schema
            ),
            &[&id, &bytes],
        )
        .await?;
        tx.commit().await?;
        Ok(())
    }
    pub async fn index(
        &mut self,
        station: &str,
        end: chrono::DateTime<chrono::Utc>,
        identity: &str,
        key: &str,
    ) -> Result<()> {
        let tx = self.client.transaction().await?;
        tx.query_one("SELECT pg_advisory_xact_lock(288,3)", &[])
            .await?;
        if let Some(row) = tx
            .query_opt(
                &format!(
                    "SELECT identity FROM {}.sweeps WHERE station=$1 AND scan_end=$2",
                    self.schema
                ),
                &[&station, &end],
            )
            .await?
        {
            ensure!(
                row.get::<_, String>(0) == identity,
                "same-station timestamp collision"
            );
            return Ok(());
        }
        tx.execute(
            &format!(
                "INSERT INTO {}.sweeps(station,scan_end,identity,commit_key) VALUES($1,$2,$3,$4)",
                self.schema
            ),
            &[&station, &end, &identity, &key],
        )
        .await?;
        tx.execute(&format!("UPDATE {}.sweeps SET indexed=false WHERE station=$1 AND scan_end NOT IN(SELECT scan_end FROM {}.sweeps WHERE station=$1 ORDER BY scan_end DESC LIMIT 6)",self.schema,self.schema),&[&station]).await?;
        tx.commit().await?;
        Ok(())
    }
    pub async fn health(&self, station: &str) -> Result<crate::health::Health> {
        Ok(self
            .client
            .query_opt(
                &format!(
                    "SELECT document FROM {}.health WHERE station=$1",
                    self.schema
                ),
                &[&station],
            )
            .await?
            .map(|r| serde_json::from_str(&r.get::<_, String>(0)))
            .transpose()?
            .unwrap_or_default())
    }
    pub async fn save_health(&self, station: &str, health: &crate::health::Health) -> Result<()> {
        self.client.execute(&format!("INSERT INTO {}.health VALUES($1,$2) ON CONFLICT(station) DO UPDATE SET document=excluded.document",self.schema),&[&station,&serde_json::to_string(health)?]).await?;
        Ok(())
    }
    pub async fn latest(
        &self,
        station: &str,
    ) -> Result<Option<(chrono::DateTime<chrono::Utc>, String, String)>> {
        Ok(self.client.query_opt(&format!("SELECT scan_end,identity,commit_key FROM {}.sweeps WHERE station=$1 AND indexed ORDER BY scan_end DESC LIMIT 1",self.schema),&[&station]).await?.map(|r|(r.get(0),r.get(1),r.get(2))))
    }
    pub async fn catalogue(&self) -> Result<Option<crate::catalogue::Catalogue>> {
        self.client
            .query_opt(
                &format!("SELECT document FROM {}.catalogue WHERE id", self.schema),
                &[],
            )
            .await?
            .map(|r| serde_json::from_str(&r.get::<_, String>(0)).map_err(Into::into))
            .transpose()
    }
    pub async fn save_catalogue(&self, c: &crate::catalogue::Catalogue) -> Result<()> {
        self.client.execute(&format!("INSERT INTO {}.catalogue(document) VALUES($1) ON CONFLICT(id) DO UPDATE SET document=excluded.document",self.schema),&[&serde_json::to_string(c)?]).await?;
        Ok(())
    }
}

async fn hot_used(tx: &tokio_postgres::Transaction<'_>, s: &str) -> Result<i64> {
    Ok(tx.query_one(&format!("SELECT ((SELECT COALESCE(sum(bytes),0) FROM {s}.decodes)+(SELECT COALESCE(sum(bytes),0) FROM {s}.reservations WHERE kind='sweep'))::bigint"), &[]).await?.get(0))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[tokio::test]
    async fn pool_caps_survive_replicas_and_detached_blocking_jobs() -> Result<()> {
        let Ok(url) = std::env::var("LEVEL2_TEST_PG") else {
            eprintln!("SKIP: LEVEL2_TEST_PG absent");
            return Ok(());
        };
        let schema = format!("level2_test_{}", uuid::Uuid::new_v4().simple());
        let mut a = Pool::connect(&url, &schema).await?;
        a.initialize().await?;
        let mut b = Pool::connect(&url, &schema).await?;
        for n in 0..8 {
            assert!(if n % 2 == 0 {
                a.station(&format!("S{n}")).await?
            } else {
                b.station(&format!("S{n}")).await?
            });
        }
        assert!(!a.station("overflow").await?);
        assert!(!b.station("S0").await?);
        a.release_station("S0").await?;
        assert!(b.station("S0").await?);
        b.release_station("S1").await?;
        b.client.execute(&format!("UPDATE {schema}.stations SET demand_until=clock_timestamp()-interval '1 second' WHERE station='S1'"),&[]).await?;
        assert!(b.station("replacement").await?);
        assert!(!b.station("S1").await?);
        let one = a.decode().await?.unwrap();
        let two = b.decode().await?.unwrap();
        assert!(a.decode().await?.is_none());
        let (send, recv) = std::sync::mpsc::channel();
        let task = tokio::spawn(async move {
            tokio::task::spawn_blocking(move || recv.recv().unwrap())
                .await
                .unwrap();
            a.release_decode(&one).await.unwrap();
        });
        assert!(
            tokio::time::timeout(std::time::Duration::from_millis(10), task)
                .await
                .is_err()
        );
        assert!(b.decode().await?.is_none());
        send.send(())?;
        tokio::time::timeout(std::time::Duration::from_secs(2), async {
            loop {
                if let Some(id) = b.decode().await? {
                    b.release_decode(&id).await?;
                    break;
                }
                tokio::task::yield_now().await;
            }
            Ok::<_, anyhow::Error>(())
        })
        .await??;
        b.release_decode(&two).await?;
        b.reserve_objects("all", OBJECT_LIMIT).await?;
        assert!(b.reserve_objects("overflow", 1).await.is_err());
        assert!(b.reserve_objects("zero", 0).await.is_err());
        let now = chrono::Utc::now();
        b.index("KGRR", now, "a", "a").await?;
        assert!(b.index("KGRR", now, "b", "b").await.is_err());
        b.index("KDTX", now, "b", "b").await?;
        for n in 1..8 {
            b.index(
                "KGRR",
                now + chrono::TimeDelta::seconds(n),
                &format!("id{n}"),
                "key",
            )
            .await?;
        }
        let indexed: i64 = b
            .client
            .query_one(
                &format!("SELECT count(*) FROM {schema}.sweeps WHERE station='KGRR' AND indexed"),
                &[],
            )
            .await?
            .get(0);
        assert_eq!(indexed, 6);

        Ok(())
    }
    #[tokio::test]
    async fn concurrent_replicas_cannot_over_admit() -> Result<()> {
        let Ok(url) = std::env::var("LEVEL2_TEST_PG") else {
            eprintln!("SKIP: LEVEL2_TEST_PG absent");
            return Ok(());
        };
        let schema = format!("level2_test_{}", uuid::Uuid::new_v4().simple());
        let pool = Pool::connect(&url, &schema).await?;
        pool.initialize().await?;
        let attempts = (0..12).map(|n| {
            let url = url.clone();
            let schema = schema.clone();
            async move {
                let mut pool = Pool::connect(&url, &schema).await?;
                pool.station(&format!("S{n}")).await
            }
        });
        let results = futures_util::future::join_all(attempts)
            .await
            .into_iter()
            .collect::<Result<Vec<_>>>()?;
        assert_eq!(results.into_iter().filter(|x| *x).count(), 8);
        let attempts = (0..12).map(|_| {
            let url = url.clone();
            let schema = schema.clone();
            async move { Pool::connect(&url, &schema).await?.decode().await }
        });
        let results = futures_util::future::join_all(attempts)
            .await
            .into_iter()
            .collect::<Result<Vec<_>>>()?;
        assert_eq!(results.into_iter().filter(Option::is_some).count(), 2);
        let bytes: i64 = pool
            .client
            .query_one(
                &format!("SELECT sum(bytes)::bigint FROM {schema}.decodes"),
                &[],
            )
            .await?
            .get(0);
        assert_eq!(bytes, crate::HOT_LIMIT as i64);
        Ok(())
    }
}
