//! Minimal S3 / RustFS object store client for RFC #389 Phase 2+ (`GRID_DIR` -> S3).
//!
//! Built directly on the workspace's existing `reqwest`, `hmac`, `sha2`, `hex`, `url` and
//! `quick-xml` crates so the backend adds zero new external dependencies.
//!
//! Opt-in via environment variables:
//! - `RADAR_S3_ENDPOINT`: S3-compatible endpoint (e.g. `http://rustfs.rustfs.svc.cluster.local:9000`).
//!   When unset, all helpers are no-ops and the backend uses `GRID_DIR` on disk as before.
//! - `RADAR_S3_BUCKET`: target bucket name (required when endpoint is set).
//! - `RADAR_S3_ACCESS_KEY_ID` (fallback `AWS_ACCESS_KEY_ID`)
//! - `RADAR_S3_SECRET_ACCESS_KEY` (fallback `AWS_SECRET_ACCESS_KEY`)
//! - `RADAR_S3_REGION`: signing region (default `"us-east-1"`).
//! - `RADAR_S3_PREFIX`: optional key prefix (default `""`).
//! - `RADAR_GRIDS_READ_S3`: `"1"` / `"true"` to serve grid reads, writes and cleanup from S3
//!   instead of `GRID_DIR` (default `false`, i.e. shadow dual-write mode).

use std::path::{Path, PathBuf};
use std::sync::{Arc, OnceLock};
use std::time::{Duration, SystemTime};

use anyhow::{Context, Result, bail, ensure};
use chrono::{DateTime, Utc};
use hmac::{Hmac, Mac};
use quick_xml::Reader;
use quick_xml::events::Event;
use reqwest::StatusCode;
use reqwest::header::{AUTHORIZATION, CONTENT_TYPE, HOST, HeaderMap, HeaderValue, RANGE};
use sha2::{Digest, Sha256};
use url::Url;

type HmacSha256 = Hmac<Sha256>;

/// Default S3 signing region for RustFS / MinIO.
pub const DEFAULT_REGION: &str = "us-east-1";
const REQUEST_TIMEOUT: Duration = Duration::from_secs(30);
const CONNECT_TIMEOUT: Duration = Duration::from_secs(10);

/// Configuration parsed from `RADAR_S3_*` / `RADAR_GRIDS_READ_S3` environment variables.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct S3Config {
    /// Endpoint URL without trailing slash (e.g. `http://rustfs:9000`).
    pub endpoint: String,
    /// Bucket name.
    pub bucket: String,
    /// S3 access key ID.
    pub access_key_id: String,
    /// S3 secret access key.
    pub secret_access_key: String,
    /// Signing region (default `"us-east-1"`).
    pub region: String,
    /// Optional key prefix without leading/trailing slash.
    pub prefix: String,
    /// When `true`, grid reads/writes/cleanup use S3 as authoritative (`RADAR_GRIDS_READ_S3=1`).
    pub read_grids_s3: bool,
    /// Local `GRID_DIR` root used to derive relative object keys from filesystem paths.
    pub grid_dir: PathBuf,
}

impl S3Config {
    /// Read from process environment. Returns `Ok(None)` when `RADAR_S3_ENDPOINT` is unset.
    pub fn from_env() -> Result<Option<Self>> {
        Self::from_lookup(|k| std::env::var(k).ok())
    }

    /// Read from a custom lookup function (used in tests).
    pub fn from_lookup<F>(mut get: F) -> Result<Option<Self>>
    where
        F: FnMut(&str) -> Option<String>,
    {
        let Some(endpoint) = get("RADAR_S3_ENDPOINT")
            .map(|s| s.trim().trim_end_matches('/').to_string())
            .filter(|s| !s.is_empty())
        else {
            return Ok(None);
        };
        let parsed = Url::parse(&endpoint).context("invalid RADAR_S3_ENDPOINT URL")?;
        ensure!(
            matches!(parsed.scheme(), "http" | "https") && parsed.host_str().is_some(),
            "RADAR_S3_ENDPOINT must be an http(s) URL with a host"
        );
        let bucket = get("RADAR_S3_BUCKET")
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
            .context("RADAR_S3_BUCKET must be set when RADAR_S3_ENDPOINT is set")?;
        let access_key_id = get("RADAR_S3_ACCESS_KEY_ID")
            .or_else(|| get("AWS_ACCESS_KEY_ID"))
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
            .context("RADAR_S3_ACCESS_KEY_ID (or AWS_ACCESS_KEY_ID) must be set")?;
        let secret_access_key = get("RADAR_S3_SECRET_ACCESS_KEY")
            .or_else(|| get("AWS_SECRET_ACCESS_KEY"))
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
            .context("RADAR_S3_SECRET_ACCESS_KEY (or AWS_SECRET_ACCESS_KEY) must be set")?;
        let region = get("RADAR_S3_REGION")
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
            .unwrap_or_else(|| DEFAULT_REGION.to_string());
        let prefix = get("RADAR_S3_PREFIX")
            .map(|s| s.trim().trim_matches('/').to_string())
            .unwrap_or_default();
        let read_grids_s3 = get("RADAR_GRIDS_READ_S3").is_some_and(|v| {
            matches!(
                v.trim().to_ascii_lowercase().as_str(),
                "1" | "true" | "yes" | "on"
            )
        });
        let grid_dir = get("GRID_DIR")
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
            .map_or_else(|| PathBuf::from("/data/grids"), PathBuf::from);
        Ok(Some(Self {
            endpoint,
            bucket,
            access_key_id,
            secret_access_key,
            region,
            prefix,
            read_grids_s3,
            grid_dir,
        }))
    }
}

