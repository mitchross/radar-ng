//! Pool operations for the isolated worker: demand polling, dead-process recovery and idle eviction.

use crate::leases::Pool;
use anyhow::Result;
use serde::Serialize;

/// A decode, render or station lease older than this belongs to a process that is gone.
pub const STALE_LEASE_S: i64 = 900;
/// A cache reservation is reaped once its owner stopped beating for this long.
pub const INSTANCE_STALE_S: i64 = 120;
/// A station is evicted this long after its last demand expired.
pub const IDLE_EVICT_S: i64 = 1800;

#[derive(Debug, Default, PartialEq, Eq, Serialize)]
pub struct Reaped {
    pub stations: u64,
    pub decodes: u64,
    pub renders: u64,
    pub reservations: u64,
}

#[derive(Debug, Default, Serialize)]
pub struct Usage {
    pub active_stations: i64,
    pub busy_stations: i64,
    pub decodes_running: i64,
    pub sweep_bytes: i64,
    pub render_bytes: i64,
    pub object_bytes: i64,
}

/// What an eviction pass found idle: claimed for deletion, or kept because a live pin names it.
#[derive(Debug, Default)]
pub struct Idle {
    pub claimed: Vec<String>,
    pub pinned: Vec<String>,
}

impl Pool {
    /// Stations with live tile demand, at most the pool-wide eight.
    pub async fn demanded(&self) -> Result<Vec<String>> {
        let rows = self
            .client
            .query(
                &format!(
                    "SELECT station FROM {}.stations WHERE demand_until>=clock_timestamp() ORDER BY station LIMIT 8",
                    self.schema()
                ),
                &[],
            )
            .await?;
        Ok(rows.iter().map(|r| r.get(0)).collect())
    }

    /// Marks a demanded station busy for one poll without extending its demand, so polling alone never keeps it active.
    pub async fn claim_poll(&mut self, station: &str) -> Result<bool> {
        let s = self.schema().to_owned();
        let tx = self.client.transaction().await?;
        tx.query_one("SELECT pg_advisory_xact_lock(288,1)", &[])
            .await?;
        let claimed = tx
            .execute(
                &format!("UPDATE {s}.stations SET busy=true,busy_since=clock_timestamp() WHERE station=$1 AND demand_until>=clock_timestamp() AND NOT busy"),
                &[&station],
            )
            .await?;
        tx.commit().await?;
        Ok(claimed == 1)
    }

    /// Records that `instance` is alive.
    pub async fn beat(&self, instance: &str) -> Result<()> {
        self.client
            .execute(
                &format!(
                    "INSERT INTO {}.instances(id,seen) VALUES($1,clock_timestamp()) ON CONFLICT(id) DO UPDATE SET seen=excluded.seen",
                    self.schema()
                ),
                &[&instance],
            )
            .await?;
        Ok(())
    }

    /// Frees leases and cache reservations that dead processes left behind.
    pub async fn reap(&mut self) -> Result<Reaped> {
        let s = self.schema().to_owned();
        let tx = self.client.transaction().await?;
        tx.query_one("SELECT pg_advisory_xact_lock(288,1)", &[])
            .await?;
        tx.query_one("SELECT pg_advisory_xact_lock(288,4)", &[])
            .await?;
        let stale = STALE_LEASE_S as f64;
        let quiet = INSTANCE_STALE_S as f64;
        let stations = tx
            .execute(
                &format!("UPDATE {s}.stations SET busy=false,busy_since=NULL WHERE busy AND (busy_since IS NULL OR busy_since<clock_timestamp()-make_interval(secs=>$1))"),
                &[&stale],
            )
            .await?;
        let decodes = tx
            .execute(
                &format!("UPDATE {s}.decodes SET owner=NULL,bytes=0,leased_at=NULL WHERE owner IS NOT NULL AND (leased_at IS NULL OR leased_at<clock_timestamp()-make_interval(secs=>$1))"),
                &[&stale],
            )
            .await?;
        let renders = tx
            .execute(
                &format!("UPDATE {s}.renders SET owner=NULL,leased_at=NULL WHERE owner IS NOT NULL AND (leased_at IS NULL OR leased_at<clock_timestamp()-make_interval(secs=>$1))"),
                &[&stale],
            )
            .await?;
        let reservations = tx
            .execute(
                &format!("DELETE FROM {s}.reservations r WHERE split_part(r.id,':',1) IN ('tile','sweep') AND r.created_at<clock_timestamp()-make_interval(secs=>$1) AND NOT EXISTS(SELECT 1 FROM {s}.instances i WHERE i.id=split_part(r.id,':',2) AND i.seen>=clock_timestamp()-make_interval(secs=>$1))"),
                &[&quiet],
            )
            .await?;
        tx.execute(
            &format!("DELETE FROM {s}.instances WHERE seen<clock_timestamp()-interval '1 day'"),
            &[],
        )
        .await?;
        tx.commit().await?;
        Ok(Reaped {
            stations,
            decodes,
            renders,
            reservations,
        })
    }

