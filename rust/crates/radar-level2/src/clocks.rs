//! Per-sweep pipeline clocks, written by the pool when it indexes a new lowest sweep. The shadow
//! recorder joins these with what it saw at the edge. Rows outlive the six indexed sweeps (45 days).

use crate::leases::Pool;
use anyhow::Result;
use chrono::{DateTime, Utc};
use serde::Serialize;

/// Days of clock rows kept; longer than the 30-day shadow run plus slack.
pub const KEEP_DAYS: i32 = 45;

/// One indexed lowest sweep. Field names are the column names (see docs/develop/level2-shadow.md).
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct SweepClock {
    pub station: String,
    pub sweep_id: String,
    pub volume_start: DateTime<Utc>,
    pub scan_start: DateTime<Utc>,
    pub scan_end: DateTime<Utc>,
    pub vcp: Option<i32>,
    pub elevation_number: i32,
    /// `live` (chunks bucket) or `archive` (recovery volume).
    pub source_kind: String,
    /// Newest NOAA `LastModified` among the objects that completed the sweep.
    pub source_arrival_at: Option<DateTime<Utc>>,
    /// When the verified artifact was indexed and usable by the publisher.
    pub first_available_at: DateTime<Utc>,
    /// QC time for this sweep alone, in milliseconds.
    pub qc_ms: f64,
    pub rays: i32,
    pub missing_sectors: i32,
    pub missing_deg: f64,
    pub source_bytes: i64,
    pub artifact_bytes: i64,
    /// Wall time of the poll that produced it, in milliseconds.
    pub poll_ms: f64,
}

/// Total degrees covered by missing sectors, each `[from, to]` clockwise.
pub fn missing_degrees(sectors: &[[f32; 2]]) -> f64 {
    sectors
        .iter()
        .map(|[a, b]| (f64::from(*b) - f64::from(*a)).rem_euclid(360.0))
        .sum()
}

const COLUMNS: &str = "station,sweep_id,volume_start,scan_start,scan_end,vcp,elevation_number,source_kind,source_arrival_at,first_available_at,qc_ms,rays,missing_sectors,missing_deg,source_bytes,artifact_bytes,poll_ms";

impl Pool {
    pub async fn initialize_clocks(&self) -> Result<()> {
        let s = self.schema();
        self.client.batch_execute(&format!("
            CREATE TABLE IF NOT EXISTS {s}.sweep_clocks(station text NOT NULL,sweep_id text NOT NULL,volume_start timestamptz NOT NULL,scan_start timestamptz NOT NULL,scan_end timestamptz NOT NULL,vcp integer,elevation_number integer NOT NULL,source_kind text NOT NULL,source_arrival_at timestamptz,first_available_at timestamptz NOT NULL,qc_ms double precision NOT NULL,rays integer NOT NULL,missing_sectors integer NOT NULL,missing_deg double precision NOT NULL,source_bytes bigint NOT NULL,artifact_bytes bigint NOT NULL,poll_ms double precision NOT NULL,PRIMARY KEY(station,sweep_id));
            CREATE INDEX IF NOT EXISTS sweep_clocks_end ON {s}.sweep_clocks(scan_end);
        ")).await?;
        Ok(())
    }

    /// Records the first indexing of a sweep; a repeat keeps the original clocks.
    pub async fn record_clock(&self, c: &SweepClock) -> Result<()> {
        let s = self.schema();
        self.client
            .execute(
                &format!("INSERT INTO {s}.sweep_clocks({COLUMNS}) VALUES($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12,$13,$14,$15,$16,$17) ON CONFLICT DO NOTHING"),
                &[
                    &c.station,
                    &c.sweep_id,
                    &c.volume_start,
                    &c.scan_start,
                    &c.scan_end,
                    &c.vcp,
                    &c.elevation_number,
                    &c.source_kind,
                    &c.source_arrival_at,
                    &c.first_available_at,
                    &c.qc_ms,
                    &c.rays,
                    &c.missing_sectors,
                    &c.missing_deg,
                    &c.source_bytes,
                    &c.artifact_bytes,
                    &c.poll_ms,
                ],
            )
            .await?;
        self.client
            .execute(
                &format!("DELETE FROM {s}.sweep_clocks WHERE scan_end < now() - make_interval(days => $1)"),
                &[&KEEP_DAYS],
            )
            .await?;
        Ok(())
    }

    /// Clock rows with `scan_end` in `[from, to)`, oldest first.
    pub async fn clocks(&self, from: DateTime<Utc>, to: DateTime<Utc>) -> Result<Vec<SweepClock>> {
        let s = self.schema();
        let rows = self
            .client
            .query(
                &format!("SELECT {COLUMNS} FROM {s}.sweep_clocks WHERE scan_end >= $1 AND scan_end < $2 ORDER BY station,scan_end"),
                &[&from, &to],
            )
            .await?;
        Ok(rows
            .iter()
            .map(|r| SweepClock {
                station: r.get(0),
                sweep_id: r.get(1),
                volume_start: r.get(2),
                scan_start: r.get(3),
                scan_end: r.get(4),
                vcp: r.get(5),
                elevation_number: r.get(6),
                source_kind: r.get(7),
                source_arrival_at: r.get(8),
                first_available_at: r.get(9),
                qc_ms: r.get(10),
                rays: r.get(11),
                missing_sectors: r.get(12),
                missing_deg: r.get(13),
                source_bytes: r.get(14),
                artifact_bytes: r.get(15),
                poll_ms: r.get(16),
            })
            .collect())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn missing_degrees_wraps_north() {
        assert_eq!(missing_degrees(&[]), 0.0);
        assert!((missing_degrees(&[[10.0, 13.0], [359.0, 1.0]]) - 5.0).abs() < 1e-9);
    }
}