/// Metadata for an object returned by [`S3Store::list_prefix`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ListedObject {
    /// Object key (with any configured `RADAR_S3_PREFIX` stripped).
    pub key: String,
    /// Size in bytes.
    pub size: u64,
    /// Last-modified timestamp, when present and parseable.
    pub last_modified: Option<SystemTime>,
}

/// Metadata for an object returned by [`S3Store::head`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ObjectMeta {
    /// Content-Length in bytes.
    pub size: u64,
}

/// Path-style S3 client with AWS Signature Version 4 authentication.
#[derive(Clone)]
pub struct S3Store {
    cfg: Arc<S3Config>,
    host_header: String,
    http: reqwest::Client,
    rt: Arc<OnceLock<tokio::runtime::Runtime>>,
}

impl S3Store {
    /// Build a new client from `cfg`.
    pub fn new(cfg: S3Config) -> Result<Self> {
        let parsed = Url::parse(&cfg.endpoint).context("invalid RADAR_S3_ENDPOINT")?;
        let host = parsed
            .host_str()
            .context("RADAR_S3_ENDPOINT missing host")?;
        let host_header = match parsed.port() {
            Some(port) => format!("{host}:{port}"),
            None => host.to_string(),
        };
        let http = reqwest::Client::builder()
            .connect_timeout(CONNECT_TIMEOUT)
            .timeout(REQUEST_TIMEOUT)
            .pool_max_idle_per_host(16)
            .build()
            .context("building S3 HTTP client")?;
        Ok(Self {
            cfg: Arc::new(cfg),
            host_header,
            http,
            rt: Arc::new(OnceLock::new()),
        })
    }

    fn blocking_rt(&self) -> Result<&tokio::runtime::Runtime> {
        if let Some(rt) = self.rt.get() {
            return Ok(rt);
        }
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .thread_name("radar-s3")
            .build()
            .context("building S3 runtime")?;
        let _ = self.rt.set(rt);
        Ok(self.rt.get().expect("OnceLock initialized"))
    }

    /// True when `RADAR_GRIDS_READ_S3=1`.
    #[must_use]
    pub fn read_grids_s3(&self) -> bool {
        self.cfg.read_grids_s3
    }

    /// Derive the S3 key `"grids/<rel>"` from a filesystem path under `GRID_DIR`.
    #[must_use]
    pub fn grid_key_for_path(&self, path: &Path) -> Option<String> {
        let rel = path.strip_prefix(&self.cfg.grid_dir).ok()?;
        let mut parts = Vec::new();
        for comp in rel.components() {
            let std::path::Component::Normal(s) = comp else {
                return None;
            };
            parts.push(s.to_str()?);
        }
        if parts.is_empty() {
            return None;
        }
        Some(format!("grids/{}", parts.join("/")))
    }

    fn full_key(&self, key: &str) -> String {
        let clean = key.trim_start_matches('/');
        if self.cfg.prefix.is_empty() {
            clean.to_string()
        } else {
            format!("{}/{clean}", self.cfg.prefix)
        }
    }

    fn strip_prefix(&self, full: &str) -> String {
        if self.cfg.prefix.is_empty() {
            full.to_string()
        } else {
            let with_slash = format!("{}/", self.cfg.prefix);
            full.strip_prefix(&with_slash).unwrap_or(full).to_string()
        }
    }

    fn object_canonical_uri(&self, key: &str) -> String {
        let full = self.full_key(key);
        let encoded_bucket = aws_uri_encode(&self.cfg.bucket, true);
        let encoded_key = aws_uri_encode(&full, false);
        format!("/{encoded_bucket}/{encoded_key}")
    }

