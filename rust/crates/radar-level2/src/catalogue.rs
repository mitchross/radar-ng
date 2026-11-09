use anyhow::{Result, ensure};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

pub const SOURCE: &str = "https://www.ncei.noaa.gov/access/homr/file/nexrad-stations.txt";
pub const SNAPSHOT: &str = include_str!("../testdata/nexrad-stations.txt");
pub const FEED_VERIFIED: &[&str] = &["KGRR", "KDTX", "KLOT", "KAPX"];

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Site {
    pub id: String,
    pub name: String,
    pub lat: f64,
    pub lon: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Catalogue {
    pub sites: BTreeMap<String, Site>,
    pub source: String,
    pub fetched_at: DateTime<Utc>,
    pub sha256: String,
    pub etag: Option<String>,
    pub last_modified: Option<String>,
}

impl Catalogue {
    pub fn parse(text: &str, fetched_at: DateTime<Utc>) -> Result<Self> {
        ensure!(
            text.len() <= 1024 * 1024 && text.is_ascii(),
            "catalogue size/encoding"
        );
        ensure!(
            text.lines()
                .next()
                .is_some_and(|s| s.starts_with("NCDCID   ICAO WBAN")),
            "catalogue schema"
        );
        let mut sites = BTreeMap::new();
        for line in text.lines().skip(2) {
            ensure!(line.len() >= 146, "short catalogue row");
            if line[140..].trim() != "NEXRAD" {
                continue;
            }
            let id = line[9..13].trim();
            ensure!(
                id.len() == 4
                    && id
                        .bytes()
                        .all(|b| b.is_ascii_uppercase() || b.is_ascii_digit()),
                "invalid station ID"
            );
            let site = Site {
                id: id.into(),
                name: line[20..50].trim().into(),
                lat: line[106..115].trim().parse()?,
                lon: line[116..126].trim().parse()?,
            };
            ensure!(
                site.lat.is_finite()
                    && site.lon.is_finite()
                    && (-90.0..=90.0).contains(&site.lat)
                    && (-180.0..=180.0).contains(&site.lon),
                "invalid coordinates"
            );
            ensure!(
                sites.insert(id.to_owned(), site).is_none(),
                "duplicate station"
            );
        }
        ensure!(
            sites.len() >= 150 && sites.len() <= 250,
            "implausible catalogue count"
        );
        for (id, lat, lon) in [
            ("KGRR", 42.893889, -85.544889),
            ("KDTX", 42.7, -83.471667),
            ("KLOT", 41.604444, -88.084444),
        ] {
            let s = sites
                .get(id)
                .ok_or_else(|| anyhow::anyhow!("missing sample {id}"))?;
            ensure!(
                (s.lat - lat).abs() < 0.01 && (s.lon - lon).abs() < 0.01,
                "sample moved: {id}"
            );
        }
        Ok(Self {
            sites,
            source: SOURCE.into(),
            fetched_at,
            sha256: crate::checksum(text.as_bytes()),
            etag: None,
            last_modified: None,
        })
    }

    pub fn snapshot() -> Result<Self> {
        Self::parse(SNAPSHOT, "2026-11-09T16:48:19.318495Z".parse()?)
    }

    pub async fn refresh(
        &mut self,
        http: &reqwest::Client,
        now: DateTime<Utc>,
    ) -> Result<Vec<String>> {
        self.refresh_from(http, SOURCE, now).await
    }

    async fn refresh_from(
        &mut self,
        http: &reqwest::Client,
        url: &str,
        now: DateTime<Utc>,
    ) -> Result<Vec<String>> {
        if (now - self.fetched_at).num_seconds() < 86400 {
            return Ok(vec![]);
        }
        let mut req = http.get(url);
        if let Some(e) = &self.etag {
            req = req.header("If-None-Match", e);
        }
        if let Some(e) = &self.last_modified {
            req = req.header("If-Modified-Since", e);
        }
        let resp = req.send().await?;
        if resp.status() == reqwest::StatusCode::NOT_MODIFIED {
            self.fetched_at = now;
            return Ok(vec![]);
        }
        let resp = resp.error_for_status()?;
        let etag = resp
            .headers()
            .get("etag")
            .and_then(|v| v.to_str().ok())
            .map(str::to_owned);
        let last_modified = resp
            .headers()
            .get("last-modified")
            .and_then(|v| v.to_str().ok())
            .map(str::to_owned);
        let bytes = crate::feed::bounded_body(resp, 1024 * 1024).await?;
        let mut next = Self::parse(std::str::from_utf8(&bytes)?, now)?;
        next.etag = etag;
        next.last_modified = last_modified;
        let mut diff = vec![];
        for id in self.sites.keys().chain(next.sites.keys()) {
            if self.sites.get(id) != next.sites.get(id) && !diff.contains(id) {
                diff.push(id.clone());
            }
        }
        *self = next;
        Ok(diff)
    }

    pub fn candidates(&self, lat: f64, lon: f64, enabled: &[String]) -> Result<Vec<&Site>> {
        ensure!(
            lat.is_finite()
                && lon.is_finite()
                && (-90.0..=90.0).contains(&lat)
                && (-180.0..=180.0).contains(&lon),
            "invalid centre"
        );
        ensure!(
            enabled
                .iter()
                .all(|id| FEED_VERIFIED.contains(&id.as_str()) && self.sites.contains_key(id)),
            "unverified feed allowlist"
        );
        let mut candidates: Vec<_> = enabled
            .iter()
            .filter_map(|id| self.sites.get(id))
            .map(|s| (distance_m(lat, lon, s.lat, s.lon), s))
            .filter(|(d, _)| in_range(*d))
            .collect();
        candidates.sort_by(|a, b| a.0.total_cmp(&b.0).then(a.1.id.cmp(&b.1.id)));
        Ok(candidates.into_iter().take(3).map(|(_, s)| s).collect())
    }
}

fn in_range(distance_m: f64) -> bool {
    distance_m <= 150000.0 + f64::EPSILON * 150000.0
}

pub fn distance_m(lat: f64, lon: f64, other_lat: f64, other_lon: f64) -> f64 {
    let a = ((other_lat - lat).to_radians() / 2.0).sin().powi(2)
        + lat.to_radians().cos()
            * other_lat.to_radians().cos()
            * ((other_lon - lon).to_radians() / 2.0).sin().powi(2);
    6371000.0 * 2.0 * a.clamp(0.0, 1.0).sqrt().asin()
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn snapshot_and_rejection_keep_last_good() {
        let c = Catalogue::snapshot().unwrap();
        assert_eq!(c.sites["KGRR"].name, "GRAND RAPIDS");
        assert!(Catalogue::parse("bad", Utc::now()).is_err());
        assert!(Catalogue::parse(&SNAPSHOT.replace("42.893889", "99.893889"), Utc::now()).is_err());
        let duplicate = format!(
            "{}\n{}",
            SNAPSHOT,
            SNAPSHOT.lines().find(|s| s.contains("KGRR")).unwrap()
        );
        assert!(Catalogue::parse(&duplicate, Utc::now()).is_err());
        assert!(c.candidates(42.89, -85.54, &["KGRR".into()]).unwrap().len() == 1);
        assert!(c.candidates(0.0, 0.0, &["KGRR".into()]).unwrap().is_empty());
        assert!(c.candidates(42.89, -85.54, &["PHKI".into()]).is_err());
        let delta = (150000.0 / 6371000.0_f64).to_degrees();
        assert!((distance_m(0.0, 0.0, delta, 0.0) - 150000.0).abs() < 1e-8);
    }
    #[tokio::test]
    async fn conditional_refresh_is_atomic_and_keeps_last_good() -> Result<()> {
        use axum::{
            Router,
            http::{HeaderMap, StatusCode},
            routing::get,
        };
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
        let addr = listener.local_addr()?;
        let server = tokio::spawn(
            axum::serve(
                listener,
                Router::new()
                    .route("/bad", get(|| async { "invalid" }))
                    .route(
                        "/unchanged",
                        get(|h: HeaderMap| async move {
                            assert_eq!(h["if-none-match"], "checked");
                            assert_eq!(h["if-modified-since"], "checked-time");
                            StatusCode::NOT_MODIFIED
                        }),
                    )
                    .route(
                        "/changed",
                        get(|| async { SNAPSHOT.replace("ABERDEEN", "NEW NAME") }),
                    ),
            )
            .into_future(),
        );
        let mut c = Catalogue::snapshot()?;
        c.etag = Some("checked".into());
        c.last_modified = Some("checked-time".into());
        let now = c.fetched_at + chrono::TimeDelta::days(1);
        let http = reqwest::Client::new();
        let before = c.sha256.clone();
        assert!(
            c.refresh_from(&http, &format!("http://{addr}/bad"), now)
                .await
                .is_err()
        );
        assert_eq!(c.sha256, before);
        assert!(c.fetched_at < now);
        assert!(
            c.refresh_from(&http, &format!("http://{addr}/unchanged"), now)
                .await?
                .is_empty()
        );
        assert_eq!(c.fetched_at, now);
        assert_eq!(c.sha256, before);
        assert_eq!(
            c.refresh_from(
                &http,
                &format!("http://{addr}/changed"),
                now + chrono::TimeDelta::days(1)
            )
            .await?,
            ["KABR"]
        );
        assert_ne!(c.sha256, before);
        server.abort();
        Ok(())
    }
    #[test]
    fn candidate_order_ties_three_attempts_and_range() -> Result<()> {
        let mut c = Catalogue::snapshot()?;
        for (id, lon) in [("KGRR", 0.0), ("KDTX", 0.0), ("KLOT", 0.1), ("KAPX", 0.2)] {
            let s = c.sites.get_mut(id).unwrap();
            s.lat = 0.0;
            s.lon = lon;
        }
        let enabled = vec!["KGRR".into(), "KDTX".into(), "KLOT".into(), "KAPX".into()];
        let candidates = c.candidates(0.0, 0.0, &enabled)?;
        assert_eq!(
            candidates.iter().map(|s| s.id.as_str()).collect::<Vec<_>>(),
            ["KDTX", "KGRR", "KLOT"]
        );
        assert!(in_range(150000.0));
        assert!(!in_range(150000.0 + 1e-6));
        let north_boundary = (150000.0 / 6371000.0_f64).to_degrees();
        assert_eq!(
            c.candidates(north_boundary, 0.0, &["KGRR".into()])?.len(),
            1
        );
        let north = (150000.0 / 6371000.0_f64).to_degrees();
        assert_eq!(c.candidates(north - 1e-8, 0.0, &["KGRR".into()])?.len(), 1);
        assert!(
            c.candidates(north + 1e-8, 0.0, &["KGRR".into()])?
                .is_empty()
        );
        Ok(())
    }
}
