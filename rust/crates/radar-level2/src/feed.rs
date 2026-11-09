use crate::{
    COMPRESSED_LIMIT,
    decode::{Decoder, Sweep},
    health::Health,
};
use anyhow::{Context, Result, ensure};
use chrono::{DateTime, Utc};
use serde::Deserialize;
use std::time::Duration;

pub const CHUNKS: &str = "https://unidata-nexrad-level2-chunks.s3.amazonaws.com";
pub const ARCHIVE: &str = "https://unidata-nexrad-level2.s3.amazonaws.com";
const LIST_LIMIT: usize = 1024 * 1024;

pub async fn bounded_body(mut response: reqwest::Response, limit: usize) -> Result<Vec<u8>> {
    ensure!(
        response.content_length().is_none_or(|n| n <= limit as u64),
        "HTTP content length limit"
    );
    let mut bytes = vec![];
    while let Some(chunk) = response.chunk().await? {
        ensure!(
            bytes
                .len()
                .checked_add(chunk.len())
                .is_some_and(|n| n <= limit),
            "HTTP streaming byte limit"
        );
        bytes.try_reserve_exact(chunk.len())?;
        bytes.extend_from_slice(&chunk);
    }
    Ok(bytes)
}

#[derive(Debug, Clone, Deserialize)]
pub struct Object {
    #[serde(rename = "Key")]
    pub key: String,
    #[serde(rename = "LastModified")]
    pub modified: DateTime<Utc>,
    #[serde(rename = "Size")]
    pub size: usize,
}
#[derive(Deserialize)]
struct Page {
    #[serde(rename = "Contents", default)]
    contents: Vec<Object>,
    #[serde(rename = "IsTruncated", default)]
    truncated: bool,
    #[serde(rename = "NextContinuationToken")]
    next: Option<String>,
}

#[derive(Clone)]
pub struct Feed {
    pub http: reqwest::Client,
    pub(crate) chunks_base: String,
    pub(crate) archive_base: String,
}

impl Feed {
    pub fn new() -> Result<Self> {
        Ok(Self {
            http: reqwest::Client::builder()
                .timeout(Duration::from_secs(10))
                .redirect(reqwest::redirect::Policy::none())
                .build()?,
            chunks_base: CHUNKS.into(),
            archive_base: ARCHIVE.into(),
        })
    }
    pub async fn list(&self, base: &str, prefix: &str) -> Result<Vec<Object>> {
        let mut token = None;
        let mut all = vec![];
        for _ in 0..4 {
            let mut request = self.http.get(base).query(&[
                ("list-type", "2"),
                ("prefix", prefix),
                ("max-keys", "1000"),
            ]);
            if let Some(t) = &token {
                request = request.query(&[("continuation-token", t)]);
            }
            let response = request.send().await?.error_for_status()?;
            let bytes = bounded_body(response, LIST_LIMIT).await?;
            let page: Page = quick_xml::de::from_reader(bytes.as_slice())?;
            ensure!(page.contents.len() <= 1000, "listing object count limit");
            ensure!(
                page.contents
                    .iter()
                    .all(|o| o.key.starts_with(prefix) && o.key.len() < 256),
                "invalid source listing"
            );
            all.extend(page.contents);
            if !page.truncated {
                return Ok(all);
            }
            let next = page.next.context("missing listing token")?;
            ensure!(token.as_ref() != Some(&next), "repeated listing token");
            token = Some(next);
        }
        anyhow::bail!("listing page limit")
    }
    pub async fn get(&self, base: &str, object: &Object) -> Result<Vec<u8>> {
        ensure!(object.size <= COMPRESSED_LIMIT, "source object byte limit");
        let bytes = bounded_body(
            self.http
                .get(format!("{base}/{}", object.key))
                .send()
                .await?
                .error_for_status()?,
            object.size,
        )
        .await?;
        ensure!(bytes.len() == object.size, "source size changed");
        Ok(bytes)
    }
    pub async fn archive_candidates(
        &self,
        station: &str,
        now: DateTime<Utc>,
    ) -> Result<Vec<Object>> {
        let mut objects = vec![];
        for day in [
            now.date_naive(),
            now.date_naive().pred_opt().context("date underflow")?,
        ] {
            let prefix = format!("{}/{station}/", day.format("%Y/%m/%d"));
            objects.extend(
                self.list(&self.archive_base, &prefix)
                    .await?
                    .into_iter()
                    .filter(|o| o.key.ends_with("_V06") || o.key.ends_with("_V08")),
            );
        }
        objects.sort_by(|a, b| b.key.cmp(&a.key));
        objects.truncate(2);
        Ok(objects)
    }
    pub async fn archive_header(&self, object: &Object) -> Result<Vec<u8>> {
        let resp = self
            .http
            .get(format!("{}/{}", self.archive_base, object.key))
            .header("Range", "bytes=0-23")
            .send()
            .await?;
        ensure!(
            resp.status() == reqwest::StatusCode::PARTIAL_CONTENT,
            "archive ignored range"
        );
        ensure!(
            resp.headers()
                .get("content-range")
                .and_then(|s| s.to_str().ok())
                .is_some_and(|s| s.starts_with("bytes 0-23/")),
            "archive wrong range"
        );
        let b = bounded_body(resp, 24).await?;
        ensure!(b.len() == 24, "short archive header");
        Ok(b)
    }
    pub async fn live_groups(&self, station: &str, volume: u16) -> Result<Vec<Vec<Object>>> {
        let mut groups = std::collections::BTreeMap::<String, Vec<Object>>::new();
        for offset in 0..3 {
            let number = (volume - 1 + offset) % 999 + 1;
            for o in self
                .list(&self.chunks_base, &format!("{station}/{number}/"))
                .await?
            {
                if let Some((stamp, _, _)) = chunk_identity(&o.key) {
                    groups.entry(stamp).or_default().push(o);
                }
            }
        }
        let mut result: Vec<_> = groups
            .into_iter()
            .rev()
            .take(3)
            .map(|(_, mut group)| {
                group.sort_by(|a, b| a.key.cmp(&b.key));
                group
            })
            .collect();
        result.retain(|group| {
            chunk_identity(&group[0].key).is_some_and(|(_, seq, kind)| seq == 1 && kind == 'S')
        });
        Ok(result)
    }
}