    fn bucket_canonical_uri(&self) -> String {
        format!("/{}", aws_uri_encode(&self.cfg.bucket, true))
    }

    fn signed_headers(
        &self,
        method: &str,
        canonical_uri: &str,
        query: &[(&str, &str)],
        payload: &[u8],
        extra: &[(&str, &str)],
    ) -> Result<(String, HeaderMap)> {
        let now = Utc::now();
        let amz_date = now.format("%Y%m%dT%H%M%SZ").to_string();
        let date_stamp = now.format("%Y%m%d").to_string();
        let payload_hash = sha256_hex(payload);

        let mut sorted_query: Vec<(String, String)> = query
            .iter()
            .map(|(k, v)| (aws_uri_encode(k, true), aws_uri_encode(v, true)))
            .collect();
        sorted_query.sort();
        let canonical_querystring = sorted_query
            .iter()
            .map(|(k, v)| format!("{k}={v}"))
            .collect::<Vec<_>>()
            .join("&");

        let mut sign_headers: Vec<(String, String)> = vec![
            ("host".into(), self.host_header.clone()),
            ("x-amz-content-sha256".into(), payload_hash.clone()),
            ("x-amz-date".into(), amz_date.clone()),
        ];
        for (k, v) in extra {
            sign_headers.push((k.to_ascii_lowercase(), v.trim().to_string()));
        }
        sign_headers.sort_by(|a, b| a.0.cmp(&b.0));

        let canonical_headers: String = sign_headers
            .iter()
            .map(|(k, v)| format!("{k}:{v}\n"))
            .collect();
        let signed_headers_list = sign_headers
            .iter()
            .map(|(k, _)| k.as_str())
            .collect::<Vec<_>>()
            .join(";");

        let canonical_request = format!(
            "{method}\n{canonical_uri}\n{canonical_querystring}\n{canonical_headers}\n{signed_headers_list}\n{payload_hash}"
        );
        let credential_scope = format!("{date_stamp}/{}/s3/aws4_request", self.cfg.region);
        let string_to_sign = format!(
            "AWS4-HMAC-SHA256\n{amz_date}\n{credential_scope}\n{}",
            sha256_hex(canonical_request.as_bytes())
        );
        let signing_key =
            derive_signing_key(&self.cfg.secret_access_key, &date_stamp, &self.cfg.region);
        let signature = hex::encode(hmac_sha256(&signing_key, string_to_sign.as_bytes()));
        let auth = format!(
            "AWS4-HMAC-SHA256 Credential={}/{credential_scope}, SignedHeaders={signed_headers_list}, Signature={signature}",
            self.cfg.access_key_id
        );

        let mut headers = HeaderMap::new();
        headers.insert(HOST, HeaderValue::from_str(&self.host_header)?);
        headers.insert("x-amz-date", HeaderValue::from_str(&amz_date)?);
        headers.insert(
            "x-amz-content-sha256",
            HeaderValue::from_str(&payload_hash)?,
        );
        headers.insert(AUTHORIZATION, HeaderValue::from_str(&auth)?);
        for (k, v) in extra {
            if k.eq_ignore_ascii_case("content-type") {
                headers.insert(CONTENT_TYPE, HeaderValue::from_str(v)?);
            } else if k.eq_ignore_ascii_case("range") {
                headers.insert(RANGE, HeaderValue::from_str(v)?);
            }
        }

        let url = if canonical_querystring.is_empty() {
            format!("{}{canonical_uri}", self.cfg.endpoint)
        } else {
            format!(
                "{}{canonical_uri}?{canonical_querystring}",
                self.cfg.endpoint
            )
        };
        Ok((url, headers))
    }

    /// Upload `body` to `key`.
    pub async fn put(&self, key: &str, body: &[u8], content_type: &str) -> Result<()> {
        let uri = self.object_canonical_uri(key);
        let (url, headers) =
            self.signed_headers("PUT", &uri, &[], body, &[("content-type", content_type)])?;
        let resp = self
            .http
            .put(&url)
            .headers(headers)
            .body(body.to_vec())
            .send()
            .await
            .with_context(|| format!("S3 PUT {key}"))?;
        let status = resp.status();
        if !status.is_success() {
            let text = resp.text().await.unwrap_or_default();
            bail!("S3 PUT {key} failed with HTTP {status}: {text}");
        }
        Ok(())
    }