    /// Stations idle past the retention window. Pinned ones stay; the rest are claimed so no poll starts on them.
    pub async fn idle_stations(&mut self) -> Result<Idle> {
        let s = self.schema().to_owned();
        let tx = self.client.transaction().await?;
        tx.query_one("SELECT pg_advisory_xact_lock(288,1)", &[])
            .await?;
        let retention = IDLE_EVICT_S as f64;
        let rows = tx
            .query(
                &format!("SELECT st.station,EXISTS(SELECT 1 FROM {s}.pins p WHERE p.until>=clock_timestamp() AND position('/tiles/'||st.station||'/' in p.key)>0) FROM {s}.stations st WHERE NOT st.busy AND st.demand_until<clock_timestamp()-make_interval(secs=>$1)"),
                &[&retention],
            )
            .await?;
        let mut idle = Idle::default();
        for r in rows {
            if r.get::<_, bool>(1) {
                idle.pinned.push(r.get(0));
            } else {
                idle.claimed.push(r.get(0));
            }
        }
        if !idle.claimed.is_empty() {
            tx.execute(
                &format!("UPDATE {s}.stations SET busy=true,busy_since=clock_timestamp() WHERE station=ANY($1)"),
                &[&idle.claimed],
            )
            .await?;
        }
        tx.commit().await?;
        Ok(idle)
    }

    /// Forgets an evicted station's sweeps, object reservations and demand row.
    pub async fn finish_evict(&mut self, station: &str, namespace: &str) -> Result<()> {
        let s = self.schema().to_owned();
        let like = format!("{namespace}/sweeps/{station}/%");
        let tiles = format!("{namespace}/tiles/{station}/%");
        let tx = self.client.transaction().await?;
        tx.query_one("SELECT pg_advisory_xact_lock(288,1)", &[])
            .await?;
        tx.query_one("SELECT pg_advisory_xact_lock(288,2)", &[])
            .await?;
        tx.query_one("SELECT pg_advisory_xact_lock(288,3)", &[])
            .await?;
        tx.execute(
            &format!("DELETE FROM {s}.sweeps WHERE station=$1"),
            &[&station],
        )
        .await?;
        tx.execute(
            &format!("DELETE FROM {s}.objects WHERE id LIKE $1 OR id LIKE $2"),
            &[&like, &tiles],
        )
        .await?;
        tx.execute(
            &format!("DELETE FROM {s}.tiles WHERE key LIKE $1"),
            &[&tiles],
        )
        .await?;
        tx.execute(
            &format!("DELETE FROM {s}.stations WHERE station=$1"),
            &[&station],
        )
        .await?;
        tx.commit().await?;
        Ok(())
    }

    /// Sweeps past the six-sweep window that no live pin references, as `(scan_end, commit_key)`.
    pub async fn prunable(
        &self,
        station: &str,
    ) -> Result<Vec<(chrono::DateTime<chrono::Utc>, String)>> {
        let s = self.schema();
        let rows = self
            .client
            .query(
                &format!("SELECT w.scan_end,w.commit_key FROM {s}.sweeps w WHERE w.station=$1 AND NOT w.indexed AND NOT EXISTS(SELECT 1 FROM {s}.pins p WHERE p.until>=clock_timestamp() AND position('/'||w.identity||'/' in p.key)>0) ORDER BY w.scan_end"),
                &[&station],
            )
            .await?;
        Ok(rows.iter().map(|r| (r.get(0), r.get(1))).collect())
    }

    /// Drops one pruned sweep's index row and object accounting after its objects are deleted.
    pub async fn forget_sweep(
        &mut self,
        station: &str,
        scan_end: chrono::DateTime<chrono::Utc>,
        prefix: &str,
    ) -> Result<()> {
        let s = self.schema().to_owned();
        let like = format!("{prefix}%");
        let tx = self.client.transaction().await?;
        tx.query_one("SELECT pg_advisory_xact_lock(288,3)", &[])
            .await?;
        tx.execute(
            &format!("DELETE FROM {s}.sweeps WHERE station=$1 AND scan_end=$2 AND NOT indexed"),
            &[&station, &scan_end],
        )
        .await?;
        tx.execute(
            &format!("DELETE FROM {s}.objects WHERE id LIKE $1"),
            &[&like],
        )
        .await?;
        tx.commit().await?;
        Ok(())
    }

    /// Releases the eviction claim without forgetting anything, after a failed delete.
    pub async fn abort_evict(&self, station: &str) -> Result<()> {
        self.release_station(station).await
    }

    pub async fn usage(&self) -> Result<Usage> {
        let s = self.schema();
        let row = self
            .client
            .query_one(
                &format!("SELECT (SELECT count(*) FROM {s}.stations WHERE demand_until>=clock_timestamp()),(SELECT count(*) FROM {s}.stations WHERE busy),(SELECT count(*) FROM {s}.decodes WHERE owner IS NOT NULL),(SELECT COALESCE(sum(bytes),0)::bigint FROM {s}.reservations WHERE kind='sweep')+(SELECT COALESCE(sum(bytes),0)::bigint FROM {s}.decodes),(SELECT COALESCE(sum(bytes),0)::bigint FROM {s}.reservations WHERE kind='render'),(SELECT COALESCE(sum(bytes),0)::bigint FROM {s}.objects)"),
                &[],
            )
            .await?;
        Ok(Usage {
            active_stations: row.get(0),
            busy_stations: row.get(1),
            decodes_running: row.get(2),
            sweep_bytes: row.get(3),
            render_bytes: row.get(4),
            object_bytes: row.get(5),
        })
    }
}