pub fn chunk_identity(key: &str) -> Option<(String, u16, char)> {
    let name = key.rsplit('/').next()?;
    let mut parts = name.split('-');
    let date = parts.next()?;
    let time = parts.next()?;
    let seq = parts.next()?;
    let kind = parts.next()?;
    if date.len() != 8
        || time.len() != 6
        || seq.len() != 3
        || !["S", "I", "E"].contains(&kind)
        || parts.next().is_some()
    {
        return None;
    }
    if !date
        .bytes()
        .chain(time.bytes())
        .chain(seq.bytes())
        .all(|b| b.is_ascii_digit())
    {
        return None;
    }
    Some((
        format!("{date}-{time}"),
        seq.parse().ok()?,
        kind.chars().next()?,
    ))
}

pub struct Acquisition {
    pub sweeps: Vec<Sweep>,
    pub sources: Vec<Source>,
}
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct Source {
    pub bucket: String,
    pub key: String,
    pub sha256: String,
    pub bytes: usize,
}

impl Feed {
    pub async fn chunks(
        &self,
        station: &str,
        group: &[Object],
        health: &mut Health,
        after: Option<DateTime<Utc>>,
    ) -> Result<Acquisition> {
        let mut decoder = Decoder::new(station);
        let mut sweeps = vec![];
        let mut sources = vec![];
        let mut total = 0;
        let stamp = chunk_identity(&group.first().context("empty chunks")?.key)
            .context("bad chunk")?
            .0;
        for (i, object) in group.iter().enumerate() {
            let (identity, seq, kind) = chunk_identity(&object.key).context("bad chunk key")?;
            ensure!(
                identity == stamp && usize::from(seq) == i + 1 && ((i == 0) == (kind == 'S')),
                "chunk sequence gap"
            );
            total += object.size;
            ensure!(total <= COMPRESSED_LIMIT, "chunk volume byte limit");
            let bytes = match self.get(&self.chunks_base, object).await {
                Ok(bytes) => bytes,
                Err(e) => {
                    health.access_failure = Some("chunk_get_failed".into());
                    return Err(e);
                }
            };
            health.observe(&object.key, object.modified, Utc::now());
            sources.push(Source {
                bucket: "unidata-nexrad-level2-chunks".into(),
                key: object.key.clone(),
                sha256: crate::checksum(&bytes),
                bytes: bytes.len(),
            });
            let result = tokio::task::spawn_blocking(move || {
                let completed = decoder.push(&bytes)?;
                Ok::<_, anyhow::Error>((decoder, completed))
            })
            .await?;
            let (next, completed) = match result {
                Ok(result) => result,
                Err(error) => {
                    health.processing_failure = Some(error.to_string());
                    return Err(error);
                }
            };
            decoder = next;
            sweeps.extend(
                completed
                    .into_iter()
                    .filter(|s| after.is_none_or(|t| s.scan_end > t)),
            );
            if !sweeps.is_empty() {
                break;
            }
        }
        ensure!(!sweeps.is_empty(), "low sweep still warming");
        health.processing_failure = None;
        Ok(Acquisition { sweeps, sources })
    }
    pub async fn archive(
        &self,
        station: &str,
        object: &Object,
        health: &mut Health,
    ) -> Result<Acquisition> {
        let bytes = match self.get(&self.archive_base, object).await {
            Ok(bytes) => bytes,
            Err(error) => {
                health.access_failure = Some("archive_get_failed".into());
                return Err(error);
            }
        };
        let station = station.to_owned();
        let result = tokio::task::spawn_blocking(move || {
            let mut decoder = Decoder::new(&station);
            let sweeps = decoder.push(&bytes)?;
            decoder.finish()?;
            Ok::<_, anyhow::Error>((sweeps, crate::checksum(&bytes), bytes.len()))
        })
        .await?;
        let (sweeps, sha256, size) = match result {
            Ok(result) => result,
            Err(error) => {
                health.processing_failure = Some(error.to_string());
                return Err(error);
            }
        };
        health.processing_failure = None;
        ensure!(!sweeps.is_empty(), "archive has no completed low sweep");
        Ok(Acquisition {
            sweeps,
            sources: vec![Source {
                bucket: "unidata-nexrad-level2".into(),
                key: object.key.clone(),
                sha256,
                bytes: size,
            }],
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn chunk_keys_and_sequence() {
        assert_eq!(
            chunk_identity("KGRR/471/20261109-025422-007-I"),
            Some(("20261109-025422".into(), 7, 'I'))
        );
        assert!(chunk_identity("KGRR/471/20261109-025422-007-X").is_none());
        assert!(chunk_identity("../../junk").is_none());
    }
    #[tokio::test]
    async fn bounded_http_with_and_without_content_length() {
        use axum::{Router, body::Body, response::Response, routing::get};
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let server = tokio::spawn(
            axum::serve(
                listener,
                Router::new()
                    .route("/", get(|| async { Response::new(Body::from("12345")) }))
                    .route(
                        "/stream",
                        get(|| async {
                            Response::new(Body::from_stream(futures_util::stream::iter([
                                Ok::<_, std::io::Error>("123"),
                                Ok("45"),
                            ])))
                        }),
                    ),
            )
            .into_future(),
        );
        let c = reqwest::Client::new();
        assert!(
            bounded_body(c.get(format!("http://{addr}/")).send().await.unwrap(), 4)
                .await
                .is_err()
        );
        assert_eq!(
            bounded_body(c.get(format!("http://{addr}/")).send().await.unwrap(), 5)
                .await
                .unwrap(),
            b"12345"
        );
        let response = c.get(format!("http://{addr}/stream")).send().await.unwrap();
        assert!(response.content_length().is_none());
        assert!(bounded_body(response, 4).await.is_err());
        server.abort();
    }
    #[tokio::test]
    async fn listing_pages_and_recovery_size_are_bounded() -> Result<()> {
        use axum::{Router, extract::Query, routing::get};
        async fn page(Query(q): Query<std::collections::HashMap<String, String>>) -> String {
            let n = q
                .get("continuation-token")
                .and_then(|n| n.parse::<usize>().ok())
                .unwrap_or(0);
            format!(
                "<ListBucketResult><IsTruncated>true</IsTruncated><NextContinuationToken>{}</NextContinuationToken></ListBucketResult>",
                n + 1
            )
        }
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
        let addr = listener.local_addr()?;
        let task =
            tokio::spawn(axum::serve(listener, Router::new().route("/", get(page))).into_future());
        let feed = Feed::new()?;
        assert!(
            feed.list(&format!("http://{addr}/"), "KGRR/")
                .await
                .unwrap_err()
                .to_string()
                .contains("listing page limit")
        );
        let object = Object {
            key: "too_large".into(),
            modified: Utc::now(),
            size: COMPRESSED_LIMIT + 1,
        };
        assert!(
            feed.get("http://127.0.0.1:1", &object)
                .await
                .unwrap_err()
                .to_string()
                .contains("source object byte limit")
        );
        task.abort();
        Ok(())
    }
}