    /// Download `key`, returning `Ok(None)` on 404.
    pub async fn get(&self, key: &str) -> Result<Option<Vec<u8>>> {
        let uri = self.object_canonical_uri(key);
        let (url, headers) = self.signed_headers("GET", &uri, &[], &[], &[])?;
        let resp = self
            .http
            .get(&url)
            .headers(headers)
            .send()
            .await
            .with_context(|| format!("S3 GET {key}"))?;
        let status = resp.status();
        if status == StatusCode::NOT_FOUND {
            return Ok(None);
        }
        if !status.is_success() {
            let text = resp.text().await.unwrap_or_default();
            bail!("S3 GET {key} failed with HTTP {status}: {text}");
        }
        Ok(Some(resp.bytes().await?.to_vec()))
    }

    /// Download a byte range `[offset, offset + len)` of `key`, returning `Ok(None)` on 404.
    pub async fn get_range(&self, key: &str, offset: u64, len: usize) -> Result<Option<Vec<u8>>> {
        if len == 0 {
            return Ok(Some(Vec::new()));
        }
        let end = offset
            .checked_add(len as u64 - 1)
            .context("byte range overflow")?;
        let range = format!("bytes={offset}-{end}");
        let uri = self.object_canonical_uri(key);
        let (url, headers) = self.signed_headers("GET", &uri, &[], &[], &[("range", &range)])?;
        let resp = self
            .http
            .get(&url)
            .headers(headers)
            .send()
            .await
            .with_context(|| format!("S3 GET range {key} ({range})"))?;
        let status = resp.status();
        if status == StatusCode::NOT_FOUND {
            return Ok(None);
        }
        if status == StatusCode::RANGE_NOT_SATISFIABLE {
            bail!("S3 GET range {key} ({range}) out of range");
        }
        if !status.is_success() {
            let text = resp.text().await.unwrap_or_default();
            bail!("S3 GET range {key} failed with HTTP {status}: {text}");
        }
        let bytes = resp.bytes().await?.to_vec();
        if bytes.len() < len {
            bail!(
                "S3 GET range {key} short read: expected {len} bytes, got {}",
                bytes.len()
            );
        }
        Ok(Some(bytes[..len].to_vec()))
    }

    /// Check object existence and size via `HEAD`. Returns `Ok(None)` on 404.
    pub async fn head(&self, key: &str) -> Result<Option<ObjectMeta>> {
        let uri = self.object_canonical_uri(key);
        let (url, headers) = self.signed_headers("HEAD", &uri, &[], &[], &[])?;
        let resp = self
            .http
            .head(&url)
            .headers(headers)
            .send()
            .await
            .with_context(|| format!("S3 HEAD {key}"))?;
        let status = resp.status();
        if status == StatusCode::NOT_FOUND {
            return Ok(None);
        }
        if !status.is_success() {
            bail!("S3 HEAD {key} failed with HTTP {status}");
        }
        let size = resp
            .headers()
            .get(reqwest::header::CONTENT_LENGTH)
            .and_then(|v| v.to_str().ok())
            .and_then(|s| s.parse::<u64>().ok())
            .unwrap_or(0);
        Ok(Some(ObjectMeta { size }))
    }

    /// Delete `key`. Succeeds even when the key does not exist (S3 returns 204/404).
    pub async fn delete(&self, key: &str) -> Result<()> {
        let uri = self.object_canonical_uri(key);
        let (url, headers) = self.signed_headers("DELETE", &uri, &[], &[], &[])?;
        let resp = self
            .http
            .delete(&url)
            .headers(headers)
            .send()
            .await
            .with_context(|| format!("S3 DELETE {key}"))?;
        let status = resp.status();
        if status == StatusCode::NOT_FOUND || status.is_success() {
            return Ok(());
        }
        let text = resp.text().await.unwrap_or_default();
        bail!("S3 DELETE {key} failed with HTTP {status}: {text}");
    }

    /// List all objects under `prefix` using `ListObjectsV2` pagination.
    pub async fn list_prefix(&self, prefix: &str) -> Result<Vec<ListedObject>> {
        let full_prefix = self.full_key(prefix);
        let uri = self.bucket_canonical_uri();
        let mut out = Vec::new();
        let mut token: Option<String> = None;
        loop {
            let mut query: Vec<(&str, &str)> = vec![("list-type", "2"), ("prefix", &full_prefix)];
            if let Some(ref t) = token {
                query.push(("continuation-token", t));
            }
            let (url, headers) = self.signed_headers("GET", &uri, &query, &[], &[])?;
            let resp = self
                .http
                .get(&url)
                .headers(headers)
                .send()
                .await
                .with_context(|| format!("S3 ListObjectsV2 prefix={prefix}"))?;
            let status = resp.status();
            if !status.is_success() {
                let text = resp.text().await.unwrap_or_default();
                bail!("S3 ListObjectsV2 prefix={prefix} failed with HTTP {status}: {text}");
            }
            let xml = resp.text().await?;
            let page = parse_list_v2_xml(&xml)?;
            for mut item in page.objects {
                item.key = self.strip_prefix(&item.key);
                out.push(item);
            }
            if page.is_truncated && page.next_token.is_some() {
                token = page.next_token;
            } else {
                break;
            }
        }
        Ok(out)
    }

    /// Blocking wrapper around [`Self::put`].
    pub fn put_blocking(&self, key: &str, body: &[u8], content_type: &str) -> Result<()> {
        let rt = self.blocking_rt()?;
        std::thread::scope(|s| {
            s.spawn(|| rt.block_on(self.put(key, body, content_type)))
                .join()
                .map_err(|_| anyhow::anyhow!("s3 put thread panicked"))?
        })
    }

    /// Blocking wrapper around [`Self::get`].
    pub fn get_blocking(&self, key: &str) -> Result<Option<Vec<u8>>> {
        let rt = self.blocking_rt()?;
        std::thread::scope(|s| {
            s.spawn(|| rt.block_on(self.get(key)))
                .join()
                .map_err(|_| anyhow::anyhow!("s3 get thread panicked"))?
        })
    }

    /// Blocking wrapper around [`Self::get_range`].
    pub fn get_range_blocking(
        &self,
        key: &str,
        offset: u64,
        len: usize,
    ) -> Result<Option<Vec<u8>>> {
        let rt = self.blocking_rt()?;
        std::thread::scope(|s| {
            s.spawn(|| rt.block_on(self.get_range(key, offset, len)))
                .join()
                .map_err(|_| anyhow::anyhow!("s3 get_range thread panicked"))?
        })
    }

    /// Blocking wrapper around [`Self::head`].
    pub fn head_blocking(&self, key: &str) -> Result<Option<ObjectMeta>> {
        let rt = self.blocking_rt()?;
        std::thread::scope(|s| {
            s.spawn(|| rt.block_on(self.head(key)))
                .join()
                .map_err(|_| anyhow::anyhow!("s3 head thread panicked"))?
        })
    }

    /// Blocking wrapper around [`Self::delete`].
    pub fn delete_blocking(&self, key: &str) -> Result<()> {
        let rt = self.blocking_rt()?;
        std::thread::scope(|s| {
            s.spawn(|| rt.block_on(self.delete(key)))
                .join()
                .map_err(|_| anyhow::anyhow!("s3 delete thread panicked"))?
        })
    }

    /// Blocking wrapper around [`Self::list_prefix`].
    pub fn list_prefix_blocking(&self, prefix: &str) -> Result<Vec<ListedObject>> {
        let rt = self.blocking_rt()?;
        std::thread::scope(|s| {
            s.spawn(|| rt.block_on(self.list_prefix(prefix)))
                .join()
                .map_err(|_| anyhow::anyhow!("s3 list_prefix thread panicked"))?
        })
    }
}

struct ListPage {
    objects: Vec<ListedObject>,
    is_truncated: bool,
    next_token: Option<String>,
}

fn parse_list_v2_xml(xml: &str) -> Result<ListPage> {
    let mut reader = Reader::from_str(xml);
    let mut objects = Vec::new();
    let mut is_truncated = false;
    let mut next_token = None;

    let mut in_contents = false;
    let mut current_tag = String::new();
    let mut cur_key = String::new();
    let mut cur_size: u64 = 0;
    let mut cur_lm: Option<SystemTime> = None;

    loop {
        match reader.read_event() {
            Ok(Event::Start(e)) => {
                let name = String::from_utf8_lossy(e.local_name().as_ref()).into_owned();
                if name == "Contents" {
                    in_contents = true;
                    cur_key.clear();
                    cur_size = 0;
                    cur_lm = None;
                }
                current_tag = name;
            }
            Ok(Event::End(e)) => {
                let name = String::from_utf8_lossy(e.local_name().as_ref()).into_owned();
                if name == "Contents" && in_contents {
                    if !cur_key.is_empty() {
                        objects.push(ListedObject {
                            key: std::mem::take(&mut cur_key),
                            size: cur_size,
                            last_modified: cur_lm.take(),
                        });
                    }
                    in_contents = false;
                }
                current_tag.clear();
            }
            Ok(Event::Text(e)) => {
                let text = e.decode().context("decoding S3 XML text")?.into_owned();
                if in_contents {
                    match current_tag.as_str() {
                        "Key" => cur_key.push_str(&text),
                        "Size" => {
                            if let Ok(n) = text.trim().parse::<u64>() {
                                cur_size = n;
                            }
                        }
                        "LastModified" => {
                            if let Ok(dt) = DateTime::parse_from_rfc3339(text.trim()) {
                                cur_lm = Some(SystemTime::from(dt.with_timezone(&Utc)));
                            }
                        }
                        _ => {}
                    }
                } else {
                    match current_tag.as_str() {
                        "IsTruncated" => {
                            is_truncated = text.trim().eq_ignore_ascii_case("true");
                        }
                        "NextContinuationToken" => {
                            let t = text.trim();
                            if !t.is_empty() {
                                next_token = Some(t.to_string());
                            }
                        }
                        _ => {}
                    }
                }
            }
            Ok(Event::Eof) => break,
            Err(err) => bail!("invalid S3 ListObjectsV2 XML: {err}"),
            _ => {}
        }
    }
    Ok(ListPage {
        objects,
        is_truncated,
        next_token,
    })
}

fn sha256_hex(data: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(data);
    hex::encode(hasher.finalize())
}

fn hmac_sha256(key: &[u8], data: &[u8]) -> Vec<u8> {
    let mut mac = HmacSha256::new_from_slice(key).expect("HMAC accepts any key size");
    mac.update(data);
    mac.finalize().into_bytes().to_vec()
}

fn derive_signing_key(secret: &str, date_stamp: &str, region: &str) -> Vec<u8> {
    let k_date = hmac_sha256(format!("AWS4{secret}").as_bytes(), date_stamp.as_bytes());
    let k_region = hmac_sha256(&k_date, region.as_bytes());
    let k_service = hmac_sha256(&k_region, b"s3");
    hmac_sha256(&k_service, b"aws4_request")
}

/// AWS SigV4 URI-encoding (RFC 3986 unreserved characters stay literal; `/` stays literal when
/// `encode_slash` is `false`).
fn aws_uri_encode(input: &str, encode_slash: bool) -> String {
    let mut out = String::with_capacity(input.len());
    for byte in input.bytes() {
        let keep = byte.is_ascii_alphanumeric()
            || matches!(byte, b'-' | b'_' | b'.' | b'~')
            || (!encode_slash && byte == b'/');
        if keep {
            out.push(byte as char);
        } else {
            out.push('%');
            out.push_str(&format!("{byte:02X}"));
        }
    }
    out
}

static GLOBAL: OnceLock<Option<S3Store>> = OnceLock::new();

/// Process-wide S3 store initialized once from `RADAR_S3_*` env vars.
/// Returns `None` when `RADAR_S3_ENDPOINT` is unset.
pub fn global() -> Option<&'static S3Store> {
    GLOBAL
        .get_or_init(|| match S3Config::from_env() {
            Ok(Some(cfg)) => match S3Store::new(cfg) {
                Ok(store) => Some(store),
                Err(err) => {
                    tracing::error!(err = %err, "object_store_init_failed");
                    None
                }
            },
            Ok(None) => None,
            Err(err) => {
                tracing::error!(err = %err, "object_store_config_invalid");
                None
            }
        })
        .as_ref()
}

/// True when S3 is configured AND `RADAR_GRIDS_READ_S3=1`.
#[must_use]
pub fn read_grids_s3() -> bool {
    global().is_some_and(S3Store::read_grids_s3)
}

/// Best-effort shadow write of `body` to `key` when S3 is configured (`RADAR_GRIDS_READ_S3=0`).
pub fn mirror_put(key: &str, body: &[u8], content_type: &str) {
    let Some(store) = global() else {
        return;
    };
    if let Err(err) = store.put_blocking(key, body, content_type) {
        tracing::warn!(key, err = %format!("{err:#}"), "s3_mirror_put_failed");
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;
    use std::sync::Mutex;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::TcpListener;

    #[test]
    fn config_from_lookup_defaults_and_flags() {
        let empty: HashMap<&str, &str> = HashMap::new();
        assert_eq!(
            S3Config::from_lookup(|k| empty.get(k).map(|v| (*v).to_string())).unwrap(),
            None
        );

        let shadow = HashMap::from([
            ("RADAR_S3_ENDPOINT", "http://rustfs.local:9000/"),
            ("RADAR_S3_BUCKET", "radar-ng"),
            ("RADAR_S3_ACCESS_KEY_ID", "ak"),
            ("RADAR_S3_SECRET_ACCESS_KEY", "sk"),
        ]);
        let cfg = S3Config::from_lookup(|k| shadow.get(k).map(|v| (*v).to_string()))
            .unwrap()
            .unwrap();
        assert_eq!(cfg.endpoint, "http://rustfs.local:9000");
        assert_eq!(cfg.region, "us-east-1");
        assert!(!cfg.read_grids_s3);
        assert_eq!(cfg.grid_dir, PathBuf::from("/data/grids"));

        let active = HashMap::from([
            ("RADAR_S3_ENDPOINT", "https://s3.example.com"),
            ("RADAR_S3_BUCKET", "radar"),
            ("AWS_ACCESS_KEY_ID", "ak2"),
            ("AWS_SECRET_ACCESS_KEY", "sk2"),
            ("RADAR_S3_PREFIX", "/prod/"),
            ("RADAR_GRIDS_READ_S3", "1"),
            ("GRID_DIR", "/tmp/grids"),
        ]);
        let cfg = S3Config::from_lookup(|k| active.get(k).map(|v| (*v).to_string()))
            .unwrap()
            .unwrap();
        assert_eq!(cfg.prefix, "prod");
        assert!(cfg.read_grids_s3);
        assert_eq!(cfg.grid_dir, PathBuf::from("/tmp/grids"));
    }

    #[test]
    fn uri_encoding_preserves_slashes_in_keys_and_encodes_colons() {
        assert_eq!(
            aws_uri_encode("grids/radar/2026-11-26T18:00:00+00:00.meta.json", false),
            "grids/radar/2026-11-26T18%3A00%3A00%2B00%3A00.meta.json"
        );
        assert_eq!(aws_uri_encode("a/b", true), "a%2Fb");
    }

    #[test]
    fn parses_list_v2_xml_with_continuation_and_timestamps() {
        let xml = r#"<?xml version="1.0" encoding="UTF-8"?>
        <ListBucketResult xmlns="http://s3.amazonaws.com/doc/2006-03-01/">
            <IsTruncated>true</IsTruncated>
            <NextContinuationToken>tok-2</NextContinuationToken>
            <Contents>
                <Key>grids/radar/2026-11-26T18:00:00+00:00.meta.json</Key>
                <LastModified>2026-11-26T18:00:05.000Z</LastModified>
                <Size>212</Size>
            </Contents>
        </ListBucketResult>"#;
        let page = parse_list_v2_xml(xml).unwrap();
        assert!(page.is_truncated);
        assert_eq!(page.next_token.as_deref(), Some("tok-2"));
        assert_eq!(page.objects.len(), 1);
        assert_eq!(
            page.objects[0].key,
            "grids/radar/2026-11-26T18:00:00+00:00.meta.json"
        );
        assert_eq!(page.objects[0].size, 212);
        assert!(page.objects[0].last_modified.is_some());
    }

    #[test]
    fn grid_key_for_path_strips_grid_dir() {
        let store = S3Store::new(S3Config {
            endpoint: "http://127.0.0.1:9000".into(),
            bucket: "radar".into(),
            access_key_id: "ak".into(),
            secret_access_key: "sk".into(),
            region: "us-east-1".into(),
            prefix: String::new(),
            read_grids_s3: false,
            grid_dir: PathBuf::from("/data/grids"),
        })
        .unwrap();
        assert_eq!(
            store.grid_key_for_path(Path::new("/data/grids/radar/t.meta.json")),
            Some("grids/radar/t.meta.json".into())
        );
        assert_eq!(
            store.grid_key_for_path(Path::new("/other/t.meta.json")),
            None
        );
    }

    #[tokio::test]
    async fn round_trip_put_get_range_head_list_delete_against_mock_s3() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let objects: Arc<Mutex<HashMap<String, Vec<u8>>>> = Arc::new(Mutex::new(HashMap::new()));
        let store_map = Arc::clone(&objects);
        tokio::spawn(async move {
            while let Ok((mut sock, _)) = listener.accept().await {
                let map = Arc::clone(&store_map);
                tokio::spawn(async move {
                    let mut buf = vec![0u8; 65536];
                    let n = sock.read(&mut buf).await.unwrap_or(0);
                    let req = &buf[..n];
                    let header_end = req
                        .windows(4)
                        .position(|w| w == b"\r\n\r\n")
                        .map(|i| i + 4)
                        .unwrap_or(n);
                    let head_str = String::from_utf8_lossy(&req[..header_end]);
                    let mut lines = head_str.lines();
                    let first = lines.next().unwrap_or("");
                    let mut parts = first.split_whitespace();
                    let method = parts.next().unwrap_or("");
                    let target = parts.next().unwrap_or("");
                    let (path, query) = target.split_once('?').unwrap_or((target, ""));
                    let mut range_hdr: Option<String> = None;
                    let mut has_auth = false;
                    for line in lines {
                        if let Some((k, v)) = line.split_once(':') {
                            if k.eq_ignore_ascii_case("range") {
                                range_hdr = Some(v.trim().to_string());
                            } else if k.eq_ignore_ascii_case("authorization")
                                && v.trim().starts_with("AWS4-HMAC-SHA256 ")
                            {
                                has_auth = true;
                            }
                        }
                    }
                    assert!(has_auth, "every S3 request must carry SigV4 Authorization");
                    let body = &req[header_end..];
                    let key = path.strip_prefix("/radar/").unwrap_or("");

                    let (status, resp_body): (&str, Vec<u8>) = match method {
                        "PUT" => {
                            map.lock().unwrap().insert(key.to_string(), body.to_vec());
                            ("200 OK", Vec::new())
                        }
                        "HEAD" => {
                            let len = map.lock().unwrap().get(key).map(Vec::len);
                            if let Some(len) = len {
                                let head = format!(
                                    "HTTP/1.1 200 OK\r\nContent-Length: {len}\r\nConnection: close\r\n\r\n"
                                );
                                let _ = sock.write_all(head.as_bytes()).await;
                                return;
                            }
                            ("404 Not Found", Vec::new())
                        }
                        "GET" if query.contains("list-type=2") => {
                            let guard = map.lock().unwrap();
                            let mut xml =
                                String::from("<ListBucketResult><IsTruncated>false</IsTruncated>");
                            for (k, v) in guard.iter() {
                                xml.push_str(&format!(
                                    "<Contents><Key>{k}</Key><Size>{}</Size><LastModified>2026-11-26T18:00:00Z</LastModified></Contents>",
                                    v.len()
                                ));
                            }
                            xml.push_str("</ListBucketResult>");
                            ("200 OK", xml.into_bytes())
                        }
                        "GET" => {
                            if let Some(v) = map.lock().unwrap().get(key).cloned() {
                                if let Some(r) = range_hdr
                                    .and_then(|s| s.strip_prefix("bytes=").map(str::to_string))
                                    && let Some((a, b)) = r.split_once('-')
                                {
                                    let a: usize = a.parse().unwrap();
                                    let b: usize = b.parse().unwrap();
                                    ("206 Partial Content", v[a..=b].to_vec())
                                } else {
                                    ("200 OK", v)
                                }
                            } else {
                                ("404 Not Found", Vec::new())
                            }
                        }
                        "DELETE" => {
                            map.lock().unwrap().remove(key);
                            ("204 No Content", Vec::new())
                        }
                        _ => ("405 Method Not Allowed", Vec::new()),
                    };
                    let head = format!(
                        "HTTP/1.1 {status}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                        resp_body.len()
                    );
                    let _ = sock.write_all(head.as_bytes()).await;
                    let _ = sock.write_all(&resp_body).await;
                });
            }
        });

        let client = S3Store::new(S3Config {
            endpoint: format!("http://{addr}"),
            bucket: "radar".into(),
            access_key_id: "test-ak".into(),
            secret_access_key: "test-sk".into(),
            region: "us-east-1".into(),
            prefix: String::new(),
            read_grids_s3: true,
            grid_dir: PathBuf::from("/data/grids"),
        })
        .unwrap();

        let payload = b"0123456789abcdef";
        client
            .put(
                "grids/radar/sample.bin",
                payload,
                "application/octet-stream",
            )
            .await
            .unwrap();
        assert_eq!(
            client.head("grids/radar/sample.bin").await.unwrap(),
            Some(ObjectMeta { size: 16 })
        );
        assert_eq!(
            client
                .get("grids/radar/sample.bin")
                .await
                .unwrap()
                .as_deref(),
            Some(payload.as_slice())
        );
        assert_eq!(
            client
                .get_range("grids/radar/sample.bin", 4, 4)
                .await
                .unwrap()
                .as_deref(),
            Some(b"4567".as_slice())
        );
        let listed = client.list_prefix("grids/radar/").await.unwrap();
        assert_eq!(listed.len(), 1);
        assert_eq!(listed[0].key, "grids/radar/sample.bin");

        client.delete("grids/radar/sample.bin").await.unwrap();
        assert_eq!(client.head("grids/radar/sample.bin").await.unwrap(), None);
        assert_eq!(client.get("grids/radar/sample.bin").await.unwrap(), None);
    }
}
